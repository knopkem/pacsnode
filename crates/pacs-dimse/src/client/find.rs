//! C-FIND SCU (Query/Retrieve — Query Service).

use dicom_toolkit_net::{c_find, FindRequest, PresentationContextRq};

use crate::client::DicomClient;
use crate::error::DimseError;
use crate::server::DicomNode;

/// Explicit VR Little Endian transfer syntax UID.
const EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";

impl DicomClient {
    /// Queries a remote PACS using C-FIND and returns the encoded result datasets.
    ///
    /// `sop_class_uid` selects the query model, for example:
    /// * Study Root Query/Retrieve — FIND: `1.2.840.10008.5.1.4.1.2.2.1`
    /// * Patient Root Query/Retrieve — FIND: `1.2.840.10008.5.1.4.1.2.1.1`
    ///
    /// `query_bytes` must be a pre-encoded DICOM dataset containing the query
    /// identifier attributes.
    ///
    /// Returns one `Vec<u8>` (encoded dataset bytes) per matching result.
    /// An empty `Vec` means the query matched no results.
    ///
    /// # Errors
    ///
    /// Returns [`DimseError`] if the association cannot be established or if
    /// a protocol-level error occurs.
    ///
    /// # Cancellation Safety
    ///
    /// This function is **not** cancellation-safe.
    pub async fn find(
        &self,
        node: &DicomNode,
        sop_class_uid: &str,
        query_bytes: Vec<u8>,
    ) -> Result<Vec<Vec<u8>>, DimseError> {
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

        let req = FindRequest {
            sop_class_uid: sop_class_uid.into(),
            query: query_bytes,
            context_id: ctx_id,
            priority: 0, // medium
        };

        let results = c_find(&mut assoc, req).await;

        if let Err(e) = assoc.release().await {
            tracing::debug!(error = %e, "C-FIND association release error (non-fatal)");
        }

        results.map_err(DimseError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Study Root Q/R — FIND SOP Class UID.
    const STUDY_ROOT_FIND: &str = "1.2.840.10008.5.1.4.1.2.2.1";

    #[tokio::test]
    async fn find_connection_refused() {
        let client = DicomClient::new("TESTSCU", 5);
        let node = DicomNode {
            ae_title: "QUERYNODE".into(),
            host: "127.0.0.1".into(),
            port: 65532,
        };
        let result = client.find(&node, STUDY_ROOT_FIND, vec![]).await;
        assert!(
            result.is_err(),
            "find() must fail when connection is refused"
        );
    }
}
