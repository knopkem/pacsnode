//! STOW-RS (Store Over the Web) handler — `POST /wado/studies`.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use bytes::Bytes;
use pacs_core::{blob_key_for, PacsError, PolicyAction, PolicyEngine, PolicyResource, UserRole};
use pacs_dicom::{transcode_part10, ParsedDicom};
use pacs_plugin::{AuthenticatedUser, PacsEvent};
use serde_json::json;

use crate::{error::ApiError, policy::authorize_action, state::AppState};

fn prepare_part_for_storage(
    part: &ParsedDicom,
    storage_transfer_syntax: Option<&str>,
) -> Result<ParsedDicom, ApiError> {
    let Some(target_ts_uid) = storage_transfer_syntax
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(part.clone());
    };

    if part.transfer_syntax_uid == target_ts_uid {
        return Ok(part.clone());
    }

    // For image compression transfer syntaxes, attempt transcoding and handle the special case
    // where there's no pixel data to compress. This is more robust than trying to detect
    // pixel data presence upfront, as the transcoding library knows exactly what it needs.
    if is_image_compression_transfer_syntax(target_ts_uid) {
        match transcode_part10(part.encoded_bytes.clone(), target_ts_uid) {
            Err(error) => {
                let error_msg = error.to_string();
                if error_msg.contains("without PixelData") || error_msg.contains("without pixel data") {
                    // This instance doesn't have pixel data, so skip image compression
                    return Ok(part.clone());
                }
                // For other transcoding errors, propagate them as before
                return Err(ApiError::from(PacsError::DicomParse(error_msg)));
            }
            Ok(transcoded) => {
                return ParsedDicom::from_bytes(transcoded).map_err(ApiError::from);
            }
        }
    }

    let transcoded = transcode_part10(part.encoded_bytes.clone(), target_ts_uid)
        .map_err(|error| ApiError::from(PacsError::DicomParse(error.to_string())))?;
    ParsedDicom::from_bytes(transcoded).map_err(ApiError::from)
}



/// Check if a transfer syntax UID represents image compression
fn is_image_compression_transfer_syntax(ts_uid: &str) -> bool {
    matches!(ts_uid,
        // JPEG 2000 variants
        "1.2.840.10008.1.2.4.90" |  // JPEG 2000 Lossless
        "1.2.840.10008.1.2.4.91" |  // JPEG 2000
        "1.2.840.10008.1.2.4.92" |  // JPEG 2000 Part 2 Multi-component Lossless
        "1.2.840.10008.1.2.4.93" |  // JPEG 2000 Part 2 Multi-component
        // HTJ2K variants  
        "1.2.840.10008.1.2.4.201" | // HTJ2K Lossless
        "1.2.840.10008.1.2.4.202" | // HTJ2K Lossless RPT
        "1.2.840.10008.1.2.4.203" | // HTJ2K
        // JPEG variants
        "1.2.840.10008.1.2.4.50" |  // JPEG Baseline
        "1.2.840.10008.1.2.4.51" |  // JPEG Extended
        "1.2.840.10008.1.2.4.52" |  // JPEG Extended (3,5)
        "1.2.840.10008.1.2.4.53" |  // JPEG Spectral Selection Non-Hierarchical (6,8)
        "1.2.840.10008.1.2.4.54" |  // JPEG Spectral Selection Non-Hierarchical (7,9)
        "1.2.840.10008.1.2.4.55" |  // JPEG Full Progression Non-Hierarchical (10,12)
        "1.2.840.10008.1.2.4.56" |  // JPEG Full Progression Non-Hierarchical (11,13)
        "1.2.840.10008.1.2.4.57" |  // JPEG Lossless Non-Hierarchical (14)
        "1.2.840.10008.1.2.4.58" |  // JPEG Lossless Non-Hierarchical (15)
        "1.2.840.10008.1.2.4.59" |  // JPEG Extended Hierarchical (16,18)
        "1.2.840.10008.1.2.4.60" |  // JPEG Extended Hierarchical (17,19)
        "1.2.840.10008.1.2.4.61" |  // JPEG Spectral Selection Hierarchical (20,22)
        "1.2.840.10008.1.2.4.62" |  // JPEG Spectral Selection Hierarchical (21,23)
        "1.2.840.10008.1.2.4.63" |  // JPEG Full Progression Hierarchical (24,26)
        "1.2.840.10008.1.2.4.64" |  // JPEG Full Progression Hierarchical (25,27)
        "1.2.840.10008.1.2.4.65" |  // JPEG Lossless Hierarchical (28)
        "1.2.840.10008.1.2.4.66" |  // JPEG Lossless Hierarchical (29)
        "1.2.840.10008.1.2.4.70" |  // JPEG Lossless Non-Hierarchical First-Order Prediction
        // JPEG-LS variants
        "1.2.840.10008.1.2.4.80" |  // JPEG-LS Lossless
        "1.2.840.10008.1.2.4.81" |  // JPEG-LS Lossy Near-Lossless
        // RLE
        "1.2.840.10008.1.2.5" |     // RLE Lossless
        // MPEG variants
        "1.2.840.10008.1.2.4.100" | // MPEG2 Main Profile @ Main Level
        "1.2.840.10008.1.2.4.101" | // MPEG2 Main Profile @ High Level
        "1.2.840.10008.1.2.4.102" | // MPEG-4 AVC/H.264 High Profile
        "1.2.840.10008.1.2.4.103" | // MPEG-4 AVC/H.264 BD-compatible High Profile
        "1.2.840.10008.1.2.4.104" | // MPEG-4 AVC/H.264 High Profile For 2D Video
        "1.2.840.10008.1.2.4.105" | // MPEG-4 AVC/H.264 High Profile For 3D Video
        "1.2.840.10008.1.2.4.106" | // MPEG-4 AVC/H.264 Stereo High Profile
        "1.2.840.10008.1.2.4.107" | // HEVC/H.265 Main Profile
        "1.2.840.10008.1.2.4.108"   // HEVC/H.265 Main 10 Profile
    )
}

