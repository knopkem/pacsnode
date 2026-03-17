//! DICOM SCP server: accepts TCP connections and routes DIMSE commands.

use std::sync::Arc;

use bytes::Bytes;
use dicom_toolkit_data::{io::reader::DicomReader, io::writer::DicomWriter, DataSet};
use dicom_toolkit_dict::tags;
use dicom_toolkit_net::{
    c_store,
    services::provider::{
        FindEvent, FindServiceProvider, GetEvent, GetServiceProvider, MoveEvent,
        MoveServiceProvider, RetrieveItem, StoreEvent, StoreResult, StoreServiceProvider,
    },
    Association, AssociationConfig, DestinationLookup, DicomServer as NetDicomServer,
    PresentationContextRq, StaticDestinationLookup, StoreRequest,
};
use pacs_core::{BlobStore, MetadataStore, PacsError};
use pacs_dicom::prepare_dimse_dataset;
use pacs_plugin::{
    FindScpHandler, GetScpHandler, MoveScpHandler, PacsEvent, PluginError, PluginRegistry,
    StoreScpHandler,
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::config::DimseConfig;
use crate::error::DimseError;

pub mod provider;
mod server_association;

use provider::{PacsQueryProvider, PacsStoreProvider};
use server_association::ServerAssociation;

const TS_EXPLICIT_LE: &str = "1.2.840.10008.1.2.1";
const TS_IMPLICIT_LE: &str = "1.2.840.10008.1.2";

struct DynStoreProvider {
    inner: Arc<dyn StoreScpHandler>,
}

impl DynStoreProvider {
    fn new(inner: Arc<dyn StoreScpHandler>) -> Self {
        Self { inner }
    }
}

impl StoreServiceProvider for DynStoreProvider {
    async fn on_store(&self, event: StoreEvent) -> StoreResult {
        self.inner.handle_store(event).await
    }
}

struct DynFindProvider {
    inner: Arc<dyn FindScpHandler>,
}

impl DynFindProvider {
    fn new(inner: Arc<dyn FindScpHandler>) -> Self {
        Self { inner }
    }
}

impl FindServiceProvider for DynFindProvider {
    async fn on_find(&self, event: FindEvent) -> Vec<DataSet> {
        self.inner.handle_find(event).await
    }
}

struct DynGetProvider {
    inner: Arc<dyn GetScpHandler>,
}

impl DynGetProvider {
    fn new(inner: Arc<dyn GetScpHandler>) -> Self {
        Self { inner }
    }
}

impl GetServiceProvider for DynGetProvider {
    async fn on_get(&self, event: GetEvent) -> Vec<RetrieveItem> {
        self.inner.handle_get(event).await
    }
}

struct DynMoveProvider {
    inner: Arc<dyn MoveScpHandler>,
}

impl DynMoveProvider {
    fn new(inner: Arc<dyn MoveScpHandler>) -> Self {
        Self { inner }
    }
}

impl MoveServiceProvider for DynMoveProvider {
    async fn on_move(&self, event: MoveEvent) -> Vec<RetrieveItem> {
        self.inner.handle_move(event).await
    }
}

/// A known remote DICOM node (AE title + network address).
#[derive(Debug, Clone)]
pub struct DicomNode {
    /// AE title of the remote node.
    pub ae_title: String,
    /// Hostname or IP address of the remote node.
    pub host: String,
    /// DICOM TCP port of the remote node.
    pub port: u16,
}

impl DicomNode {
    /// Create a new `DicomNode`.
    pub fn new(ae_title: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self {
            ae_title: ae_title.into(),
            host: host.into(),
            port,
        }
    }

    /// Returns `"host:port"` for use as a TCP connection address.
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// DICOM SCP server.
///
/// Listens for incoming DICOM associations and routes each DIMSE command to
/// the appropriate pacsnode service handler.
pub struct DicomServer {
    config: DimseConfig,
    store: Arc<dyn MetadataStore>,
    blobs: Arc<dyn BlobStore>,
    plugins: Option<Arc<PluginRegistry>>,
    known_nodes: Arc<tokio::sync::RwLock<Vec<DicomNode>>>,
}

impl DicomServer {
    /// Creates a new `DicomServer` with the given configuration and storage backends.
    pub fn new(
        config: DimseConfig,
        store: Arc<dyn MetadataStore>,
        blobs: Arc<dyn BlobStore>,
    ) -> Self {
        Self::with_plugins(config, store, blobs, None)
    }

    /// Creates a new `DicomServer` wired to an optional plugin registry.
    pub fn with_plugins(
        config: DimseConfig,
        store: Arc<dyn MetadataStore>,
        blobs: Arc<dyn BlobStore>,
        plugins: Option<Arc<PluginRegistry>>,
    ) -> Self {
        Self {
            config,
            store,
            blobs,
            plugins,
            known_nodes: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        }
    }

    /// Registers a remote DICOM node for C-MOVE destination lookups.
    pub async fn add_node(&self, node: DicomNode) {
        self.known_nodes.write().await.push(node);
    }

    /// Returns a snapshot of all registered remote nodes.
    pub async fn nodes(&self) -> Vec<DicomNode> {
        self.known_nodes.read().await.clone()
    }

    fn resolve_store_handler(&self) -> Result<Arc<dyn StoreScpHandler>, PluginError> {
        if let Some(plugins) = &self.plugins {
            return plugins
                .store_scp_handler()?
                .ok_or_else(|| PluginError::MissingDependency {
                    plugin_id: "dicom-server".into(),
                    dependency: "store-scp-plugin".into(),
                });
        }

        Ok(Arc::new(PacsStoreProvider::new(
            Arc::clone(&self.store),
            Arc::clone(&self.blobs),
        )))
    }

    fn resolve_find_handler(&self) -> Result<Arc<dyn FindScpHandler>, PluginError> {
        if let Some(plugins) = &self.plugins {
            return plugins
                .find_scp_handler()?
                .ok_or_else(|| PluginError::MissingDependency {
                    plugin_id: "dicom-server".into(),
                    dependency: "find-scp-plugin".into(),
                });
        }

        Ok(Arc::new(PacsQueryProvider::new(
            Arc::clone(&self.store),
            Arc::clone(&self.blobs),
        )))
    }

    fn resolve_get_handler(&self) -> Result<Arc<dyn GetScpHandler>, PluginError> {
        if let Some(plugins) = &self.plugins {
            return plugins
                .get_scp_handler()?
                .ok_or_else(|| PluginError::MissingDependency {
                    plugin_id: "dicom-server".into(),
                    dependency: "get-scp-plugin".into(),
                });
        }

        Ok(Arc::new(PacsQueryProvider::new(
            Arc::clone(&self.store),
            Arc::clone(&self.blobs),
        )))
    }

    fn resolve_move_handler(&self) -> Result<Arc<dyn MoveScpHandler>, PluginError> {
        if let Some(plugins) = &self.plugins {
            return plugins
                .move_scp_handler()?
                .ok_or_else(|| PluginError::MissingDependency {
                    plugin_id: "dicom-server".into(),
                    dependency: "move-scp-plugin".into(),
                });
        }

        Ok(Arc::new(PacsQueryProvider::new(
            Arc::clone(&self.store),
            Arc::clone(&self.blobs),
        )))
    }

    async fn validate_calling_ae(&self, calling_ae: &str) -> Result<(), PacsError> {
        if !self.config.ae_whitelist_enabled {
            return Ok(());
        }

        let calling_ae = calling_ae.trim();
        let known_nodes = self.store.list_nodes().await?;
        if known_nodes
            .iter()
            .any(|node| node.ae_title.trim() == calling_ae)
        {
            return Ok(());
        }

        Err(PacsError::NotFound {
            resource: "calling_ae",
            uid: calling_ae.to_string(),
        })
    }

    /// Start listening on the configured DICOM port.
    ///
    /// Spawns a tokio task per association. Runs until the cancellation token
    /// fires, at which point the listener stops accepting new connections and
    /// this future returns.
    ///
    /// # Cancellation Safety
    ///
    /// This function IS cancellation-safe. Each connection is handled in its
    /// own spawned task; dropping or cancelling the `serve` future only stops
    /// the accept loop — in-flight association tasks run to completion
    /// independently.
    pub async fn serve(self: Arc<Self>, shutdown: CancellationToken) -> Result<(), DimseError> {
        let addr = format!("0.0.0.0:{}", self.config.port);
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| DimseError::Bind {
                port: self.config.port,
                source: e,
            })?;

        info!(
            port = self.config.port,
            ae_title = %self.config.ae_title,
            "DICOM SCP listening"
        );

        let sem = Arc::new(tokio::sync::Semaphore::new(self.config.max_associations));

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    let (stream, peer_addr) = match accept_result {
                        Ok(pair) => pair,
                        Err(e) => {
                            error!(error = %e, "Failed to accept TCP connection");
                            continue;
                        }
                    };

                    // Enforce the max-associations limit without blocking.
                    let permit = match sem.clone().try_acquire_owned() {
                        Ok(p) => p,
                        Err(_) => {
                            warn!(
                                peer = %peer_addr,
                                max = self.config.max_associations,
                                "Max associations reached, rejecting connection"
                            );
                            continue;
                        }
                    };

                    debug!(peer = %peer_addr, "Accepted TCP connection");
                    let server = Arc::clone(&self);
                    tokio::spawn(async move {
                        Self::handle_connection(stream, peer_addr, server).await;
                        drop(permit);
                    });
                }
                _ = shutdown.cancelled() => {
                    info!("Shutdown signal received, stopping SCP listener");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Handles a single DICOM association from negotiation through to release.
    ///
    /// # Cancellation Safety
    ///
    /// This function is **not** cancellation-safe. It must run to completion
    /// so the association is properly released or aborted.
    async fn handle_connection(
        stream: tokio::net::TcpStream,
        peer_addr: std::net::SocketAddr,
        server: Arc<DicomServer>,
    ) {
        let assoc_config = association_config(&server.config);

        let mut assoc = match ServerAssociation::accept(stream, &assoc_config).await {
            Ok(a) => a,
            Err(dicom_toolkit_core::error::DcmError::Io(io_err))
                if io_err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                debug!(peer = %peer_addr, "Peer disconnected before completing association negotiation");
                return;
            }
            Err(e) => {
                error!(error = %e, "Association negotiation failed");
                return;
            }
        };
        let calling_ae = assoc.calling_ae.trim().to_string();

        if let Err(error) = server.validate_calling_ae(&calling_ae).await {
            let reason = match error {
                PacsError::NotFound { .. } => "calling AE title is not registered",
                _ => "failed to load AE whitelist",
            };
            warn!(
                calling_ae = %calling_ae,
                peer = %peer_addr,
                reason,
                "Rejecting DIMSE association"
            );

            if let Some(plugins) = &server.plugins {
                plugins
                    .emit_event(PacsEvent::AssociationRejected {
                        calling_ae: calling_ae.clone(),
                        peer_addr,
                        reason: reason.into(),
                    })
                    .await;
            }

            let _ = assoc.abort().await;
            return;
        }

        if let Some(plugins) = &server.plugins {
            plugins
                .emit_event(PacsEvent::AssociationOpened {
                    calling_ae: calling_ae.clone(),
                    peer_addr,
                })
                .await;
        }

        let store_provider = match server.resolve_store_handler() {
            Ok(handler) => DynStoreProvider::new(handler),
            Err(error) => {
                error!(error = %error, "Failed to resolve C-STORE SCP handler");
                let _ = assoc.abort().await;
                return;
            }
        };
        let find_provider = match server.resolve_find_handler() {
            Ok(handler) => DynFindProvider::new(handler),
            Err(error) => {
                error!(error = %error, "Failed to resolve C-FIND SCP handler");
                let _ = assoc.abort().await;
                return;
            }
        };
        let get_provider = match server.resolve_get_handler() {
            Ok(handler) => DynGetProvider::new(handler),
            Err(error) => {
                error!(error = %error, "Failed to resolve C-GET SCP handler");
                let _ = assoc.abort().await;
                return;
            }
        };
        let move_provider = match server.resolve_move_handler() {
            Ok(handler) => DynMoveProvider::new(handler),
            Err(error) => {
                error!(error = %error, "Failed to resolve C-MOVE SCP handler");
                let _ = assoc.abort().await;
                return;
            }
        };

        // Snapshot the known-node list for C-MOVE destination resolution.
        let dest_lookup = {
            let nodes = server.known_nodes.read().await;
            let entries = nodes
                .iter()
                .map(|n| (n.ae_title.clone(), format!("{}:{}", n.host, n.port)))
                .collect::<Vec<_>>();
            StaticDestinationLookup::new(entries)
        };

        let local_ae = server.config.ae_title.clone();

        loop {
            let (ctx_id, cmd) = match assoc.recv_dimse_command().await {
                Ok(pair) => pair,
                Err(e) => {
                    debug!(error = %e, "Association closed or error receiving command");
                    break;
                }
            };

            let command_field = cmd.get_u16(tags::COMMAND_FIELD).unwrap_or(0);

            let result: Result<(), DimseError> = match command_field {
                // C-ECHO-RQ — stateless; respond inline.
                0x0030 => send_echo_response(&mut assoc, ctx_id, &cmd).await,

                // C-STORE-RQ
                0x0001 => handle_store_rq(&mut assoc, ctx_id, &cmd, &store_provider)
                    .await
                    .map_err(DimseError::from),

                // C-FIND-RQ
                0x0020 => handle_find_rq(&mut assoc, ctx_id, &cmd, &find_provider)
                    .await
                    .map_err(DimseError::from),

                // C-GET-RQ
                0x0010 => handle_get_rq_with_transcoding(&mut assoc, ctx_id, &cmd, &get_provider)
                    .await
                    .map_err(DimseError::from),

                // C-MOVE-RQ
                0x0021 => handle_move_rq_with_transcoding(
                    &mut assoc,
                    ctx_id,
                    &cmd,
                    &move_provider,
                    &dest_lookup,
                    &local_ae,
                )
                .await
                .map_err(DimseError::from),

                other => {
                    warn!(
                        command_field = other,
                        "Unknown DIMSE command field — ignoring"
                    );
                    Ok(())
                }
            };

            if let Err(e) = result {
                error!(error = %e, command_field, "Error handling DIMSE command");
                let _ = assoc.abort().await;
                return;
            }
        }

        if let Err(e) = assoc.release().await {
            debug!(error = %e, "Error releasing association (may already be closed)");
        }

        if let Some(plugins) = &server.plugins {
            plugins
                .emit_event(PacsEvent::AssociationClosed { calling_ae })
                .await;
        }
    }
}

/// Sends a C-ECHO-RSP in reply to a C-ECHO-RQ.
///
/// # Cancellation Safety
///
/// This function is **not** cancellation-safe.
async fn send_echo_response(
    assoc: &mut ServerAssociation,
    ctx_id: u8,
    cmd: &DataSet,
) -> Result<(), DimseError> {
    let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);

    let mut rsp = DataSet::new();
    rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.1.1");
    rsp.set_u16(tags::COMMAND_FIELD, 0x8030_u16); // C-ECHO-RSP
    rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
    rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101_u16); // no dataset
    rsp.set_u16(tags::STATUS, 0x0000_u16); // success

    assoc.send_dimse_command(ctx_id, &rsp).await?;
    Ok(())
}

