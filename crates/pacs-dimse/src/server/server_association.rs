use std::collections::VecDeque;

use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_data::DataSet;
use dicom_toolkit_net::{
    dimse,
    pdu::{
        self, AAbort, AssociateAc, Pdu, Pdv, PresentationContextAcItem, PresentationContextRqItem,
    },
    AssociationConfig, PcResult, PresentationContextAc,
};
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    time::{timeout, Duration},
};

const APP_CONTEXT_UID: &str = "1.2.840.10008.3.1.1.1";
const TS_IMPLICIT_VR_LE: &str = "1.2.840.10008.1.2";
const TS_EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";
const PDU_OVERHEAD: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerAssociationState {
    Established,
    ReleaseRequested,
    Closed,
}

/// Server-side association wrapper used by pacsnode's DIMSE listener.
///
/// This preserves every PDV from inbound `P-DATA-TF` PDUs instead of dropping
/// all but the first one, which keeps fo-dicom clients interoperable when they
/// coalesce a DIMSE command and its dataset into one PDU.
pub(crate) struct ServerAssociation {
    stream: TcpStream,
    state: ServerAssociationState,
    pending_pdvs: VecDeque<Pdv>,
    pub(crate) calling_ae: String,
    presentation_contexts: Vec<PresentationContextAc>,
    outbound_max_pdu_length: u32,
}

impl ServerAssociation {
    pub(crate) async fn accept(stream: TcpStream, config: &AssociationConfig) -> DcmResult<Self> {
        let mut stream = stream;

        let incoming = timeout(
            Duration::from_secs(config.dimse_timeout_secs),
            pdu::read_pdu(&mut stream),
        )
        .await
        .map_err(|_| DcmError::Timeout {
            seconds: config.dimse_timeout_secs,
        })??;

        let rq = match incoming {
            Pdu::AssociateRq(rq) => rq,
            _ => {
                return Err(DcmError::Other(
                    "expected A-ASSOCIATE-RQ as first PDU".into(),
                ))
            }
        };

        let mut accepted_pcs = Vec::new();
        let mut ac_items = Vec::new();

        for pc in &rq.presentation_contexts {
            let (result_byte, transfer_syntax) = negotiate_pc(pc, config);
            ac_items.push(PresentationContextAcItem {
                id: pc.id,
                result: result_byte,
                transfer_syntax: transfer_syntax.clone(),
            });

            if result_byte == 0 {
                accepted_pcs.push(PresentationContextAc {
                    id: pc.id,
                    result: PcResult::Acceptance,
                    transfer_syntax,
                    abstract_syntax: pc.abstract_syntax.clone(),
                });
            }
        }

        let ac = AssociateAc {
            called_ae_title: rq.called_ae_title.clone(),
            calling_ae_title: rq.calling_ae_title.clone(),
            application_context: if rq.application_context.is_empty() {
                APP_CONTEXT_UID.to_string()
            } else {
                rq.application_context.clone()
            },
            presentation_contexts: ac_items,
            max_pdu_length: config.max_pdu_length,
            implementation_class_uid: config.implementation_class_uid.clone(),
            implementation_version_name: config.implementation_version_name.clone(),
        };

        stream.write_all(&pdu::encode_associate_ac(&ac)).await?;

        Ok(Self {
            stream,
            state: ServerAssociationState::Established,
            pending_pdvs: VecDeque::new(),
            calling_ae: rq.calling_ae_title,
            presentation_contexts: accepted_pcs,
            outbound_max_pdu_length: rq.max_pdu_length,
        })
    }

    pub(crate) fn find_context(&self, abstract_syntax: &str) -> Option<&PresentationContextAc> {
        self.presentation_contexts
            .iter()
            .find(|pc| pc.result.is_accepted() && pc.abstract_syntax == abstract_syntax)
    }

    pub(crate) fn context_by_id(&self, id: u8) -> Option<&PresentationContextAc> {
        self.presentation_contexts.iter().find(|pc| pc.id == id)
    }

    pub(crate) async fn send_dimse_command(
        &mut self,
        context_id: u8,
        command: &DataSet,
    ) -> DcmResult<()> {
        let bytes = dimse::encode_command_dataset(command);
        self.send_pdata(context_id, &bytes, true, true).await
    }

    pub(crate) async fn send_dimse_data(&mut self, context_id: u8, data: &[u8]) -> DcmResult<()> {
        self.send_pdata(context_id, data, false, true).await
    }

