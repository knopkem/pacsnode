//! WADO-RS / WADO-URI retrieve, frame, rendered, bulk-data, and metadata handlers.

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use bytes::Bytes;
use pacs_core::{
    DicomJson, Instance, InstanceQuery, PacsError, SeriesQuery, SeriesUid, SopInstanceUid, StudyUid,
};
use pacs_dicom::{
    extract_bulk_data_path, extract_frames, metadata_with_bulk_data_uris,
    render_frames_with_options, supports_retrieve_transfer_syntax, transcode_part10, BulkDataValue,
    RenderedFrameOptions, RenderedMediaType, RenderedRegion,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{error::ApiError, state::AppState};

const DEFAULT_JPEG_QUALITY: u8 = 90;

/// `GET /wado?requestType=WADO&studyUID=...&seriesUID=...&objectUID=...`
/// — legacy WADO-URI retrieval for a single instance.
pub async fn wado_uri(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WadoUriQuery>,
) -> Result<Response, ApiError> {
    if !query.request_type.eq_ignore_ascii_case("WADO") {
        return Err(ApiError(PacsError::DicomParse(
            "requestType must be WADO".into(),
        )));
    }

    let requested_transfer_syntax = query.transfer_syntax.as_deref();
    let response_kind = resolve_wado_uri_response(query.content_type.as_deref(), &headers)?;
    let render_options = parse_rendered_query(&query.rendered)?;

    match response_kind {
        WadoUriResponseKind::Dicom => {
            reject_rendered_parameters_for_dicom(&query, &render_options)?
        }
        WadoUriResponseKind::Rendered(_) if requested_transfer_syntax.is_some() => {
            return Err(ApiError(PacsError::UnsupportedMediaType(
                "transferSyntax is only supported for application/dicom WADO-URI responses".into(),
            )))
        }
        WadoUriResponseKind::Rendered(_) => {}
    }

    let uid = SopInstanceUid::from(query.object_uid.as_str());
    let instance = state.store.get_instance(&uid).await?;
    if instance.study_uid.as_ref() != query.study_uid
        || instance.series_uid.as_ref() != query.series_uid
    {
        return Err(not_found("instance", query.object_uid));
    }
    let blob = state.blobs.get(&instance.blob_key).await?;

    match response_kind {
        WadoUriResponseKind::Dicom => {
            let body = transcode_blob_if_requested(blob, requested_transfer_syntax)?;
            single_dicom_response(body, requested_transfer_syntax)
        }
        WadoUriResponseKind::Rendered(media_type) => {
            let frame_number = query.frame_number.unwrap_or(1);
            render_blob_response(blob, &[frame_number], media_type, &render_options)
        }
    }
}

// ── Retrieve endpoints ────────────────────────────────────────────────────────

/// `GET /wado/studies/:study_uid` — retrieve all instances in a study.
///
/// Returns a `multipart/related; type="application/dicom"` response.
pub async fn retrieve_study(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(study_uid): Path<String>,
) -> Result<Response, ApiError> {
    let retrieve_request = parse_retrieve_request(&headers)?;
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
                parts.push(transcode_blob_if_requested(
                    blob,
                    retrieve_request.transfer_syntax.as_deref(),
                )?);
            }
        }
    }

    multipart_dicom_response(parts, retrieve_request.transfer_syntax.as_deref())
}