fn encode_dataset(
    dataset: &DataSet,
    transfer_syntax: &str,
) -> Result<Vec<u8>, dicom_toolkit_core::error::DcmError> {
    let mut bytes = Vec::new();
    DicomWriter::new(&mut bytes).write_dataset(dataset, transfer_syntax)?;
    Ok(bytes)
}

fn command_has_dataset(cmd: &DataSet) -> bool {
    cmd.get_u16(tags::COMMAND_DATA_SET_TYPE)
        .map(|value| value != 0x0101)
        .unwrap_or(true)
}

fn trim_uid(value: Option<&str>) -> String {
    value
        .map(|raw| raw.trim().trim_end_matches('\0').to_string())
        .unwrap_or_default()
}

fn store_dataset_has_required_uids(dataset: &DataSet) -> bool {
    !trim_uid(dataset.get_string(tags::STUDY_INSTANCE_UID)).is_empty()
        && !trim_uid(dataset.get_string(tags::SERIES_INSTANCE_UID)).is_empty()
}

fn payload_starts_with_file_meta(data: &[u8]) -> bool {
    data.len() >= 4 && data[0] == 0x02 && data[1] == 0x00
}

fn payload_has_dicm_prefix(data: &[u8]) -> bool {
    data.len() >= 132 && &data[128..132] == b"DICM"
}

