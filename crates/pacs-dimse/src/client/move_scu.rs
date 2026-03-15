//! C-MOVE SCU (Query/Retrieve — Move Service).

use dicom_toolkit_net::{c_move, MoveRequest, MoveResponse, PresentationContextRq};

use crate::client::DicomClient;
use crate::error::DimseError;
use crate::server::DicomNode;

/// Explicit VR Little Endian transfer syntax UID.
const EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";

impl DicomClient {
    /// Requests a remote PACS to forward matching instances to `destination_ae`
    /// via C-MOVE.
    ///
    /// `sop_class_uid` selects the query/retrieve model, for example:
    /// * Study Root Query/Retrieve — MOVE: `1.2.840.10008.5.1.4.1.2.2.2`
    /// * Patient Root Query/Retrieve — MOVE: `1.2.840.10008.5.1.4.1.2.1.2`
    ///
    /// `query_bytes` must be a pre-encoded DICOM dataset containing the
    /// query identifier attributes.
    ///
    /// `destination_ae` is the AE title of the node to which the SCP should
    /// forward the instances. The SCP must know how to resolve that AE title
    /// to a network address.
    ///
    /// Returns one [`MoveResponse`] per C-MOVE-RSP received (including
    /// pending responses).
    ///
    /// # Errors
    ///
    /// Returns [`DimseError`] if the association cannot be established or if
    /// a protocol-level error occurs.
    ///
    /// # Cancellation Safety
    ///
    /// This function is **not** cancellation-safe.
    pub async fn move_instances(
        &self,
        node: &DicomNode,
        destination_ae: &str,
        sop_class_uid: &str,
        query_bytes: Vec<u8>,
    ) -> Result<Vec<MoveResponse>, DimseError> {
        let contexts = vec![PresentationContextRq {
            id: 1,
            abstract_syntax: sop_class_uid.into(),
            transfer_syntaxes: vec![EXPLICIT_VR_LE.into()],
        }];

        let mut assoc = self.connect(node, &contexts).await?;

        let ctx_id = assoc
            .find_context(sop_class_uid)
            .map(|pc| pc.id)
            .ok_or_else(|| DimseError::NoPresentationContext(sop_class_uid.into()))?;

        let req = MoveRequest {
            sop_class_uid: sop_class_uid.into(),
            destination: destination_ae.into(),
            query: query_bytes,
            context_id: ctx_id,
            priority: 0, // medium
        };

        let responses = c_move(&mut assoc, req).await;

        if let Err(e) = assoc.release().await {
            tracing::debug!(error = %e, "C-MOVE association release error (non-fatal)");
        }

        responses.map_err(DimseError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Study Root Q/R — MOVE SOP Class UID.
    const STUDY_ROOT_MOVE: &str = "1.2.840.10008.5.1.4.1.2.2.2";

    #[tokio::test]
    async fn move_instances_connection_refused() {
        let client = DicomClient::new("TESTSCU", 5);
        let node = DicomNode {
            ae_title: "MOVENODE".into(),
            host: "127.0.0.1".into(),
            port: 65531,
        };
        let result = client
            .move_instances(&node, "DEST_AE", STUDY_ROOT_MOVE, vec![])
            .await;
        assert!(
            result.is_err(),
            "move_instances() must fail when connection is refused"
        );
    }
}
