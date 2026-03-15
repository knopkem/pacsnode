//! SCP service provider implementations backed by the pacsnode storage layer.

use std::sync::Arc;

use bytes::Bytes;
use dicom_toolkit_data::{DataSet, DicomWriter};
use dicom_toolkit_dict::{tags, Vr};
use dicom_toolkit_net::services::provider::{
    FindEvent, FindServiceProvider, GetEvent, GetServiceProvider, MoveEvent, MoveServiceProvider,
    RetrieveItem, StoreEvent, StoreResult, StoreServiceProvider, STATUS_PROCESSING_FAILURE,
};
use pacs_core::{
    blob_key_for, BlobStore, DicomJson, Instance, InstanceQuery, MetadataStore, Series,
    SeriesQuery, SeriesUid, SopInstanceUid, Study, StudyQuery, StudyUid,
};
use tracing::{debug, error, instrument, warn};

// ── C-STORE provider ──────────────────────────────────────────────────────────

/// SCP handler that stores received DICOM instances into the pacsnode backends.
pub struct PacsStoreProvider {
    store: Arc<dyn MetadataStore>,
    blobs: Arc<dyn BlobStore>,
}

impl PacsStoreProvider {
    /// Creates a new `PacsStoreProvider` backed by the given stores.
    pub fn new(store: Arc<dyn MetadataStore>, blobs: Arc<dyn BlobStore>) -> Self {
        Self { store, blobs }
    }
}