    pub(crate) async fn recv_dimse_command(&mut self) -> DcmResult<(u8, DataSet)> {
        self.ensure_established()?;

        let mut all_data = Vec::new();
        let mut ctx_id = None;

        loop {
            let pdv = self.recv_pdv().await?;
            if !pdv.is_command() {
                return Err(DcmError::Other(
                    "received DIMSE data while waiting for a command dataset".into(),
                ));
            }

            ctx_id.get_or_insert(pdv.context_id);
            all_data.extend_from_slice(&pdv.data);

            if pdv.is_last() {
                break;
            }
        }

        let ds = dimse::decode_command_dataset(&all_data)?;
        Ok((ctx_id.unwrap_or(0), ds))
    }

    #[cfg(test)]
    pub(crate) async fn recv_dimse_data(&mut self) -> DcmResult<Vec<u8>> {
        self.ensure_established()?;

        let mut all_data = Vec::new();

        loop {
            let pdv = self.recv_pdv().await?;
            if pdv.is_command() {
                self.pending_pdvs.push_front(pdv);
                return Err(DcmError::Other(
                    "received a DIMSE command PDV while waiting for DIMSE data".into(),
                ));
            }

            all_data.extend_from_slice(&pdv.data);
            if pdv.is_last() {
                break;
            }
        }

        Ok(all_data)
    }

    pub(crate) async fn recv_optional_dimse_data(&mut self) -> DcmResult<Option<Vec<u8>>> {
        self.ensure_established()?;
        let mut all_data = Vec::new();
        let mut saw_data_pdv = false;

        loop {
            self.fill_pending_pdvs().await?;

            if self.pending_pdvs.front().is_some_and(Pdv::is_command) {
                return if saw_data_pdv {
                    Ok(Some(all_data))
                } else {
                    Ok(None)
                };
            }

            let Some(pdv) = self.pending_pdvs.pop_front() else {
                return if saw_data_pdv {
                    Ok(Some(all_data))
                } else {
                    Ok(None)
                };
            };

            saw_data_pdv = true;
            all_data.extend_from_slice(&pdv.data);
            if pdv.is_last() {
                return Ok(Some(all_data));
            }
        }
    }

    pub(crate) async fn release(&mut self) -> DcmResult<()> {
        if self.state != ServerAssociationState::Established {
            return Ok(());
        }

        self.state = ServerAssociationState::ReleaseRequested;
        self.stream.write_all(&pdu::encode_release_rq()).await?;

        let result = timeout(Duration::from_secs(30), pdu::read_pdu(&mut self.stream)).await;
        self.state = ServerAssociationState::Closed;

        match result {
            Ok(Ok(Pdu::ReleaseRp)) | Ok(Ok(_)) => Ok(()),
            Ok(Err(err)) => Err(err),
            Err(_) => Ok(()),
        }
    }

    pub(crate) async fn abort(&mut self) -> DcmResult<()> {
        let _ = self
            .stream
            .write_all(&pdu::encode_a_abort(&AAbort {
                source: 0,
                reason: 0,
            }))
            .await;
        self.state = ServerAssociationState::Closed;
        Ok(())
    }

    async fn send_pdata(
        &mut self,
        context_id: u8,
        data: &[u8],
        is_command: bool,
        is_last: bool,
    ) -> DcmResult<()> {
        self.ensure_established()?;

        let max_data = max_pdv_data_length(self.outbound_max_pdu_length, data.len());
        let send_empty = data.is_empty();
        let chunks: Vec<&[u8]> = if send_empty {
            vec![&[]]
        } else {
            data.chunks(max_data).collect()
        };

        let total_chunks = chunks.len();
        for (index, chunk) in chunks.iter().enumerate() {
            let last_fragment = is_last && index + 1 == total_chunks;
            // DICOM PS3.8 §9.3.1: bit 0 = command, bit 1 = last
            let mut msg_control = 0u8;
            if is_command {
                msg_control |= 0x01;
            }
            if last_fragment {
                msg_control |= 0x02;
            }

            let pdv = Pdv {
                context_id,
                msg_control,
                data: chunk.to_vec(),
            };
            self.stream
                .write_all(&pdu::encode_p_data_tf(&[pdv]))
                .await?;
        }

        Ok(())
    }

    async fn recv_pdv(&mut self) -> DcmResult<Pdv> {
        self.ensure_established()?;

        self.fill_pending_pdvs().await?;

        if let Some(pdv) = self.pending_pdvs.pop_front() {
            return Ok(pdv);
        }

        Err(DcmError::Other(
            "expected a P-DATA-TF PDU but none was available".into(),
        ))
    }

