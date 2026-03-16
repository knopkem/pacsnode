//! REST handlers for series resources.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use pacs_core::{SeriesQuery, SeriesUid, StudyUid};
use pacs_plugin::{AuthenticatedUser, PacsEvent, ResourceLevel};

use crate::{error::ApiError, state::AppState};

use super::{cleanup_blob_keys, collect_series_blob_keys};

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
    user: Option<axum::Extension<AuthenticatedUser>>,
    Path(uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let series_uid = SeriesUid::from(uid.as_str());
    let blob_keys = collect_series_blob_keys(&state, &series_uid).await?;
    state.store.delete_series(&series_uid).await?;
    cleanup_blob_keys(&state, blob_keys).await;
    state
        .plugins
        .emit_event(PacsEvent::ResourceDeleted {
            level: ResourceLevel::Series,
            uid: series_uid.to_string(),
            user_id: user.map(|extension| extension.0.user_id),
        })
        .await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use pacs_core::{DicomJson, Instance, PacsError, SeriesUid, SopInstanceUid, StudyUid};
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
        store
            .expect_query_instances()
            .once()
            .returning(|_| Ok(Vec::new()));
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

    #[tokio::test]
    async fn test_delete_series_deletes_all_instance_blobs() {
        let series_uid = SeriesUid::from("1.2.3");
        let study_uid = StudyUid::from("1.2");

        let mut store = MockMetaStore::new();
        store.expect_query_instances().once().returning({
            let series_uid = series_uid.clone();
            let study_uid = study_uid.clone();
            move |_| {
                Ok(vec![
                    Instance {
                        instance_uid: SopInstanceUid::from("1.2.3.1"),
                        series_uid: series_uid.clone(),
                        study_uid: study_uid.clone(),
                        sop_class_uid: None,
                        instance_number: Some(1),
                        transfer_syntax: None,
                        rows: None,
                        columns: None,
                        blob_key: "blob/one".into(),
                        metadata: DicomJson::empty(),
                        created_at: None,
                    },
                    Instance {
                        instance_uid: SopInstanceUid::from("1.2.3.2"),
                        series_uid: series_uid.clone(),
                        study_uid: study_uid.clone(),
                        sop_class_uid: None,
                        instance_number: Some(2),
                        transfer_syntax: None,
                        rows: None,
                        columns: None,
                        blob_key: "blob/two".into(),
                        metadata: DicomJson::empty(),
                        created_at: None,
                    },
                ])
            }
        });
        store.expect_delete_series().once().returning(|_| Ok(()));

        let mut blobs = MockBlobStr::new();
        blobs
            .expect_delete()
            .withf(|key| key == "blob/one")
            .once()
            .returning(|_| Ok(()));
        blobs
            .expect_delete()
            .withf(|key| key == "blob/two")
            .once()
            .returning(|_| Ok(()));

        let app = build_router(make_test_state(store, blobs));
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
