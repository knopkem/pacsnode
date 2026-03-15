//! C-STORE SCU (Storage Service Class).

use std::collections::HashSet;

use dicom_toolkit_net::{c_store, PresentationContextRq, StoreRequest, StoreResponse};
use tracing::warn;

use crate::client::DicomClient;
use crate::error::DimseError;
use crate::server::DicomNode;

/// Explicit VR Little Endian transfer syntax UID.
const EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";

/// Maximum number of unique presentation contexts in a single association (DICOM limit).
const MAX_CONTEXTS: usize = 128;

impl DicomClient {
    /// Sends one or more DICOM instances to a remote node via C-STORE.
    ///
    /// Each element of `instances` is a tuple of
    /// `(sop_class_uid, sop_instance_uid, encoded_dataset_bytes)`.
    ///
    /// A single association is opened for all instances. Duplicate SOP Class
    /// UIDs are de-duplicated when building presentation contexts. At most
    /// 128 unique SOP classes can be sent in a single call (DICOM protocol
    /// limit); any excess classes are silently skipped.
    ///
    /// # Errors
    ///
    /// Returns [`DimseError`] if the association cannot be established or if
    /// a send-level error occurs. Individual C-STORE responses with non-success
    /// status codes are returned in the `Vec<StoreResponse>` rather than as
    /// errors.
    ///
    /// # Cancellation Safety
    ///
    /// This function is **not** cancellation-safe. Dropping the future after
    /// the association has opened may leave the remote SCP in an undefined
    /// state.
    pub async fn store(
        &self,
        node: &DicomNode,
        instances: Vec<(String, String, bytes::Bytes)>,
    ) -> Result<Vec<StoreResponse>, DimseError> {
        if instances.is_empty() {
            return Ok(Vec::new());
        }

        // De-duplicate SOP classes and build presentation contexts (odd IDs: 1, 3, …).
        let mut seen: HashSet<String> = HashSet::new();
        let contexts: Vec<PresentationContextRq> = instances
            .iter()
            .filter_map(|(sop_class, _, _)| {
                if seen.insert(sop_class.clone()) {
                    Some(sop_class.clone())
                } else {
                    None
                }
            })
            .take(MAX_CONTEXTS)
            .enumerate()
            .map(|(i, sop_class)| PresentationContextRq {
                id: ((i * 2 + 1) & 0xFF) as u8,
                abstract_syntax: sop_class,
                transfer_syntaxes: vec![EXPLICIT_VR_LE.into()],
            })
            .collect();

        let mut assoc = self.connect(node, &contexts).await?;
        let mut responses = Vec::with_capacity(instances.len());

        for (sop_class, sop_instance, data) in instances {
            let ctx_id = match assoc.find_context(&sop_class).map(|pc| pc.id) {
                Some(id) => id,
                None => {
                    warn!(
                        sop_class = %sop_class,
                        "No accepted presentation context for SOP class — skipping instance"
                    );
                    continue;
                }
            };

            let req = StoreRequest {
                sop_class_uid: sop_class,
                sop_instance_uid: sop_instance,
                priority: 0, // medium
                dataset_bytes: data.to_vec(),
                context_id: ctx_id,
            };

            match c_store(&mut assoc, req).await {
                Ok(rsp) => responses.push(rsp),
                Err(e) => {
                    // Abort on network-level errors; DIMSE-level failures are
                    // surfaced via the response status.
                    let _ = assoc.abort().await;
                    return Err(DimseError::from(e));
                }
            }
        }

        if let Err(e) = assoc.release().await {
            tracing::debug!(error = %e, "C-STORE association release error (non-fatal)");
        }

        Ok(responses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn store_empty_list_returns_ok() {
        let client = DicomClient::new("TESTSCU", 5);
        let node = DicomNode {
            ae_title: "STORESCP".into(),
            host: "127.0.0.1".into(),
            port: 65533,
        };
        // Empty list should not attempt any connection.
        let result = client.store(&node, vec![]).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn store_connection_refused() {
        let client = DicomClient::new("TESTSCU", 5);
        let node = DicomNode {
            ae_title: "STORESCP".into(),
            host: "127.0.0.1".into(),
            port: 65533,
        };
        let instances = vec![(
            "1.2.840.10008.5.1.4.1.1.2".into(),
            "1.2.3.4.5".into(),
            bytes::Bytes::from_static(b"DICOM"),
        )];
        let result = client.store(&node, instances).await;
        assert!(
            result.is_err(),
            "store() must fail when connection is refused"
        );
    }
}
