//! WADO-RS (Web Access to DICOM Objects) retrieve and metadata handlers.

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::Response,
};
use bytes::Bytes;
use pacs_core::{InstanceQuery, PacsError, SeriesQuery, SeriesUid, SopInstanceUid, StudyUid};
use uuid::Uuid;

use crate::{error::ApiError, state::AppState};

// ── Retrieve endpoints ────────────────────────────────────────────────────────

/// `GET /wado/studies/:study_uid` — retrieve all instances in a study.
///
/// Returns a `multipart/related; type="application/dicom"` response.
pub async fn retrieve_study(
    State(state): State<AppState>,
    Path(study_uid): Path<String>,
) -> Result<Response, ApiError> {
    let s_uid = StudyUid::from(study_uid.as_str());
    let series_list = state
        .store
        .query_series(&SeriesQuery {
            study_uid: s_uid,
            series_uid: None,
            modality: None,
            series_number: None,
            limit: None,
            offset: None,
        })
        .await?;

    let mut parts: Vec<Bytes> = Vec::new();
    for s in series_list {
        let instances = state
            .store
            .query_instances(&InstanceQuery {
                series_uid: s.series_uid,
                instance_uid: None,
                sop_class_uid: None,
                instance_number: None,
                limit: None,
                offset: None,
            })
            .await?;
        for inst in instances {
            if let Ok(blob) = state.blobs.get(&inst.blob_key).await {
                parts.push(blob);
            }
        }
    }

    multipart_response(parts)
}

/// `GET /wado/studies/:study_uid/series/:series_uid` — retrieve all instances in a series.
pub async fn retrieve_series(
    State(state): State<AppState>,
    Path((_study_uid, series_uid)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let instances = state
        .store
        .query_instances(&InstanceQuery {
            series_uid: SeriesUid::from(series_uid.as_str()),
            instance_uid: None,
            sop_class_uid: None,
            instance_number: None,
            limit: None,
            offset: None,
        })
        .await?;

    let mut parts: Vec<Bytes> = Vec::new();
    for inst in instances {
        if let Ok(blob) = state.blobs.get(&inst.blob_key).await {
            parts.push(blob);
        }
    }

    multipart_response(parts)
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances/:instance_uid`
/// — retrieve a single DICOM instance.
pub async fn retrieve_instance(
    State(state): State<AppState>,
    Path((_study_uid, _series_uid, instance_uid)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let uid = SopInstanceUid::from(instance_uid.as_str());
    let inst = state.store.get_instance(&uid).await?;
    let blob = state.blobs.get(&inst.blob_key).await?;
    multipart_response(vec![blob])
}

// ── Metadata endpoints ────────────────────────────────────────────────────────

/// `GET /wado/studies/:study_uid/metadata` — study-level DICOM JSON metadata.
pub async fn study_metadata(
    State(state): State<AppState>,
    Path(study_uid): Path<String>,
) -> Result<Response, ApiError> {
    let study = state
        .store
        .get_study(&StudyUid::from(study_uid.as_str()))
        .await?;
    dicom_json_response(&[study.metadata.as_value()])
}

/// `GET /wado/studies/:study_uid/series/:series_uid/metadata` — series-level DICOM JSON metadata.
pub async fn series_metadata(
    State(state): State<AppState>,
    Path((_study_uid, series_uid)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let series = state
        .store
        .get_series(&pacs_core::SeriesUid::from(series_uid.as_str()))
        .await?;
    dicom_json_response(&[series.metadata.as_value()])
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances/:instance_uid/metadata`
/// — instance-level DICOM JSON metadata.
pub async fn instance_metadata(
    State(state): State<AppState>,
    Path((_study_uid, _series_uid, instance_uid)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let uid = SopInstanceUid::from(instance_uid.as_str());
    let metadata = state.store.get_instance_metadata(&uid).await?;
    dicom_json_response(&[metadata.as_value()])
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a `multipart/related; type="application/dicom"` response.
fn multipart_response(parts: Vec<Bytes>) -> Result<Response, ApiError> {
    let boundary = Uuid::new_v4().simple().to_string();
    let body = build_multipart_body(&parts, &boundary);
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            format!(
                "multipart/related; type=\"application/dicom\"; boundary={boundary}"
            ),
        )
        .body(axum::body::Body::from(body))
        .map_err(|e| ApiError(PacsError::Internal(e.to_string())))
}

/// Build a `multipart/related` body from raw DICOM byte parts.
fn build_multipart_body(parts: &[Bytes], boundary: &str) -> Bytes {
    let mut body: Vec<u8> = Vec::new();
    for part in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Type: application/dicom\r\n\r\n");
        body.extend_from_slice(part);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Bytes::from(body)
}

/// Serialize `values` as a JSON array and return `application/dicom+json`.
fn dicom_json_response(values: &[&serde_json::Value]) -> Result<Response, ApiError> {
    let body = serde_json::to_vec(values)
        .map_err(|e| ApiError(PacsError::Internal(e.to_string())))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/dicom+json")
        .body(axum::body::Body::from(body))
        .map_err(|e| ApiError(PacsError::Internal(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use pacs_core::DicomJson;
    use tower::ServiceExt;

    use crate::{
        router::build_router,
        test_support::{make_test_state, MockBlobStr, MockMetaStore},
    };

    #[tokio::test]
    async fn test_instance_metadata_returns_dicom_json() {
        let mut store = MockMetaStore::new();
        store
            .expect_get_instance_metadata()
            .once()
            .returning(|_| Ok(DicomJson::empty()));
        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/series/4.5.6/instances/7.8.9/metadata")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.contains("application/dicom+json"));
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.is_array());
    }

    #[test]
    fn test_build_multipart_body_contains_boundary() {
        let data = Bytes::from_static(b"DICOM-DATA");
        let body = build_multipart_body(&[data], "TESTBOUNDARY");
        let s = std::str::from_utf8(&body).unwrap();
        assert!(s.contains("--TESTBOUNDARY"));
        assert!(s.contains("--TESTBOUNDARY--"));
    }
}
