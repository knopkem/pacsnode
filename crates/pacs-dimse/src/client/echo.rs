//! C-ECHO SCU (Verification Service Class).

use dicom_toolkit_net::{c_echo, PresentationContextRq};

use crate::client::DicomClient;
use crate::error::DimseError;
use crate::server::DicomNode;

/// Verification SOP Class UID (PS3.4 §A.1).
const VERIFICATION_SOP_CLASS: &str = "1.2.840.10008.1.1";

/// Explicit VR Little Endian transfer syntax UID.
const EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";

impl DicomClient {
    /// Sends a C-ECHO-RQ to `node` and verifies a successful C-ECHO-RSP.
    ///
    /// Opens a DICOM association, sends the verification request, and releases
    /// the association.
    ///
    /// # Errors
    ///
    /// Returns [`DimseError::Dcm`] when the connection is refused or when the
    /// SCP returns a non-success DIMSE status.
    ///
    /// Returns [`DimseError::NoPresentationContext`] when the SCP does not
    /// accept the Verification SOP Class.
    ///
    /// # Cancellation Safety
    ///
    /// This function is **not** cancellation-safe. Dropping the future mid-way
    /// may leave the association in an undefined state.
    pub async fn echo(&self, node: &DicomNode) -> Result<(), DimseError> {
        let contexts = vec![PresentationContextRq {
            id: 1,
            abstract_syntax: VERIFICATION_SOP_CLASS.into(),
            transfer_syntaxes: vec![EXPLICIT_VR_LE.into()],
        }];

        let mut assoc = self.connect(node, &contexts).await?;

        let ctx_id = assoc
            .find_context(VERIFICATION_SOP_CLASS)
            .map(|pc| pc.id)
            .ok_or_else(|| DimseError::NoPresentationContext(VERIFICATION_SOP_CLASS.into()))?;

        let echo_result = c_echo(&mut assoc, ctx_id).await;

        // Always attempt to release the association, even on error.
        let release_result = assoc.release().await;

        echo_result.map_err(DimseError::from)?;

        if let Err(e) = release_result {
            tracing::debug!(error = %e, "C-ECHO association release error (non-fatal)");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that `echo()` returns a [`DimseError`] when the remote host
    /// is not listening (connection refused).
    ///
    /// Uses a loopback address with a port that is almost certainly not bound.
    #[tokio::test]
    async fn echo_connection_refused() {
        let client = DicomClient::new("TESTSCU", 5);
        let node = DicomNode {
            ae_title: "NONEXISTENT".into(),
            host: "127.0.0.1".into(),
            // Port 65534 is in the ephemeral range and should not be listening.
            port: 65534,
        };

        let result = client.echo(&node).await;
        assert!(
            result.is_err(),
            "echo() must fail when no server is listening"
        );
    }
}
