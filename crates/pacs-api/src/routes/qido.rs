//! QIDO-RS (Query based on ID for DICOM Objects) handlers.

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Json,
};
use chrono::NaiveDate;
use pacs_core::{
    DicomJson, InstanceQuery, Series, SeriesQuery, SeriesUid, SopInstanceUid, Study, StudyQuery,
    StudyUid,
};
use pacs_plugin::{AuthenticatedUser, PacsEvent, QuerySource};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Map, Value};

use crate::{error::ApiError, state::AppState};

/// QIDO-RS query parameters for study-level searches (`GET /wado/studies`).
#[derive(Debug, Default, Deserialize)]
pub struct StudySearchParams {
    /// Filter by Patient ID (0010,0020).
    #[serde(rename = "PatientID")]
    pub patient_id: Option<String>,
    /// Filter by Patient Name (0010,0010).
    #[serde(rename = "PatientName")]
    pub patient_name: Option<String>,
    /// Date range `YYYYMMDD-YYYYMMDD` or single `YYYYMMDD` (0008,0020).
    #[serde(rename = "StudyDate")]
    pub study_date: Option<String>,
    /// Filter by Accession Number (0008,0050).
    #[serde(rename = "AccessionNumber")]
    pub accession_number: Option<String>,
    /// Filter by a specific Study Instance UID.
    #[serde(rename = "StudyInstanceUID")]
    pub study_instance_uid: Option<String>,
    /// Filter by modality.
    #[serde(rename = "Modality")]
    pub modality: Option<String>,
    /// Maximum number of results.
    pub limit: Option<u32>,
    /// Results to skip (pagination).
    pub offset: Option<u32>,
    /// Enable fuzzy matching for string attributes.
    pub fuzzymatching: Option<bool>,
    /// Additional tags to include in the response.
    #[serde(
        rename = "includeField",
        default,
        deserialize_with = "deserialize_include_fields"
    )]
    pub include_fields: Vec<String>,
}

/// QIDO-RS query parameters for series-level searches.
#[derive(Debug, Default, Deserialize)]
pub struct SeriesSearchParams {
    /// Filter by a specific Series Instance UID.
    #[serde(rename = "SeriesInstanceUID")]
    pub series_instance_uid: Option<String>,
    /// Filter by modality (0008,0060).
    #[serde(rename = "Modality")]
    pub modality: Option<String>,
    /// Filter by Series Number (0020,0011).
    #[serde(rename = "SeriesNumber")]
    pub series_number: Option<i32>,
    /// Maximum number of results.
    pub limit: Option<u32>,
    /// Results to skip (pagination).
    pub offset: Option<u32>,
    /// Additional tags to include in the response.
    #[serde(
        rename = "includeField",
        default,
        deserialize_with = "deserialize_include_fields"
    )]
    pub include_fields: Vec<String>,
}

/// QIDO-RS query parameters for instance-level searches.
#[derive(Debug, Default, Deserialize)]
pub struct InstanceSearchParams {
    /// Filter by a specific SOP Instance UID.
    #[serde(rename = "SOPInstanceUID")]
    pub sop_instance_uid: Option<String>,
    /// Filter by SOP Class UID (0008,0016).
    #[serde(rename = "SOPClassUID")]
    pub sop_class_uid: Option<String>,
    /// Filter by Instance Number (0020,0013).
    #[serde(rename = "InstanceNumber")]
    pub instance_number: Option<i32>,
    /// Maximum number of results.
    pub limit: Option<u32>,
    /// Results to skip (pagination).
    pub offset: Option<u32>,
    /// Additional tags to include in the response.
    #[serde(
        rename = "includeField",
        default,
        deserialize_with = "deserialize_include_fields"
    )]
    pub include_fields: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrManyStrings {
    One(String),
    Many(Vec<String>),
}

fn deserialize_include_fields<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<OneOrManyStrings>::deserialize(deserializer)?;
    Ok(match value {
        None => Vec::new(),
        Some(OneOrManyStrings::One(item)) => normalize_include_fields(vec![item]),
        Some(OneOrManyStrings::Many(items)) => normalize_include_fields(items),
    })
}

