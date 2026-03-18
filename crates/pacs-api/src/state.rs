//! Re-exports of the shared application state types from `pacs-plugin`.

pub use pacs_core::DicomNode;
pub use pacs_plugin::{AppState, ServerInfo};

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pacs_core::ServerSettings;
    use pacs_plugin::PluginRegistry;

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
        let json = r#"{"ae_title":"SCU","host":"1.2.3.4","port":4242}"#;
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

    #[test]
    fn test_app_state_holds_plugin_registry() {
        let state = AppState {
            server_info: ServerInfo {
                ae_title: "PACSNODE".into(),
                http_port: 8042,
                dicom_port: 4242,
                version: "0.1.0",
            },
            server_settings: ServerSettings::default(),
            store: Arc::new(MockMetaStore::new()),
            blobs: Arc::new(MockBlobStr::new()),
            plugins: Arc::new(PluginRegistry::new()),
        };

        let _ = state.clone();
    }
}
