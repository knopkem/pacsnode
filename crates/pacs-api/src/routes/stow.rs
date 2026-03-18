//! STOW-RS (Store Over the Web) handler — `POST /wado/studies`.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use bytes::Bytes;
use pacs_core::{blob_key_for, PacsError};
use pacs_plugin::{AuthenticatedUser, PacsEvent};
use serde_json::json;

use crate::{error::ApiError, state::AppState};

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
    let user_id = user.map(|extension| extension.0.user_id);
    let mut stored = Vec::new();

    for p in &parsed {
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
    };
    use bytes::Bytes;
    use dicom_toolkit_data::{DataSet, DicomWriter, FileFormat};
    use dicom_toolkit_dict::{tags, Vr};
    use http_body_util::BodyExt;
    use serde_json::json;
    use tower::ServiceExt;

    use crate::{
        router::build_router,
        test_support::{make_test_state, MockBlobStr, MockMetaStore},
    };

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
