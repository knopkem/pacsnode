use async_trait::async_trait;

use crate::domain::{
    DicomJson, Instance, InstanceQuery, PacsStatistics, Series, SeriesQuery, SeriesUid,
    SopInstanceUid, Study, StudyQuery, StudyUid,
};
use crate::error::PacsResult;

/// Persistent storage interface for DICOM metadata.
///
/// Implementors provide WADO-RS retrieve semantics and QIDO-RS query semantics
/// over the study → series → instance hierarchy.
///
/// The trait is object-safe and requires both `Send` and `Sync` so that
/// implementations can be shared across async task boundaries.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MetadataStore: Send + Sync {
    /// Stores (inserts or upserts) a [`Study`] in the metadata store.
    async fn store_study(&self, study: &Study) -> PacsResult<()>;

    /// Stores (inserts or upserts) a [`Series`] in the metadata store.
    async fn store_series(&self, series: &Series) -> PacsResult<()>;

    /// Stores (inserts or upserts) an [`Instance`] in the metadata store.
    async fn store_instance(&self, instance: &Instance) -> PacsResult<()>;

    /// Returns all studies matching the given query parameters.
    async fn query_studies(&self, q: &StudyQuery) -> PacsResult<Vec<Study>>;

    /// Returns all series matching the given query parameters.
    async fn query_series(&self, q: &SeriesQuery) -> PacsResult<Vec<Series>>;

    /// Returns all instances matching the given query parameters.
    async fn query_instances(&self, q: &InstanceQuery) -> PacsResult<Vec<Instance>>;

    /// Retrieves a single study by its UID.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PacsError::NotFound`] if no study with `uid` exists.
    async fn get_study(&self, uid: &StudyUid) -> PacsResult<Study>;

    /// Retrieves a single series by its UID.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PacsError::NotFound`] if no series with `uid` exists.
    async fn get_series(&self, uid: &SeriesUid) -> PacsResult<Series>;

    /// Retrieves a single instance by its UID.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PacsError::NotFound`] if no instance with `uid` exists.
    async fn get_instance(&self, uid: &SopInstanceUid) -> PacsResult<Instance>;

    /// Retrieves only the DICOM JSON metadata for an instance (no binary data).
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PacsError::NotFound`] if no instance with `uid` exists.
    async fn get_instance_metadata(&self, uid: &SopInstanceUid) -> PacsResult<DicomJson>;

    /// Deletes a study and all its dependent series and instances.
    async fn delete_study(&self, uid: &StudyUid) -> PacsResult<()>;

    /// Deletes a series and all its dependent instances.
    async fn delete_series(&self, uid: &SeriesUid) -> PacsResult<()>;

    /// Deletes a single instance.
    async fn delete_instance(&self, uid: &SopInstanceUid) -> PacsResult<()>;

    /// Returns aggregate statistics for the PACS system.
    async fn get_statistics(&self) -> PacsResult<PacsStatistics>;
}

#[cfg(test)]
mod tests {
    use super::{MetadataStore, MockMetadataStore};
    use crate::domain::{PacsStatistics, StudyUid};
    use crate::error::PacsError;

    #[tokio::test]
    async fn test_mock_get_statistics() {
        let mut mock = MockMetadataStore::new();
        mock.expect_get_statistics().once().returning(|| {
            Ok(PacsStatistics {
                num_studies: 42,
                num_series: 210,
                num_instances: 2100,
                disk_usage_bytes: 10_737_418_240,
            })
        });

        let stats = mock.get_statistics().await.unwrap();
        assert_eq!(stats.num_studies, 42);
        assert_eq!(stats.num_series, 210);
        assert_eq!(stats.num_instances, 2100);
    }

    #[tokio::test]
    async fn test_mock_get_study_not_found() {
        let mut mock = MockMetadataStore::new();
        mock.expect_get_study().once().returning(|uid| {
            Err(PacsError::NotFound {
                resource: "study",
                uid: uid.to_string(),
            })
        });

        let uid = StudyUid::from("1.2.3.4.5");
        let result = mock.get_study(&uid).await;
        assert!(matches!(result, Err(PacsError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_mock_delete_study_ok() {
        let mut mock = MockMetadataStore::new();
        mock.expect_delete_study().once().returning(|_| Ok(()));

        let uid = StudyUid::from("1.2.3");
        let result: crate::error::PacsResult<()> = mock.delete_study(&uid).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_mock_query_studies_empty() {
        let mut mock = MockMetadataStore::new();
        mock.expect_query_studies().once().returning(|_| Ok(vec![]));

        let q = crate::domain::StudyQuery::default();
        let results: Vec<crate::domain::Study> = mock.query_studies(&q).await.unwrap();
        assert!(results.is_empty());
    }
}