impl StoreServiceProvider for PacsStoreProvider {
    #[instrument(skip(self, event), fields(
        sop_class = %event.sop_class_uid,
        sop_instance = %event.sop_instance_uid,
        calling_ae = %event.calling_ae,
    ))]
    async fn on_store(&self, event: StoreEvent) -> StoreResult {
        debug!("C-STORE received");
        // ── Extract required UIDs ──────────────────────────────────────────
        let study_uid_str = event
            .dataset
            .get_string(tags::STUDY_INSTANCE_UID)
            .map(|s| s.trim().trim_end_matches('\0').to_string())
            .unwrap_or_default();

        let series_uid_str = event
            .dataset
            .get_string(tags::SERIES_INSTANCE_UID)
            .map(|s| s.trim().trim_end_matches('\0').to_string())
            .unwrap_or_default();

        let sop_instance_uid_str = event
            .sop_instance_uid
            .trim()
            .trim_end_matches('\0')
            .to_string();

        if study_uid_str.is_empty() || series_uid_str.is_empty() || sop_instance_uid_str.is_empty()
        {
            error!(
                study_uid = %study_uid_str,
                series_uid = %series_uid_str,
                sop_instance_uid = %sop_instance_uid_str,
                "C-STORE missing required UIDs"
            );
            return StoreResult::failure(STATUS_PROCESSING_FAILURE);
        }

        let study_uid = StudyUid::from(study_uid_str.as_str());
        let series_uid = SeriesUid::from(series_uid_str.as_str());
        let instance_uid = SopInstanceUid::from(sop_instance_uid_str.as_str());

        // ── Encode dataset to bytes (Explicit VR Little Endian) ────────────
        let encoded = {
            let mut buf = Vec::new();
            {
                let mut writer = DicomWriter::new(&mut buf);
                if let Err(e) = writer.write_dataset(&event.dataset, "1.2.840.10008.1.2.1") {
                    error!(error = %e, "Failed to encode received DICOM dataset");
                    return StoreResult::failure(STATUS_PROCESSING_FAILURE);
                }
            }
            buf
        };

        // ── Store raw bytes in the blob store ──────────────────────────────
        let blob_key = blob_key_for(&study_uid, &series_uid, &instance_uid);
        if let Err(e) = self.blobs.put(&blob_key, Bytes::from(encoded)).await {
            error!(error = %e, blob_key = %blob_key, "Failed to persist DICOM blob");
            return StoreResult::failure(STATUS_PROCESSING_FAILURE);
        }

        // ── Build domain objects from dataset attributes ───────────────────
        let patient_id = event
            .dataset
            .get_string(tags::PATIENT_ID)
            .map(|s| s.trim().trim_end_matches('\0').to_string());
        let patient_name = event
            .dataset
            .get_string(tags::PATIENT_NAME)
            .map(|s| s.trim().trim_end_matches('\0').to_string());
        let accession_number = event
            .dataset
            .get_string(tags::ACCESSION_NUMBER)
            .map(|s| s.trim().trim_end_matches('\0').to_string());
        let study_time = event
            .dataset
            .get_string(tags::STUDY_TIME)
            .map(|s| s.trim().to_string());
        let modalities: Vec<String> = event
            .dataset
            .get_string(tags::MODALITY)
            .map(|m| vec![m.trim().to_string()])
            .unwrap_or_default();
        let referring_physician = event
            .dataset
            .get_string(tags::REFERRING_PHYSICIAN_NAME)
            .map(|s| s.trim().to_string());
        let study_description = event
            .dataset
            .get_string(tags::STUDY_DESCRIPTION)
            .map(|s| s.trim().to_string());

        let study = Study {
            study_uid: study_uid.clone(),
            patient_id,
            patient_name,
            study_date: None, // requires chrono parsing; left for metadata indexing
            study_time,
            accession_number,
            modalities,
            referring_physician,
            description: study_description,
            num_series: 1,
            num_instances: 1,
            metadata: DicomJson::empty(),
            created_at: None,
            updated_at: None,
        };

        let modality = event
            .dataset
            .get_string(tags::MODALITY)
            .map(|s| s.trim().to_string());
        let series_number = event.dataset.get_i32(tags::SERIES_NUMBER);
        let series_description = event
            .dataset
            .get_string(tags::SERIES_DESCRIPTION)
            .map(|s| s.trim().to_string());

        let series = Series {
            series_uid: series_uid.clone(),
            study_uid: study_uid.clone(),
            modality,
            series_number,
            description: series_description,
            body_part: None,
            num_instances: 1,
            metadata: DicomJson::empty(),
            created_at: None,
        };

        let sop_class_uid =
            Some(event.sop_class_uid.trim().trim_end_matches('\0').to_string());
        let instance_number = event.dataset.get_i32(tags::INSTANCE_NUMBER);
        let rows = event.dataset.get_u16(tags::ROWS).map(|v| v as i32);
        let columns = event.dataset.get_u16(tags::COLUMNS).map(|v| v as i32);

        let instance = Instance {
            instance_uid: instance_uid.clone(),
            series_uid: series_uid.clone(),
            study_uid: study_uid.clone(),
            sop_class_uid,
            instance_number,
            transfer_syntax: None,
            rows,
            columns,
            blob_key,
            metadata: DicomJson::empty(),
            created_at: None,
        };

        // ── Persist metadata ───────────────────────────────────────────────
        if let Err(e) = self.store.store_study(&study).await {
            error!(error = %e, study_uid = %study_uid, "Failed to store study metadata");
            return StoreResult::failure(STATUS_PROCESSING_FAILURE);
        }
        if let Err(e) = self.store.store_series(&series).await {
            error!(error = %e, series_uid = %series_uid, "Failed to store series metadata");
            return StoreResult::failure(STATUS_PROCESSING_FAILURE);
        }
        if let Err(e) = self.store.store_instance(&instance).await {
            error!(error = %e, instance_uid = %instance_uid, "Failed to store instance metadata");
            return StoreResult::failure(STATUS_PROCESSING_FAILURE);
        }

        StoreResult::success()
    }
}

// ── C-FIND / C-GET / C-MOVE provider ─────────────────────────────────────────

/// SCP handler for C-FIND, C-GET, and C-MOVE operations.
///
/// Queries the [`MetadataStore`] to find matching instances and retrieves
/// their encoded bytes from the [`BlobStore`] for C-GET/C-MOVE.
pub struct PacsQueryProvider {
    store: Arc<dyn MetadataStore>,
    blobs: Arc<dyn BlobStore>,
}

