//! QIDO-RS (Query based on ID for DICOM Objects) handlers.

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Json,
};
use chrono::NaiveDate;
use pacs_core::{InstanceQuery, SeriesQuery, SeriesUid, SopInstanceUid, StudyQuery, StudyUid};
use serde::Deserialize;

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
}

/// `GET /wado/studies` — QIDO-RS study search.
///
/// Returns a JSON array of DICOM JSON metadata objects (PS3.18 §6.7).
pub async fn search_studies(
    State(state): State<AppState>,
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
        include_fields: vec![],
    };
    let studies = state.store.query_studies(&query).await?;
    let metadata: Vec<serde_json::Value> = studies
        .iter()
        .map(|s| s.metadata.as_value().clone())
        .collect();
    Ok(Json(metadata))
}

/// `GET /wado/studies/:study_uid/series` — QIDO-RS series search.
pub async fn search_series(
    State(state): State<AppState>,
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
        .map(|s| s.metadata.as_value().clone())
        .collect();
    Ok(Json(metadata))
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances` — QIDO-RS instance search.
pub async fn search_instances(
    State(state): State<AppState>,
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
    Ok(Json(metadata))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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
}