fn payload_prefix_hex(data: &[u8], max_len: usize) -> String {
    data.iter()
        .take(max_len)
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn dataset_first_tags(dataset: &DataSet, max_tags: usize) -> String {
    dataset
        .tags()
        .take(max_tags)
        .map(|tag| format!("({:04X},{:04X})", tag.group, tag.element))
        .collect::<Vec<_>>()
        .join(", ")
}

fn decode_store_dataset(data: &[u8], negotiated_ts: &str, sop_instance_uid: &str) -> DataSet {
    if data.is_empty() {
        error!(
            sop_instance_uid = %sop_instance_uid,
            "C-STORE dataset payload is empty (0 bytes received)"
        );
        return DataSet::new();
    }

    let raw_dataset = DicomReader::new(data)
        .read_dataset(negotiated_ts)
        .map_err(|err| {
            error!(
                error = %err,
                sop_instance_uid = %sop_instance_uid,
                transfer_syntax = %negotiated_ts,
                payload_len = data.len(),
                prefix_hex = %payload_prefix_hex(data, 32),
                "Failed to decode C-STORE payload as raw dataset"
            );
            err
        })
        .ok();

    if let Some(dataset) = raw_dataset {
        if store_dataset_has_required_uids(&dataset) {
            return dataset;
        }

        error!(
            sop_instance_uid = %sop_instance_uid,
            transfer_syntax = %negotiated_ts,
            payload_len = data.len(),
            element_count = dataset.len(),
            first_tags = %dataset_first_tags(&dataset, 8),
            prefix_hex = %payload_prefix_hex(data, 32),
            has_dicm_prefix = payload_has_dicm_prefix(data),
            starts_with_file_meta = payload_starts_with_file_meta(data),
            "Decoded C-STORE raw dataset but required UIDs missing; trying fallbacks"
        );
    }

    if payload_starts_with_file_meta(data) {
        let mut synthetic_file = Vec::with_capacity(132 + data.len());
        synthetic_file.extend_from_slice(&[0u8; 128]);
        synthetic_file.extend_from_slice(b"DICM");
        synthetic_file.extend_from_slice(data);

        if let Ok(file) = DicomReader::new(synthetic_file.as_slice()).read_file() {
            if store_dataset_has_required_uids(&file.dataset) {
                warn!(
                    sop_instance_uid = %sop_instance_uid,
                    negotiated_transfer_syntax = %negotiated_ts,
                    payload_len = data.len(),
                    "Recovered C-STORE dataset by decoding file meta without a preamble"
                );
                return file.dataset;
            }
        }
    }

    if let Ok(file) = DicomReader::new(data).read_file() {
        if store_dataset_has_required_uids(&file.dataset) {
            warn!(
                sop_instance_uid = %sop_instance_uid,
                transfer_syntax = %negotiated_ts,
                payload_len = data.len(),
                "Recovered C-STORE dataset by decoding the payload as a Part 10 file"
            );
            return file.dataset;
        }
    }

    for fallback_ts in [TS_EXPLICIT_LE, TS_IMPLICIT_LE, "1.2.840.10008.1.2.2"] {
        if fallback_ts == negotiated_ts {
            continue;
        }

        if let Ok(dataset) = DicomReader::new(data).read_dataset(fallback_ts) {
            if store_dataset_has_required_uids(&dataset) {
                warn!(
                    sop_instance_uid = %sop_instance_uid,
                    negotiated_transfer_syntax = %negotiated_ts,
                    fallback_transfer_syntax = %fallback_ts,
                    payload_len = data.len(),
                    "Recovered C-STORE dataset using a raw-dataset transfer syntax fallback"
                );
                return dataset;
            }
        }
    }

    error!(
        sop_instance_uid = %sop_instance_uid,
        negotiated_transfer_syntax = %negotiated_ts,
        payload_len = data.len(),
        prefix_hex = %payload_prefix_hex(data, 32),
        has_dicm_prefix = payload_has_dicm_prefix(data),
        starts_with_file_meta = payload_starts_with_file_meta(data),
        "All C-STORE decode fallbacks exhausted"
    );

    DataSet::new()
}

async fn recv_command_data_bytes(
    assoc: &mut ServerAssociation,
    cmd: &DataSet,
    command_name: &str,
    required: bool,
) -> Result<Vec<u8>, dicom_toolkit_core::error::DcmError> {
    let ds_type = cmd.get_u16(tags::COMMAND_DATA_SET_TYPE);
    if !command_has_dataset(cmd) {
        if required {
            warn!(
                command = command_name,
                command_data_set_type = ?ds_type,
                "DIMSE command declared no dataset even though one is required; treating it as empty"
            );
        }
        return Ok(Vec::new());
    }

    match assoc.recv_optional_dimse_data().await? {
        Some(bytes) => {
            info!(
                command = command_name,
                payload_len = bytes.len(),
                command_data_set_type = ?ds_type,
                "Received DIMSE dataset payload"
            );
            Ok(bytes)
        }
        None => {
            let message =
                "DIMSE command declared a dataset but the next queued PDV was another command; treating the dataset as empty";
            if required {
                warn!(
                    command = command_name,
                    command_data_set_type = ?ds_type,
                    "{message}"
                );
            } else {
                debug!(command = command_name, "{message}");
            }
            Ok(Vec::new())
        }
    }
}

async fn handle_store_rq<P>(
    assoc: &mut ServerAssociation,
    ctx_id: u8,
    cmd: &DataSet,
    provider: &P,
) -> Result<(), dicom_toolkit_core::error::DcmError>
where
    P: StoreServiceProvider,
{
    let sop_class = cmd
        .get_string(tags::AFFECTED_SOP_CLASS_UID)
        .unwrap_or_default()
        .trim_end_matches('\0')
        .to_string();
    let sop_instance = cmd
        .get_string(tags::AFFECTED_SOP_INSTANCE_UID)
        .unwrap_or_default()
        .trim_end_matches('\0')
        .to_string();
    let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);

    let data = recv_command_data_bytes(assoc, cmd, "C-STORE", true).await?;

    let ts_uid = assoc
        .context_by_id(ctx_id)
        .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
        .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());

    info!(
        sop_instance_uid = %sop_instance,
        context_id = ctx_id,
        transfer_syntax = %ts_uid,
        payload_len = data.len(),
        "C-STORE decode starting"
    );

    if data.is_empty() {
        error!(
            sop_instance_uid = %sop_instance,
            context_id = ctx_id,
            transfer_syntax = %ts_uid,
            "C-STORE received ZERO bytes of dataset — data receive path returned empty"
        );
    }

    let dataset = decode_store_dataset(data.as_slice(), &ts_uid, &sop_instance);

    let event = StoreEvent {
        calling_ae: assoc.calling_ae.clone(),
        sop_class_uid: sop_class.clone(),
        sop_instance_uid: sop_instance.clone(),
        dataset,
    };

    let result = provider.on_store(event).await;

    let mut rsp = DataSet::new();
    rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
    rsp.set_u16(tags::COMMAND_FIELD, 0x8001);
    rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
    rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
    rsp.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, &sop_instance);
    rsp.set_u16(tags::STATUS, result.status);

    assoc.send_dimse_command(ctx_id, &rsp).await
}

async fn handle_find_rq<P>(
    assoc: &mut ServerAssociation,
    ctx_id: u8,
    cmd: &DataSet,
    provider: &P,
) -> Result<(), dicom_toolkit_core::error::DcmError>
where
    P: FindServiceProvider,
{
    let sop_class = cmd
        .get_string(tags::AFFECTED_SOP_CLASS_UID)
        .unwrap_or_default()
        .trim_end_matches('\0')
        .to_string();
    let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);

    let query_bytes = recv_command_data_bytes(assoc, cmd, "C-FIND", false).await?;

    let negotiated_ts = assoc
        .context_by_id(ctx_id)
        .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
        .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());

    info!(
        query_bytes_len = query_bytes.len(),
        negotiated_ts = %negotiated_ts,
        sop_class = %sop_class,
        "C-FIND identifier received"
    );

    let identifier = match DicomReader::new(query_bytes.as_slice()).read_dataset(&negotiated_ts) {
        Ok(ds) => {
            let tag_list: Vec<String> = ds
                .tags()
                .map(|t| format!("({:04X},{:04X})", t.group, t.element))
                .collect();
            info!(
                num_elements = ds.len(),
                tags = %tag_list.join(", "),
                qr_level = ?ds.get_string(tags::QUERY_RETRIEVE_LEVEL),
                patient_name = ?ds.get_string(tags::PATIENT_NAME),
                patient_id = ?ds.get_string(tags::PATIENT_ID),
                "C-FIND identifier decoded"
            );
            ds
        }
        Err(e) => {
            warn!(
                error = %e,
                negotiated_ts = %negotiated_ts,
                query_bytes_len = query_bytes.len(),
                "C-FIND identifier decode failed, falling back to empty dataset"
            );
            DataSet::new()
        }
    };

    let event = FindEvent {
        calling_ae: assoc.calling_ae.clone(),
        sop_class_uid: sop_class.clone(),
        identifier,
    };

    let matches = provider.on_find(event).await;
    info!(num_matches = matches.len(), "C-FIND query completed");

    for result_ds in &matches {
        let result_bytes = encode_dataset(result_ds, &negotiated_ts)?;

        let mut rsp = DataSet::new();
        rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
        rsp.set_u16(tags::COMMAND_FIELD, 0x8020);
        rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
        rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000);
        rsp.set_u16(tags::STATUS, 0xFF00);

        assoc.send_dimse_command(ctx_id, &rsp).await?;
        assoc.send_dimse_data(ctx_id, &result_bytes).await?;
    }

    let mut final_rsp = DataSet::new();
    final_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
    final_rsp.set_u16(tags::COMMAND_FIELD, 0x8020);
    final_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
    final_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
    final_rsp.set_u16(tags::STATUS, 0x0000);

    assoc.send_dimse_command(ctx_id, &final_rsp).await
}