impl PacsQueryProvider {
    /// Creates a new `PacsQueryProvider`.
    pub fn new(store: Arc<dyn MetadataStore>, blobs: Arc<dyn BlobStore>) -> Self {
        Self { store, blobs }
    }
}

impl FindServiceProvider for PacsQueryProvider {
    #[instrument(skip(self, event), fields(
        sop_class = %event.sop_class_uid,
        calling_ae = %event.calling_ae,
    ))]
    async fn on_find(&self, event: FindEvent) -> Vec<DataSet> {
        debug!("C-FIND received");
        let patient_id = event
            .identifier
            .get_string(tags::PATIENT_ID)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let patient_name = event
            .identifier
            .get_string(tags::PATIENT_NAME)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let study_uid = event
            .identifier
            .get_string(tags::STUDY_INSTANCE_UID)
            .map(|s| s.trim().trim_end_matches('\0'))
            .filter(|s| !s.is_empty())
            .map(StudyUid::from);

        let accession_number = event
            .identifier
            .get_string(tags::ACCESSION_NUMBER)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let modality = event
            .identifier
            .get_string(tags::MODALITY)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let query = StudyQuery {
            patient_id,
            patient_name,
            study_uid,
            accession_number,
            modality,
            ..StudyQuery::default()
        };

        match self.store.query_studies(&query).await {
            Ok(studies) => studies.iter().map(build_study_response).collect(),
            Err(e) => {
                error!(error = %e, "C-FIND study query failed");
                Vec::new()
            }
        }
    }
}

impl GetServiceProvider for PacsQueryProvider {
    #[instrument(skip(self, event), fields(
        sop_class = %event.sop_class_uid,
        calling_ae = %event.calling_ae,
    ))]
    async fn on_get(&self, event: GetEvent) -> Vec<RetrieveItem> {
        debug!("C-GET received");
        retrieve_items_for(
            self.store.as_ref(),
            self.blobs.as_ref(),
            &event.identifier,
        )
        .await
    }
}

impl MoveServiceProvider for PacsQueryProvider {
    #[instrument(skip(self, event), fields(
        sop_class = %event.sop_class_uid,
        calling_ae = %event.calling_ae,
        destination = %event.destination,
    ))]
    async fn on_move(&self, event: MoveEvent) -> Vec<RetrieveItem> {
        debug!("C-MOVE received");
        retrieve_items_for(
            self.store.as_ref(),
            self.blobs.as_ref(),
            &event.identifier,
        )
        .await
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Retrieves all instances matching the query identifier dataset.
async fn retrieve_items_for(
    store: &dyn MetadataStore,
    blobs: &dyn BlobStore,
    identifier: &DataSet,
) -> Vec<RetrieveItem> {
    let study_uid_str = identifier
        .get_string(tags::STUDY_INSTANCE_UID)
        .map(|s| s.trim().trim_end_matches('\0').to_string())
        .unwrap_or_default();

    let series_uid_str = identifier
        .get_string(tags::SERIES_INSTANCE_UID)
        .map(|s| s.trim().trim_end_matches('\0').to_string())
        .unwrap_or_default();

    if study_uid_str.is_empty() && series_uid_str.is_empty() {
        warn!("C-GET/C-MOVE identifier is missing both Study and Series UIDs");
        return Vec::new();
    }

    // Determine which series to retrieve.
    let study_uid = StudyUid::from(study_uid_str.as_str());
    let series_list: Vec<pacs_core::Series> = {
        let q = SeriesQuery {
            study_uid: study_uid.clone(),
            series_uid: if series_uid_str.is_empty() {
                None
            } else {
                Some(SeriesUid::from(series_uid_str.as_str()))
            },
            modality: None,
            series_number: None,
            limit: None,
            offset: None,
        };
        match store.query_series(&q).await {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "Failed to query series for retrieve");
                return Vec::new();
            }
        }
    };

    let mut items = Vec::new();

    for series in &series_list {
        let iq = InstanceQuery {
            series_uid: series.series_uid.clone(),
            instance_uid: None,
            sop_class_uid: None,
            instance_number: None,
            limit: None,
            offset: None,
        };

        let instances = match store.query_instances(&iq).await {
            Ok(insts) => insts,
            Err(e) => {
                error!(
                    error = %e,
                    series_uid = %series.series_uid,
                    "Failed to query instances for retrieve"
                );
                continue;
            }
        };

        for inst in instances {
            match blobs.get(&inst.blob_key).await {
                Ok(data) => {
                    items.push(RetrieveItem {
                        sop_class_uid: inst.sop_class_uid.unwrap_or_default(),
                        sop_instance_uid: inst.instance_uid.to_string(),
                        dataset: data.to_vec(),
                    });
                }
                Err(e) => {
                    error!(
                        error = %e,
                        instance_uid = %inst.instance_uid,
                        "Failed to retrieve instance blob"
                    );
                }
            }
        }
    }

    items
}

