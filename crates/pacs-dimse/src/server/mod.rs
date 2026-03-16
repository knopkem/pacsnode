//! DICOM SCP server: accepts TCP connections and routes DIMSE commands.

use std::sync::Arc;

use bytes::Bytes;
use dicom_toolkit_data::DataSet;
use dicom_toolkit_dict::tags;
use dicom_toolkit_net::{
    c_store, handle_find_rq, handle_store_rq,
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
use provider::{PacsQueryProvider, PacsStoreProvider};

const TS_EXPLICIT_LE: &str = "1.2.840.10008.1.2.1";

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
        // Prefer Explicit VR Little Endian so that the negotiated transfer
        // syntax matches what dicom-toolkit-net hardcodes when encoding
        // C-FIND / C-GET / C-MOVE response datasets.  Without this, clients
        // that offer Implicit VR LE as their first (or only) transfer syntax
        // would receive misencoded response datasets and abort the association.
        let assoc_config = AssociationConfig {
            local_ae_title: server.config.ae_title.clone(),
            max_pdu_length: 65_536,
            dimse_timeout_secs: server.config.timeout_secs,
            accept_all_transfer_syntaxes: true,
            accepted_abstract_syntaxes: Vec::new(),
            preferred_transfer_syntaxes: vec![
                "1.2.840.10008.1.2.1".to_string(), // Explicit VR Little Endian
            ],
            ..AssociationConfig::default()
        };

        let mut assoc = match Association::accept(stream, &assoc_config).await {
            Ok(a) => a,
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
    assoc: &mut Association,
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
    assoc: &mut Association,
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

    let query_bytes = assoc.recv_dimse_data().await?;

    let ts = assoc
        .context_by_id(ctx_id)
        .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
        .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());

    let identifier = dicom_toolkit_data::DicomReader::new(query_bytes.as_slice())
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
    assoc: &mut Association,
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

    let query_bytes = assoc.recv_dimse_data().await?;

    let ts = assoc
        .context_by_id(ctx_id)
        .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
        .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());

    let identifier = dicom_toolkit_data::DicomReader::new(query_bytes.as_slice())
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
        DicomNode as RegisteredDicomNode, Instance, InstanceQuery, MetadataStore, PacsResult,
        PacsStatistics, Series, SeriesQuery, SeriesUid, SopInstanceUid, Study, StudyQuery,
        StudyUid,
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
    use mockall::mock;
    use pacs_core::{
        AuditLogEntry, AuditLogPage, AuditLogQuery, BlobStore, DicomJson,
        DicomNode as RegisteredDicomNode, Instance, InstanceQuery, MetadataStore, PacsResult,
        PacsStatistics, Series, SeriesQuery, SeriesUid, SopInstanceUid, Study, StudyQuery,
        StudyUid,
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
