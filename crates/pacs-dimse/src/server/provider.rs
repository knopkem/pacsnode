//! SCP service provider implementations backed by the pacsnode storage layer.

use std::{collections::HashSet, sync::Arc};

use bytes::Bytes;
use chrono::NaiveDate;
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
use pacs_dicom::tags::{
    dataset_to_dicom_json, date_display_string, optional_i32, optional_i32_from_u16,
    optional_string, parse_dicom_date, BODY_PART_EXAMINED, MODALITIES_IN_STUDY,
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

        // ── Generate DICOM JSON metadata from the received dataset ────────
        let metadata = match dataset_to_dicom_json(&event.dataset) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    error = %e,
                    sop_instance_uid = %sop_instance_uid_str,
                    "Failed to serialise received dataset to DICOM JSON; storing empty metadata"
                );
                DicomJson::empty()
            }
        };

        // ── Build domain objects from dataset attributes ───────────────────
        let patient_id = optional_string(&event.dataset, tags::PATIENT_ID);
        let patient_name = optional_string(&event.dataset, tags::PATIENT_NAME);
        let accession_number = optional_string(&event.dataset, tags::ACCESSION_NUMBER);
        let study_time = optional_string(&event.dataset, tags::STUDY_TIME);
        let study_date = date_display_string(&event.dataset, tags::STUDY_DATE)
            .and_then(|s| parse_dicom_date(&s).ok());
        let modalities: Vec<String> = event
            .dataset
            .get_strings(MODALITIES_IN_STUDY)
            .map(|v| v.to_vec())
            .or_else(|| optional_string(&event.dataset, tags::MODALITY).map(|m| vec![m]))
            .unwrap_or_default();
        let referring_physician = optional_string(&event.dataset, tags::REFERRING_PHYSICIAN_NAME);
        let study_description = optional_string(&event.dataset, tags::STUDY_DESCRIPTION);

        let study = Study {
            study_uid: study_uid.clone(),
            patient_id,
            patient_name,
            study_date,
            study_time,
            accession_number,
            modalities,
            referring_physician,
            description: study_description,
            num_series: 1,
            num_instances: 1,
            metadata: metadata.clone(),
            created_at: None,
            updated_at: None,
        };

        let modality = optional_string(&event.dataset, tags::MODALITY);
        let series_number = optional_i32(&event.dataset, tags::SERIES_NUMBER);
        let series_description = optional_string(&event.dataset, tags::SERIES_DESCRIPTION);
        let body_part = optional_string(&event.dataset, BODY_PART_EXAMINED);

        let series = Series {
            series_uid: series_uid.clone(),
            study_uid: study_uid.clone(),
            modality,
            series_number,
            description: series_description,
            body_part,
            num_instances: 1,
            metadata: metadata.clone(),
            created_at: None,
        };

        let sop_class_uid = Some(
            event
                .sop_class_uid
                .trim()
                .trim_end_matches('\0')
                .to_string(),
        );
        let instance_number = optional_i32(&event.dataset, tags::INSTANCE_NUMBER);
        let rows = optional_i32_from_u16(&event.dataset, tags::ROWS);
        let columns = optional_i32_from_u16(&event.dataset, tags::COLUMNS);

        let instance = Instance {
            instance_uid: instance_uid.clone(),
            series_uid: series_uid.clone(),
            study_uid: study_uid.clone(),
            sop_class_uid,
            instance_number,
            transfer_syntax: Some("1.2.840.10008.1.2.1".to_string()),
            rows,
            columns,
            blob_key,
            metadata,
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

const DIMSE_FIND_RESULT_LIMIT: u32 = 10_000;

impl FindServiceProvider for PacsQueryProvider {
    #[instrument(skip(self, event), fields(
        sop_class = %event.sop_class_uid,
        calling_ae = %event.calling_ae,
    ))]
    async fn on_find(&self, event: FindEvent) -> Vec<DataSet> {
        let Some(level) = QueryRetrieveLevel::from_identifier(&event.identifier) else {
            warn!("C-FIND request missing or using unsupported QueryRetrieveLevel");
            return Vec::new();
        };

        debug!(query_level = level.as_str(), "C-FIND received");
        let keys = FindQueryKeys::from_identifier(&event.identifier);

        let studies = match self.store.query_studies(&keys.study_query()).await {
            Ok(studies) => studies,
            Err(e) => {
                error!(error = %e, query_level = level.as_str(), "C-FIND study query failed");
                return Vec::new();
            }
        };

        match level {
            QueryRetrieveLevel::Patient => build_patient_responses(&studies),
            QueryRetrieveLevel::Study => studies.iter().map(build_study_response).collect(),
            QueryRetrieveLevel::Series => {
                query_series_matches(self.store.as_ref(), &studies, &keys).await
            }
            QueryRetrieveLevel::Image => {
                query_instance_matches(self.store.as_ref(), &studies, &keys).await
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
        retrieve_items_for(self.store.as_ref(), self.blobs.as_ref(), &event.identifier).await
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
        retrieve_items_for(self.store.as_ref(), self.blobs.as_ref(), &event.identifier).await
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryRetrieveLevel {
    Patient,
    Study,
    Series,
    Image,
}

impl QueryRetrieveLevel {
    fn from_identifier(identifier: &DataSet) -> Option<Self> {
        let level = normalized_identifier_string(identifier, tags::QUERY_RETRIEVE_LEVEL)?
            .to_ascii_uppercase();

        match level.as_str() {
            "PATIENT" => Some(Self::Patient),
            "STUDY" => Some(Self::Study),
            "SERIES" => Some(Self::Series),
            "IMAGE" | "INSTANCE" => Some(Self::Image),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Patient => "PATIENT",
            Self::Study => "STUDY",
            Self::Series => "SERIES",
            Self::Image => "IMAGE",
        }
    }
}

#[derive(Debug, Clone)]
struct FindQueryKeys {
    patient_id: Option<String>,
    patient_name: Option<String>,
    study_date_from: Option<NaiveDate>,
    study_date_to: Option<NaiveDate>,
    accession_number: Option<String>,
    study_uid: Option<StudyUid>,
    modality: Option<String>,
    series_uid: Option<SeriesUid>,
    series_number: Option<i32>,
    instance_uid: Option<SopInstanceUid>,
    sop_class_uid: Option<String>,
    instance_number: Option<i32>,
}

impl FindQueryKeys {
    fn from_identifier(identifier: &DataSet) -> Self {
        let (study_date_from, study_date_to) =
            parse_dicom_date_range(normalized_identifier_string(identifier, tags::STUDY_DATE));

        Self {
            patient_id: normalized_identifier_string(identifier, tags::PATIENT_ID),
            patient_name: normalized_identifier_string(identifier, tags::PATIENT_NAME),
            study_date_from,
            study_date_to,
            accession_number: normalized_identifier_string(identifier, tags::ACCESSION_NUMBER),
            study_uid: normalized_identifier_string(identifier, tags::STUDY_INSTANCE_UID)
                .map(StudyUid::from),
            modality: normalized_identifier_string(identifier, MODALITIES_IN_STUDY)
                .or_else(|| normalized_identifier_string(identifier, tags::MODALITY)),
            series_uid: normalized_identifier_string(identifier, tags::SERIES_INSTANCE_UID)
                .map(SeriesUid::from),
            series_number: normalized_identifier_i32(identifier, tags::SERIES_NUMBER),
            instance_uid: normalized_identifier_string(identifier, tags::SOP_INSTANCE_UID)
                .map(SopInstanceUid::from),
            sop_class_uid: normalized_identifier_string(identifier, tags::SOP_CLASS_UID),
            instance_number: normalized_identifier_i32(identifier, tags::INSTANCE_NUMBER),
        }
    }

    fn study_query(&self) -> StudyQuery {
        StudyQuery {
            patient_id: self.patient_id.clone(),
            patient_name: self.patient_name.clone().map(|name| name.replace('?', "_")),
            study_date_from: self.study_date_from,
            study_date_to: self.study_date_to,
            accession_number: self.accession_number.clone(),
            study_uid: self.study_uid.clone(),
            modality: self.modality.clone(),
            limit: Some(DIMSE_FIND_RESULT_LIMIT),
            offset: Some(0),
            include_fields: vec![],
            fuzzy_matching: self
                .patient_name
                .as_deref()
                .is_some_and(contains_dicom_wildcards),
        }
    }

    fn series_query(&self, study_uid: StudyUid) -> SeriesQuery {
        SeriesQuery {
            study_uid,
            series_uid: self.series_uid.clone(),
            modality: self.modality.clone(),
            series_number: self.series_number,
            limit: Some(DIMSE_FIND_RESULT_LIMIT),
            offset: Some(0),
        }
    }

    fn instance_query(&self, series_uid: SeriesUid) -> InstanceQuery {
        InstanceQuery {
            series_uid,
            instance_uid: self.instance_uid.clone(),
            sop_class_uid: self.sop_class_uid.clone(),
            instance_number: self.instance_number,
            limit: Some(DIMSE_FIND_RESULT_LIMIT),
            offset: Some(0),
        }
    }
}

fn contains_dicom_wildcards(value: &str) -> bool {
    value.contains('*') || value.contains('?')
}

fn normalized_identifier_string(
    identifier: &DataSet,
    tag: dicom_toolkit_dict::Tag,
) -> Option<String> {
    identifier
        .get_string(tag)
        .map(|s| s.trim().trim_end_matches('\0').to_string())
        .filter(|s| !s.is_empty())
}

fn normalized_identifier_i32(identifier: &DataSet, tag: dicom_toolkit_dict::Tag) -> Option<i32> {
    identifier
        .get_i32(tag)
        .or_else(|| identifier.get_u16(tag).map(i32::from))
}

fn parse_dicom_date_range(value: Option<String>) -> (Option<NaiveDate>, Option<NaiveDate>) {
    match value {
        None => (None, None),
        Some(value) => {
            if let Some((from, to)) = value.split_once('-') {
                let from = if from.trim().is_empty() {
                    None
                } else {
                    parse_dicom_date(from.trim()).ok()
                };
                let to = if to.trim().is_empty() {
                    None
                } else {
                    parse_dicom_date(to.trim()).ok()
                };
                (from, to)
            } else {
                let date = parse_dicom_date(value.trim()).ok();
                (date, date)
            }
        }
    }
}

async fn query_series_matches(
    store: &dyn MetadataStore,
    studies: &[Study],
    keys: &FindQueryKeys,
) -> Vec<DataSet> {
    let mut responses = Vec::new();

    for study in studies {
        match store
            .query_series(&keys.series_query(study.study_uid.clone()))
            .await
        {
            Ok(series_list) => responses.extend(
                series_list
                    .iter()
                    .map(|series| build_series_response(study, series)),
            ),
            Err(e) => {
                error!(
                    error = %e,
                    study_uid = %study.study_uid,
                    "C-FIND series query failed"
                );
            }
        }
    }

    responses
}

async fn query_instance_matches(
    store: &dyn MetadataStore,
    studies: &[Study],
    keys: &FindQueryKeys,
) -> Vec<DataSet> {
    let mut responses = Vec::new();

    for study in studies {
        let series_list = match store
            .query_series(&keys.series_query(study.study_uid.clone()))
            .await
        {
            Ok(series_list) => series_list,
            Err(e) => {
                error!(
                    error = %e,
                    study_uid = %study.study_uid,
                    "C-FIND series query failed while resolving image matches"
                );
                continue;
            }
        };

        for series in &series_list {
            match store
                .query_instances(&keys.instance_query(series.series_uid.clone()))
                .await
            {
                Ok(instances) => responses.extend(
                    instances
                        .iter()
                        .map(|instance| build_instance_response(study, series, instance)),
                ),
                Err(e) => {
                    error!(
                        error = %e,
                        study_uid = %study.study_uid,
                        series_uid = %series.series_uid,
                        "C-FIND instance query failed"
                    );
                }
            }
        }
    }

    responses
}

fn build_patient_responses(studies: &[Study]) -> Vec<DataSet> {
    let mut seen = HashSet::new();
    let mut responses = Vec::new();

    for study in studies {
        let Some(key) = patient_response_key(study) else {
            continue;
        };

        if seen.insert(key) {
            responses.push(build_patient_response(study));
        }
    }

    responses
}

fn patient_response_key(study: &Study) -> Option<String> {
    if study.patient_id.is_none() && study.patient_name.is_none() {
        return None;
    }

    Some(format!(
        "{}\0{}",
        study.patient_id.as_deref().unwrap_or(""),
        study.patient_name.as_deref().unwrap_or("")
    ))
}

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

fn build_patient_response(study: &Study) -> DataSet {
    let mut ds = DataSet::new();
    ds.set_string(
        tags::QUERY_RETRIEVE_LEVEL,
        Vr::CS,
        QueryRetrieveLevel::Patient.as_str(),
    );
    add_patient_fields(&mut ds, study);
    ds
}

/// Builds a C-FIND result [`DataSet`] from a [`Study`].
fn build_study_response(study: &Study) -> DataSet {
    let mut ds = DataSet::new();
    ds.set_string(
        tags::QUERY_RETRIEVE_LEVEL,
        Vr::CS,
        QueryRetrieveLevel::Study.as_str(),
    );
    add_study_fields(&mut ds, study);
    ds
}

fn build_series_response(study: &Study, series: &Series) -> DataSet {
    let mut ds = DataSet::new();
    ds.set_string(
        tags::QUERY_RETRIEVE_LEVEL,
        Vr::CS,
        QueryRetrieveLevel::Series.as_str(),
    );
    add_study_fields(&mut ds, study);
    add_series_fields(&mut ds, series);
    ds
}

fn build_instance_response(study: &Study, series: &Series, instance: &Instance) -> DataSet {
    let mut ds = DataSet::new();
    ds.set_string(
        tags::QUERY_RETRIEVE_LEVEL,
        Vr::CS,
        QueryRetrieveLevel::Image.as_str(),
    );
    add_study_fields(&mut ds, study);
    add_series_fields(&mut ds, series);
    add_instance_fields(&mut ds, instance);
    ds
}

fn add_patient_fields(ds: &mut DataSet, study: &Study) {
    if let Some(pid) = &study.patient_id {
        ds.set_string(tags::PATIENT_ID, Vr::LO, pid);
    }
    if let Some(name) = &study.patient_name {
        ds.set_string(tags::PATIENT_NAME, Vr::PN, name);
    }
}

fn add_study_fields(ds: &mut DataSet, study: &Study) {
    add_patient_fields(ds, study);
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
    if let Some(date) = study.study_date {
        ds.set_string(tags::STUDY_DATE, Vr::DA, &date.format("%Y%m%d").to_string());
    }
    if let Some(time) = &study.study_time {
        ds.set_string(tags::STUDY_TIME, Vr::TM, time);
    }
    if !study.modalities.is_empty() {
        ds.set_strings(MODALITIES_IN_STUDY, Vr::CS, study.modalities.clone());
    }
}

fn add_series_fields(ds: &mut DataSet, series: &Series) {
    ds.set_uid(tags::SERIES_INSTANCE_UID, series.series_uid.as_ref());
    if let Some(modality) = &series.modality {
        ds.set_string(tags::MODALITY, Vr::CS, modality);
    }
    if let Some(series_number) = series.series_number {
        ds.set_string(tags::SERIES_NUMBER, Vr::IS, &series_number.to_string());
    }
    if let Some(description) = &series.description {
        ds.set_string(tags::SERIES_DESCRIPTION, Vr::LO, description);
    }
    if let Some(body_part) = &series.body_part {
        ds.set_string(BODY_PART_EXAMINED, Vr::CS, body_part);
    }
}

fn add_instance_fields(ds: &mut DataSet, instance: &Instance) {
    ds.set_uid(tags::SOP_INSTANCE_UID, instance.instance_uid.as_ref());
    if let Some(sop_class_uid) = &instance.sop_class_uid {
        ds.set_uid(tags::SOP_CLASS_UID, sop_class_uid);
    }
    if let Some(instance_number) = instance.instance_number {
        ds.set_string(tags::INSTANCE_NUMBER, Vr::IS, &instance_number.to_string());
    }
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
            async fn list_nodes(&self) -> PacsResult<Vec<pacs_core::DicomNode>>;
            async fn upsert_node(&self, node: &pacs_core::DicomNode) -> PacsResult<()>;
            async fn delete_node(&self, ae_title: &str) -> PacsResult<()>;
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

    fn sample_study(uid: &str) -> Study {
        Study {
            study_uid: StudyUid::from(uid),
            patient_id: Some("P001".into()),
            patient_name: Some("Test^Patient".into()),
            study_date: NaiveDate::from_ymd_opt(2024, 1, 2),
            study_time: Some("101112".into()),
            accession_number: Some("ACC001".into()),
            modalities: vec!["CT".into(), "MR".into()],
            referring_physician: Some("Ref^Doctor".into()),
            description: Some("Test Study".into()),
            num_series: 1,
            num_instances: 1,
            metadata: DicomJson::empty(),
            created_at: None,
            updated_at: None,
        }
    }

    fn sample_series(study_uid: &str, series_uid: &str) -> Series {
        Series {
            study_uid: StudyUid::from(study_uid),
            series_uid: SeriesUid::from(series_uid),
            modality: Some("CT".into()),
            series_number: Some(7),
            description: Some("Axial".into()),
            body_part: Some("CHEST".into()),
            num_instances: 1,
            metadata: DicomJson::empty(),
            created_at: None,
        }
    }

    fn sample_instance(study_uid: &str, series_uid: &str, instance_uid: &str) -> Instance {
        Instance {
            instance_uid: SopInstanceUid::from(instance_uid),
            series_uid: SeriesUid::from(series_uid),
            study_uid: StudyUid::from(study_uid),
            sop_class_uid: Some("1.2.840.10008.5.1.4.1.1.2".into()),
            instance_number: Some(3),
            transfer_syntax: Some("1.2.840.10008.1.2.1".into()),
            rows: Some(512),
            columns: Some(512),
            blob_key: format!("{study_uid}/{series_uid}/{instance_uid}"),
            metadata: DicomJson::empty(),
            created_at: None,
        }
    }

    fn find_event(level: &str) -> FindEvent {
        let mut identifier = DataSet::new();
        identifier.set_string(tags::QUERY_RETRIEVE_LEVEL, Vr::CS, level);
        FindEvent {
            calling_ae: "TESTSCU".into(),
            sop_class_uid: "1.2.840.10008.5.1.4.1.2.2.1".into(),
            identifier,
        }
    }

    // ── PacsStoreProvider tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn on_store_success() {
        let mut mock_store = MockTestStore::new();
        mock_store.expect_store_study().once().returning(|_| Ok(()));
        mock_store
            .expect_store_series()
            .once()
            .returning(|_| Ok(()));
        mock_store
            .expect_store_instance()
            .once()
            .returning(|_| Ok(()));

        let mut mock_blobs = MockTestBlobs::new();
        mock_blobs.expect_put().once().returning(|_, _| Ok(()));

        let provider = PacsStoreProvider::new(Arc::new(mock_store), Arc::new(mock_blobs));

        let result = provider.on_store(minimal_store_event()).await;
        assert_eq!(result.status, 0x0000, "Expected success status");
    }

    #[tokio::test]
    async fn on_store_missing_study_uid_returns_failure() {
        let mock_store = MockTestStore::new();
        let mock_blobs = MockTestBlobs::new();

        let provider = PacsStoreProvider::new(Arc::new(mock_store), Arc::new(mock_blobs));

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
        mock_blobs
            .expect_put()
            .once()
            .returning(|_, _| Err(pacs_core::PacsError::Internal("blob unavailable".into())));

        let provider = PacsStoreProvider::new(Arc::new(mock_store), Arc::new(mock_blobs));

        let result = provider.on_store(minimal_store_event()).await;
        assert_ne!(
            result.status, 0x0000,
            "Expected failure when blob store fails"
        );
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

        let provider = PacsStoreProvider::new(Arc::new(mock_store), Arc::new(mock_blobs));

        let result = provider.on_store(minimal_store_event()).await;
        assert_ne!(
            result.status, 0x0000,
            "Expected failure when metadata store fails"
        );
    }

    // ── PacsQueryProvider tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn on_find_patient_level_deduplicates_patients() {
        let first = sample_study("1.2.3.4");
        let second = Study {
            study_uid: StudyUid::from("1.2.3.5"),
            ..first.clone()
        };

        let mut mock_store = MockTestStore::new();
        mock_store
            .expect_query_studies()
            .once()
            .returning(move |_| Ok(vec![first.clone(), second.clone()]));

        let provider = PacsQueryProvider::new(Arc::new(mock_store), Arc::new(MockTestBlobs::new()));

        let results = provider.on_find(find_event("PATIENT")).await;
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].get_string(tags::QUERY_RETRIEVE_LEVEL),
            Some("PATIENT")
        );
        assert_eq!(results[0].get_string(tags::PATIENT_ID), Some("P001"));
        assert_eq!(
            results[0].get_string(tags::PATIENT_NAME),
            Some("Test^Patient")
        );
    }

    #[tokio::test]
    async fn on_find_study_level_parses_wildcards_and_date_ranges() {
        let expected_from = NaiveDate::from_ymd_opt(2024, 1, 2);
        let expected_to = NaiveDate::from_ymd_opt(2024, 1, 31);

        let mut mock_store = MockTestStore::new();
        mock_store
            .expect_query_studies()
            .once()
            .withf(move |q| {
                q.patient_name.as_deref() == Some("Doe*")
                    && q.fuzzy_matching
                    && q.study_date_from == expected_from
                    && q.study_date_to == expected_to
                    && q.limit == Some(DIMSE_FIND_RESULT_LIMIT)
                    && q.offset == Some(0)
            })
            .returning(|_| Ok(vec![]));

        let provider = PacsQueryProvider::new(Arc::new(mock_store), Arc::new(MockTestBlobs::new()));

        let mut event = find_event("STUDY");
        event
            .identifier
            .set_string(tags::PATIENT_NAME, Vr::PN, "Doe*");
        event
            .identifier
            .set_string(tags::STUDY_DATE, Vr::DA, "20240102-20240131");

        let results = provider.on_find(event).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn on_find_series_level_returns_series_matches() {
        let study = sample_study("1.2.3.4");
        let series = sample_series("1.2.3.4", "1.2.3.4.1");

        let mut mock_store = MockTestStore::new();
        mock_store
            .expect_query_studies()
            .once()
            .withf(|q| q.study_uid.as_ref().map(|uid| uid.as_ref()) == Some("1.2.3.4"))
            .returning(move |_| Ok(vec![study.clone()]));
        mock_store
            .expect_query_series()
            .once()
            .withf(|q| {
                q.study_uid.as_ref() == "1.2.3.4"
                    && q.series_number == Some(7)
                    && q.limit == Some(DIMSE_FIND_RESULT_LIMIT)
            })
            .returning(move |_| Ok(vec![series.clone()]));

        let provider = PacsQueryProvider::new(Arc::new(mock_store), Arc::new(MockTestBlobs::new()));

        let mut event = find_event("SERIES");
        event
            .identifier
            .set_uid(tags::STUDY_INSTANCE_UID, "1.2.3.4");
        event.identifier.set_i32(tags::SERIES_NUMBER, 7);

        let results = provider.on_find(event).await;
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].get_string(tags::QUERY_RETRIEVE_LEVEL),
            Some("SERIES")
        );
        assert_eq!(
            results[0].get_string(tags::STUDY_INSTANCE_UID),
            Some("1.2.3.4")
        );
        assert_eq!(
            results[0].get_string(tags::SERIES_INSTANCE_UID),
            Some("1.2.3.4.1")
        );
        assert_eq!(results[0].get_string(tags::MODALITY), Some("CT"));
    }

    #[tokio::test]
    async fn on_find_image_level_returns_instance_matches() {
        let study = sample_study("1.2.3.4");
        let series = sample_series("1.2.3.4", "1.2.3.4.1");
        let instance = sample_instance("1.2.3.4", "1.2.3.4.1", "1.2.3.4.1.1");

        let mut mock_store = MockTestStore::new();
        mock_store
            .expect_query_studies()
            .once()
            .withf(|q| q.study_uid.as_ref().map(|uid| uid.as_ref()) == Some("1.2.3.4"))
            .returning(move |_| Ok(vec![study.clone()]));
        mock_store
            .expect_query_series()
            .once()
            .withf(|q| {
                q.study_uid.as_ref() == "1.2.3.4"
                    && q.series_uid.as_ref().map(|uid| uid.as_ref()) == Some("1.2.3.4.1")
            })
            .returning(move |_| Ok(vec![series.clone()]));
        mock_store
            .expect_query_instances()
            .once()
            .withf(|q| {
                q.series_uid.as_ref() == "1.2.3.4.1"
                    && q.instance_number == Some(3)
                    && q.limit == Some(DIMSE_FIND_RESULT_LIMIT)
            })
            .returning(move |_| Ok(vec![instance.clone()]));

        let provider = PacsQueryProvider::new(Arc::new(mock_store), Arc::new(MockTestBlobs::new()));

        let mut event = find_event("IMAGE");
        event
            .identifier
            .set_uid(tags::STUDY_INSTANCE_UID, "1.2.3.4");
        event
            .identifier
            .set_uid(tags::SERIES_INSTANCE_UID, "1.2.3.4.1");
        event.identifier.set_i32(tags::INSTANCE_NUMBER, 3);

        let results = provider.on_find(event).await;
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].get_string(tags::QUERY_RETRIEVE_LEVEL),
            Some("IMAGE")
        );
        assert_eq!(
            results[0].get_string(tags::STUDY_INSTANCE_UID),
            Some("1.2.3.4")
        );
        assert_eq!(
            results[0].get_string(tags::SERIES_INSTANCE_UID),
            Some("1.2.3.4.1")
        );
        assert_eq!(
            results[0].get_string(tags::SOP_INSTANCE_UID),
            Some("1.2.3.4.1.1")
        );
        assert_eq!(
            results[0].get_string(tags::SOP_CLASS_UID),
            Some("1.2.840.10008.5.1.4.1.1.2")
        );
    }

    // ── C-FIND response builder tests ─────────────────────────────────────────

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
        assert_eq!(ds.get_string(tags::STUDY_INSTANCE_UID), Some("1.2.3.4"),);
        assert_eq!(ds.get_string(tags::QUERY_RETRIEVE_LEVEL), Some("STUDY"));
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
        assert_eq!(
            ds.get_strings(MODALITIES_IN_STUDY)
                .and_then(|values| values.first())
                .map(String::as_str),
            Some("MR")
        );
    }
}