/// `GET /wado/studies` — QIDO-RS study search.
///
/// Returns a JSON array of DICOM JSON metadata objects (PS3.18 §6.7).
pub async fn search_studies(
    State(state): State<AppState>,
    user: Option<axum::Extension<AuthenticatedUser>>,
    Query(params): Query<StudySearchParams>,
) -> Result<impl IntoResponse, ApiError> {
    let (date_from, date_to) = parse_date_range(params.study_date.as_deref());
    let query = StudyQuery {
        patient_id: params.patient_id,
        patient_name: params.patient_name,
        study_date_from: date_from,
        study_date_to: date_to,
        accession_number: params.accession_number,
        study_uid: params.study_instance_uid.map(StudyUid::from),
        modality: params.modality,
        limit: params.limit,
        offset: params.offset,
        fuzzy_matching: params.fuzzymatching.unwrap_or(false),
        include_fields: params.include_fields.clone(),
    };
    let studies = state.store.query_studies(&query).await?;
    let metadata: Vec<serde_json::Value> = studies
        .iter()
        .map(|study| study_qido_metadata(study, &query.include_fields))
        .collect();
    state
        .plugins
        .emit_event(PacsEvent::QueryPerformed {
            level: "STUDY".into(),
            source: QuerySource::Dicomweb,
            num_results: metadata.len(),
            user_id: user.map(|extension| extension.0.user_id),
        })
        .await;
    Ok(Json(metadata))
}