    async fn fill_pending_pdvs(&mut self) -> DcmResult<()> {
        self.ensure_established()?;
        if !self.pending_pdvs.is_empty() {
            return Ok(());
        }

        loop {
            match pdu::read_pdu(&mut self.stream).await? {
                Pdu::PDataTf(pd) => {
                    if pd.pdvs.is_empty() {
                        continue;
                    }

                    self.pending_pdvs.extend(pd.pdvs);
                    return Ok(());
                }
                Pdu::AAbort(abort) => {
                    self.state = ServerAssociationState::Closed;
                    return Err(DcmError::AssociationAborted {
                        abort_source: abort.source.to_string(),
                        reason: abort.reason.to_string(),
                    });
                }
                Pdu::ReleaseRq => {
                    let _ = self.stream.write_all(&pdu::encode_release_rp()).await;
                    self.state = ServerAssociationState::Closed;
                    return Err(DcmError::Other("association released by peer".into()));
                }
                _ => {}
            }
        }
    }

    fn ensure_established(&self) -> DcmResult<()> {
        if self.state != ServerAssociationState::Established {
            return Err(DcmError::Other(
                "operation requires an established association".into(),
            ));
        }

        Ok(())
    }
}

fn max_pdv_data_length(max_pdu_length: u32, data_len: usize) -> usize {
    if max_pdu_length == 0 {
        return data_len.max(1);
    }

    (max_pdu_length as usize)
        .saturating_sub(PDU_OVERHEAD)
        .max(1)
}

fn negotiate_pc(pc: &PresentationContextRqItem, config: &AssociationConfig) -> (u8, String) {
    if !config.accepted_abstract_syntaxes.is_empty()
        && !config
            .accepted_abstract_syntaxes
            .iter()
            .any(|accepted| accepted == &pc.abstract_syntax)
    {
        return (3, TS_IMPLICIT_VR_LE.to_string());
    }

    match choose_ts(&pc.transfer_syntaxes, config) {
        Some(transfer_syntax) => (0, transfer_syntax),
        None => (4, TS_IMPLICIT_VR_LE.to_string()),
    }
}

fn choose_ts(offered: &[String], config: &AssociationConfig) -> Option<String> {
    if config.accept_all_transfer_syntaxes {
        return choose_preferred_ts(offered, &config.preferred_transfer_syntaxes)
            .or_else(|| offered.first().cloned());
    }

    let allowed: Vec<&String> = if config.accepted_transfer_syntaxes.is_empty() {
        offered.iter().collect()
    } else {
        offered
            .iter()
            .filter(|ts| {
                config
                    .accepted_transfer_syntaxes
                    .iter()
                    .any(|allowed| allowed == *ts)
            })
            .collect()
    };

    if allowed.is_empty() {
        return None;
    }

    choose_preferred_ts_refs(&allowed, &config.preferred_transfer_syntaxes).or_else(|| {
        if config.accepted_transfer_syntaxes.is_empty() {
            choose_default_uncompressed_ts(&allowed)
        } else {
            allowed.first().map(|ts| (*ts).clone())
        }
    })
}

fn choose_preferred_ts(offered: &[String], preferred: &[String]) -> Option<String> {
    preferred.iter().find_map(|candidate| {
        offered
            .iter()
            .find(|offered_ts| *offered_ts == candidate)
            .cloned()
    })
}

fn choose_preferred_ts_refs(offered: &[&String], preferred: &[String]) -> Option<String> {
    preferred.iter().find_map(|candidate| {
        offered
            .iter()
            .find(|offered_ts| ***offered_ts == *candidate)
            .map(|ts| (*ts).clone())
    })
}

