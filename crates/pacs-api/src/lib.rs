//! pacsnode — Axum HTTP server (DICOMweb + REST).
//!
//! ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for
//! diagnostic or therapeutic purposes.
//!
//! This crate exposes:
//! - [`router::build_router`] — constructs the full Axum [`axum::Router`]
//! - DICOMweb endpoints: STOW-RS, QIDO-RS, WADO-RS
//! - A REST management API for studies, series, instances, and DICOM nodes

pub mod error;
pub mod router;
pub mod routes;
pub mod state;

pub use router::build_router;
pub use state::{AppState, DicomNode, ServerInfo};

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Arc;

    use async_trait::async_trait;
    use bytes::Bytes;
    use mockall::mock;
    use pacs_core::{
        BlobStore, DicomJson, DicomNode, Instance, InstanceQuery, MetadataStore, PacsResult,
        PacsStatistics, Series, SeriesQuery, SeriesUid, SopInstanceUid, Study, StudyQuery,
        StudyUid,
    };

    use crate::state::AppState;

    mock! {
        pub MetaStore {}

        #[async_trait]
        impl MetadataStore for MetaStore {
            async fn store_study(&self, study: &Study) -> PacsResult<()>;
            async fn store_series(&self, series: &Series) -> PacsResult<()>;
            async fn store_instance(&self, instance: &Instance) -> PacsResult<()>;
            async fn query_studies(&self, q: &StudyQuery) -> PacsResult<Vec<Study>>;
            async fn query_series(&self, q: &SeriesQuery) -> PacsResult<Vec<Series>>;
            async fn query_instances(&self, q: &InstanceQuery) -> PacsResult<Vec<Instance>>;
            async fn get_study(&self, uid: &StudyUid) -> PacsResult<Study>;
            async fn get_series(&self, uid: &SeriesUid) -> PacsResult<Series>;
            async fn get_instance(&self, uid: &SopInstanceUid) -> PacsResult<Instance>;
            async fn get_instance_metadata(&self, uid: &SopInstanceUid) -> PacsResult<DicomJson>;
            async fn delete_study(&self, uid: &StudyUid) -> PacsResult<()>;
            async fn delete_series(&self, uid: &SeriesUid) -> PacsResult<()>;
            async fn delete_instance(&self, uid: &SopInstanceUid) -> PacsResult<()>;
            async fn get_statistics(&self) -> PacsResult<PacsStatistics>;
            async fn list_nodes(&self) -> PacsResult<Vec<DicomNode>>;
            async fn upsert_node(&self, node: &DicomNode) -> PacsResult<()>;
            async fn delete_node(&self, ae_title: &str) -> PacsResult<()>;
        }
    }

    mock! {
        pub BlobStr {}

        #[async_trait]
        impl BlobStore for BlobStr {
            async fn put(&self, key: &str, data: Bytes) -> PacsResult<()>;
            async fn get(&self, key: &str) -> PacsResult<Bytes>;
            async fn delete(&self, key: &str) -> PacsResult<()>;
            async fn exists(&self, key: &str) -> PacsResult<bool>;
            async fn presigned_url(&self, key: &str, ttl_secs: u32) -> PacsResult<String>;
        }
    }

    /// Build an [`AppState`] backed by the provided mock stores.
    pub fn make_test_state(store: MockMetaStore, blobs: MockBlobStr) -> AppState {
        use crate::state::ServerInfo;
        AppState {
            server_info: ServerInfo {
                ae_title: "TESTPACS".into(),
                http_port: 8042,
                dicom_port: 4242,
                version: env!("CARGO_PKG_VERSION"),
            },
            store: Arc::new(store),
            blobs: Arc::new(blobs),
        }
    }
}