/// `POST /wado/studies` — STOW-RS store endpoint (PS3.18 §10.5).
///
/// Accepts a `multipart/related; type="application/dicom"` request body,
/// parses each DICOM part, persists blobs and metadata, and returns the
/// PS3.18 store response JSON.
pub async fn stow_store(
    State(state): State<AppState>,
    user: Option<axum::Extension<AuthenticatedUser>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let auth_user = user.as_ref().map(|extension| &extension.0);
    authorize_action(auth_user, PolicyAction::Upload)?;

    // Extract Content-Type and boundary.
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| PacsError::DicomParse("missing Content-Type header".into()))?
        .to_owned();

    let boundary = extract_boundary(&content_type)
        .ok_or_else(|| PacsError::DicomParse("missing boundary in Content-Type".into()))?;

    // Parse multipart body.
    let parsed = pacs_dicom::parse_stow_multipart(body, &boundary).await?;

    if parsed.is_empty() {
        return Err(PacsError::DicomParse("no valid DICOM instances in request".into()).into());
    }

    // Persist each instance.
    let study_uid_str = parsed[0].instance.study_uid.to_string();
    let user_id = auth_user.map(|user| user.user_id.clone());
    let mut stored = Vec::new();

    if let Some(user) = auth_user {
        let role = user
            .role
            .parse::<UserRole>()
            .map_err(|_| PacsError::Internal("authenticated user has invalid role state".into()))?;
        let subject = pacs_core::PolicyUser::new(role, &user.attributes);
        let engine = PolicyEngine::new();
        for part in &parsed {
            if !engine.check_permission(
                &subject,
                PolicyAction::Upload,
                PolicyResource::Series {
                    modality: part.series.modality.as_deref(),
                },
            ) {
                return Err(PacsError::Forbidden(format!(
                    "role '{}' cannot upload this series modality",
                    user.role
                ))
                .into());
            }
        }
    }

    for parsed_part in &parsed {
        let p = prepare_part_for_storage(
            parsed_part,
            state.server_settings.storage_transfer_syntax.as_deref(),
        )?;
        let blob_key = blob_key_for(
            &p.instance.study_uid,
            &p.instance.series_uid,
            &p.instance.instance_uid,
        );
        state
            .blobs
            .put(&blob_key, p.encoded_bytes.clone())
            .await
            .map_err(ApiError::from)?;
        state
            .store
            .store_study(&p.study)
            .await
            .map_err(ApiError::from)?;
        state
            .store
            .store_series(&p.series)
            .await
            .map_err(ApiError::from)?;
        state
            .store
            .store_instance(&p.instance)
            .await
            .map_err(ApiError::from)?;

        state
            .plugins
            .emit_event(PacsEvent::InstanceStored {
                study_uid: p.instance.study_uid.to_string(),
                series_uid: p.instance.series_uid.to_string(),
                sop_instance_uid: p.instance.instance_uid.to_string(),
                sop_class_uid: p.instance.sop_class_uid.clone().unwrap_or_default(),
                source: "STOW-RS".into(),
                user_id: user_id.clone(),
            })
            .await;

        stored.push(json!({
            "00081150": { "vr": "UI", "Value": [p.instance.sop_class_uid.as_deref().unwrap_or("")] },
            "00081155": { "vr": "UI", "Value": [p.instance.instance_uid.to_string()] },
        }));
    }

    // PS3.18 STOW-RS response
    let response_body = json!({
        "00081190": { "vr": "UR", "Value": [format!("wado/studies/{study_uid_str}")] },
        "00081199": { "vr": "SQ", "Value": stored },
    });

    Ok((StatusCode::OK, Json(response_body)))
}

