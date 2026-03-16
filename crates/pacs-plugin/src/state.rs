use std::sync::Arc;

use pacs_core::{BlobStore, MetadataStore};
use serde::Serialize;

use crate::registry::PluginRegistry;

/// Static server identity exposed through the API.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServerInfo {
    /// DICOM Application Entity title.
    pub ae_title: String,
    /// HTTP port bound by the API server.
    pub http_port: u16,
    /// DIMSE port bound by the SCP.
    pub dicom_port: u16,
    /// Application version string.
    pub version: &'static str,
}

/// Shared runtime state injected into every Axum handler.
#[derive(Clone)]
pub struct AppState {
    /// Static server identity.
    pub server_info: ServerInfo,
    /// Active metadata store.
    pub store: Arc<dyn MetadataStore>,
    /// Active blob store.
    pub blobs: Arc<dyn BlobStore>,
    /// Active plugin registry.
    pub plugins: Arc<PluginRegistry>,
}
