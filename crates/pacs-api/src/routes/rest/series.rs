//! REST handlers for series resources.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use pacs_core::{SeriesQuery, SeriesUid, StudyUid};

use crate::{error::ApiError, state::AppState};

/// `GET /api/studies/:study_uid/series` — list series for a study.
pub async fn list_series_for_study(
    State(state): State<AppState>,
    Path(study_uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let series = state
        .store
        .query_series(&SeriesQuery {
            study_uid: StudyUid::from(study_uid.as_str()),
            series_uid: None,
            modality: None,
            series_number: None,
            limit: None,
            offset: None,
        })
        .await?;
    Ok(Json(series))
}

/// `GET /api/series/:series_uid` — fetch a single series by UID.
pub async fn get_series(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let series = state
        .store
        .get_series(&SeriesUid::from(uid.as_str()))
        .await?;
    Ok(Json(series))
}

/// `DELETE /api/series/:series_uid` — delete a series and its instances.
///
/// Returns `204 No Content` on success.
pub async fn delete_series(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .store
        .delete_series(&SeriesUid::from(uid.as_str()))
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
    async fn test_get_series_not_found_returns_404() {
        let mut store = MockMetaStore::new();
        store.expect_get_series().once().returning(|uid| {
            Err(PacsError::NotFound {
                resource: "series",
                uid: uid.to_string(),
            })
        });
        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/series/missing_uid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_series_returns_204() {
        let mut store = MockMetaStore::new();
        store.expect_delete_series().once().returning(|_| Ok(()));
        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/series/1.2.3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }
}