/// Extract the `boundary` parameter from a `Content-Type` header value.
fn extract_boundary(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .find(|part| part.trim().to_ascii_lowercase().starts_with("boundary="))
        .map(|part| {
            let raw = part.trim()["boundary=".len()..].trim();
            raw.trim_matches('"').to_owned()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        Extension,
    };
    use bytes::Bytes;
    use dicom_toolkit_data::{DataSet, DicomWriter, FileFormat};
    use dicom_toolkit_dict::{tags, ts::transfer_syntaxes, Vr};
    use http_body_util::BodyExt;
    use pacs_core::UserRole;
    use pacs_plugin::AuthenticatedUser;
    use serde_json::json;
    use tower::ServiceExt;

    use crate::{
        router::build_router,
        test_support::{make_test_state, MockBlobStr, MockMetaStore},
    };

    fn auth_user(role: UserRole, attributes: serde_json::Value) -> AuthenticatedUser {
        AuthenticatedUser::new("1", "alice", role.as_str(), attributes)
    }

    fn make_dicom_part(sop_class_uid: &str, modality: &str, instance_uid: &str) -> Vec<u8> {
        let mut ds = DataSet::new();
        ds.set_string(tags::STUDY_INSTANCE_UID, Vr::UI, "1.2.3");
        ds.set_string(tags::SERIES_INSTANCE_UID, Vr::UI, "1.2.3.4");
        ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, instance_uid);
        ds.set_string(tags::SOP_CLASS_UID, Vr::UI, sop_class_uid);
        ds.set_string(tags::MODALITY, Vr::CS, modality);

        let ff = FileFormat::from_dataset(sop_class_uid, instance_uid, ds);
        let mut buf = Vec::new();
        DicomWriter::new(std::io::Cursor::new(&mut buf))
            .write_file(&ff)
            .unwrap();
        buf
    }

    #[test]
    fn prepare_part_for_storage_preserves_source_when_unconfigured() {
        let bytes = Bytes::from(make_dicom_part(
            "1.2.840.10008.5.1.4.1.1.2",
            "CT",
            "1.2.3.4.5",
        ));
        let parsed = ParsedDicom::from_bytes(bytes).unwrap();

        let prepared = prepare_part_for_storage(&parsed, None).unwrap();
        assert_eq!(prepared.transfer_syntax_uid, parsed.transfer_syntax_uid);
        assert_eq!(prepared.encoded_bytes, parsed.encoded_bytes);
    }

    #[test]
    fn prepare_part_for_storage_transcodes_when_configured() {
        let bytes = Bytes::from(make_dicom_part(
            "1.2.840.10008.5.1.4.1.1.2",
            "CT",
            "1.2.3.4.6",
        ));
        let parsed = ParsedDicom::from_bytes(bytes).unwrap();

        let prepared = prepare_part_for_storage(
            &parsed,
            Some(transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid),
        )
        .unwrap();
        assert_eq!(
            prepared.transfer_syntax_uid,
            transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid
        );
        assert_eq!(
            prepared.instance.transfer_syntax.as_deref(),
            Some(transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid)
        );
    }

    fn build_multipart_body(boundary: &str, parts: &[Vec<u8>]) -> Bytes {
        let mut body = Vec::new();
        for part in parts {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            body.extend_from_slice(b"Content-Type: application/dicom\r\n");
            body.extend_from_slice(b"\r\n");
            body.extend_from_slice(part);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        Bytes::from(body)
    }

    #[test]
    fn test_extract_boundary_unquoted() {
        assert_eq!(
            extract_boundary("multipart/related; boundary=abc"),
            Some("abc".into())
        );
    }

    #[test]
    fn test_extract_boundary_quoted() {
        assert_eq!(
            extract_boundary("multipart/related; boundary=\"my-boundary\""),
            Some("my-boundary".into())
        );
    }

    #[test]
    fn test_extract_boundary_missing() {
        assert_eq!(extract_boundary("application/json"), None);
    }

    #[tokio::test]
    async fn test_stow_missing_content_type_returns_400() {
        let app = build_router(make_test_state(MockMetaStore::new(), MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/wado/studies")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_stow_invalid_content_type_returns_400() {
        let app = build_router(make_test_state(MockMetaStore::new(), MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/wado/studies")
                    .header("content-type", "application/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_stow_forbidden_for_viewer_returns_403() {
        let app = build_router(make_test_state(MockMetaStore::new(), MockBlobStr::new()))
            .layer(Extension(auth_user(UserRole::Viewer, json!({}))));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/wado/studies")
                    .header(
                        "content-type",
                        "multipart/related; type=\"application/dicom\"; boundary=blocked",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_stow_accepts_sr_and_seg_sop_classes() {
        let mut store = MockMetaStore::new();
        store.expect_store_study().times(2).returning(|_| Ok(()));
        store.expect_store_series().times(2).returning(|series| {
            assert!(matches!(
                series.modality.as_deref(),
                Some("SR") | Some("SEG")
            ));
            Ok(())
        });
        store
            .expect_store_instance()
            .times(2)
            .returning(|instance| {
                assert!(matches!(
                    instance.sop_class_uid.as_deref(),
                    Some("1.2.840.10008.5.1.4.1.1.88.22") | Some("1.2.840.10008.5.1.4.1.1.66.4")
                ));
                Ok(())
            });

        let mut blobs = MockBlobStr::new();
        blobs.expect_put().times(2).returning(|_, _| Ok(()));

        let boundary = "stow-boundary";
        let sr = make_dicom_part("1.2.840.10008.5.1.4.1.1.88.22", "SR", "1.2.3.4.5");
        let seg = make_dicom_part("1.2.840.10008.5.1.4.1.1.66.4", "SEG", "1.2.3.4.6");
        let body = build_multipart_body(boundary, &[sr, seg]);

        let app = build_router(make_test_state(store, blobs));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/wado/studies")
                    .header(
                        "content-type",
                        format!(
                            "multipart/related; type=\"application/dicom\"; boundary={boundary}"
                        ),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let stored = json["00081199"]["Value"].as_array().unwrap();
        assert_eq!(stored.len(), 2);
        assert_eq!(
            stored[0]["00081150"]["Value"][0],
            json!("1.2.840.10008.5.1.4.1.1.88.22")
        );
        assert_eq!(
            stored[1]["00081150"]["Value"][0],
            json!("1.2.840.10008.5.1.4.1.1.66.4")
        );
    }
}