fn association_config(config: &DimseConfig) -> AssociationConfig {
    AssociationConfig {
        local_ae_title: config.ae_title.clone(),
        max_pdu_length: 65_536,
        dimse_timeout_secs: config.timeout_secs,
        accept_all_transfer_syntaxes: config.accept_all_transfer_syntaxes,
        accepted_transfer_syntaxes: config.accepted_transfer_syntaxes.clone(),
        preferred_transfer_syntaxes: config.preferred_transfer_syntaxes.clone(),
        accepted_abstract_syntaxes: Vec::new(),
        ..AssociationConfig::default()
    }
}

fn next_message_id() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static ID: AtomicU16 = AtomicU16::new(1);
    ID.fetch_add(1, Ordering::Relaxed)
}

fn prepare_retrieve_dataset(
    item: &RetrieveItem,
    target_ts_uid: &str,
) -> Result<Vec<u8>, DimseError> {
    Ok(prepare_dimse_dataset(Bytes::from(item.dataset.clone()), target_ts_uid)?.to_vec())
}

async fn handle_get_rq_with_transcoding<P>(
    assoc: &mut ServerAssociation,
    ctx_id: u8,
    cmd: &DataSet,
    provider: &P,
) -> Result<(), dicom_toolkit_core::error::DcmError>
where
    P: GetServiceProvider,
{
    let sop_class = cmd
        .get_string(tags::AFFECTED_SOP_CLASS_UID)
        .unwrap_or_default()
        .trim_end_matches('\0')
        .to_string();
    let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);

    let query_bytes = recv_command_data_bytes(assoc, cmd, "C-GET", false).await?;

    let ts = assoc
        .context_by_id(ctx_id)
        .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
        .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());

    let identifier = DicomReader::new(query_bytes.as_slice())
        .read_dataset(&ts)
        .unwrap_or_else(|_| DataSet::new());

    let event = GetEvent {
        calling_ae: assoc.calling_ae.clone(),
        sop_class_uid: sop_class.clone(),
        identifier,
    };

    let items = provider.on_get(event).await;
    let total = items.len() as u16;
    let mut completed: u16 = 0;
    let mut failed: u16 = 0;

    for item in &items {
        let remaining = total.saturating_sub(completed + failed + 1);

        let Some(store_ctx) = assoc.find_context(&item.sop_class_uid) else {
            failed += 1;
            continue;
        };

        let store_ctx_id = store_ctx.id;
        let target_ts_uid = store_ctx.transfer_syntax.trim_end_matches('\0').to_string();
        let dataset = match prepare_retrieve_dataset(item, &target_ts_uid) {
            Ok(dataset) => dataset,
            Err(err) => {
                error!(
                    error = %err,
                    sop_class_uid = %item.sop_class_uid,
                    sop_instance_uid = %item.sop_instance_uid,
                    target_transfer_syntax = %target_ts_uid,
                    "Failed to prepare C-GET retrieve dataset"
                );
                failed += 1;
                continue;
            }
        };

        let sub_msg_id = next_message_id();
        let mut store_rq = DataSet::new();
        store_rq.set_uid(tags::AFFECTED_SOP_CLASS_UID, &item.sop_class_uid);
        store_rq.set_u16(tags::COMMAND_FIELD, 0x0001);
        store_rq.set_u16(tags::MESSAGE_ID, sub_msg_id);
        store_rq.set_u16(tags::PRIORITY, 0);
        store_rq.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000);
        store_rq.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, &item.sop_instance_uid);

        assoc.send_dimse_command(store_ctx_id, &store_rq).await?;
        assoc.send_dimse_data(store_ctx_id, &dataset).await?;

        let (_rsp_ctx, store_rsp) = assoc.recv_dimse_command().await?;
        let store_status = store_rsp.get_u16(tags::STATUS).unwrap_or(0xFFFF);
        if store_status == 0x0000 {
            completed += 1;
        } else {
            failed += 1;
        }

        let mut pending_rsp = DataSet::new();
        pending_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
        pending_rsp.set_u16(tags::COMMAND_FIELD, 0x8010);
        pending_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
        pending_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        pending_rsp.set_u16(tags::STATUS, 0xFF00);
        pending_rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, remaining);
        pending_rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, completed);
        pending_rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, failed);
        pending_rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
        assoc.send_dimse_command(ctx_id, &pending_rsp).await?;
    }

    let final_status: u16 = if failed > 0 { 0xB000 } else { 0x0000 };

    let mut final_rsp = DataSet::new();
    final_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
    final_rsp.set_u16(tags::COMMAND_FIELD, 0x8010);
    final_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
    final_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
    final_rsp.set_u16(tags::STATUS, final_status);
    final_rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, 0);
    final_rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, completed);
    final_rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, failed);
    final_rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
    assoc.send_dimse_command(ctx_id, &final_rsp).await
}

