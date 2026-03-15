//! [`DicomNode`] — remote Application Entity configuration.

use serde::{Deserialize, Serialize};

/// A remote DICOM Application Entity used as an AE whitelist entry and as the
/// target for SCU operations (C-STORE, C-FIND, C-MOVE, C-ECHO).
///
/// # Example
///
/// ```
/// use pacs_core::DicomNode;
///
/// let node = DicomNode {
///     ae_title:    "MODALITY1".to_string(),
///     host:        "192.168.1.10".to_string(),
///     port:        104,
///     description: Some("CT Scanner — Room 3".to_string()),
///     tls_enabled: false,
/// };
/// assert_eq!(node.ae_title, "MODALITY1");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DicomNode {
    /// DICOM Application Entity title (max 16 characters per the DICOM standard).
    pub ae_title: String,
    /// Hostname or IP address of the remote AE.
    pub host: String,
    /// TCP port the remote AE listens on (standard DICOM ports are 104 or 11112).
    pub port: u16,
    /// Optional human-readable label for this node.
    pub description: Option<String>,
    /// Connect using TLS (dicom-toolkit-rs rustls). Defaults to `false`.
    #[serde(default)]
    pub tls_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip_with_tls_enabled() {
        let node = DicomNode {
            ae_title: "PACS1".into(),
            host: "10.0.0.1".into(),
            port: 104,
            description: Some("Primary PACS".into()),
            tls_enabled: true,
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: DicomNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, node);
    }

    #[test]
    fn tls_enabled_defaults_to_false_when_absent() {
        let json = r#"{"ae_title":"SCU","host":"1.2.3.4","port":11112}"#;
        let node: DicomNode = serde_json::from_str(json).unwrap();
        assert!(!node.tls_enabled);
        assert!(node.description.is_none());
    }
}
