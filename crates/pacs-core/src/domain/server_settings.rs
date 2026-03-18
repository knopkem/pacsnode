//! Persisted DIMSE listener settings managed through the admin UI.

use serde::{Deserialize, Serialize};

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
            storage_transfer_syntax: None,
            max_associations: 64,
            dimse_timeout_secs: 30,
        }
    }
}