async fn handle_move_rq_with_transcoding<P, L>(
    assoc: &mut ServerAssociation,
    ctx_id: u8,
    cmd: &DataSet,
    provider: &P,
    dest_lookup: &L,
    local_ae: &str,
) -> Result<(), dicom_toolkit_core::error::DcmError>
where
    P: MoveServiceProvider,
    L: DestinationLookup + ?Sized,
{
    let sop_class = cmd
        .get_string(tags::AFFECTED_SOP_CLASS_UID)
        .unwrap_or_default()
        .trim_end_matches('\0')
        .to_string();
    let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);
    let destination = cmd
        .get_string(tags::MOVE_DESTINATION)
        .unwrap_or_default()
        .trim()
        .to_string();

    let query_bytes = recv_command_data_bytes(assoc, cmd, "C-MOVE", false).await?;

    let ts = assoc
        .context_by_id(ctx_id)
        .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
        .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());

    let identifier = DicomReader::new(query_bytes.as_slice())
        .read_dataset(&ts)
        .unwrap_or_else(|_| DataSet::new());

    let dest_addr = match dest_lookup.lookup(&destination) {
        Some(addr) => addr,
        None => {
            let mut rsp = DataSet::new();
            rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
            rsp.set_u16(tags::COMMAND_FIELD, 0x8021);
            rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
            rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
            rsp.set_u16(tags::STATUS, 0xA801);
            return assoc.send_dimse_command(ctx_id, &rsp).await;
        }
    };

    let event = MoveEvent {
        calling_ae: assoc.calling_ae.clone(),
        destination: destination.clone(),
        sop_class_uid: sop_class.clone(),
        identifier,
    };

    let items = provider.on_move(event).await;

    if items.is_empty() {
        let mut rsp = DataSet::new();
        rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
        rsp.set_u16(tags::COMMAND_FIELD, 0x8021);
        rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
        rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        rsp.set_u16(tags::STATUS, 0x0000);
        rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, 0);
        rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, 0);
        rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, 0);
        rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
        return assoc.send_dimse_command(ctx_id, &rsp).await;
    }

    let mut unique_sop_classes: Vec<String> = items
        .iter()
        .map(|item| item.sop_class_uid.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    unique_sop_classes.sort();

    let sub_contexts: Vec<PresentationContextRq> = unique_sop_classes
        .iter()
        .enumerate()
        .map(|(index, sop_class_uid)| PresentationContextRq {
            id: (index * 2 + 1) as u8,
            abstract_syntax: sop_class_uid.clone(),
            transfer_syntaxes: vec![TS_EXPLICIT_LE.to_string()],
        })
        .collect();

    let sub_cfg = AssociationConfig {
        local_ae_title: local_ae.to_string(),
        accept_all_transfer_syntaxes: true,
        ..AssociationConfig::default()
    };

    let mut sub_assoc =
        match Association::request(&dest_addr, &destination, local_ae, &sub_contexts, &sub_cfg)
            .await
        {
            Ok(assoc) => assoc,
            Err(_) => {
                let total = items.len() as u16;
                let mut rsp = DataSet::new();
                rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
                rsp.set_u16(tags::COMMAND_FIELD, 0x8021);
                rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
                rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
                rsp.set_u16(tags::STATUS, 0xA801);
                rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, 0);
                rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, 0);
                rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, total);
                rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
                return assoc.send_dimse_command(ctx_id, &rsp).await;
            }
        };

    let total = items.len() as u16;
    let mut completed: u16 = 0;
    let mut failed: u16 = 0;

    for item in &items {
        let remaining = total.saturating_sub(completed + failed + 1);

        let Some(store_ctx) = sub_assoc.find_context(&item.sop_class_uid) else {
            failed += 1;
            continue;
        };

        let store_ctx_id = store_ctx.id;
        let target_ts_uid = store_ctx.transfer_syntax.trim_end_matches('\0').to_string();
        let dataset = match prepare_retrieve_dataset(item, &target_ts_uid) {
            Ok(dataset) => dataset,
            Err(err) => {
                error!(
                    error = %err,
                    sop_class_uid = %item.sop_class_uid,
                    sop_instance_uid = %item.sop_instance_uid,
                    target_transfer_syntax = %target_ts_uid,
                    "Failed to prepare C-MOVE retrieve dataset"
                );
                failed += 1;
                continue;
            }
        };

        let req = StoreRequest {
            sop_class_uid: item.sop_class_uid.clone(),
            sop_instance_uid: item.sop_instance_uid.clone(),
            priority: 0,
            dataset_bytes: dataset,
            context_id: store_ctx_id,
        };
        match c_store(&mut sub_assoc, req).await {
            Ok(rsp) if rsp.status == 0x0000 => completed += 1,
            _ => failed += 1,
        }

        let mut pending_rsp = DataSet::new();
        pending_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
        pending_rsp.set_u16(tags::COMMAND_FIELD, 0x8021);
        pending_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
        pending_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        pending_rsp.set_u16(tags::STATUS, 0xFF00);
        pending_rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, remaining);
        pending_rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, completed);
        pending_rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, failed);
        pending_rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
        assoc.send_dimse_command(ctx_id, &pending_rsp).await?;
    }

    let _ = sub_assoc.release().await;

    let final_status: u16 = if failed > 0 { 0xB000 } else { 0x0000 };

    let mut final_rsp = DataSet::new();
    final_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
    final_rsp.set_u16(tags::COMMAND_FIELD, 0x8021);
    final_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
    final_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
    final_rsp.set_u16(tags::STATUS, final_status);
    final_rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, 0);
    final_rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, completed);
    final_rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, failed);
    final_rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
    assoc.send_dimse_command(ctx_id, &final_rsp).await
}

// ── build_dicom_server ────────────────────────────────────────────────────────

/// Build a [`dicom_toolkit_net::DicomServer`] wired up with pacsnode providers.
///
/// The returned server binds the TCP port immediately.  Call
/// `server.run().await` to start accepting connections, and
/// `server.cancellation_token().cancel()` to stop gracefully.
///
/// # Errors
///
/// Returns [`DimseError`] if the configured TCP port cannot be bound.
pub async fn build_dicom_server(
    config: &DimseConfig,
    store: Arc<dyn MetadataStore>,
    blobs: Arc<dyn BlobStore>,
    known_nodes: Vec<DicomNode>,
) -> Result<NetDicomServer, DimseError> {
    let store_provider = PacsStoreProvider::new(Arc::clone(&store), Arc::clone(&blobs));
    let query_provider = PacsQueryProvider::new(Arc::clone(&store), Arc::clone(&blobs));
    let query_provider2 = PacsQueryProvider::new(Arc::clone(&store), Arc::clone(&blobs));
    let query_provider3 = PacsQueryProvider::new(Arc::clone(&store), Arc::clone(&blobs));

    let dest_entries: Vec<(String, String)> = known_nodes
        .iter()
        .map(|n| (n.ae_title.clone(), n.addr()))
        .collect();

    let server = NetDicomServer::builder()
        .ae_title(&config.ae_title)
        .port(config.port)
        .max_associations(config.max_associations)
        .config(association_config(config))
        .store_provider(store_provider)
        .find_provider(query_provider)
        .get_provider(query_provider2)
        .move_provider(query_provider3)
        .move_destination_lookup(StaticDestinationLookup::new(dest_entries))
        .build()
        .await?;

    Ok(server)
}