/// `GET /wado/studies/:study_uid/series` — QIDO-RS series search.
pub async fn search_series(
    State(state): State<AppState>,
    user: Option<axum::Extension<AuthenticatedUser>>,
    Path(study_uid): Path<String>,
    Query(params): Query<SeriesSearchParams>,
) -> Result<impl IntoResponse, ApiError> {
    let query = SeriesQuery {
        study_uid: StudyUid::from(study_uid.as_str()),
        series_uid: params.series_instance_uid.map(SeriesUid::from),
        modality: params.modality,
        series_number: params.series_number,
        limit: params.limit,
        offset: params.offset,
    };
    let series = state.store.query_series(&query).await?;
    let metadata: Vec<serde_json::Value> = series
        .iter()
        .map(|series| series_qido_metadata(series, &params.include_fields))
        .collect();
    state
        .plugins
        .emit_event(PacsEvent::QueryPerformed {
            level: "SERIES".into(),
            source: QuerySource::Dicomweb,
            num_results: metadata.len(),
            user_id: user.map(|extension| extension.0.user_id),
        })
        .await;
    Ok(Json(metadata))
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances` — QIDO-RS instance search.
pub async fn search_instances(
    State(state): State<AppState>,
    user: Option<axum::Extension<AuthenticatedUser>>,
    Path((_study_uid, series_uid)): Path<(String, String)>,
    Query(params): Query<InstanceSearchParams>,
) -> Result<impl IntoResponse, ApiError> {
    let query = InstanceQuery {
        series_uid: SeriesUid::from(series_uid.as_str()),
        instance_uid: params.sop_instance_uid.map(SopInstanceUid::from),
        sop_class_uid: params.sop_class_uid,
        instance_number: params.instance_number,
        limit: params.limit,
        offset: params.offset,
    };
    let instances = state.store.query_instances(&query).await?;
    let metadata: Vec<serde_json::Value> = instances
        .iter()
        .map(|i| i.metadata.as_value().clone())
        .collect();
    state
        .plugins
        .emit_event(PacsEvent::QueryPerformed {
            level: "IMAGE".into(),
            source: QuerySource::Dicomweb,
            num_results: metadata.len(),
            user_id: user.map(|extension| extension.0.user_id),
        })
        .await;
    Ok(Json(metadata))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn study_qido_metadata(study: &Study, include_fields: &[String]) -> Value {
    let mut object = Map::new();
    object.insert(
        "0020000D".into(),
        string_attribute("UI", study.study_uid.to_string()),
    );
    insert_optional_string_attribute(&mut object, "00100020", "LO", study.patient_id.as_deref());
    insert_optional_person_name_attribute(&mut object, "00100010", study.patient_name.as_deref());
    insert_optional_string_attribute(
        &mut object,
        "00080020",
        "DA",
        study
            .study_date
            .as_ref()
            .map(|date| date.format("%Y%m%d").to_string())
            .as_deref(),
    );
    insert_optional_string_attribute(&mut object, "00080030", "TM", study.study_time.as_deref());
    insert_optional_string_attribute(
        &mut object,
        "00080050",
        "SH",
        study.accession_number.as_deref(),
    );
    insert_optional_string_list_attribute(&mut object, "00080061", "CS", &study.modalities);
    insert_optional_person_name_attribute(
        &mut object,
        "00080090",
        study.referring_physician.as_deref(),
    );
    insert_optional_string_attribute(&mut object, "00081030", "LO", study.description.as_deref());
    object.insert(
        "00201206".into(),
        integer_string_attribute(study.num_series),
    );
    object.insert(
        "00201208".into(),
        integer_string_attribute(study.num_instances),
    );
    append_requested_metadata_fields(&mut object, &study.metadata, include_fields);
    Value::Object(object)
}

fn series_qido_metadata(series: &Series, include_fields: &[String]) -> Value {
    let mut object = Map::new();
    object.insert(
        "0020000D".into(),
        string_attribute("UI", series.study_uid.to_string()),
    );
    object.insert(
        "0020000E".into(),
        string_attribute("UI", series.series_uid.to_string()),
    );
    insert_optional_string_attribute(&mut object, "00080060", "CS", series.modality.as_deref());
    insert_optional_integer_string_attribute(&mut object, "00200011", series.series_number);
    insert_optional_string_attribute(&mut object, "0008103E", "LO", series.description.as_deref());
    insert_optional_string_attribute(&mut object, "00180015", "CS", series.body_part.as_deref());
    object.insert(
        "00201209".into(),
        integer_string_attribute(series.num_instances),
    );
    append_requested_metadata_fields(&mut object, &series.metadata, include_fields);
    Value::Object(object)
}

fn normalize_include_fields(values: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for value in values {
        for field in value.split(',') {
            let trimmed = field.trim();
            if trimmed.is_empty() {
                continue;
            }
            let candidate = trimmed.to_ascii_uppercase();
            if candidate == "ALL" {
                return vec![candidate];
            }
            if !normalized.iter().any(|existing| existing == &candidate) {
                normalized.push(candidate);
            }
        }
    }
    normalized
}

fn append_requested_metadata_fields(
    object: &mut Map<String, Value>,
    metadata: &DicomJson,
    include_fields: &[String],
) {
    if include_fields.is_empty() {
        return;
    }

    let Some(metadata_object) = metadata.as_value().as_object() else {
        return;
    };

    if include_fields.iter().any(|field| field == "ALL") {
        for (tag, value) in metadata_object {
            object.entry(tag.clone()).or_insert_with(|| value.clone());
        }
        return;
    }

    for field in include_fields {
        if let Some(value) = metadata_object.get(field) {
            object.entry(field.clone()).or_insert_with(|| value.clone());
        }
    }
}

fn string_attribute(vr: &'static str, value: impl Into<String>) -> Value {
    json!({ "vr": vr, "Value": [value.into()] })
}

fn integer_string_attribute(value: i32) -> Value {
    json!({ "vr": "IS", "Value": [value.to_string()] })
}

fn person_name_attribute(value: &str) -> Value {
    json!({ "vr": "PN", "Value": [{ "Alphabetic": value }] })
}

fn insert_optional_string_attribute(
    object: &mut Map<String, Value>,
    tag: &'static str,
    vr: &'static str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        object.insert(tag.into(), string_attribute(vr, value));
    }
}

fn insert_optional_string_list_attribute(
    object: &mut Map<String, Value>,
    tag: &'static str,
    vr: &'static str,
    values: &[String],
) {
    if !values.is_empty() {
        object.insert(tag.into(), json!({ "vr": vr, "Value": values }));
    }
}

fn insert_optional_person_name_attribute(
    object: &mut Map<String, Value>,
    tag: &'static str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        object.insert(tag.into(), person_name_attribute(value));
    }
}

fn insert_optional_integer_string_attribute(
    object: &mut Map<String, Value>,
    tag: &'static str,
    value: Option<i32>,
) {
    if let Some(value) = value {
        object.insert(tag.into(), integer_string_attribute(value));
    }
}

/// Parse `YYYYMMDD-YYYYMMDD` or a single `YYYYMMDD` into an optional date range.
fn parse_date_range(s: Option<&str>) -> (Option<NaiveDate>, Option<NaiveDate>) {
    match s {
        None => (None, None),
        Some(s) => {
            if let Some((from, to)) = s.split_once('-') {
                let from_date = NaiveDate::parse_from_str(from.trim(), "%Y%m%d").ok();
                let to_date = NaiveDate::parse_from_str(to.trim(), "%Y%m%d").ok();
                (from_date, to_date)
            } else {
                let date = NaiveDate::parse_from_str(s.trim(), "%Y%m%d").ok();
                (date, date)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use pacs_core::{DicomJson, Instance, Series, SeriesUid, SopInstanceUid, Study, StudyUid};
    use serde_json::json;
    use tower::ServiceExt;

    use crate::{
        router::build_router,
        test_support::{make_test_state, MockBlobStr, MockMetaStore},
    };

    #[test]
    fn test_parse_date_range_none() {
        assert_eq!(parse_date_range(None), (None, None));
    }

    #[test]
    fn test_parse_date_range_single() {
        let (from, to) = parse_date_range(Some("20200101"));
        assert!(from.is_some());
        assert_eq!(from, to);
    }

    #[test]
    fn test_parse_date_range_range() {
        let (from, to) = parse_date_range(Some("20200101-20201231"));
        assert!(from.is_some());
        assert!(to.is_some());
        assert!(from.unwrap() < to.unwrap());
    }

    #[tokio::test]
    async fn test_search_studies_returns_empty_array() {
        let mut store = MockMetaStore::new();
        store
            .expect_query_studies()
            .once()
            .returning(|_| Ok(vec![]));
        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_search_studies_returns_aggregate_dicom_json() {
        let mut store = MockMetaStore::new();
        store.expect_query_studies().once().returning(|_| {
            Ok(vec![Study {
                study_uid: StudyUid::from("1.2.3"),
                patient_id: Some("PID001".into()),
                patient_name: Some("Doe^Jane".into()),
                study_date: NaiveDate::from_ymd_opt(2024, 1, 2),
                study_time: Some("101112".into()),
                accession_number: Some("ACC001".into()),
                modalities: vec!["CT".into()],
                referring_physician: Some("Doctor^Ref".into()),
                description: Some("Chest CT".into()),
                num_series: 1,
                num_instances: 5,
                metadata: DicomJson::from(json!({
                    "00080018": {"vr": "UI", "Value": ["should-not-leak"]}
                })),
                created_at: None,
                updated_at: None,
            }])
        });

        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json[0]["0020000D"]["Value"][0], json!("1.2.3"));
        assert_eq!(json[0]["00201206"]["Value"][0], json!("1"));
        assert_eq!(json[0]["00201208"]["Value"][0], json!("5"));
        assert!(json[0].get("00080018").is_none());
    }

    #[tokio::test]
    async fn test_search_studies_passes_include_fields_to_store() {
        let mut store = MockMetaStore::new();
        store.expect_query_studies().once().returning(|query| {
            assert_eq!(query.include_fields, vec!["00080016", "00280010"]);
            Ok(vec![])
        });

        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies?includeField=00080016,00280010")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_search_studies_passes_fuzzy_matching_to_store() {
        let mut store = MockMetaStore::new();
        store.expect_query_studies().once().returning(|query| {
            assert!(query.fuzzy_matching);
            assert_eq!(query.patient_name.as_deref(), Some("Doe*"));
            Ok(vec![])
        });

        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies?PatientName=Doe*&fuzzymatching=true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_search_studies_includes_requested_metadata_fields() {
        let mut store = MockMetaStore::new();
        store.expect_query_studies().once().returning(|_| {
            Ok(vec![Study {
                study_uid: StudyUid::from("1.2.3"),
                patient_id: Some("PID001".into()),
                patient_name: Some("Doe^Jane".into()),
                study_date: None,
                study_time: None,
                accession_number: None,
                modalities: vec!["CT".into()],
                referring_physician: None,
                description: Some("Chest CT".into()),
                num_series: 1,
                num_instances: 5,
                metadata: DicomJson::from(json!({
                    "00080016": {"vr": "UI", "Value": ["1.2.840.10008.5.1.4.1.1.2"]},
                    "00280010": {"vr": "US", "Value": [512]}
                })),
                created_at: None,
                updated_at: None,
            }])
        });

        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies?includeField=00080016")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json[0]["00080016"]["Value"][0],
            json!("1.2.840.10008.5.1.4.1.1.2")
        );
        assert!(json[0].get("00280010").is_none());
    }

    #[tokio::test]
    async fn test_search_series_returns_aggregate_dicom_json() {
        let mut store = MockMetaStore::new();
        store.expect_query_series().once().returning(|_| {
            Ok(vec![Series {
                series_uid: SeriesUid::from("1.2.3.4"),
                study_uid: StudyUid::from("1.2.3"),
                modality: Some("CT".into()),
                series_number: Some(7),
                description: Some("Axial".into()),
                body_part: Some("CHEST".into()),
                num_instances: 5,
                metadata: DicomJson::from(json!({
                    "00080018": {"vr": "UI", "Value": ["should-not-leak"]}
                })),
                created_at: None,
            }])
        });

        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/series")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json[0]["0020000D"]["Value"][0], json!("1.2.3"));
        assert_eq!(json[0]["0020000E"]["Value"][0], json!("1.2.3.4"));
        assert_eq!(json[0]["00201209"]["Value"][0], json!("5"));
        assert!(json[0].get("00080018").is_none());
    }

    #[tokio::test]
    async fn test_search_series_includes_all_requested_metadata_fields() {
        let mut store = MockMetaStore::new();
        store.expect_query_series().once().returning(|_| {
            Ok(vec![Series {
                series_uid: SeriesUid::from("1.2.3.4"),
                study_uid: StudyUid::from("1.2.3"),
                modality: Some("CT".into()),
                series_number: Some(7),
                description: Some("Axial".into()),
                body_part: Some("CHEST".into()),
                num_instances: 5,
                metadata: DicomJson::from(json!({
                    "00080021": {"vr": "DA", "Value": ["20240102"]},
                    "00080031": {"vr": "TM", "Value": ["101112"]}
                })),
                created_at: None,
            }])
        });

        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/series?includeField=all")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json[0]["00080021"]["Value"][0], json!("20240102"));
        assert_eq!(json[0]["00080031"]["Value"][0], json!("101112"));
    }

    #[tokio::test]
    async fn test_search_instances_returns_required_multiframe_metadata_fields() {
        let mut store = MockMetaStore::new();
        store.expect_query_instances().once().returning(|_| {
            Ok(vec![Instance {
                instance_uid: SopInstanceUid::from("1.2.3.4.5"),
                series_uid: SeriesUid::from("1.2.3.4"),
                study_uid: StudyUid::from("1.2.3"),
                sop_class_uid: Some("1.2.840.10008.5.1.4.1.1.2".into()),
                instance_number: Some(1),
                transfer_syntax: Some("1.2.840.10008.1.2.1".into()),
                rows: Some(512),
                columns: Some(512),
                blob_key: "1.2.3/1.2.3.4/1.2.3.4.5".into(),
                metadata: DicomJson::from(json!({
                    "00080018": {"vr": "UI", "Value": ["1.2.3.4.5"]},
                    "00080016": {"vr": "UI", "Value": ["1.2.840.10008.5.1.4.1.1.2"]},
                    "00200013": {"vr": "IS", "Value": ["1"]},
                    "00280010": {"vr": "US", "Value": [512]},
                    "00280011": {"vr": "US", "Value": [512]},
                    "00280008": {"vr": "IS", "Value": ["12"]},
                    "00080008": {"vr": "CS", "Value": ["ORIGINAL", "PRIMARY", "AXIAL"]}
                })),
                created_at: None,
            }])
        });

        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/instances")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json[0]["00280008"]["Value"][0], json!("12"));
        assert_eq!(json[0]["00080008"]["Value"][0], json!("ORIGINAL"));
        assert_eq!(json[0]["00080008"]["Value"][2], json!("AXIAL"));
    }
}
