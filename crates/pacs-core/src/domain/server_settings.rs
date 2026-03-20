//! Persisted DIMSE listener settings managed through the admin UI.

use serde::{Deserialize, Serialize};

/// Default archival transfer syntax for newly ingested instances.
pub const DEFAULT_STORAGE_TRANSFER_SYNTAX_UID: &str = "1.2.840.10008.1.2.4.201";

/// DIMSE listener settings that pacsnode persists in the metadata store.
///
/// These values drive the DICOM SCP listener and are applied on process start.
/// Updating them at runtime requires a server restart before the active DIMSE
/// listener picks up the change.
///
/// # Example
///
/// ```
/// use pacs_core::ServerSettings;
///
/// let settings = ServerSettings::default();
/// assert_eq!(settings.dicom_port, 4242);
/// assert_eq!(settings.ae_title, "PACSNODE");
/// assert_eq!(
///     settings.storage_transfer_syntax.as_deref(),
///     Some(pacs_core::DEFAULT_STORAGE_TRANSFER_SYNTAX_UID)
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSettings {
    /// TCP port for the DICOM DIMSE SCP.
    pub dicom_port: u16,
    /// DICOM Application Entity title for this PACS node.
    pub ae_title: String,
    /// Require inbound DIMSE callers to be registered in the node registry.
    pub ae_whitelist_enabled: bool,
    /// Whether the DIMSE SCP accepts any offered transfer syntax by default.
    pub accept_all_transfer_syntaxes: bool,
    /// Explicit DIMSE SCP transfer syntax allow-list.
    pub accepted_transfer_syntaxes: Vec<String>,
    /// Preferred DIMSE SCP transfer syntax order, highest priority first.
    pub preferred_transfer_syntaxes: Vec<String>,
    /// Optional transfer syntax that newly ingested objects should be stored as.
    ///
    /// When `None`, pacsnode stores each received object using its inbound
    /// transfer syntax instead of forcing archive-wide recoding.
    pub storage_transfer_syntax: Option<String>,
    /// Maximum number of concurrent DIMSE associations.
    pub max_associations: usize,
    /// DIMSE association timeout in seconds.
    pub dimse_timeout_secs: u64,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            dicom_port: 4242,
            ae_title: "PACSNODE".to_string(),
            ae_whitelist_enabled: false,
            accept_all_transfer_syntaxes: true,
            accepted_transfer_syntaxes: Vec::new(),
            preferred_transfer_syntaxes: Vec::new(),
            storage_transfer_syntax: Some(DEFAULT_STORAGE_TRANSFER_SYNTAX_UID.to_string()),
            max_associations: 64,
            dimse_timeout_secs: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ServerSettings, DEFAULT_STORAGE_TRANSFER_SYNTAX_UID};

    #[test]
    fn default_storage_transfer_syntax_is_htj2k_lossless() {
        assert_eq!(
            ServerSettings::default().storage_transfer_syntax.as_deref(),
            Some(DEFAULT_STORAGE_TRANSFER_SYNTAX_UID)
        );
    }
}
