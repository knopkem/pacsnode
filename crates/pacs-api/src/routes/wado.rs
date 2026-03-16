//! WADO-RS / WADO-URI retrieve, frame, rendered, bulk-data, and metadata handlers.

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::Response,
};
use bytes::Bytes;
use pacs_core::{
    Instance, InstanceQuery, PacsError, SeriesQuery, SeriesUid, SopInstanceUid, StudyUid,
};
use pacs_dicom::{
    extract_bulk_data, extract_frames, parse_bulk_data_tag_path, render_frames_png, BulkDataValue,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{error::ApiError, state::AppState};

const PIXEL_DATA_TAG_HEX: &str = "7FE00010";

/// `GET /wado?requestType=WADO&studyUID=...&seriesUID=...&objectUID=...`
/// — legacy WADO-URI retrieval for a single instance.
pub async fn wado_uri(
    State(state): State<AppState>,
    Query(query): Query<WadoUriQuery>,
) -> Result<Response, ApiError> {
    if !query.request_type.eq_ignore_ascii_case("WADO") {
        return Err(ApiError(PacsError::DicomParse(
            "requestType must be WADO".into(),
        )));
    }

    let uid = SopInstanceUid::from(query.object_uid.as_str());
    let instance = state.store.get_instance(&uid).await?;
    if instance.study_uid.as_ref() != query.study_uid
        || instance.series_uid.as_ref() != query.series_uid
    {
        return Err(not_found("instance", query.object_uid));
    }

    let blob = state.blobs.get(&instance.blob_key).await?;
    let requested_content_type = query
        .content_type
        .as_deref()
        .map(normalize_content_type)
        .unwrap_or_else(|| "application/dicom".into());

    match requested_content_type.as_str() {
        "application/dicom" => single_part_response(blob, "application/dicom"),
        "image/png" => {
            let frame_number = query.frame_number.unwrap_or(1);
            let png = render_frames_png(blob, &[frame_number])
                .map_err(PacsError::from)
                .map_err(ApiError)?;
            single_part_response(png.into_iter().next().unwrap_or_default(), "image/png")
        }
        other => Err(ApiError(PacsError::DicomParse(format!(
            "unsupported WADO-URI contentType: {other}"
        )))),
    }
}

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
    let blob = load_instance_blob(&state, &SopInstanceUid::from(instance_uid.as_str())).await?;
    multipart_response(vec![blob])
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances/:instance_uid/frames/:frame_list`
/// — retrieve one or more raw frames as native octet-stream parts.
pub async fn retrieve_frames(
    State(state): State<AppState>,
    Path((_study_uid, _series_uid, instance_uid, frame_list)): Path<(
        String,
        String,
        String,
        String,
    )>,
) -> Result<Response, ApiError> {
    let frames = parse_frame_list(&frame_list)?;
    let blob = load_instance_blob(&state, &SopInstanceUid::from(instance_uid.as_str())).await?;
    let parts = extract_frames(blob, &frames)
        .map_err(PacsError::from)
        .map_err(ApiError)?;
    multipart_response_with_type(parts, "application/octet-stream")
}

/// `GET /wado/studies/:study_uid/rendered` — render the first frame of the first
/// retrievable instance in a study as PNG.
pub async fn render_study(
    State(state): State<AppState>,
    Path(study_uid): Path<String>,
) -> Result<Response, ApiError> {
    let study_uid = StudyUid::from(study_uid.as_str());
    let instance = first_instance_for_study(&state, &study_uid).await?;
    render_instance_blob(&state, &instance, &[1]).await
}

/// `GET /wado/studies/:study_uid/series/:series_uid/rendered` — render the first
/// frame of the first retrievable instance in a series as PNG.
pub async fn render_series(
    State(state): State<AppState>,
    Path((_study_uid, series_uid)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let series_uid = SeriesUid::from(series_uid.as_str());
    let instance = first_instance_for_series(&state, &series_uid).await?;
    render_instance_blob(&state, &instance, &[1]).await
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances/:instance_uid/rendered`
/// — render the first frame of an instance as PNG.
pub async fn render_instance(
    State(state): State<AppState>,
    Path((_study_uid, _series_uid, instance_uid)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let uid = SopInstanceUid::from(instance_uid.as_str());
    let instance = state.store.get_instance(&uid).await?;
    render_instance_blob(&state, &instance, &[1]).await
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances/:instance_uid/frames/:frame_list/rendered`
/// — render one or more frames as PNG images.
pub async fn render_frames(
    State(state): State<AppState>,
    Path((_study_uid, _series_uid, instance_uid, frame_list)): Path<(
        String,
        String,
        String,
        String,
    )>,
) -> Result<Response, ApiError> {
    let frames = parse_frame_list(&frame_list)?;
    let blob = load_instance_blob(&state, &SopInstanceUid::from(instance_uid.as_str())).await?;
    let pngs = render_frames_png(blob, &frames)
        .map_err(PacsError::from)
        .map_err(ApiError)?;

    match pngs.len() {
        0 => Err(ApiError(PacsError::Internal(
            "rendered response unexpectedly contained no frames".into(),
        ))),
        1 => single_part_response(pngs.into_iter().next().unwrap_or_default(), "image/png"),
        _ => multipart_response_with_type(pngs, "image/png"),
    }
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances/:instance_uid/bulkdata/:tag_path`
/// — retrieve raw bulk data for a top-level element such as `7FE00010`.
pub async fn instance_bulkdata(
    State(state): State<AppState>,
    Path((_study_uid, _series_uid, instance_uid, tag_path)): Path<(String, String, String, String)>,
) -> Result<Response, ApiError> {
    let tag = parse_bulk_data_tag_path(&tag_path)
        .map_err(PacsError::from)
        .map_err(ApiError)?;
    let blob = load_instance_blob(&state, &SopInstanceUid::from(instance_uid.as_str())).await?;
    match extract_bulk_data(blob, tag)
        .map_err(PacsError::from)
        .map_err(ApiError)?
    {
        BulkDataValue::Single(bytes) => single_part_response(bytes, "application/octet-stream"),
        BulkDataValue::Multipart(parts) => {
            multipart_response_with_type(parts, "application/octet-stream")
        }
    }
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
        .get_series(&SeriesUid::from(series_uid.as_str()))
        .await?;
    dicom_json_response(&[series.metadata.as_value()])
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances/:instance_uid/metadata`
/// — instance-level DICOM JSON metadata with a `BulkDataURI` for Pixel Data when present.
pub async fn instance_metadata(
    State(state): State<AppState>,
    Path((study_uid, series_uid, instance_uid)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let uid = SopInstanceUid::from(instance_uid.as_str());
    let metadata = state.store.get_instance_metadata(&uid).await?;
    let bulk_data_uri = format!(
        "/wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/bulkdata/{PIXEL_DATA_TAG_HEX}"
    );
    let patched = attach_pixel_data_bulkdata_uri(metadata.as_value(), &bulk_data_uri);
    dicom_json_response(&[&patched])
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
/// Query parameters accepted by the legacy WADO-URI endpoint.
pub struct WadoUriQuery {
    #[serde(rename = "requestType")]
    request_type: String,
    #[serde(rename = "studyUID")]
    study_uid: String,
    #[serde(rename = "seriesUID")]
    series_uid: String,
    #[serde(rename = "objectUID")]
    object_uid: String,
    #[serde(rename = "contentType")]
    content_type: Option<String>,
    #[serde(rename = "frameNumber")]
    frame_number: Option<u32>,
}

async fn render_instance_blob(
    state: &AppState,
    instance: &Instance,
    frames: &[u32],
) -> Result<Response, ApiError> {
    let blob = state.blobs.get(&instance.blob_key).await?;
    let pngs = render_frames_png(blob, frames)
        .map_err(PacsError::from)
        .map_err(ApiError)?;
    single_part_response(pngs.into_iter().next().unwrap_or_default(), "image/png")
}

async fn load_instance_blob(state: &AppState, uid: &SopInstanceUid) -> Result<Bytes, ApiError> {
    let instance = state.store.get_instance(uid).await?;
    state.blobs.get(&instance.blob_key).await.map_err(ApiError)
}

async fn first_instance_for_study(
    state: &AppState,
    study_uid: &StudyUid,
) -> Result<Instance, ApiError> {
    let series = state
        .store
        .query_series(&SeriesQuery {
            study_uid: study_uid.clone(),
            series_uid: None,
            modality: None,
            series_number: None,
            limit: Some(1),
            offset: None,
        })
        .await?;
    let first_series = series
        .into_iter()
        .next()
        .ok_or_else(|| not_found("renderable instance", study_uid.to_string()))?;
    first_instance_for_series(state, &first_series.series_uid).await
}

async fn first_instance_for_series(
    state: &AppState,
    series_uid: &SeriesUid,
) -> Result<Instance, ApiError> {
    state
        .store
        .query_instances(&InstanceQuery {
            series_uid: series_uid.clone(),
            instance_uid: None,
            sop_class_uid: None,
            instance_number: None,
            limit: Some(1),
            offset: None,
        })
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| not_found("renderable instance", series_uid.to_string()))
}

fn parse_frame_list(frame_list: &str) -> Result<Vec<u32>, ApiError> {
    if frame_list.trim().is_empty() {
        return Err(ApiError(PacsError::DicomParse(
            "frame list must not be empty".into(),
        )));
    }

    frame_list
        .split(',')
        .map(str::trim)
        .map(|segment| {
            segment.parse::<u32>().map_err(|_| {
                ApiError(PacsError::DicomParse(format!(
                    "invalid frame number '{segment}'"
                )))
            })
        })
        .collect()
}

fn attach_pixel_data_bulkdata_uri(
    metadata: &serde_json::Value,
    bulk_data_uri: &str,
) -> serde_json::Value {
    let mut patched = metadata.clone();
    if let Some(entries) = patched.as_object_mut() {
        if let Some(pixel_data) = entries
            .get_mut(PIXEL_DATA_TAG_HEX)
            .and_then(serde_json::Value::as_object_mut)
        {
            pixel_data.remove("InlineBinary");
            pixel_data.insert(
                "BulkDataURI".into(),
                serde_json::Value::String(bulk_data_uri.to_owned()),
            );
        }
    }
    patched
}

fn not_found(resource: &'static str, uid: impl Into<String>) -> ApiError {
    ApiError(PacsError::NotFound {
        resource,
        uid: uid.into(),
    })
}

fn normalize_content_type(content_type: &str) -> String {
    content_type
        .split(',')
        .next()
        .unwrap_or(content_type)
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase()
}

/// Build a `multipart/related; type="application/dicom"` response.
fn multipart_response(parts: Vec<Bytes>) -> Result<Response, ApiError> {
    multipart_response_with_type(parts, "application/dicom")
}

fn multipart_response_with_type(
    parts: Vec<Bytes>,
    part_content_type: &str,
) -> Result<Response, ApiError> {
    let boundary = Uuid::new_v4().simple().to_string();
    let body = build_multipart_body(&parts, &boundary, part_content_type);
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/related; type=\"{part_content_type}\"; boundary={boundary}"),
        )
        .body(axum::body::Body::from(body))
        .map_err(|e| ApiError(PacsError::Internal(e.to_string())))
}

fn single_part_response(body: Bytes, content_type: &str) -> Result<Response, ApiError> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(axum::body::Body::from(body))
        .map_err(|e| ApiError(PacsError::Internal(e.to_string())))
}

/// Build a `multipart/related` body from raw byte parts.
fn build_multipart_body(parts: &[Bytes], boundary: &str, part_content_type: &str) -> Bytes {
    let mut body: Vec<u8> = Vec::new();
    for part in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(format!("Content-Type: {part_content_type}\r\n\r\n").as_bytes());
        body.extend_from_slice(part);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Bytes::from(body)
}

/// Serialize `values` as a JSON array and return `application/dicom+json`.
fn dicom_json_response(values: &[&serde_json::Value]) -> Result<Response, ApiError> {
    let body =
        serde_json::to_vec(values).map_err(|e| ApiError(PacsError::Internal(e.to_string())))?;
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
    use bytes::Bytes;
    use dicom_toolkit_data::{DataSet, DicomWriter, Element, FileFormat, PixelData, Value};
    use dicom_toolkit_dict::{tags, Vr};
    use http_body_util::BodyExt;
    use pacs_core::{DicomJson, Instance, SeriesUid, SopInstanceUid, StudyUid};
    use serde_json::json;
    use tower::ServiceExt;

    use crate::{
        router::build_router,
        test_support::{make_test_state, MockBlobStr, MockMetaStore},
    };

    fn make_instance() -> Instance {
        Instance {
            instance_uid: SopInstanceUid::from("1.2.3.4.5"),
            series_uid: SeriesUid::from("1.2.3.4"),
            study_uid: StudyUid::from("1.2.3"),
            sop_class_uid: Some("1.2.840.10008.5.1.4.1.1.2".into()),
            instance_number: Some(1),
            transfer_syntax: Some("1.2.840.10008.1.2.1".into()),
            rows: Some(1),
            columns: Some(2),
            blob_key: "1.2.3/1.2.3.4/1.2.3.4.5".into(),
            metadata: DicomJson::empty(),
            created_at: None,
        }
    }

    fn make_multiframe_dicom() -> Bytes {
        let mut ds = DataSet::new();
        ds.set_string(tags::STUDY_INSTANCE_UID, Vr::UI, "1.2.3");
        ds.set_string(tags::SERIES_INSTANCE_UID, Vr::UI, "1.2.3.4");
        ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, "1.2.3.4.5");
        ds.set_string(tags::SOP_CLASS_UID, Vr::UI, "1.2.840.10008.5.1.4.1.1.2");
        ds.set_u16(tags::ROWS, 1);
        ds.set_u16(tags::COLUMNS, 2);
        ds.set_u16(tags::SAMPLES_PER_PIXEL, 1);
        ds.set_u16(tags::BITS_ALLOCATED, 8);
        ds.set_u16(tags::BITS_STORED, 8);
        ds.set_u16(tags::HIGH_BIT, 7);
        ds.set_u16(tags::PIXEL_REPRESENTATION, 0);
        ds.set_string(tags::PHOTOMETRIC_INTERPRETATION, Vr::CS, "MONOCHROME2");
        ds.set_string(tags::NUMBER_OF_FRAMES, Vr::IS, "2");
        ds.insert(Element::new(
            tags::PIXEL_DATA,
            Vr::OB,
            Value::PixelData(PixelData::Native {
                bytes: vec![0x11, 0x22, 0x33, 0x44],
            }),
        ));

        let ff = FileFormat::from_dataset("1.2.840.10008.5.1.4.1.1.2", "1.2.3.4.5", ds);
        let mut buf = Vec::new();
        DicomWriter::new(std::io::Cursor::new(&mut buf))
            .write_file(&ff)
            .unwrap();
        Bytes::from(buf)
    }

    #[tokio::test]
    async fn test_instance_metadata_returns_dicom_json() {
        let mut store = MockMetaStore::new();
        store.expect_get_instance_metadata().once().returning(|_| {
            Ok(DicomJson::from(json!({
                "7FE00010": {
                    "vr": "OB",
                    "InlineBinary": "AA=="
                }
            })))
        });
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
        assert_eq!(
            json[0]["7FE00010"]["BulkDataURI"],
            json!("/wado/studies/1.2.3/series/4.5.6/instances/7.8.9/bulkdata/7FE00010")
        );
        assert!(json[0]["7FE00010"].get("InlineBinary").is_none());
    }

    #[tokio::test]
    async fn test_retrieve_frames_returns_octet_stream_multipart() {
        let mut store = MockMetaStore::new();
        let instance = make_instance();
        store
            .expect_get_instance()
            .once()
            .returning(move |_| Ok(instance.clone()));

        let dicom = make_multiframe_dicom();
        let mut blobs = MockBlobStr::new();
        blobs
            .expect_get()
            .once()
            .returning(move |_| Ok(dicom.clone()));

        let app = build_router(make_test_state(store, blobs));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/frames/1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(content_type.contains("multipart/related"));
        assert!(content_type.contains("application/octet-stream"));
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(body.windows(2).any(|window| window == [0x11, 0x22]));
    }

    #[tokio::test]
    async fn test_render_instance_returns_png() {
        let mut store = MockMetaStore::new();
        let instance = make_instance();
        store
            .expect_get_instance()
            .once()
            .returning(move |_| Ok(instance.clone()));

        let dicom = make_multiframe_dicom();
        let mut blobs = MockBlobStr::new();
        blobs
            .expect_get()
            .once()
            .returning(move |_| Ok(dicom.clone()));

        let app = build_router(make_test_state(store, blobs));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/rendered")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(body.starts_with(&[0x89, 0x50, 0x4E, 0x47]));
    }

    #[tokio::test]
    async fn test_bulkdata_returns_octet_stream() {
        let mut store = MockMetaStore::new();
        let instance = make_instance();
        store
            .expect_get_instance()
            .once()
            .returning(move |_| Ok(instance.clone()));

        let dicom = make_multiframe_dicom();
        let mut blobs = MockBlobStr::new();
        blobs
            .expect_get()
            .once()
            .returning(move |_| Ok(dicom.clone()));

        let app = build_router(make_test_state(store, blobs));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/bulkdata/7FE00010")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/octet-stream"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body.as_ref(), &[0x11, 0x22, 0x33, 0x44]);
    }

    #[tokio::test]
    async fn test_wado_uri_returns_application_dicom() {
        let mut store = MockMetaStore::new();
        let instance = make_instance();
        store
            .expect_get_instance()
            .once()
            .returning(move |_| Ok(instance.clone()));

        let payload = Bytes::from_static(b"DICOM-URI");
        let mut blobs = MockBlobStr::new();
        blobs
            .expect_get()
            .once()
            .returning(move |_| Ok(payload.clone()));

        let app = build_router(make_test_state(store, blobs));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado?requestType=WADO&studyUID=1.2.3&seriesUID=1.2.3.4&objectUID=1.2.3.4.5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/dicom"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body.as_ref(), b"DICOM-URI");
    }

    #[test]
    fn test_build_multipart_body_contains_boundary() {
        let data = Bytes::from_static(b"DICOM-DATA");
        let body = build_multipart_body(&[data], "TESTBOUNDARY", "application/dicom");
        let s = std::str::from_utf8(&body).unwrap();
        assert!(s.contains("--TESTBOUNDARY"));
        assert!(s.contains("--TESTBOUNDARY--"));
    }
}
