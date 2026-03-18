//! REST handlers for instance resources.

use std::collections::BTreeSet;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use pacs_core::{InstanceQuery, PolicyAction, SeriesUid, SopInstanceUid};
use pacs_plugin::{AuthenticatedUser, PacsEvent, ResourceLevel};

use crate::{
    error::ApiError,
    policy::{authorize_action, authorize_series},
    state::AppState,
};

use super::cleanup_blob_keys;

/// `GET /api/series/:series_uid/instances` — list instances for a series.
pub async fn list_instances_for_series(
    State(state): State<AppState>,
    user: Option<axum::Extension<AuthenticatedUser>>,
    Path(series_uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let auth_user = user.as_ref().map(|extension| &extension.0);
    authorize_action(auth_user, PolicyAction::Read)?;
    let series_uid = SeriesUid::from(series_uid.as_str());
    if let Some(auth_user) = auth_user {
        let series = state.store.get_series(&series_uid).await?;
        authorize_series(Some(auth_user), &series, PolicyAction::Read)?;
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
    Ok(Json(instances))
}

/// `GET /api/instances/:instance_uid` — fetch a single instance by UID.
pub async fn get_instance(
    State(state): State<AppState>,
    user: Option<axum::Extension<AuthenticatedUser>>,
    Path(uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let instance = state
        .store
        .get_instance(&SopInstanceUid::from(uid.as_str()))
        .await?;
    if let Some(auth_user) = user.as_ref().map(|extension| &extension.0) {
        let series = state.store.get_series(&instance.series_uid).await?;
        authorize_series(Some(auth_user), &series, PolicyAction::Read)?;
    }
    Ok(Json(instance))
}

/// `DELETE /api/instances/:instance_uid` — delete a single instance.
///
/// Returns `204 No Content` on success.
pub async fn delete_instance(
    State(state): State<AppState>,
    user: Option<axum::Extension<AuthenticatedUser>>,
    Path(uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let instance_uid = SopInstanceUid::from(uid.as_str());
    let instance = state.store.get_instance(&instance_uid).await?;
    if let Some(auth_user) = user.as_ref().map(|extension| &extension.0) {
        let series = state.store.get_series(&instance.series_uid).await?;
        authorize_series(Some(auth_user), &series, PolicyAction::Delete)?;
    }
    let blob_keys = BTreeSet::from([instance.blob_key.clone()]);
    state.store.delete_instance(&instance_uid).await?;
    cleanup_blob_keys(&state, blob_keys).await;
    state
        .plugins
        .emit_event(PacsEvent::ResourceDeleted {
            level: ResourceLevel::Instance,
            uid: instance_uid.to_string(),
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
    async fn test_list_instances_for_series_forbidden_for_modality_scoped_viewer_returns_403() {
        let mut store = MockMetaStore::new();
        store.expect_get_series().once().returning(|_| {
            Ok(Series {
                series_uid: SeriesUid::from("1.2"),
                study_uid: StudyUid::from("1"),
                modality: Some("US".into()),
                series_number: Some(1),
                description: None,
                body_part: None,
                num_instances: 1,
                metadata: DicomJson::empty(),
                created_at: None,
            })
        });

        let app = build_router(make_test_state(store, MockBlobStr::new())).layer(Extension(
            auth_user(UserRole::Viewer, json!({"modality_access": ["CT"]})),
        ));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/series/1.2/instances")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_delete_instance_returns_204() {
        let instance_uid = SopInstanceUid::from("1.2.3");
        let series_uid = SeriesUid::from("1.2");
        let study_uid = StudyUid::from("1");

        let mut store = MockMetaStore::new();
        store.expect_get_instance().once().returning({
            let instance_uid = instance_uid.clone();
            let series_uid = series_uid.clone();
            let study_uid = study_uid.clone();
            move |_| {
                Ok(Instance {
                    instance_uid: instance_uid.clone(),
                    series_uid: series_uid.clone(),
                    study_uid: study_uid.clone(),
                    sop_class_uid: None,
                    instance_number: Some(1),
                    transfer_syntax: None,
                    rows: None,
                    columns: None,
                    blob_key: "blob/instance".into(),
                    metadata: DicomJson::empty(),
                    created_at: None,
                })
            }
        });
        store.expect_delete_instance().once().returning(|_| Ok(()));
        let mut blobs = MockBlobStr::new();
        blobs.expect_delete().once().returning(|_| Ok(()));

        let app = build_router(make_test_state(store, blobs));
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

    #[tokio::test]
    async fn test_delete_instance_deletes_blob() {
        let instance_uid = SopInstanceUid::from("1.2.3");
        let series_uid = SeriesUid::from("1.2");
        let study_uid = StudyUid::from("1");

        let mut store = MockMetaStore::new();
        store.expect_get_instance().once().returning({
            let instance_uid = instance_uid.clone();
            let series_uid = series_uid.clone();
            let study_uid = study_uid.clone();
            move |_| {
                Ok(Instance {
                    instance_uid: instance_uid.clone(),
                    series_uid: series_uid.clone(),
                    study_uid: study_uid.clone(),
                    sop_class_uid: None,
                    instance_number: Some(1),
                    transfer_syntax: None,
                    rows: None,
                    columns: None,
                    blob_key: "blob/instance".into(),
                    metadata: DicomJson::empty(),
                    created_at: None,
                })
            }
        });
        store.expect_delete_instance().once().returning(|_| Ok(()));

        let mut blobs = MockBlobStr::new();
        blobs
            .expect_delete()
            .withf(|key| key == "blob/instance")
            .once()
            .returning(|_| Ok(()));

        let app = build_router(make_test_state(store, blobs));
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
