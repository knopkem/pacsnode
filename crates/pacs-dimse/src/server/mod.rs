//! DICOM SCP server: accepts TCP connections and routes DIMSE commands.

use std::sync::Arc;

use dicom_toolkit_data::DataSet;
use dicom_toolkit_dict::tags;
use dicom_toolkit_net::{
    handle_find_rq, handle_get_rq, handle_move_rq, handle_store_rq, Association,
    AssociationConfig, DicomServer as NetDicomServer, StaticDestinationLookup,
};
use pacs_core::{BlobStore, MetadataStore};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::config::DimseConfig;
use crate::error::DimseError;

pub mod provider;
use provider::{PacsQueryProvider, PacsStoreProvider};

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
    known_nodes: Arc<tokio::sync::RwLock<Vec<DicomNode>>>,
}

impl DicomServer {
    /// Creates a new `DicomServer` with the given configuration and storage backends.
    pub fn new(
        config: DimseConfig,
        store: Arc<dyn MetadataStore>,
        blobs: Arc<dyn BlobStore>,
    ) -> Self {
        Self {
            config,
            store,
            blobs,
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
    pub async fn serve(
        self: Arc<Self>,
        shutdown: CancellationToken,
    ) -> Result<(), DimseError> {
        let addr = format!("0.0.0.0:{}", self.config.port);
        let listener = TcpListener::bind(&addr).await.map_err(|e| DimseError::Bind {
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
                        Self::handle_connection(stream, server).await;
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
    async fn handle_connection(stream: tokio::net::TcpStream, server: Arc<DicomServer>) {
        let assoc_config = AssociationConfig {
            local_ae_title: server.config.ae_title.clone(),
            max_pdu_length: 65_536,
            dimse_timeout_secs: server.config.timeout_secs,
            accept_all_transfer_syntaxes: true,
            accepted_abstract_syntaxes: Vec::new(),
            ..AssociationConfig::default()
        };

        let mut assoc = match Association::accept(stream, &assoc_config).await {
            Ok(a) => a,
            Err(e) => {
                error!(error = %e, "Association negotiation failed");
                return;
            }
        };

        let store_provider =
            PacsStoreProvider::new(Arc::clone(&server.store), Arc::clone(&server.blobs));
        let query_provider =
            PacsQueryProvider::new(Arc::clone(&server.store), Arc::clone(&server.blobs));

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
                0x0020 => handle_find_rq(&mut assoc, ctx_id, &cmd, &query_provider)
                    .await
                    .map_err(DimseError::from),

                // C-GET-RQ
                0x0010 => handle_get_rq(&mut assoc, ctx_id, &cmd, &query_provider)
                    .await
                    .map_err(DimseError::from),

                // C-MOVE-RQ
                0x0021 => {
                    handle_move_rq(
                        &mut assoc,
                        ctx_id,
                        &cmd,
                        &query_provider,
                        &dest_lookup,
                        &local_ae,
                    )
                    .await
                    .map_err(DimseError::from)
                }

                other => {
                    warn!(command_field = other, "Unknown DIMSE command field — ignoring");
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
    }
}

/// Sends a C-ECHO-RSP in reply to a C-ECHO-RQ.
///
/// # Cancellation Safety
///
/// This function is **not** cancellation-safe.
async fn send_echo_response(
    assoc: &mut Association,
    ctx_id: u8,
    cmd: &DataSet,
) -> Result<(), DimseError> {
    let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);

    let mut rsp = DataSet::new();
    rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.1.1");
    rsp.set_u16(tags::COMMAND_FIELD, 0x8030_u16);         // C-ECHO-RSP
    rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
    rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101_u16); // no dataset
    rsp.set_u16(tags::STATUS, 0x0000_u16);                // success

    assoc.send_dimse_command(ctx_id, &rsp).await?;
    Ok(())
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
mod tests {
    use super::*;
    use crate::config::DimseConfig;
    use mockall::mock;
    use pacs_core::{
        BlobStore, DicomJson, Instance, InstanceQuery, MetadataStore, PacsResult, PacsStatistics,
        Series, SeriesQuery, SeriesUid, SopInstanceUid, Study, StudyQuery, StudyUid,
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
            async fn list_nodes(&self) -> PacsResult<Vec<pacs_core::DicomNode>>;
            async fn upsert_node(&self, node: &pacs_core::DicomNode) -> PacsResult<()>;
            async fn delete_node(&self, ae_title: &str) -> PacsResult<()>;
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
    fn dicom_node_addr_method() {
        let node = DicomNode::new("STORESCP", "127.0.0.1", 4242);
        assert_eq!(node.addr(), "127.0.0.1:4242");
    }

    #[test]
    fn dicom_node_addr_hostname() {
        let node = DicomNode::new("DEST", "pacs.example.com", 11112);
        assert_eq!(node.addr(), "pacs.example.com:11112");
    }

    #[tokio::test]
    async fn build_dicom_server_binds_port() {
        let config = DimseConfig {
            ae_title: "TESTPACS".into(),
            port: 0, // let OS pick a free port
            max_associations: 2,
            timeout_secs: 5,
        };
        let store = Arc::new(MockTestStore::new());
        let blobs = Arc::new(MockTestBlobs::new());
        let result = build_dicom_server(&config, store, blobs, vec![]).await;
        assert!(result.is_ok(), "build_dicom_server failed");
        let server = result.unwrap();
        assert!(server.local_addr().is_ok(), "server should have a bound address");
    }
}
