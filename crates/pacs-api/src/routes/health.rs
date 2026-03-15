//! Health and statistics handlers.

use axum::{extract::State, response::IntoResponse, Json};
use serde_json::json;

use crate::{error::ApiError, state::AppState};

/// `GET /health` — returns `{"status":"ok"}` with HTTP 200.
pub async fn get_health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

/// `GET /statistics` — returns aggregate PACS counts and disk usage.
pub async fn get_statistics(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let s = state.store.get_statistics().await?;
    Ok(Json(json!({
        "studies":          s.num_studies,
        "series":           s.num_series,
        "instances":        s.num_instances,
        "disk_usage_bytes": s.disk_usage_bytes,
    })))
}

/// `GET /system` — returns server identity and all registered remote nodes.
///
/// Useful for DICOM clients that need to discover the AE title, ports, and
/// the whitelist of known remote Application Entities.
///
/// # Response
///
/// ```json
/// {
///   "ae_title":   "PACSNODE",
///   "http_port":  8042,
///   "dicom_port": 4242,
///   "version":    "0.1.0",
///   "nodes": [
///     {
///       "ae_title": "MODALITY1",
///       "host": "192.168.1.10",
///       "port": 104,
///       "description": "CT Scanner",
///       "tls_enabled": false
///     }
///   ]
/// }
/// ```
pub async fn get_system_info(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let nodes = state.store.list_nodes().await?;
    Ok(Json(json!({
        "ae_title":   state.server_info.ae_title,
        "http_port":  state.server_info.http_port,
        "dicom_port": state.server_info.dicom_port,
        "version":    state.server_info.version,
        "nodes":      nodes,
    })))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use pacs_core::{DicomNode, PacsStatistics};
    use tower::ServiceExt;

    use crate::{
        router::build_router,
        test_support::{make_test_state, MockBlobStr, MockMetaStore},
    };

    #[tokio::test]
    async fn test_health_returns_200() {
        let app = build_router(make_test_state(MockMetaStore::new(), MockBlobStr::new()));
        let resp = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_statistics_returns_counts() {
        let mut store = MockMetaStore::new();
        store.expect_get_statistics().once().returning(|| {
            Ok(PacsStatistics {
                num_studies: 5,
                num_series: 20,
                num_instances: 200,
                disk_usage_bytes: 1024,
            })
        });
        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/statistics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["studies"], 5);
        assert_eq!(json["series"], 20);
        assert_eq!(json["instances"], 200);
    }

    #[tokio::test]
    async fn test_system_info_returns_server_fields() {
        let mut store = MockMetaStore::new();
        store.expect_list_nodes().once().returning(|| Ok(vec![]));
        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/system")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["ae_title"], "TESTPACS");
        assert_eq!(json["http_port"], 8042);
        assert_eq!(json["dicom_port"], 4242);
        assert!(json["version"].is_string());
        assert!(json["nodes"].is_array());
    }

    #[tokio::test]
    async fn test_system_info_includes_registered_nodes() {
        let mut store = MockMetaStore::new();
        store.expect_list_nodes().once().returning(|| {
            Ok(vec![DicomNode {
                ae_title: "MODALITY1".into(),
                host: "192.168.1.10".into(),
                port: 104,
                description: Some("CT Scanner".into()),
                tls_enabled: false,
            }])
        });
        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/system")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["nodes"][0]["ae_title"], "MODALITY1");
        assert_eq!(json["nodes"][0]["port"], 104);
        assert_eq!(json["nodes"][0]["tls_enabled"], false);
    }
}

