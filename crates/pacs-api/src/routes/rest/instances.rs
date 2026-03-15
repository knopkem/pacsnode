//! REST handlers for instance resources.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use pacs_core::{InstanceQuery, SeriesUid, SopInstanceUid};

use crate::{error::ApiError, state::AppState};

/// `GET /api/series/:series_uid/instances` — list instances for a series.
pub async fn list_instances_for_series(
    State(state): State<AppState>,
    Path(series_uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
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
    Ok(Json(instances))
}

/// `GET /api/instances/:instance_uid` — fetch a single instance by UID.
pub async fn get_instance(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let instance = state
        .store
        .get_instance(&SopInstanceUid::from(uid.as_str()))
        .await?;
    Ok(Json(instance))
}

/// `DELETE /api/instances/:instance_uid` — delete a single instance.
///
/// Returns `204 No Content` on success.
pub async fn delete_instance(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .store
        .delete_instance(&SopInstanceUid::from(uid.as_str()))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use pacs_core::PacsError;
    use tower::ServiceExt;

    use crate::{
        router::build_router,
        test_support::{make_test_state, MockBlobStr, MockMetaStore},
    };

    #[tokio::test]
    async fn test_get_instance_not_found_returns_404() {
        let mut store = MockMetaStore::new();
        store.expect_get_instance().once().returning(|uid| {
            Err(PacsError::NotFound {
                resource: "instance",
                uid: uid.to_string(),
            })
        });
        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/instances/missing_uid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_instance_returns_204() {
        let mut store = MockMetaStore::new();
        store.expect_delete_instance().once().returning(|_| Ok(()));
        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/instances/1.2.3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }
}
