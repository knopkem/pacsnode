//! REST handlers for study resources — `GET/DELETE /api/studies[/:uid]`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use pacs_core::{StudyQuery, StudyUid};

use crate::{error::ApiError, state::AppState};

/// `GET /api/studies` — list all studies.
pub async fn list_studies(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let studies = state.store.query_studies(&StudyQuery::default()).await?;
    Ok(Json(studies))
}

/// `GET /api/studies/:study_uid` — fetch a single study by UID.
pub async fn get_study(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let study = state
        .store
        .get_study(&StudyUid::from(uid.as_str()))
        .await?;
    Ok(Json(study))
}

/// `DELETE /api/studies/:study_uid` — delete a study and its dependants.
///
/// Returns `204 No Content` on success.
pub async fn delete_study(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .store
        .delete_study(&StudyUid::from(uid.as_str()))
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
    async fn test_get_study_not_found_returns_404() {
        let mut store = MockMetaStore::new();
        store.expect_get_study().once().returning(|uid| {
            Err(PacsError::NotFound {
                resource: "study",
                uid: uid.to_string(),
            })
        });
        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/studies/bad_uid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_list_studies_returns_empty_array() {
        let mut store = MockMetaStore::new();
        store
            .expect_query_studies()
            .once()
            .returning(|_| Ok(vec![]));
        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/studies")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_delete_study_returns_204() {
        let mut store = MockMetaStore::new();
        store.expect_delete_study().once().returning(|_| Ok(()));
        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/studies/1.2.3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }
}