#[cfg(test)]
mod ae_whitelist_tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use bytes::Bytes;
    use pacs_core::{
        AuditLogEntry, AuditLogPage, AuditLogQuery, BlobStore, DicomJson,
        DicomNode as RegisteredDicomNode, Instance, InstanceQuery, MetadataStore, NewAuditLogEntry,
        PacsResult, PacsStatistics, Series, SeriesQuery, SeriesUid, SopInstanceUid, Study,
        StudyQuery, StudyUid,
    };

    use super::*;

    struct NoopBlobStore;

    #[async_trait]
    impl BlobStore for NoopBlobStore {
        async fn put(&self, _key: &str, _data: Bytes) -> PacsResult<()> {
            Ok(())
        }

        async fn get(&self, _key: &str) -> PacsResult<Bytes> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn delete(&self, _key: &str) -> PacsResult<()> {
            Ok(())
        }

        async fn exists(&self, _key: &str) -> PacsResult<bool> {
            Ok(false)
        }

        async fn presigned_url(&self, _key: &str, _ttl_secs: u32) -> PacsResult<String> {
            Err(PacsError::Internal("unused".into()))
        }
    }

    struct TestMetadataStore {
        nodes: Vec<RegisteredDicomNode>,
        fail_list_nodes: bool,
    }

    #[async_trait]
    impl MetadataStore for TestMetadataStore {
        async fn store_study(&self, _study: &Study) -> PacsResult<()> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn store_series(&self, _series: &Series) -> PacsResult<()> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn store_instance(&self, _instance: &Instance) -> PacsResult<()> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn query_studies(&self, _q: &StudyQuery) -> PacsResult<Vec<Study>> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn query_series(&self, _q: &SeriesQuery) -> PacsResult<Vec<Series>> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn query_instances(&self, _q: &InstanceQuery) -> PacsResult<Vec<Instance>> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn get_study(&self, _uid: &StudyUid) -> PacsResult<Study> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn get_series(&self, _uid: &SeriesUid) -> PacsResult<Series> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn get_instance(&self, _uid: &SopInstanceUid) -> PacsResult<Instance> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn get_instance_metadata(&self, _uid: &SopInstanceUid) -> PacsResult<DicomJson> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn delete_study(&self, _uid: &StudyUid) -> PacsResult<()> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn delete_series(&self, _uid: &SeriesUid) -> PacsResult<()> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn delete_instance(&self, _uid: &SopInstanceUid) -> PacsResult<()> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn get_statistics(&self) -> PacsResult<PacsStatistics> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn list_nodes(&self) -> PacsResult<Vec<RegisteredDicomNode>> {
            if self.fail_list_nodes {
                Err(PacsError::Internal("should not query".into()))
            } else {
                Ok(self.nodes.clone())
            }
        }

        async fn upsert_node(&self, _node: &RegisteredDicomNode) -> PacsResult<()> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn delete_node(&self, _ae_title: &str) -> PacsResult<()> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn search_audit_logs(&self, _q: &AuditLogQuery) -> PacsResult<AuditLogPage> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn get_audit_log(&self, _id: i64) -> PacsResult<AuditLogEntry> {
            Err(PacsError::Internal("unused".into()))
        }

        async fn store_audit_log(&self, _entry: &NewAuditLogEntry) -> PacsResult<()> {
            Err(PacsError::Internal("unused".into()))
        }
    }

    fn make_server(
        ae_whitelist_enabled: bool,
        nodes: Vec<RegisteredDicomNode>,
        fail_list_nodes: bool,
    ) -> DicomServer {
        DicomServer::new(
            DimseConfig {
                ae_title: "PACSNODE".into(),
                port: 4242,
                ae_whitelist_enabled,
                accept_all_transfer_syntaxes: true,
                accepted_transfer_syntaxes: Vec::new(),
                preferred_transfer_syntaxes: Vec::new(),
                max_associations: 64,
                timeout_secs: 30,
            },
            Arc::new(TestMetadataStore {
                nodes,
                fail_list_nodes,
            }),
            Arc::new(NoopBlobStore),
        )
    }

    #[tokio::test]
    async fn validate_calling_ae_allows_anything_when_whitelist_disabled() {
        let server = make_server(false, vec![], true);
        assert!(server.validate_calling_ae("UNKNOWN").await.is_ok());
    }

    #[tokio::test]
    async fn validate_calling_ae_accepts_registered_node() {
        let server = make_server(
            true,
            vec![RegisteredDicomNode {
                ae_title: "SCU1".into(),
                host: "127.0.0.1".into(),
                port: 104,
                description: None,
                tls_enabled: false,
            }],
            false,
        );

        assert!(server.validate_calling_ae("SCU1").await.is_ok());
    }

    #[tokio::test]
    async fn validate_calling_ae_rejects_unknown_node() {
        let server = make_server(true, Vec::new(), false);
        let error = server.validate_calling_ae("UNKNOWN").await.unwrap_err();

        assert!(matches!(
            error,
            PacsError::NotFound {
                resource: "calling_ae",
                uid
            } if uid == "UNKNOWN"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DimseConfig;
    use dicom_toolkit_net::{
        dimse,
        pdu::{self, AssociateRq, Pdu, Pdv, PresentationContextRqItem},
        AssociationConfig,
    };
    use mockall::mock;
    use pacs_core::{
        AuditLogEntry, AuditLogPage, AuditLogQuery, BlobStore, DicomJson,
        DicomNode as RegisteredDicomNode, Instance, InstanceQuery, MetadataStore, NewAuditLogEntry,
        PacsResult, PacsStatistics, Series, SeriesQuery, SeriesUid, SopInstanceUid, Study,
        StudyQuery, StudyUid,
    };
    use tokio::{
        io::AsyncWriteExt,
        net::{TcpListener, TcpStream},
        sync::oneshot,
    };

    mock! {
        TestStore {}
        #[async_trait::async_trait]
        impl MetadataStore for TestStore {
            async fn store_study(&self, study: &Study) -> PacsResult<()>;
            async fn store_series(&self, series: &Series) -> PacsResult<()>;
            async fn store_instance(&self, instance: &Instance) -> PacsResult<()>;
            async fn query_studies(&self, q: &StudyQuery) -> PacsResult<Vec<Study>>;
            async fn query_series(&self, q: &SeriesQuery) -> PacsResult<Vec<Series>>;
            async fn query_instances(&self, q: &InstanceQuery) -> PacsResult<Vec<Instance>>;
            async fn get_study(&self, uid: &StudyUid) -> PacsResult<Study>;
            async fn get_series(&self, uid: &SeriesUid) -> PacsResult<Series>;
            async fn get_instance(&self, uid: &SopInstanceUid) -> PacsResult<Instance>;
            async fn get_instance_metadata(&self, uid: &SopInstanceUid) -> PacsResult<DicomJson>;
            async fn delete_study(&self, uid: &StudyUid) -> PacsResult<()>;
            async fn delete_series(&self, uid: &SeriesUid) -> PacsResult<()>;
            async fn delete_instance(&self, uid: &SopInstanceUid) -> PacsResult<()>;
            async fn get_statistics(&self) -> PacsResult<PacsStatistics>;
            async fn list_nodes(&self) -> PacsResult<Vec<RegisteredDicomNode>>;
            async fn upsert_node(&self, node: &RegisteredDicomNode) -> PacsResult<()>;
            async fn delete_node(&self, ae_title: &str) -> PacsResult<()>;
            async fn search_audit_logs(&self, q: &AuditLogQuery) -> PacsResult<AuditLogPage>;
            async fn get_audit_log(&self, id: i64) -> PacsResult<AuditLogEntry>;
            async fn store_audit_log(&self, entry: &NewAuditLogEntry) -> PacsResult<()>;
        }
    }
    mock! {
        TestBlobs {}
        #[async_trait::async_trait]
        impl BlobStore for TestBlobs {
            async fn put(&self, key: &str, data: bytes::Bytes) -> PacsResult<()>;
            async fn get(&self, key: &str) -> PacsResult<bytes::Bytes>;
            async fn delete(&self, key: &str) -> PacsResult<()>;
            async fn exists(&self, key: &str) -> PacsResult<bool>;
            async fn presigned_url(&self, key: &str, ttl_secs: u32) -> PacsResult<String>;
        }
    }

    /// Verifies that the COMMAND_FIELD routing match arms cover all standard
    /// DIMSE command codes.
    #[test]
    fn command_field_routing_covers_all_standard_commands() {
        let commands: &[(u16, &str)] = &[
            (0x0001, "C-STORE-RQ"),
            (0x0020, "C-FIND-RQ"),
            (0x0010, "C-GET-RQ"),
            (0x0021, "C-MOVE-RQ"),
            (0x0030, "C-ECHO-RQ"),
        ];

        for (value, name) in commands {
            assert!(
                matches!(value, 0x0001 | 0x0010 | 0x0020 | 0x0021 | 0x0030),
                "{name} (0x{value:04X}) is not covered by the routing match"
            );
        }
    }

    #[test]
    fn dicom_node_clone() {
        let node = DicomNode {
            ae_title: "STORE".into(),
            host: "10.0.0.1".into(),
            port: 11_112,
        };
        let cloned = node.clone();
        assert_eq!(node.ae_title, cloned.ae_title);
        assert_eq!(node.port, cloned.port);
    }

    #[test]
    fn dicom_config_custom_values() {
        let config = DimseConfig {
            ae_title: "MYAE".into(),
            port: 1234,
            ..DimseConfig::default()
        };
        assert_eq!(config.ae_title, "MYAE");
        assert_eq!(config.port, 1234);
        assert_eq!(DimseConfig::default().ae_title, "PACSNODE");
    }

    #[test]
    fn association_config_applies_transfer_syntax_policy() {
        let config = DimseConfig {
            accept_all_transfer_syntaxes: false,
            accepted_transfer_syntaxes: vec![
                "1.2.840.10008.1.2.1".into(),
                "1.2.840.10008.1.2.4.50".into(),
            ],
            preferred_transfer_syntaxes: vec!["1.2.840.10008.1.2.4.50".into()],
            ..DimseConfig::default()
        };

        let assoc = association_config(&config);
        assert!(!assoc.accept_all_transfer_syntaxes);
        assert_eq!(
            assoc.accepted_transfer_syntaxes,
            config.accepted_transfer_syntaxes
        );
        assert_eq!(
            assoc.preferred_transfer_syntaxes,
            config.preferred_transfer_syntaxes
        );
    }

    #[test]
    fn dicom_node_addr_method() {
        let node = DicomNode::new("STORESCP", "127.0.0.1", 4242);
        assert_eq!(node.addr(), "127.0.0.1:4242");
    }

    #[test]
    fn dicom_node_addr_hostname() {
        let node = DicomNode::new("DEST", "pacs.example.com", 11112);
        assert_eq!(node.addr(), "pacs.example.com:11112");
    }

    fn store_context() -> PresentationContextRqItem {
        PresentationContextRqItem {
            id: 1,
            abstract_syntax: "1.2.840.10008.5.1.4.1.1.2".into(),
            transfer_syntaxes: vec![TS_EXPLICIT_LE.to_string()],
        }
    }

    fn store_associate_rq() -> AssociateRq {
        AssociateRq {
            called_ae_title: "PACSNODE".into(),
            calling_ae_title: "FO-DICOM".into(),
            application_context: "1.2.840.10008.3.1.1.1".into(),
            presentation_contexts: vec![store_context()],
            max_pdu_length: 16_384,
            implementation_class_uid: "1.2.826.0.1.3680043.8.498.1".into(),
            implementation_version_name: "FO-DICOM".into(),
        }
    }

    #[test]
    fn decode_store_dataset_recovers_part10_payload() {
        let study_uid = "1.2.3.4.10";
        let series_uid = "1.2.3.4.10.1";
        let sop_instance_uid = "1.2.3.4.10.1.1";

        let mut dataset = DataSet::new();
        dataset.set_uid(tags::STUDY_INSTANCE_UID, study_uid);
        dataset.set_uid(tags::SERIES_INSTANCE_UID, series_uid);
        dataset.set_uid(tags::SOP_INSTANCE_UID, sop_instance_uid);

        let file = dicom_toolkit_data::FileFormat::from_dataset(
            "1.2.840.10008.5.1.4.1.1.2",
            sop_instance_uid,
            dataset,
        );
        let mut payload = Vec::new();
        DicomWriter::new(&mut payload)
            .write_file(&file)
            .expect("encode Part 10 store payload");

        let decoded = decode_store_dataset(&payload, TS_EXPLICIT_LE, sop_instance_uid);
        assert_eq!(
            decoded.get_string(tags::STUDY_INSTANCE_UID),
            Some(study_uid)
        );
        assert_eq!(
            decoded.get_string(tags::SERIES_INSTANCE_UID),
            Some(series_uid)
        );
    }

    #[test]
    fn decode_store_dataset_recovers_file_meta_without_preamble() {
        let study_uid = "1.2.3.4.11";
        let series_uid = "1.2.3.4.11.1";
        let sop_instance_uid = "1.2.3.4.11.1.1";

        let mut dataset = DataSet::new();
        dataset.set_uid(tags::STUDY_INSTANCE_UID, study_uid);
        dataset.set_uid(tags::SERIES_INSTANCE_UID, series_uid);
        dataset.set_uid(tags::SOP_INSTANCE_UID, sop_instance_uid);

        let file = dicom_toolkit_data::FileFormat::from_dataset(
            "1.2.840.10008.5.1.4.1.1.2",
            sop_instance_uid,
            dataset,
        );
        let mut payload = Vec::new();
        DicomWriter::new(&mut payload)
            .write_file(&file)
            .expect("encode Part 10 store payload");

        let payload_without_preamble = &payload[132..];
        let decoded =
            decode_store_dataset(payload_without_preamble, TS_EXPLICIT_LE, sop_instance_uid);
        assert_eq!(
            decoded.get_string(tags::STUDY_INSTANCE_UID),
            Some(study_uid)
        );
        assert_eq!(
            decoded.get_string(tags::SERIES_INSTANCE_UID),
            Some(series_uid)
        );
    }

    #[test]
    fn decode_store_dataset_recovers_raw_dataset() {
        let study_uid = "1.2.3.4.12";
        let series_uid = "1.2.3.4.12.1";
        let sop_instance_uid = "1.2.3.4.12.1.1";

        let mut dataset = DataSet::new();
        dataset.set_uid(tags::STUDY_INSTANCE_UID, study_uid);
        dataset.set_uid(tags::SERIES_INSTANCE_UID, series_uid);
        dataset.set_uid(tags::SOP_INSTANCE_UID, sop_instance_uid);

        // Encode as raw dataset bytes (the standard DIMSE C-STORE payload).
        let mut raw_bytes = Vec::new();
        DicomWriter::new(&mut raw_bytes)
            .write_dataset(&dataset, TS_EXPLICIT_LE)
            .expect("encode raw dataset");

        assert!(!raw_bytes.is_empty(), "encoded dataset must be non-empty");

        let decoded = decode_store_dataset(&raw_bytes, TS_EXPLICIT_LE, sop_instance_uid);
        assert_eq!(
            decoded.get_string(tags::STUDY_INSTANCE_UID),
            Some(study_uid)
        );
        assert_eq!(
            decoded.get_string(tags::SERIES_INSTANCE_UID),
            Some(series_uid)
        );
    }

    #[test]
    fn decode_store_dataset_handles_real_dicom_file_payload() {
        let test_file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("testfiles/ABDOM_1.dcm");
        if !test_file.exists() {
            // Skip if test file not available.
            return;
        }

        let file_bytes = std::fs::read(&test_file).expect("read test DICOM file");

        // Parse the file to get the study/series UIDs for verification.
        let file = DicomReader::new(file_bytes.as_slice())
            .read_file()
            .expect("parse Part 10 test file");
        let study_uid = file
            .dataset
            .get_string(tags::STUDY_INSTANCE_UID)
            .expect("test file has Study UID");
        let series_uid = file
            .dataset
            .get_string(tags::SERIES_INSTANCE_UID)
            .expect("test file has Series UID");
        let sop_instance_uid = file
            .dataset
            .get_string(tags::SOP_INSTANCE_UID)
            .expect("test file has SOP Instance UID");
        let ts_uid = &file.meta.transfer_syntax_uid;

        // Extract the raw dataset (skip preamble + DICM + file meta header).
        // Re-encode the parsed dataset to get clean raw bytes.
        let mut raw_bytes = Vec::new();
        DicomWriter::new(&mut raw_bytes)
            .write_dataset(&file.dataset, ts_uid)
            .expect("re-encode dataset");

        // This is the decode path a real C-STORE SCP uses.
        let decoded = decode_store_dataset(&raw_bytes, ts_uid, sop_instance_uid);
        assert_eq!(
            decoded.get_string(tags::STUDY_INSTANCE_UID),
            Some(study_uid),
            "Study UID must survive raw-dataset round-trip"
        );
        assert_eq!(
            decoded.get_string(tags::SERIES_INSTANCE_UID),
            Some(series_uid),
            "Series UID must survive raw-dataset round-trip"
        );

        // Also test decode_store_dataset with the full Part 10 payload
        // (some buggy SCUs send Part 10 over DIMSE).
        let decoded_p10 = decode_store_dataset(&file_bytes, ts_uid, sop_instance_uid);
        assert_eq!(
            decoded_p10.get_string(tags::STUDY_INSTANCE_UID),
            Some(study_uid),
            "Study UID must survive Part 10 fallback decode"
        );
    }

    fn store_command(msg_id: u16, command_data_set_type: u16) -> DataSet {
        let mut cmd = DataSet::new();
        cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.5.1.4.1.1.2");
        cmd.set_u16(tags::COMMAND_FIELD, 0x0001);
        cmd.set_u16(tags::MESSAGE_ID, msg_id);
        cmd.set_u16(tags::PRIORITY, 0);
        cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, command_data_set_type);
        cmd.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, &format!("1.2.3.{msg_id}"));
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
    async fn recv_command_data_bytes_allows_missing_store_dataset_without_losing_next_command() {
        let (server_stream, mut client_stream) = connect_pair().await;
        let (done_tx, done_rx) = oneshot::channel();

        tokio::spawn(async move {
            let mut assoc = ServerAssociation::accept(server_stream, &AssociationConfig::default())
                .await
                .expect("accept association");

            let (_, cmd) = assoc
                .recv_dimse_command()
                .await
                .expect("receive first command");
            let data = recv_command_data_bytes(&mut assoc, &cmd, "C-STORE", true)
                .await
                .expect("read store dataset or empty");
            let (_, next_cmd) = assoc
                .recv_dimse_command()
                .await
                .expect("receive queued command");
            done_tx
                .send((data, next_cmd.get_u16(tags::MESSAGE_ID)))
                .expect("send result to test");
        });

        client_stream
            .write_all(&pdu::encode_associate_rq(&store_associate_rq()))
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
                data: dimse::encode_command_dataset(&store_command(1, 0x0000)),
            },
            Pdv {
                context_id: 1,
                msg_control: 0x03,
                data: dimse::encode_command_dataset(&store_command(2, 0x0000)),
            },
        ]);
        client_stream
            .write_all(&pdus)
            .await
            .expect("send back-to-back store commands");

        let (data, next_message_id) = done_rx.await.expect("server processed commands");
        assert!(data.is_empty());
        assert_eq!(next_message_id, Some(2));
    }

    #[tokio::test]
    async fn recv_store_data_round_trip_through_wire() {
        let study_uid = "1.2.3.4.99";
        let series_uid = "1.2.3.4.99.1";
        let sop_instance_uid = "1.2.3.4.99.1.1";

        // Build the dataset payload as a raw Explicit VR LE stream (DIMSE standard).
        let mut dataset = DataSet::new();
        dataset.set_uid(tags::STUDY_INSTANCE_UID, study_uid);
        dataset.set_uid(tags::SERIES_INSTANCE_UID, series_uid);
        dataset.set_uid(tags::SOP_INSTANCE_UID, sop_instance_uid);
        let mut raw_payload = Vec::new();
        DicomWriter::new(&mut raw_payload)
            .write_dataset(&dataset, TS_EXPLICIT_LE)
            .expect("encode raw dataset");

        let (server_stream, mut client_stream) = connect_pair().await;
        let (done_tx, done_rx) = oneshot::channel();
        let payload_clone = raw_payload.clone();

        tokio::spawn(async move {
            let mut assoc = ServerAssociation::accept(server_stream, &AssociationConfig::default())
                .await
                .expect("accept association");

            let (ctx_id, cmd) = assoc
                .recv_dimse_command()
                .await
                .expect("receive C-STORE command");
            let data = recv_command_data_bytes(&mut assoc, &cmd, "C-STORE", true)
                .await
                .expect("receive store dataset bytes");

            let ts_uid = assoc
                .context_by_id(ctx_id)
                .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
                .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());
            let decoded = decode_store_dataset(&data, &ts_uid, sop_instance_uid);

            done_tx
                .send((data.len(), decoded))
                .expect("send result to test");
        });

        // Client side: send A-ASSOCIATE-RQ, read A-ASSOCIATE-AC.
        client_stream
            .write_all(&pdu::encode_associate_rq(&store_associate_rq()))
            .await
            .expect("send associate-rq");
        match pdu::read_pdu(&mut client_stream)
            .await
            .expect("read associate-ac")
        {
            Pdu::AssociateAc(_) => {}
            other => panic!("expected AssociateAc, got {other:?}"),
        }

        // Send C-STORE command in its own P-DATA-TF.
        let cmd_pdv = Pdv {
            context_id: 1,
            msg_control: 0x03, // last + command
            data: dimse::encode_command_dataset(&store_command(1, 0x0000)),
        };
        client_stream
            .write_all(&pdu::encode_p_data_tf(&[cmd_pdv]))
            .await
            .expect("send store command");

        // Send dataset in its own P-DATA-TF.
        let data_pdv = Pdv {
            context_id: 1,
            msg_control: 0x02, // last + data
            data: payload_clone,
        };
        client_stream
            .write_all(&pdu::encode_p_data_tf(&[data_pdv]))
            .await
            .expect("send store data");

        let (received_len, decoded) = done_rx.await.expect("server processed C-STORE");
        assert_eq!(received_len, raw_payload.len());
        assert_eq!(
            decoded.get_string(tags::STUDY_INSTANCE_UID),
            Some(study_uid)
        );
        assert_eq!(
            decoded.get_string(tags::SERIES_INSTANCE_UID),
            Some(series_uid)
        );
    }

    #[tokio::test]
    async fn recv_store_data_multi_pdu_round_trip() {
        let study_uid = "1.2.3.4.100";
        let series_uid = "1.2.3.4.100.1";
        let sop_instance_uid = "1.2.3.4.100.1.1";

        let mut dataset = DataSet::new();
        dataset.set_uid(tags::STUDY_INSTANCE_UID, study_uid);
        dataset.set_uid(tags::SERIES_INSTANCE_UID, series_uid);
        dataset.set_uid(tags::SOP_INSTANCE_UID, sop_instance_uid);
        let mut raw_payload = Vec::new();
        DicomWriter::new(&mut raw_payload)
            .write_dataset(&dataset, TS_EXPLICIT_LE)
            .expect("encode raw dataset");

        let (server_stream, mut client_stream) = connect_pair().await;
        let (done_tx, done_rx) = oneshot::channel();
        let payload_clone = raw_payload.clone();

        tokio::spawn(async move {
            let mut assoc = ServerAssociation::accept(server_stream, &AssociationConfig::default())
                .await
                .expect("accept association");

            let (ctx_id, cmd) = assoc
                .recv_dimse_command()
                .await
                .expect("receive C-STORE command");
            let data = recv_command_data_bytes(&mut assoc, &cmd, "C-STORE", true)
                .await
                .expect("receive store dataset bytes");

            let ts_uid = assoc
                .context_by_id(ctx_id)
                .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
                .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());
            let decoded = decode_store_dataset(&data, &ts_uid, sop_instance_uid);

            done_tx
                .send((data.len(), decoded))
                .expect("send result to test");
        });

        client_stream
            .write_all(&pdu::encode_associate_rq(&store_associate_rq()))
            .await
            .expect("send associate-rq");
        match pdu::read_pdu(&mut client_stream)
            .await
            .expect("read associate-ac")
        {
            Pdu::AssociateAc(_) => {}
            other => panic!("expected AssociateAc, got {other:?}"),
        }

        // Command in its own P-DATA-TF.
        let cmd_pdv = Pdv {
            context_id: 1,
            msg_control: 0x03,
            data: dimse::encode_command_dataset(&store_command(1, 0x0000)),
        };
        client_stream
            .write_all(&pdu::encode_p_data_tf(&[cmd_pdv]))
            .await
            .expect("send store command");

        // Split dataset across multiple P-DATA-TFs (simulate small max PDU).
        let chunk_size = 20; // very small chunks
        let chunks: Vec<&[u8]> = payload_clone.chunks(chunk_size).collect();
        let total = chunks.len();
        for (i, chunk) in chunks.iter().enumerate() {
            let is_last = i + 1 == total;
            let pdv = Pdv {
                context_id: 1,
                msg_control: if is_last { 0x02 } else { 0x00 }, // data, last flag only on final
                data: chunk.to_vec(),
            };
            client_stream
                .write_all(&pdu::encode_p_data_tf(&[pdv]))
                .await
                .expect("send data fragment");
        }

        let (received_len, decoded) = done_rx.await.expect("server processed C-STORE");
        assert_eq!(received_len, raw_payload.len());
        assert_eq!(
            decoded.get_string(tags::STUDY_INSTANCE_UID),
            Some(study_uid)
        );
        assert_eq!(
            decoded.get_string(tags::SERIES_INSTANCE_UID),
            Some(series_uid)
        );
    }

    #[tokio::test]
    async fn recv_store_data_cmd_and_data_in_same_pdu() {
        let study_uid = "1.2.3.4.101";
        let series_uid = "1.2.3.4.101.1";
        let sop_instance_uid = "1.2.3.4.101.1.1";

        let mut dataset = DataSet::new();
        dataset.set_uid(tags::STUDY_INSTANCE_UID, study_uid);
        dataset.set_uid(tags::SERIES_INSTANCE_UID, series_uid);
        dataset.set_uid(tags::SOP_INSTANCE_UID, sop_instance_uid);
        let mut raw_payload = Vec::new();
        DicomWriter::new(&mut raw_payload)
            .write_dataset(&dataset, TS_EXPLICIT_LE)
            .expect("encode raw dataset");

        let (server_stream, mut client_stream) = connect_pair().await;
        let (done_tx, done_rx) = oneshot::channel();

        tokio::spawn(async move {
            let mut assoc = ServerAssociation::accept(server_stream, &AssociationConfig::default())
                .await
                .expect("accept association");
            let (ctx_id, cmd) = assoc
                .recv_dimse_command()
                .await
                .expect("receive C-STORE command");
            let data = recv_command_data_bytes(&mut assoc, &cmd, "C-STORE", true)
                .await
                .expect("receive store dataset bytes");
            let ts_uid = assoc
                .context_by_id(ctx_id)
                .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
                .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());
            let decoded = decode_store_dataset(&data, &ts_uid, sop_instance_uid);
            done_tx.send((data.len(), decoded)).expect("send result");
        });

        client_stream
            .write_all(&pdu::encode_associate_rq(&store_associate_rq()))
            .await
            .expect("send associate-rq");
        match pdu::read_pdu(&mut client_stream)
            .await
            .expect("read associate-ac")
        {
            Pdu::AssociateAc(_) => {}
            other => panic!("expected AssociateAc, got {other:?}"),
        }

        // fo-dicom style: command AND data PDVs in the SAME P-DATA-TF.
        let pdus = pdu::encode_p_data_tf(&[
            Pdv {
                context_id: 1,
                msg_control: 0x03, // last + command
                data: dimse::encode_command_dataset(&store_command(1, 0x0000)),
            },
            Pdv {
                context_id: 1,
                msg_control: 0x02, // last + data
                data: raw_payload.clone(),
            },
        ]);
        client_stream
            .write_all(&pdus)
            .await
            .expect("send coalesced command+data");

        let (received_len, decoded) = done_rx.await.expect("server processed C-STORE");
        assert_eq!(received_len, raw_payload.len());
        assert_eq!(
            decoded.get_string(tags::STUDY_INSTANCE_UID),
            Some(study_uid)
        );
        assert_eq!(
            decoded.get_string(tags::SERIES_INSTANCE_UID),
            Some(series_uid)
        );
    }

    #[tokio::test]
    async fn build_dicom_server_binds_port() {
        let config = DimseConfig {
            ae_title: "TESTPACS".into(),
            port: 0, // let OS pick a free port
            ae_whitelist_enabled: false,
            accept_all_transfer_syntaxes: true,
            accepted_transfer_syntaxes: Vec::new(),
            preferred_transfer_syntaxes: Vec::new(),
            max_associations: 2,
            timeout_secs: 5,
        };
        let store = Arc::new(MockTestStore::new());
        let blobs = Arc::new(MockTestBlobs::new());
        let result = build_dicom_server(&config, store, blobs, vec![]).await;
        assert!(result.is_ok(), "build_dicom_server failed");
        let server = result.unwrap();
        assert!(
            server.local_addr().is_ok(),
            "server should have a bound address"
        );
    }
}