/// `GET /wado/studies/:study_uid/series/:series_uid` — retrieve all instances in a series.
pub async fn retrieve_series(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((_study_uid, series_uid)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let retrieve_request = parse_retrieve_request(&headers)?;
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
            parts.push(transcode_blob_if_requested(
                blob,
                retrieve_request.transfer_syntax.as_deref(),
            )?);
        }
    }

    multipart_dicom_response(parts, retrieve_request.transfer_syntax.as_deref())
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances/:instance_uid`
/// — retrieve a single DICOM instance.
pub async fn retrieve_instance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((_study_uid, _series_uid, instance_uid)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let retrieve_request = parse_retrieve_request(&headers)?;
    let blob = load_instance_blob(&state, &SopInstanceUid::from(instance_uid.as_str())).await?;
    let body = transcode_blob_if_requested(blob, retrieve_request.transfer_syntax.as_deref())?;
    multipart_dicom_response(vec![body], retrieve_request.transfer_syntax.as_deref())
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
    headers: HeaderMap,
    Query(rendered_query): Query<RenderedQuery>,
    Path(study_uid): Path<String>,
) -> Result<Response, ApiError> {
    let study_uid = StudyUid::from(study_uid.as_str());
    let instance = representative_instance_for_study(&state, &study_uid).await?;
    let media_type = parse_rendered_media_type(
        rendered_query.accept.as_deref(),
        &headers,
        RenderedMediaType::Png,
    )?;
    let render_options = parse_rendered_query(&rendered_query)?;
    render_instance_blob(&state, &instance, &[1], media_type, &render_options).await
}

/// `GET /wado/studies/:study_uid/series/:series_uid/rendered` — render the first
/// frame of the first retrievable instance in a series as PNG.
pub async fn render_series(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(rendered_query): Query<RenderedQuery>,
    Path((_study_uid, series_uid)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let series_uid = SeriesUid::from(series_uid.as_str());
    let instance = representative_instance_for_series(&state, &series_uid).await?;
    let media_type = parse_rendered_media_type(
        rendered_query.accept.as_deref(),
        &headers,
        RenderedMediaType::Png,
    )?;
    let render_options = parse_rendered_query(&rendered_query)?;
    render_instance_blob(&state, &instance, &[1], media_type, &render_options).await
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances/:instance_uid/rendered`
/// — render the first frame of an instance as PNG.
pub async fn render_instance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(rendered_query): Query<RenderedQuery>,
    Path((_study_uid, _series_uid, instance_uid)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let uid = SopInstanceUid::from(instance_uid.as_str());
    let instance = state.store.get_instance(&uid).await?;
    let media_type = parse_rendered_media_type(
        rendered_query.accept.as_deref(),
        &headers,
        RenderedMediaType::Png,
    )?;
    let render_options = parse_rendered_query(&rendered_query)?;
    render_instance_blob(&state, &instance, &[1], media_type, &render_options).await
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances/:instance_uid/thumbnail`
/// — render a thumbnail image for an instance.
pub async fn thumbnail_instance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(rendered_query): Query<RenderedQuery>,
    Path((_study_uid, _series_uid, instance_uid)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let uid = SopInstanceUid::from(instance_uid.as_str());
    let instance = state.store.get_instance(&uid).await?;
    let media_type = parse_rendered_media_type(
        rendered_query.accept.as_deref(),
        &headers,
        RenderedMediaType::Jpeg {
            quality: DEFAULT_JPEG_QUALITY,
        },
    )?;
    let render_options = parse_thumbnail_query(&rendered_query)?;
    render_instance_blob(&state, &instance, &[1], media_type, &render_options).await
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances/:instance_uid/frames/:frame_list/rendered`
/// — render one or more frames as PNG images.
pub async fn render_frames(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(rendered_query): Query<RenderedQuery>,
    Path((_study_uid, _series_uid, instance_uid, frame_list)): Path<(
        String,
        String,
        String,
        String,
    )>,
) -> Result<Response, ApiError> {
    let frames = parse_frame_list(&frame_list)?;
    let blob = load_instance_blob(&state, &SopInstanceUid::from(instance_uid.as_str())).await?;
    let media_type = parse_rendered_media_type(
        rendered_query.accept.as_deref(),
        &headers,
        RenderedMediaType::Png,
    )?;
    let render_options = parse_rendered_query(&rendered_query)?;
    let rendered = render_frames_with_options(blob, &frames, media_type, &render_options)
        .map_err(PacsError::from)
        .map_err(ApiError)?;

    match rendered.len() {
        0 => Err(ApiError(PacsError::Internal(
            "rendered response unexpectedly contained no frames".into(),
        ))),
        1 => single_part_response(
            rendered.into_iter().next().unwrap_or_default(),
            media_type.content_type(),
        ),
        _ => multipart_response_with_type(rendered, media_type.content_type()),
    }
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances/:instance_uid/bulkdata/:tag_path`
/// — retrieve raw bulk data for a top-level element such as `7FE00010`.
pub async fn instance_bulkdata(
    State(state): State<AppState>,
    Path((study_uid, series_uid, instance_uid, tag_path)): Path<(String, String, String, String)>,
) -> Result<Response, ApiError> {
    let instance = state
        .store
        .get_instance(&SopInstanceUid::from(instance_uid.as_str()))
        .await?;
    let blob = state.blobs.get(&instance.blob_key).await?;
    let normalized_tag_path = tag_path.trim_matches('/').to_owned();
    let content_type = bulkdata_content_type(&instance, &normalized_tag_path);
    match extract_bulk_data_path(blob, &normalized_tag_path)
        .map_err(PacsError::from)
        .map_err(ApiError)?
    {
        BulkDataValue::Single(bytes) => single_part_response(bytes, &content_type),
        BulkDataValue::Multipart(parts) => {
            let part_count = parts.len();
            multipart_response_with_type_and_locations(
                parts,
                &content_type,
                (1..=part_count)
                    .map(|index| {
                        format!(
                            "/wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/bulkdata/{normalized_tag_path}?partNumber={index}"
                        )
                    })
                    .collect(),
            )
        }
    }
}

fn bulkdata_content_type(instance: &Instance, normalized_tag_path: &str) -> String {
    if normalized_tag_path.eq_ignore_ascii_case("7FE00010") {
        if let Some(content_type) = video_bulkdata_content_type(instance.transfer_syntax.as_deref())
        {
            return content_type.into();
        }
    }

    if normalized_tag_path.eq_ignore_ascii_case("00420011") {
        if let Some(content_type) = encapsulated_document_content_type(&instance.metadata) {
            return content_type;
        }

        if instance.sop_class_uid.as_deref() == Some("1.2.840.10008.5.1.4.1.1.104.1") {
            return "application/pdf".into();
        }
    }

    "application/octet-stream".into()
}

fn video_bulkdata_content_type(transfer_syntax_uid: Option<&str>) -> Option<&'static str> {
    match transfer_syntax_uid {
        Some("1.2.840.10008.1.2.4.100" | "1.2.840.10008.1.2.4.101") => Some("video/mpeg"),
        Some(
            "1.2.840.10008.1.2.4.102"
            | "1.2.840.10008.1.2.4.103"
            | "1.2.840.10008.1.2.4.104"
            | "1.2.840.10008.1.2.4.105"
            | "1.2.840.10008.1.2.4.106"
            | "1.2.840.10008.1.2.4.107"
            | "1.2.840.10008.1.2.4.108"
            | "1.2.840.10008.1.2.4.109",
        ) => Some("video/mp4"),
        _ => None,
    }
}

fn encapsulated_document_content_type(metadata: &DicomJson) -> Option<String> {
    metadata
        .as_value()
        .get("00420012")
        .and_then(|value| value.get("Value"))
        .and_then(serde_json::Value::as_array)
        .and_then(|values| values.first())
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

// ── Metadata endpoints ────────────────────────────────────────────────────────

/// `GET /wado/studies/:study_uid/metadata` — study-level DICOM JSON metadata.
pub async fn study_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(study_uid): Path<String>,
) -> Result<Response, ApiError> {
    let study_uid = StudyUid::from(study_uid.as_str());
    state.store.get_study(&study_uid).await?;

    let series = state
        .store
        .query_series(&SeriesQuery {
            study_uid: study_uid.clone(),
            series_uid: None,
            modality: None,
            series_number: None,
            limit: None,
            offset: None,
        })
        .await?;

    let mut metadata = Vec::new();
    for series in series {
        let instances = state
            .store
            .query_instances(&InstanceQuery {
                series_uid: series.series_uid,
                instance_uid: None,
                sop_class_uid: None,
                instance_number: None,
                limit: None,
                offset: None,
            })
            .await?;
        for instance in instances {
            metadata
                .push(instance_metadata_with_bulk_data_uris(&state, &headers, &instance).await?);
        }
    }

    dicom_json_response_from_owned(&metadata)
}

/// `GET /wado/studies/:study_uid/series/:series_uid/metadata` — series-level DICOM JSON metadata.
pub async fn series_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((study_uid, series_uid)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let study_uid = StudyUid::from(study_uid.as_str());
    let series_uid = SeriesUid::from(series_uid.as_str());
    let series = state.store.get_series(&series_uid).await?;
    if series.study_uid != study_uid {
        return Err(not_found("series", series_uid.to_string()));
    }

    let instances = state
        .store
        .query_instances(&InstanceQuery {
            series_uid,
            instance_uid: None,
            sop_class_uid: None,
            instance_number: None,
            limit: None,
            offset: None,
        })
        .await?;

    let mut metadata = Vec::with_capacity(instances.len());
    for instance in instances {
        metadata.push(instance_metadata_with_bulk_data_uris(&state, &headers, &instance).await?);
    }

    dicom_json_response_from_owned(&metadata)
}

/// `GET /wado/studies/:study_uid/series/:series_uid/instances/:instance_uid/metadata`
/// — instance-level DICOM JSON metadata with a `BulkDataURI` for Pixel Data when present.
pub async fn instance_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((study_uid, series_uid, instance_uid)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let uid = SopInstanceUid::from(instance_uid.as_str());
    let instance = state.store.get_instance(&uid).await?;
    if instance.study_uid.as_ref() != study_uid || instance.series_uid.as_ref() != series_uid {
        return Err(not_found("instance", instance_uid));
    }
    let metadata = instance_metadata_with_bulk_data_uris(&state, &headers, &instance).await?;
    dicom_json_response(&[metadata.as_value()])
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
    #[serde(rename = "transferSyntax")]
    transfer_syntax: Option<String>,
    #[serde(rename = "frameNumber")]
    frame_number: Option<u32>,
    #[serde(flatten)]
    rendered: RenderedQuery,
}

#[derive(Debug, Default, Deserialize, Clone)]
/// Query parameters accepted by rendered WADO-RS and WADO-URI responses.
pub struct RenderedQuery {
    #[serde(rename = "accept")]
    accept: Option<String>,
    #[serde(rename = "windowCenter")]
    window_center: Option<String>,
    #[serde(rename = "windowWidth")]
    window_width: Option<String>,
    #[serde(rename = "rows")]
    rows: Option<String>,
    #[serde(rename = "columns")]
    columns: Option<String>,
    #[serde(rename = "region")]
    region: Option<String>,
    #[serde(rename = "annotation")]
    annotation: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct RetrieveRequest {
    transfer_syntax: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WadoUriResponseKind {
    Dicom,
    Rendered(RenderedMediaType),
}

async fn render_instance_blob(
    state: &AppState,
    instance: &Instance,
    frames: &[u32],
    media_type: RenderedMediaType,
    render_options: &RenderedFrameOptions,
) -> Result<Response, ApiError> {
    let blob = state.blobs.get(&instance.blob_key).await?;
    render_blob_response(blob, frames, media_type, render_options)
}

async fn load_instance_blob(state: &AppState, uid: &SopInstanceUid) -> Result<Bytes, ApiError> {
    let instance = state.store.get_instance(uid).await?;
    state.blobs.get(&instance.blob_key).await.map_err(ApiError)
}

/// Selects the representative instance for a study by taking the lowest
/// `series_number` series and then the lowest `instance_number` instance within
/// that series.
async fn representative_instance_for_study(
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
    representative_instance_for_series(state, &first_series.series_uid).await
}

/// Selects the representative instance for a series by taking the lowest
/// `instance_number` instance returned by the metadata store query.
async fn representative_instance_for_series(
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

async fn instance_metadata_with_bulk_data_uris(
    state: &AppState,
    headers: &HeaderMap,
    instance: &Instance,
) -> Result<DicomJson, ApiError> {
    let study_uid = instance.study_uid.as_ref();
    let series_uid = instance.series_uid.as_ref();
    let instance_uid = instance.instance_uid.as_ref();
    let blob = state.blobs.get(&instance.blob_key).await?;
    let bulkdata_prefix = forwarded_path_prefix(headers);
    metadata_with_bulk_data_uris(&instance.metadata, blob, |path| {
        format!(
            "{bulkdata_prefix}/wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/bulkdata/{path}"
        )
    })
    .map_err(PacsError::from)
    .map_err(ApiError)
}

fn forwarded_path_prefix(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-prefix")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "/")
        .map(normalize_forwarded_prefix)
        .unwrap_or_default()
}

fn normalize_forwarded_prefix(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        String::new()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn not_found(resource: &'static str, uid: impl Into<String>) -> ApiError {
    ApiError(PacsError::NotFound {
        resource,
        uid: uid.into(),
    })
}

fn resolve_wado_uri_response(
    content_type: Option<&str>,
    headers: &HeaderMap,
) -> Result<WadoUriResponseKind, ApiError> {
    if let Some(content_type) = content_type {
        return match normalize_content_type(content_type).as_str() {
            "application/dicom" => Ok(WadoUriResponseKind::Dicom),
            "image/png" => Ok(WadoUriResponseKind::Rendered(RenderedMediaType::Png)),
            "image/jpeg" => Ok(WadoUriResponseKind::Rendered(RenderedMediaType::Jpeg {
                quality: DEFAULT_JPEG_QUALITY,
            })),
            other => Err(ApiError(PacsError::UnsupportedMediaType(format!(
                "unsupported WADO-URI contentType: {other}"
            )))),
        };
    }

    parse_wado_uri_accept(headers)
}

fn parse_wado_uri_accept(headers: &HeaderMap) -> Result<WadoUriResponseKind, ApiError> {
    let Some(raw_accept) = headers.get(header::ACCEPT) else {
        return Ok(WadoUriResponseKind::Dicom);
    };
    let accept = raw_accept.to_str().map_err(|_| {
        ApiError(PacsError::NotAcceptable(
            "Accept header contains invalid UTF-8".into(),
        ))
    })?;

    if accept.trim().is_empty() {
        return Ok(WadoUriResponseKind::Dicom);
    }

    for candidate in accept
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if let Some(kind) = parse_wado_uri_accept_candidate(candidate) {
            return Ok(kind);
        }
    }

    Err(ApiError(PacsError::NotAcceptable(
        "only application/dicom, image/png, and image/jpeg WADO-URI responses are supported".into(),
    )))
}

fn parse_wado_uri_accept_candidate(candidate: &str) -> Option<WadoUriResponseKind> {
    let media_type = candidate
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match media_type.as_str() {
        "*/*" | "application/*" | "application/dicom" => Some(WadoUriResponseKind::Dicom),
        "image/*" | "image/png" => Some(WadoUriResponseKind::Rendered(RenderedMediaType::Png)),
        "image/jpeg" => Some(WadoUriResponseKind::Rendered(RenderedMediaType::Jpeg {
            quality: DEFAULT_JPEG_QUALITY,
        })),
        _ => None,
    }
}

fn parse_rendered_accept(headers: &HeaderMap) -> Result<RenderedMediaType, ApiError> {
    let Some(raw_accept) = headers.get(header::ACCEPT) else {
        return Ok(RenderedMediaType::Png);
    };
    let accept = raw_accept.to_str().map_err(|_| {
        ApiError(PacsError::NotAcceptable(
            "Accept header contains invalid UTF-8".into(),
        ))
    })?;

    if accept.trim().is_empty() {
        return Ok(RenderedMediaType::Png);
    }

    for candidate in accept
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if let Some(kind) = parse_rendered_accept_candidate(candidate) {
            return Ok(kind);
        }
    }

    Err(ApiError(PacsError::NotAcceptable(
        "only image/png and image/jpeg rendered responses are supported".into(),
    )))
}

fn parse_rendered_media_type(
    accept_query: Option<&str>,
    headers: &HeaderMap,
    default: RenderedMediaType,
) -> Result<RenderedMediaType, ApiError> {
    if let Some(query_accept) = accept_query {
        let normalized = normalize_content_type(query_accept);
        return parse_rendered_accept_candidate(&normalized).ok_or_else(|| {
            ApiError(PacsError::NotAcceptable(
                "only image/png and image/jpeg rendered responses are supported".into(),
            ))
        });
    }

    if headers.contains_key(header::ACCEPT) {
        return parse_rendered_accept(headers);
    }

    Ok(default)
}

fn parse_rendered_accept_candidate(candidate: &str) -> Option<RenderedMediaType> {
    let mut parts = candidate.split(';').map(str::trim);
    let media_type = parts.next().unwrap_or_default().to_ascii_lowercase();
    match media_type.as_str() {
        "*/*" | "image/*" | "image/png" => Some(RenderedMediaType::Png),
        "image/jpeg" => Some(RenderedMediaType::Jpeg {
            quality: DEFAULT_JPEG_QUALITY,
        }),
        "multipart/related" => {
            let mut related_type: Option<String> = None;
            for part in parts {
                let mut kv = part.splitn(2, '=');
                let key = kv.next().unwrap_or_default().trim().to_ascii_lowercase();
                let value = kv
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .trim_matches('"')
                    .to_owned();
                if key == "type" {
                    related_type = Some(normalize_content_type(&value));
                }
            }

            match related_type.as_deref().unwrap_or("image/png") {
                "image/png" => Some(RenderedMediaType::Png),
                "image/jpeg" => Some(RenderedMediaType::Jpeg {
                    quality: DEFAULT_JPEG_QUALITY,
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse_rendered_query(query: &RenderedQuery) -> Result<RenderedFrameOptions, ApiError> {
    let window_center =
        parse_optional_query_number::<f64>(query.window_center.as_deref(), "windowCenter")?;
    let window_width =
        parse_optional_query_number::<f64>(query.window_width.as_deref(), "windowWidth")?;
    let rows = parse_optional_query_number::<u32>(query.rows.as_deref(), "rows")?;
    let columns = parse_optional_query_number::<u32>(query.columns.as_deref(), "columns")?;

    match (window_center, window_width) {
        (Some(_), Some(_)) | (None, None) => {}
        _ => {
            return Err(ApiError(PacsError::DicomParse(
                "windowCenter and windowWidth must be provided together".into(),
            )))
        }
    }

    if matches!(rows, Some(0)) || matches!(columns, Some(0)) {
        return Err(ApiError(PacsError::DicomParse(
            "rows and columns must be greater than zero when provided".into(),
        )));
    }

    if let Some(annotation) = query.annotation.as_deref() {
        let normalized = annotation.trim();
        if !normalized.is_empty() && !normalized.eq_ignore_ascii_case("none") {
            return Err(ApiError(PacsError::DicomParse(
                "annotation is not supported; omit it or use annotation=none".into(),
            )));
        }
    }

    let region = match query.region.as_deref() {
        Some(raw) => Some(parse_rendered_region(raw)?),
        None => None,
    };

    Ok(RenderedFrameOptions {
        frame: 0,
        window_center,
        window_width,
        rows,
        columns,
        region,
        burn_in_overlays: false,
    })
}

fn parse_thumbnail_query(query: &RenderedQuery) -> Result<RenderedFrameOptions, ApiError> {
    let mut options = parse_rendered_query(query)?;
    if options.rows.is_none() {
        options.rows = Some(128);
    }
    if options.columns.is_none() {
        options.columns = Some(128);
    }
    Ok(options)
}

fn parse_optional_query_number<T>(
    raw: Option<&str>,
    field_name: &str,
) -> Result<Option<T>, ApiError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value = raw.trim();
    if value.is_empty() {
        return Err(ApiError(PacsError::DicomParse(format!(
            "{field_name} must not be empty"
        ))));
    }
    value.parse::<T>().map(Some).map_err(|err| {
        ApiError(PacsError::DicomParse(format!(
            "invalid {field_name} value '{raw}': {err}"
        )))
    })
}

fn parse_rendered_region(raw: &str) -> Result<RenderedRegion, ApiError> {
    let values: Vec<f64> = raw
        .split(',')
        .map(str::trim)
        .map(|part| {
            part.parse::<f64>().map_err(|_| {
                ApiError(PacsError::DicomParse(format!(
                    "invalid rendered region component '{part}'"
                )))
            })
        })
        .collect::<Result<_, _>>()?;

    let [left, top, width, height]: [f64; 4] = values.try_into().map_err(|_| {
        ApiError(PacsError::DicomParse(
            "region must contain four comma-separated normalized values".into(),
        ))
    })?;

    if [left, top, width, height]
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(ApiError(PacsError::DicomParse(
            "region values must be finite".into(),
        )));
    }
    if left < 0.0
        || top < 0.0
        || width <= 0.0
        || height <= 0.0
        || left + width > 1.0
        || top + height > 1.0
    {
        return Err(ApiError(PacsError::DicomParse(
            "region must stay within [0.0, 1.0] and have positive width/height".into(),
        )));
    }

    Ok(RenderedRegion {
        left,
        top,
        width,
        height,
    })
}

fn reject_rendered_parameters_for_dicom(
    query: &WadoUriQuery,
    render_options: &RenderedFrameOptions,
) -> Result<(), ApiError> {
    if query.frame_number.is_some()
        || render_options.window_center.is_some()
        || render_options.window_width.is_some()
        || render_options.rows.is_some()
        || render_options.columns.is_some()
        || render_options.region.is_some()
        || query
            .rendered
            .annotation
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
    {
        return Err(ApiError(PacsError::DicomParse(
            "frameNumber and rendered query parameters require an image/* WADO-URI response".into(),
        )));
    }
    Ok(())
}

fn render_blob_response(
    blob: Bytes,
    frames: &[u32],
    media_type: RenderedMediaType,
    render_options: &RenderedFrameOptions,
) -> Result<Response, ApiError> {
    let rendered = render_frames_with_options(blob, frames, media_type, render_options)
        .map_err(PacsError::from)
        .map_err(ApiError)?;
    match rendered.len() {
        0 => Err(ApiError(PacsError::Internal(
            "rendered response unexpectedly contained no frames".into(),
        ))),
        1 => single_part_response(
            rendered.into_iter().next().unwrap_or_default(),
            media_type.content_type(),
        ),
        _ => multipart_response_with_type(rendered, media_type.content_type()),
    }
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

fn parse_retrieve_request(headers: &HeaderMap) -> Result<RetrieveRequest, ApiError> {
    let Some(raw_accept) = headers.get(header::ACCEPT) else {
        return Ok(RetrieveRequest::default());
    };
    let accept = raw_accept.to_str().map_err(|_| {
        ApiError(PacsError::NotAcceptable(
            "Accept header contains invalid UTF-8".into(),
        ))
    })?;

    if accept.trim().is_empty() {
        return Ok(RetrieveRequest::default());
    }

    for candidate in accept
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if let Some(request) = parse_retrieve_accept_candidate(candidate)? {
            return Ok(request);
        }
    }

    Err(ApiError(PacsError::NotAcceptable(
        "only application/dicom WADO-RS retrieval is supported".into(),
    )))
}

fn parse_retrieve_accept_candidate(candidate: &str) -> Result<Option<RetrieveRequest>, ApiError> {
    let mut parts = candidate.split(';').map(str::trim);
    let media_type = parts.next().unwrap_or_default().to_ascii_lowercase();
    if matches!(media_type.as_str(), "*/*" | "application/*") {
        return Ok(Some(RetrieveRequest::default()));
    }
    if media_type != "multipart/related" && media_type != "application/dicom" {
        return Ok(None);
    }

    let mut related_type: Option<String> = None;
    let mut transfer_syntax: Option<String> = None;
    for part in parts {
        if part.is_empty() {
            continue;
        }
        let mut kv = part.splitn(2, '=');
        let key = kv.next().unwrap_or_default().trim().to_ascii_lowercase();
        let value = kv
            .next()
            .unwrap_or_default()
            .trim()
            .trim_matches('"')
            .to_owned();
        match key.as_str() {
            "type" => related_type = Some(normalize_content_type(&value)),
            "transfer-syntax" => transfer_syntax = Some(value),
            "q" => {}
            _ => {}
        }
    }

    if media_type == "multipart/related"
        && related_type
            .as_deref()
            .map(|value| value != "application/dicom")
            .unwrap_or(false)
    {
        return Ok(None);
    }

    if let Some(ref ts_uid) = transfer_syntax {
        if !supports_retrieve_transfer_syntax(ts_uid) {
            return Err(ApiError(PacsError::NotAcceptable(format!(
                "transfer syntax {ts_uid} is not supported for retrieve"
            ))));
        }
    }

    Ok(Some(RetrieveRequest { transfer_syntax }))
}

fn transcode_blob_if_requested(
    blob: Bytes,
    transfer_syntax: Option<&str>,
) -> Result<Bytes, ApiError> {
    match transfer_syntax {
        Some(ts_uid) => transcode_part10(blob, ts_uid)
            .map_err(PacsError::from)
            .map_err(ApiError),
        None => Ok(blob),
    }
}

fn multipart_dicom_response(
    parts: Vec<Bytes>,
    transfer_syntax: Option<&str>,
) -> Result<Response, ApiError> {
    let boundary = Uuid::new_v4().simple().to_string();
    let body = build_multipart_body(&parts, &boundary, "application/dicom", None);
    let content_type = multipart_dicom_content_type(&boundary, transfer_syntax);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(axum::body::Body::from(body))
        .map_err(|e| ApiError(PacsError::Internal(e.to_string())))
}

fn multipart_response_with_type(
    parts: Vec<Bytes>,
    part_content_type: &str,
) -> Result<Response, ApiError> {
    multipart_response_with_type_and_locations(parts, part_content_type, Vec::new())
}

fn multipart_response_with_type_and_locations(
    parts: Vec<Bytes>,
    part_content_type: &str,
    content_locations: Vec<String>,
) -> Result<Response, ApiError> {
    let boundary = Uuid::new_v4().simple().to_string();
    let locations = (!content_locations.is_empty()).then_some(content_locations.as_slice());
    let body = build_multipart_body(&parts, &boundary, part_content_type, locations);
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/related; type=\"{part_content_type}\"; boundary={boundary}"),
        )
        .body(axum::body::Body::from(body))
        .map_err(|e| ApiError(PacsError::Internal(e.to_string())))
}

fn multipart_dicom_content_type(boundary: &str, transfer_syntax: Option<&str>) -> String {
    match transfer_syntax {
        Some(ts_uid) => format!(
            "multipart/related; type=\"application/dicom\"; transfer-syntax={ts_uid}; boundary={boundary}"
        ),
        None => format!("multipart/related; type=\"application/dicom\"; boundary={boundary}"),
    }
}

fn single_part_response(body: Bytes, content_type: &str) -> Result<Response, ApiError> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(axum::body::Body::from(body))
        .map_err(|e| ApiError(PacsError::Internal(e.to_string())))
}

fn single_dicom_response(body: Bytes, transfer_syntax: Option<&str>) -> Result<Response, ApiError> {
    let content_type = match transfer_syntax {
        Some(ts_uid) => format!("application/dicom; transfer-syntax={ts_uid}"),
        None => "application/dicom".into(),
    };
    single_part_response(body, &content_type)
}

/// Build a `multipart/related` body from raw byte parts.
fn build_multipart_body(
    parts: &[Bytes],
    boundary: &str,
    part_content_type: &str,
    content_locations: Option<&[String]>,
) -> Bytes {
    let mut body: Vec<u8> = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(format!("Content-Type: {part_content_type}\r\n").as_bytes());
        if let Some(location) = content_locations.and_then(|locations| locations.get(index)) {
            body.extend_from_slice(format!("Content-Location: {location}\r\n").as_bytes());
        }
        body.extend_from_slice(b"\r\n");
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

fn dicom_json_response_from_owned(values: &[DicomJson]) -> Result<Response, ApiError> {
    let refs: Vec<&serde_json::Value> = values.iter().map(DicomJson::as_value).collect();
    dicom_json_response(&refs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
    };
    use bytes::Bytes;
    use dicom_toolkit_data::{
        DataSet, DicomReader, DicomWriter, Element, FileFormat, PixelData, Value,
    };
    use dicom_toolkit_dict::{tags, ts::transfer_syntaxes, Tag, Vr};
    use http_body_util::BodyExt;
    use pacs_core::{DicomJson, Instance, Series, SeriesUid, SopInstanceUid, Study, StudyUid};
    use pacs_dicom::ParsedDicom;
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

    fn make_nested_bulkdata_dicom() -> Bytes {
        make_nested_bulkdata_dicom_with_uids("1.2.3", "1.2.3.4", "1.2.3.4.5")
    }

    fn make_nested_bulkdata_dicom_with_uids(
        study_uid: &str,
        series_uid: &str,
        instance_uid: &str,
    ) -> Bytes {
        let mut ds = DataSet::new();
        ds.set_string(tags::STUDY_INSTANCE_UID, Vr::UI, study_uid);
        ds.set_string(tags::SERIES_INSTANCE_UID, Vr::UI, series_uid);
        ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, instance_uid);
        ds.set_string(tags::SOP_CLASS_UID, Vr::UI, "1.2.840.10008.5.1.4.1.1.2");
        ds.insert(Element::bytes(
            Tag::new(0x0011, 0x1010),
            Vr::OB,
            vec![0xAA, 0xBB, 0xCC, 0xDD],
        ));

        let mut item = DataSet::new();
        item.insert(Element::bytes(
            Tag::new(0x0011, 0x1011),
            Vr::OB,
            vec![0xDE, 0xAD],
        ));
        ds.set_sequence(Tag::new(0x0008, 0x2112), vec![item]);

        let ff = FileFormat::from_dataset("1.2.840.10008.5.1.4.1.1.2", "1.2.3.4.5", ds);
        let mut buf = Vec::new();
        DicomWriter::new(std::io::Cursor::new(&mut buf))
            .write_file(&ff)
            .unwrap();
        Bytes::from(buf)
    }

    fn make_encapsulated_pdf_dicom() -> Bytes {
        let mut ds = DataSet::new();
        ds.set_string(tags::STUDY_INSTANCE_UID, Vr::UI, "1.2.3");
        ds.set_string(tags::SERIES_INSTANCE_UID, Vr::UI, "1.2.3.4");
        ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, "1.2.3.4.5");
        ds.set_string(tags::SOP_CLASS_UID, Vr::UI, "1.2.840.10008.5.1.4.1.1.104.1");
        ds.set_string(Tag::new(0x0042, 0x0012), Vr::LO, "application/pdf");
        ds.insert(Element::bytes(
            Tag::new(0x0042, 0x0011),
            Vr::OB,
            b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n".to_vec(),
        ));

        let ff = FileFormat::from_dataset("1.2.840.10008.5.1.4.1.1.104.1", "1.2.3.4.5", ds);
        let mut buf = Vec::new();
        DicomWriter::new(std::io::Cursor::new(&mut buf))
            .write_file(&ff)
            .unwrap();
        Bytes::from(buf)
    }

    fn make_instance_from_dicom(dicom: &Bytes) -> Instance {
        ParsedDicom::from_bytes(dicom.clone()).unwrap().instance
    }

    fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
        assert!(bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]));
        let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        (width, height)
    }

    #[tokio::test]
    async fn test_instance_metadata_returns_dicom_json() {
        let dicom = make_nested_bulkdata_dicom();
        let instance = make_instance_from_dicom(&dicom);

        let mut store = MockMetaStore::new();
        store
            .expect_get_instance()
            .once()
            .returning(move |_| Ok(instance.clone()));

        let mut blobs = MockBlobStr::new();
        blobs
            .expect_get()
            .once()
            .returning(move |_| Ok(dicom.clone()));

        let app = build_router(make_test_state(store, blobs));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/metadata")
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
            json[0]["00111010"]["BulkDataURI"],
            json!("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/bulkdata/00111010")
        );
        assert_eq!(
            json[0]["00082112"]["Value"][0]["00111011"]["BulkDataURI"],
            json!("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/bulkdata/00082112/0/00111011")
        );
        assert!(json[0]["00111010"].get("InlineBinary").is_none());
    }

    #[tokio::test]
    async fn test_instance_metadata_honors_forwarded_prefix_for_bulkdata_uris() {
        let dicom = make_nested_bulkdata_dicom();
        let instance = make_instance_from_dicom(&dicom);

        let mut store = MockMetaStore::new();
        store
            .expect_get_instance()
            .once()
            .returning(move |_| Ok(instance.clone()));

        let mut blobs = MockBlobStr::new();
        blobs
            .expect_get()
            .once()
            .returning(move |_| Ok(dicom.clone()));

        let app = build_router(make_test_state(store, blobs));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/metadata")
                    .header("x-forwarded-prefix", "/pacs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json[0]["00111010"]["BulkDataURI"],
            json!("/pacs/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/bulkdata/00111010")
        );
        assert_eq!(
            json[0]["00082112"]["Value"][0]["00111011"]["BulkDataURI"],
            json!("/pacs/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/bulkdata/00082112/0/00111011")
        );
    }

    #[tokio::test]
    async fn test_study_metadata_returns_all_instance_metadata() {
        let dicom1 = make_nested_bulkdata_dicom_with_uids("1.2.3", "1.2.3.4", "1.2.3.4.5");
        let dicom2 = make_nested_bulkdata_dicom_with_uids("1.2.3", "1.2.3.4", "1.2.3.4.6");
        let instance1 = make_instance_from_dicom(&dicom1);
        let instance2 = make_instance_from_dicom(&dicom2);

        let mut store = MockMetaStore::new();
        store.expect_get_study().once().returning(|_| {
            Ok(Study {
                study_uid: StudyUid::from("1.2.3"),
                patient_id: None,
                patient_name: None,
                study_date: None,
                study_time: None,
                accession_number: None,
                modalities: vec!["CT".into()],
                referring_physician: None,
                description: None,
                num_series: 1,
                num_instances: 2,
                metadata: DicomJson::empty(),
                created_at: None,
                updated_at: None,
            })
        });
        store.expect_query_series().once().returning(|_| {
            Ok(vec![Series {
                series_uid: SeriesUid::from("1.2.3.4"),
                study_uid: StudyUid::from("1.2.3"),
                modality: Some("CT".into()),
                series_number: Some(1),
                description: None,
                body_part: None,
                num_instances: 2,
                metadata: DicomJson::empty(),
                created_at: None,
            }])
        });
        store
            .expect_query_instances()
            .once()
            .returning(move |_| Ok(vec![instance1.clone(), instance2.clone()]));

        let mut blobs = MockBlobStr::new();
        blobs.expect_get().times(2).returning(move |key| match key {
            "1.2.3/1.2.3.4/1.2.3.4.5" => Ok(dicom1.clone()),
            "1.2.3/1.2.3.4/1.2.3.4.6" => Ok(dicom2.clone()),
            other => panic!("unexpected blob key: {other}"),
        });

        let app = build_router(make_test_state(store, blobs));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/metadata")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let entries = json.as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["00080018"]["Value"][0], json!("1.2.3.4.5"));
        assert_eq!(entries[1]["00080018"]["Value"][0], json!("1.2.3.4.6"));
    }

    #[tokio::test]
    async fn test_series_metadata_returns_all_instance_metadata() {
        let dicom1 = make_nested_bulkdata_dicom_with_uids("1.2.3", "1.2.3.4", "1.2.3.4.5");
        let dicom2 = make_nested_bulkdata_dicom_with_uids("1.2.3", "1.2.3.4", "1.2.3.4.6");
        let instance1 = make_instance_from_dicom(&dicom1);
        let instance2 = make_instance_from_dicom(&dicom2);

        let mut store = MockMetaStore::new();
        store.expect_get_series().once().returning(|_| {
            Ok(Series {
                series_uid: SeriesUid::from("1.2.3.4"),
                study_uid: StudyUid::from("1.2.3"),
                modality: Some("CT".into()),
                series_number: Some(1),
                description: None,
                body_part: None,
                num_instances: 2,
                metadata: DicomJson::empty(),
                created_at: None,
            })
        });
        store
            .expect_query_instances()
            .once()
            .returning(move |_| Ok(vec![instance1.clone(), instance2.clone()]));

        let mut blobs = MockBlobStr::new();
        blobs.expect_get().times(2).returning(move |key| match key {
            "1.2.3/1.2.3.4/1.2.3.4.5" => Ok(dicom1.clone()),
            "1.2.3/1.2.3.4/1.2.3.4.6" => Ok(dicom2.clone()),
            other => panic!("unexpected blob key: {other}"),
        });

        let app = build_router(make_test_state(store, blobs));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/metadata")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let entries = json.as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["00080018"]["Value"][0], json!("1.2.3.4.5"));
        assert_eq!(entries[1]["00080018"]["Value"][0], json!("1.2.3.4.6"));
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
    async fn test_render_instance_honors_accept_jpeg() {
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
                    .header(header::ACCEPT, "image/jpeg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/jpeg"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(body.starts_with(&[0xFF, 0xD8, 0xFF]));
    }

    #[tokio::test]
    async fn test_render_instance_honors_query_accept_jpeg() {
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
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/rendered?accept=image/jpeg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/jpeg"
        );
    }

    #[tokio::test]
    async fn test_thumbnail_instance_defaults_to_jpeg_and_128_square() {
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
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/thumbnail")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/jpeg"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(body.starts_with(&[0xFF, 0xD8, 0xFF]));
    }

    #[tokio::test]
    async fn test_thumbnail_instance_honors_png_query_accept() {
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
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/thumbnail?accept=image/png")
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
    async fn test_bulkdata_supports_nested_attribute_paths() {
        let dicom = make_nested_bulkdata_dicom();
        let instance = make_instance_from_dicom(&dicom);

        let mut store = MockMetaStore::new();
        store
            .expect_get_instance()
            .once()
            .returning(move |_| Ok(instance.clone()));

        let mut blobs = MockBlobStr::new();
        blobs
            .expect_get()
            .once()
            .returning(move |_| Ok(dicom.clone()));

        let app = build_router(make_test_state(store, blobs));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/bulkdata/00082112/0/00111011")
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
        assert_eq!(body.as_ref(), &[0xDE, 0xAD]);
    }

    #[tokio::test]
    async fn test_bulkdata_returns_declared_encapsulated_document_media_type() {
        let dicom = make_encapsulated_pdf_dicom();
        let instance = make_instance_from_dicom(&dicom);

        let mut store = MockMetaStore::new();
        store
            .expect_get_instance()
            .once()
            .returning(move |_| Ok(instance.clone()));

        let mut blobs = MockBlobStr::new();
        blobs
            .expect_get()
            .once()
            .returning(move |_| Ok(dicom.clone()));

        let app = build_router(make_test_state(store, blobs));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5/bulkdata/00420011")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/pdf"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(body.starts_with(b"%PDF-1.7"));
    }

    #[tokio::test]
    async fn test_bulkdata_returns_video_mp4_for_mpeg4_transfer_syntax() {
        let mut store = MockMetaStore::new();
        let mut instance = make_instance();
        instance.transfer_syntax = Some("1.2.840.10008.1.2.4.102".into());
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
            "video/mp4"
        );
    }

    #[tokio::test]
    async fn test_bulkdata_returns_video_mpeg_for_mpeg2_transfer_syntax() {
        let mut store = MockMetaStore::new();
        let mut instance = make_instance();
        instance.transfer_syntax = Some("1.2.840.10008.1.2.4.100".into());
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
            "video/mpeg"
        );
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

    #[tokio::test]
    async fn test_retrieve_instance_honors_accept_transfer_syntax() {
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
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5")
                    .header(
                        header::ACCEPT,
                        "multipart/related; type=\"application/dicom\"; transfer-syntax=1.2.840.10008.1.2.1.99",
                    )
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
            .unwrap()
            .to_owned();
        assert!(content_type.contains("transfer-syntax=1.2.840.10008.1.2.1.99"));
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let boundary = content_type.split("boundary=").nth(1).unwrap().trim();
        let boundary_marker = format!("\r\n--{boundary}");
        let start = body
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|idx| idx + 4)
            .unwrap();
        let end = body[start..]
            .windows(boundary_marker.len())
            .position(|window| window == boundary_marker.as_bytes())
            .map(|idx| start + idx)
            .unwrap();
        let part = &body.as_ref()[start..end];
        let file = DicomReader::new(std::io::Cursor::new(part))
            .read_file()
            .unwrap();
        assert_eq!(
            file.meta.transfer_syntax_uid,
            transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid
        );
    }

    #[tokio::test]
    async fn test_retrieve_instance_rejects_unsupported_accept_media_type() {
        let app = build_router(make_test_state(MockMetaStore::new(), MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5")
                    .header(header::ACCEPT, "image/jpeg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
    }

    #[tokio::test]
    async fn test_wado_uri_supports_transfer_syntax_query() {
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
                    .uri("/wado?requestType=WADO&studyUID=1.2.3&seriesUID=1.2.3.4&objectUID=1.2.3.4.5&transferSyntax=1.2.840.10008.1.2.1.99")
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
        assert!(content_type.contains("transfer-syntax=1.2.840.10008.1.2.1.99"));
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let file = DicomReader::new(std::io::Cursor::new(body.as_ref()))
            .read_file()
            .unwrap();
        assert_eq!(
            file.meta.transfer_syntax_uid,
            transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid
        );
    }

    #[tokio::test]
    async fn test_wado_uri_supports_jpeg_rendering() {
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
                    .uri("/wado?requestType=WADO&studyUID=1.2.3&seriesUID=1.2.3.4&objectUID=1.2.3.4.5&contentType=image/jpeg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/jpeg"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(body.starts_with(&[0xFF, 0xD8, 0xFF]));
    }

    #[tokio::test]
    async fn test_wado_uri_rendered_rows_resizes_png() {
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
                    .uri("/wado?requestType=WADO&studyUID=1.2.3&seriesUID=1.2.3.4&objectUID=1.2.3.4.5&contentType=image/png&rows=4")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert_eq!(png_dimensions(&body), (8, 4));
    }

    #[tokio::test]
    async fn test_wado_uri_rejects_incomplete_window_parameters() {
        let mut store = MockMetaStore::new();
        store.expect_get_instance().never();

        let mut blobs = MockBlobStr::new();
        blobs.expect_get().never();

        let app = build_router(make_test_state(store, blobs));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/wado?requestType=WADO&studyUID=1.2.3&seriesUID=1.2.3.4&objectUID=1.2.3.4.5&contentType=image/png&windowCenter=50")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(String::from_utf8_lossy(&body)
            .contains("windowCenter and windowWidth must be provided together"));
    }

    #[test]
    fn test_build_multipart_body_contains_boundary() {
        let data = Bytes::from_static(b"DICOM-DATA");
        let body = build_multipart_body(&[data], "TESTBOUNDARY", "application/dicom", None);
        let s = std::str::from_utf8(&body).unwrap();
        assert!(s.contains("--TESTBOUNDARY"));
        assert!(s.contains("--TESTBOUNDARY--"));
    }
}
