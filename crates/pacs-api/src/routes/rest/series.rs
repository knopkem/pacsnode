//! REST handlers for series resources.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use pacs_core::{PolicyAction, SeriesQuery, SeriesUid, StudyUid};
use pacs_plugin::{AuthenticatedUser, PacsEvent, ResourceLevel};

use crate::{
    error::ApiError,
    policy::{apply_series_query_filters, authorize_action, authorize_series, filter_series},
    state::AppState,
};

use super::{cleanup_blob_keys, collect_series_blob_keys};

/// `GET /api/studies/:study_uid/series` — list series for a study.
pub async fn list_series_for_study(
    State(state): State<AppState>,
    user: Option<axum::Extension<AuthenticatedUser>>,
    Path(study_uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let auth_user = user.as_ref().map(|extension| &extension.0);
    authorize_action(auth_user, PolicyAction::Read)?;
    let mut query = SeriesQuery {
        study_uid: StudyUid::from(study_uid.as_str()),
        series_uid: None,
        modality: None,
        series_number: None,
        limit: None,
        offset: None,
    };
    apply_series_query_filters(auth_user, &mut query)?;
    let series = filter_series(
        auth_user,
        state.store.query_series(&query).await?,
        PolicyAction::Read,
    )?;
    Ok(Json(series))
}

/// `GET /api/series/:series_uid` — fetch a single series by UID.
pub async fn get_series(
    State(state): State<AppState>,
    user: Option<axum::Extension<AuthenticatedUser>>,
    Path(uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let series = state
        .store
        .get_series(&SeriesUid::from(uid.as_str()))
        .await?;
    authorize_series(
        user.as_ref().map(|extension| &extension.0),
        &series,
        PolicyAction::Read,
    )?;
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
    if let Some(auth_user) = user.as_ref().map(|extension| &extension.0) {
        let series = state.store.get_series(&series_uid).await?;
        authorize_series(Some(auth_user), &series, PolicyAction::Delete)?;
    }
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
        Extension,
    };
    use pacs_core::{
        DicomJson, Instance, PacsError, Series, SeriesUid, SopInstanceUid, StudyUid, UserRole,
    };
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
    async fn test_delete_series_forbidden_for_viewer_returns_403() {
        let mut store = MockMetaStore::new();
        store.expect_get_series().once().returning(|_| {
            Ok(Series {
                series_uid: SeriesUid::from("1.2.3"),
                study_uid: StudyUid::from("1.2"),
                modality: Some("CT".into()),
                series_number: Some(1),
                description: None,
                body_part: None,
                num_instances: 1,
                metadata: DicomJson::empty(),
                created_at: None,
            })
        });

        let app = build_router(make_test_state(store, MockBlobStr::new()))
            .layer(Extension(auth_user(UserRole::Viewer, json!({}))));
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

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
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