/// Builds a C-FIND result [`DataSet`] from a [`Study`].
fn build_study_response(study: &Study) -> DataSet {
    let mut ds = DataSet::new();

    if let Some(pid) = &study.patient_id {
        ds.set_string(tags::PATIENT_ID, Vr::LO, pid);
    }
    if let Some(name) = &study.patient_name {
        ds.set_string(tags::PATIENT_NAME, Vr::PN, name);
    }
    ds.set_uid(tags::STUDY_INSTANCE_UID, study.study_uid.as_ref());
    if let Some(acc) = &study.accession_number {
        ds.set_string(tags::ACCESSION_NUMBER, Vr::SH, acc);
    }
    if let Some(desc) = &study.description {
        ds.set_string(tags::STUDY_DESCRIPTION, Vr::LO, desc);
    }
    if let Some(ref_phys) = &study.referring_physician {
        ds.set_string(tags::REFERRING_PHYSICIAN_NAME, Vr::PN, ref_phys);
    }
    if let Some(time) = &study.study_time {
        ds.set_string(tags::STUDY_TIME, Vr::TM, time);
    }
    if !study.modalities.is_empty() {
        // Multiple modalities are backslash-separated in DICOM CS.
        ds.set_string(tags::MODALITY, Vr::CS, &study.modalities.join("\\"));
    }

    ds
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::mock;
    use pacs_core::{PacsResult, PacsStatistics, SeriesUid, StudyUid};

    // ── Mock MetadataStore ────────────────────────────────────────────────────

    mock! {
        pub TestStore {}

        #[async_trait::async_trait]
        impl MetadataStore for TestStore {
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
        }
    }

    // ── Mock BlobStore ────────────────────────────────────────────────────────

    mock! {
        pub TestBlobs {}

        #[async_trait::async_trait]
        impl BlobStore for TestBlobs {
            async fn put(&self, key: &str, data: Bytes) -> PacsResult<()>;
            async fn get(&self, key: &str) -> PacsResult<Bytes>;
            async fn delete(&self, key: &str) -> PacsResult<()>;
            async fn exists(&self, key: &str) -> PacsResult<bool>;
            async fn presigned_url(&self, key: &str, ttl_secs: u32) -> PacsResult<String>;
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn minimal_store_event() -> StoreEvent {
        let mut ds = DataSet::new();
        ds.set_uid(tags::STUDY_INSTANCE_UID, "1.2.3.4");
        ds.set_uid(tags::SERIES_INSTANCE_UID, "1.2.3.4.1");
        ds.set_string(tags::PATIENT_ID, Vr::LO, "P001");
        ds.set_string(tags::PATIENT_NAME, Vr::PN, "Test^Patient");
        ds.set_string(tags::MODALITY, Vr::CS, "CT");
        StoreEvent {
            calling_ae: "TESTSCU".into(),
            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".into(),
            sop_instance_uid: "1.2.3.4.1.1".into(),
            dataset: ds,
        }
    }

    // ── PacsStoreProvider tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn on_store_success() {
        let mut mock_store = MockTestStore::new();
        mock_store.expect_store_study().once().returning(|_| Ok(()));
        mock_store.expect_store_series().once().returning(|_| Ok(()));
        mock_store
            .expect_store_instance()
            .once()
            .returning(|_| Ok(()));

        let mut mock_blobs = MockTestBlobs::new();
        mock_blobs
            .expect_put()
            .once()
            .returning(|_, _| Ok(()));

        let provider =
            PacsStoreProvider::new(Arc::new(mock_store), Arc::new(mock_blobs));

        let result = provider.on_store(minimal_store_event()).await;
        assert_eq!(result.status, 0x0000, "Expected success status");
    }

    #[tokio::test]
    async fn on_store_missing_study_uid_returns_failure() {
        let mock_store = MockTestStore::new();
        let mock_blobs = MockTestBlobs::new();

        let provider =
            PacsStoreProvider::new(Arc::new(mock_store), Arc::new(mock_blobs));

        let event = StoreEvent {
            calling_ae: "TESTSCU".into(),
            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".into(),
            sop_instance_uid: "1.2.3".into(),
            dataset: DataSet::new(), // no STUDY_INSTANCE_UID
        };

        let result = provider.on_store(event).await;
        assert_ne!(result.status, 0x0000, "Expected failure status");
    }

    #[tokio::test]
    async fn on_store_blob_failure_returns_failure() {
        let mock_store = MockTestStore::new();
        let mut mock_blobs = MockTestBlobs::new();
        mock_blobs.expect_put().once().returning(|_, _| {
            Err(pacs_core::PacsError::Internal("blob unavailable".into()))
        });

        let provider =
            PacsStoreProvider::new(Arc::new(mock_store), Arc::new(mock_blobs));

        let result = provider.on_store(minimal_store_event()).await;
        assert_ne!(result.status, 0x0000, "Expected failure when blob store fails");
    }

    #[tokio::test]
    async fn on_store_metadata_failure_returns_failure() {
        let mut mock_store = MockTestStore::new();
        mock_store
            .expect_store_study()
            .once()
            .returning(|_| Err(pacs_core::PacsError::Internal("db down".into())));

        let mut mock_blobs = MockTestBlobs::new();
        mock_blobs.expect_put().once().returning(|_, _| Ok(()));

        let provider =
            PacsStoreProvider::new(Arc::new(mock_store), Arc::new(mock_blobs));

        let result = provider.on_store(minimal_store_event()).await;
        assert_ne!(result.status, 0x0000, "Expected failure when metadata store fails");
    }

    // ── build_study_response tests ────────────────────────────────────────────

    #[test]
    fn build_study_response_sets_study_uid() {
        let study = Study {
            study_uid: StudyUid::from("1.2.3.4"),
            patient_id: None,
            patient_name: None,
            study_date: None,
            study_time: None,
            accession_number: None,
            modalities: Vec::new(),
            referring_physician: None,
            description: None,
            num_series: 0,
            num_instances: 0,
            metadata: DicomJson::empty(),
            created_at: None,
            updated_at: None,
        };

        let ds = build_study_response(&study);
        assert_eq!(
            ds.get_string(tags::STUDY_INSTANCE_UID),
            Some("1.2.3.4"),
        );
    }

    #[test]
    fn build_study_response_sets_patient_fields() {
        let study = Study {
            study_uid: StudyUid::from("1.2.3"),
            patient_id: Some("P002".into()),
            patient_name: Some("Doe^Jane".into()),
            study_date: None,
            study_time: None,
            accession_number: Some("ACC001".into()),
            modalities: vec!["MR".into()],
            referring_physician: None,
            description: None,
            num_series: 0,
            num_instances: 0,
            metadata: DicomJson::empty(),
            created_at: None,
            updated_at: None,
        };

        let ds = build_study_response(&study);
        assert_eq!(ds.get_string(tags::PATIENT_ID), Some("P002"));
        assert_eq!(ds.get_string(tags::PATIENT_NAME), Some("Doe^Jane"));
        assert_eq!(ds.get_string(tags::ACCESSION_NUMBER), Some("ACC001"));
    }
}
