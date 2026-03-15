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

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use pacs_core::PacsStatistics;
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
}
