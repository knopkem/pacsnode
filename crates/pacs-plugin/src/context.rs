use std::sync::Arc;

use pacs_core::{BlobStore, MetadataStore};

use crate::{event::EventBus, state::ServerInfo};

/// Context passed to plugins during initialization and startup.
#[derive(Clone)]
pub struct PluginContext {
    /// Plugin-specific configuration from the `[plugins.<id>]` section.
    pub config: serde_json::Value,
    /// Active metadata store, if one has already been initialized.
    pub metadata_store: Option<Arc<dyn MetadataStore>>,
    /// Active blob store, if one has already been initialized.
    pub blob_store: Option<Arc<dyn BlobStore>>,
    /// Static server identity information.
    pub server_info: ServerInfo,
    /// Shared event bus for emitting and observing events.
    pub event_bus: Arc<EventBus>,
}