fn choose_default_uncompressed_ts(offered: &[&String]) -> Option<String> {
    for preferred in &[TS_EXPLICIT_VR_LE, TS_IMPLICIT_VR_LE] {
        if let Some(ts) = offered
            .iter()
            .find(|offered_ts| ***offered_ts == *preferred)
        {
            return Some((*ts).clone());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{max_pdv_data_length, ServerAssociation};
    use dicom_toolkit_data::{io::writer::DicomWriter, DataSet};
    use dicom_toolkit_dict::{tags, uid_registry::sop_class, Vr};
    use dicom_toolkit_net::{
        dimse,
        pdu::{self, AssociateRq, Pdu, Pdv, PresentationContextRqItem},
        AssociationConfig,
    };
    use tokio::{
        io::AsyncWriteExt,
        net::{TcpListener, TcpStream},
        sync::oneshot,
    };

    const TS_EXPLICIT_LE: &str = "1.2.840.10008.1.2.1";

    fn find_context() -> PresentationContextRqItem {
        PresentationContextRqItem {
            id: 1,
            abstract_syntax: sop_class::PATIENT_ROOT_QR_FIND.to_string(),
            transfer_syntaxes: vec![TS_EXPLICIT_LE.to_string()],
        }
    }

    fn associate_rq(max_pdu_length: u32) -> AssociateRq {
        AssociateRq {
            called_ae_title: "PACSNODE".into(),
            calling_ae_title: "FO-DICOM".into(),
            application_context: "1.2.840.10008.3.1.1.1".into(),
            presentation_contexts: vec![find_context()],
            max_pdu_length,
            implementation_class_uid: "1.2.826.0.1.3680043.8.498.1".into(),
            implementation_version_name: "FO-DICOM".into(),
        }
    }

    fn encode_query() -> Vec<u8> {
        let mut query = DataSet::new();
        query.set_string(tags::QUERY_RETRIEVE_LEVEL, Vr::CS, "STUDY");
        query.set_string(tags::PATIENT_ID, Vr::LO, "P001");

        let mut bytes = Vec::new();
        DicomWriter::new(&mut bytes)
            .write_dataset(&query, TS_EXPLICIT_LE)
            .expect("encode C-FIND identifier");
        bytes
    }

    fn find_command(command_data_set_type: u16) -> DataSet {
        let mut cmd = DataSet::new();
        cmd.set_uid(
            tags::AFFECTED_SOP_CLASS_UID,
            sop_class::PATIENT_ROOT_QR_FIND,
        );
        cmd.set_u16(tags::COMMAND_FIELD, 0x0020);
        cmd.set_u16(tags::MESSAGE_ID, 1);
        cmd.set_u16(tags::PRIORITY, 0);
        cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, command_data_set_type);
        cmd
    }

    fn echo_command() -> DataSet {
        let mut cmd = DataSet::new();
        cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.1.1");
        cmd.set_u16(tags::COMMAND_FIELD, 0x0030);
        cmd.set_u16(tags::MESSAGE_ID, 2);
        cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        cmd
    }

    async fn connect_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("listener addr");
        let client = tokio::spawn(async move { TcpStream::connect(addr).await.expect("connect") });
        let (server, _) = listener.accept().await.expect("accept");
        let client = client.await.expect("join client task");
        (server, client)
    }

    #[tokio::test]
    async fn preserves_query_dataset_when_command_and_data_share_one_pdu() {
        let (server_stream, mut client_stream) = connect_pair().await;
        let expected_query = encode_query();

        let (done_tx, done_rx) = oneshot::channel();
        tokio::spawn(async move {
            let mut assoc = ServerAssociation::accept(server_stream, &AssociationConfig::default())
                .await
                .expect("accept association");

            let (ctx_id, cmd) = assoc.recv_dimse_command().await.expect("receive command");
            assert_eq!(ctx_id, 1);
            assert_eq!(cmd.get_u16(tags::COMMAND_FIELD), Some(0x0020));

            let query_bytes = assoc.recv_dimse_data().await.expect("receive query data");
            done_tx.send(query_bytes).expect("send query bytes to test");
        });

        client_stream
            .write_all(&pdu::encode_associate_rq(&associate_rq(16_384)))
            .await
            .expect("send associate-rq");
        match pdu::read_pdu(&mut client_stream)
            .await
            .expect("read associate-ac")
        {
            Pdu::AssociateAc(_) => {}
            other => panic!("expected AssociateAc, got {other:?}"),
        }

        let cmd = find_command(0x0000);

        let pdus = pdu::encode_p_data_tf(&[
            Pdv {
                context_id: 1,
                msg_control: 0x03,
                data: dimse::encode_command_dataset(&cmd),
            },
            Pdv {
                context_id: 1,
                msg_control: 0x02,
                data: expected_query.clone(),
            },
        ]);
        client_stream
            .write_all(&pdus)
            .await
            .expect("send command and dataset in one PDU");

        let actual_query = done_rx.await.expect("server received query bytes");
        assert_eq!(actual_query, expected_query);
    }

    #[tokio::test]
    async fn optional_dimse_data_keeps_next_command_queued() {
        let (server_stream, mut client_stream) = connect_pair().await;

        let (done_tx, done_rx) = oneshot::channel();
        tokio::spawn(async move {
            let mut assoc = ServerAssociation::accept(server_stream, &AssociationConfig::default())
                .await
                .expect("accept association");

            let (ctx_id, find_cmd) = assoc.recv_dimse_command().await.expect("receive command");
            assert_eq!(ctx_id, 1);
            assert_eq!(find_cmd.get_u16(tags::COMMAND_FIELD), Some(0x0020));

            let query_bytes = assoc
                .recv_optional_dimse_data()
                .await
                .expect("receive optional query data");
            assert!(query_bytes.is_none());

            let (_, next_cmd) = assoc
                .recv_dimse_command()
                .await
                .expect("receive queued follow-up command");
            done_tx
                .send(next_cmd.get_u16(tags::COMMAND_FIELD))
                .expect("send command field to test");
        });

        client_stream
            .write_all(&pdu::encode_associate_rq(&associate_rq(16_384)))
            .await
            .expect("send associate-rq");
        match pdu::read_pdu(&mut client_stream)
            .await
            .expect("read associate-ac")
        {
            Pdu::AssociateAc(_) => {}
            other => panic!("expected AssociateAc, got {other:?}"),
        }

        let pdus = pdu::encode_p_data_tf(&[
            Pdv {
                context_id: 1,
                msg_control: 0x03,
                data: dimse::encode_command_dataset(&find_command(0x0101)),
            },
            Pdv {
                context_id: 1,
                msg_control: 0x03,
                data: dimse::encode_command_dataset(&echo_command()),
            },
        ]);
        client_stream
            .write_all(&pdus)
            .await
            .expect("send back-to-back DIMSE commands");

        assert_eq!(
            done_rx.await.expect("server processed next command"),
            Some(0x0030)
        );
    }

    #[tokio::test]
    async fn optional_dimse_data_tolerates_empty_data_pdv_before_next_command() {
        let (server_stream, mut client_stream) = connect_pair().await;

        let (done_tx, done_rx) = oneshot::channel();
        tokio::spawn(async move {
            let mut assoc = ServerAssociation::accept(server_stream, &AssociationConfig::default())
                .await
                .expect("accept association");

            let (_, store_cmd) = assoc.recv_dimse_command().await.expect("receive command");
            assert_eq!(store_cmd.get_u16(tags::COMMAND_FIELD), Some(0x0001));

            let data = assoc
                .recv_optional_dimse_data()
                .await
                .expect("receive optional store data");
            let (_, next_cmd) = assoc
                .recv_dimse_command()
                .await
                .expect("receive queued follow-up command");

            done_tx
                .send((data, next_cmd.get_u16(tags::COMMAND_FIELD)))
                .expect("send result to test");
        });

        client_stream
            .write_all(&pdu::encode_associate_rq(&associate_rq(16_384)))
            .await
            .expect("send associate-rq");
        match pdu::read_pdu(&mut client_stream)
            .await
            .expect("read associate-ac")
        {
            Pdu::AssociateAc(_) => {}
            other => panic!("expected AssociateAc, got {other:?}"),
        }

        let store_cmd = {
            let mut cmd = DataSet::new();
            cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, sop_class::CT_IMAGE_STORAGE);
            cmd.set_u16(tags::COMMAND_FIELD, 0x0001);
            cmd.set_u16(tags::MESSAGE_ID, 1);
            cmd.set_u16(tags::PRIORITY, 0);
            cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000);
            cmd.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, "1.2.3.4.5");
            cmd
        };

        let pdus = pdu::encode_p_data_tf(&[
            Pdv {
                context_id: 1,
                msg_control: 0x03,
                data: dimse::encode_command_dataset(&store_cmd),
            },
            Pdv {
                context_id: 1,
                msg_control: 0x00,
                data: Vec::new(),
            },
            Pdv {
                context_id: 1,
                msg_control: 0x03,
                data: dimse::encode_command_dataset(&echo_command()),
            },
        ]);
        client_stream
            .write_all(&pdus)
            .await
            .expect("send store command, empty data PDV, then next command");

        let (data, next_command_field) = done_rx.await.expect("server processed PDVs");
        assert_eq!(data, Some(Vec::new()));
        assert_eq!(next_command_field, Some(0x0030));
    }

    #[test]
    fn fragments_outbound_pdvs_using_requestor_limit() {
        assert_eq!(max_pdv_data_length(0, 128), 128);
        assert_eq!(max_pdv_data_length(16_384, 32_768), 16_372);
        assert_eq!(max_pdv_data_length(8, 64), 1);
    }
}
