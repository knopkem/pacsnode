//! Shared application state injected into every Axum handler.

use std::sync::Arc;

use pacs_core::{BlobStore, MetadataStore};
use serde::Serialize;

pub use pacs_core::DicomNode;

/// Static server identity exposed via `GET /system`.
///
/// Populated once from [`AppConfig`](pacs_server::config::AppConfig) at
/// startup and shared read-only across all request handlers.
#[derive(Debug, Clone, Serialize)]
pub struct ServerInfo {
    /// DICOM Application Entity title of this PACS node.
    pub ae_title: String,
    /// TCP port the HTTP / DICOMweb API is bound to.
    pub http_port: u16,
    /// TCP port the DIMSE SCP is bound to.
    pub dicom_port: u16,
    /// Crate version from `Cargo.toml` (`CARGO_PKG_VERSION`).
    pub version: &'static str,
}

/// Shared state cloned into every Axum handler via [`axum::extract::State`].
#[derive(Clone)]
pub struct AppState {
    /// Static server identity (AE title, ports, version).
    pub server_info: ServerInfo,
    /// DICOM metadata store (study/series/instance catalogue and node registry).
    pub store: Arc<dyn MetadataStore>,
    /// Binary DICOM blob store.
    pub blobs: Arc<dyn BlobStore>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{make_test_state, MockBlobStr, MockMetaStore};

    #[test]
    fn test_app_state_clones() {
        let state = make_test_state(MockMetaStore::new(), MockBlobStr::new());
        let _ = state.clone();
    }

    #[test]
    fn test_dicom_node_serde_roundtrip() {
        let node = DicomNode {
            ae_title: "PACS1".into(),
            host: "192.168.1.1".into(),
            port: 104,
            description: Some("Primary PACS".into()),
            tls_enabled: false,
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: DicomNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ae_title, "PACS1");
        assert_eq!(back.port, 104);
        assert!(!back.tls_enabled);
    }

    #[test]
    fn test_dicom_node_tls_defaults_false() {
        let json = r#"{"ae_title":"SCU","host":"1.2.3.4","port":11112}"#;
        let node: DicomNode = serde_json::from_str(json).unwrap();
        assert!(!node.tls_enabled);
    }

    #[test]
    fn test_server_info_serializes() {
        let info = ServerInfo {
            ae_title: "PACSNODE".into(),
            http_port: 8042,
            dicom_port: 4242,
            version: "0.1.0",
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["ae_title"], "PACSNODE");
        assert_eq!(json["http_port"], 8042);
        assert_eq!(json["dicom_port"], 4242);
    }
}
