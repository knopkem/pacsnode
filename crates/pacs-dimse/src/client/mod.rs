//! DICOM SCU client: C-ECHO, C-STORE, C-FIND, and C-MOVE operations.

use dicom_toolkit_net::{Association, AssociationConfig, PresentationContextRq};

use crate::error::DimseError;
use crate::server::DicomNode;

mod echo;
mod find;
mod move_scu;
mod store;

/// SCU client for sending DICOM operations to remote nodes.
///
/// A single `DicomClient` instance can be used to connect to multiple remote
/// nodes. Each operation opens a fresh DICOM association, performs the
/// operation, and releases the association.
pub struct DicomClient {
    calling_ae: String,
    timeout_secs: u64,
}

impl DicomClient {
    /// Creates a new `DicomClient` with the given calling AE title and timeout.
    pub fn new(calling_ae: impl Into<String>, timeout_secs: u64) -> Self {
        Self {
            calling_ae: calling_ae.into(),
            timeout_secs,
        }
    }

    /// Returns the calling AE title used in association requests.
    pub fn calling_ae(&self) -> &str {
        &self.calling_ae
    }

    /// Opens a DICOM association to `node` with the given presentation contexts.
    ///
    /// # Cancellation Safety
    ///
    /// This function is **not** cancellation-safe. The association negotiation
    /// must complete before the future is dropped.
    async fn connect(
        &self,
        node: &DicomNode,
        contexts: &[PresentationContextRq],
    ) -> Result<Association, DimseError> {
        let addr = format!("{}:{}", node.host, node.port);
        let config = AssociationConfig {
            local_ae_title: self.calling_ae.clone(),
            dimse_timeout_secs: self.timeout_secs,
            accept_all_transfer_syntaxes: true,
            ..AssociationConfig::default()
        };
        let assoc = Association::request(&addr, &node.ae_title, &self.calling_ae, contexts, &config)
            .await?;
        Ok(assoc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_stores_calling_ae() {
        let client = DicomClient::new("MYSCU", 30);
        assert_eq!(client.calling_ae(), "MYSCU");
    }

    #[test]
    fn new_accepts_string_types() {
        let client = DicomClient::new(String::from("MYSCU"), 10);
        assert_eq!(client.calling_ae(), "MYSCU");
    }
}
