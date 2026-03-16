//! REST handlers for study resources — `GET/DELETE /api/studies[/:uid]`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use pacs_core::{StudyQuery, StudyUid};
use pacs_plugin::{AuthenticatedUser, PacsEvent, ResourceLevel};

use crate::{error::ApiError, state::AppState};

use super::{cleanup_blob_keys, collect_study_blob_keys};

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
    let study = state.store.get_study(&StudyUid::from(uid.as_str())).await?;
    Ok(Json(study))
}

/// `DELETE /api/studies/:study_uid` — delete a study and its dependants.
///
/// Returns `204 No Content` on success.
pub async fn delete_study(
    State(state): State<AppState>,
    user: Option<axum::Extension<AuthenticatedUser>>,
    Path(uid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let study_uid = StudyUid::from(uid.as_str());
    let blob_keys = collect_study_blob_keys(&state, &study_uid).await?;
    state.store.delete_study(&study_uid).await?;
    cleanup_blob_keys(&state, blob_keys).await;
    state
        .plugins
        .emit_event(PacsEvent::ResourceDeleted {
            level: ResourceLevel::Study,
            uid: study_uid.to_string(),
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
    use pacs_core::{DicomJson, Instance, PacsError, Series, SeriesUid, SopInstanceUid, StudyUid};
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
        store
            .expect_query_series()
            .once()
            .returning(|_| Ok(Vec::new()));
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

    #[tokio::test]
    async fn test_delete_study_deletes_unique_blob_keys() {
        let study_uid = StudyUid::from("1.2.3");
        let series_one_uid = SeriesUid::from("1.2.3.1");
        let series_two_uid = SeriesUid::from("1.2.3.2");

        let mut store = MockMetaStore::new();
        store.expect_query_series().once().returning({
            let study_uid = study_uid.clone();
            let series_one_uid = series_one_uid.clone();
            let series_two_uid = series_two_uid.clone();
            move |_| {
                Ok(vec![
                    Series {
                        series_uid: series_one_uid.clone(),
                        study_uid: study_uid.clone(),
                        modality: None,
                        series_number: Some(1),
                        description: None,
                        body_part: None,
                        num_instances: 1,
                        metadata: DicomJson::empty(),
                        created_at: None,
                    },
                    Series {
                        series_uid: series_two_uid.clone(),
                        study_uid: study_uid.clone(),
                        modality: None,
                        series_number: Some(2),
                        description: None,
                        body_part: None,
                        num_instances: 2,
                        metadata: DicomJson::empty(),
                        created_at: None,
                    },
                ])
            }
        });
        store.expect_query_instances().times(2).returning({
            let study_uid = study_uid.clone();
            let series_one_uid = series_one_uid.clone();
            let series_two_uid = series_two_uid.clone();
            move |query| {
                let instances = if query.series_uid == series_one_uid {
                    vec![Instance {
                        instance_uid: SopInstanceUid::from("1.2.3.1.1"),
                        series_uid: series_one_uid.clone(),
                        study_uid: study_uid.clone(),
                        sop_class_uid: None,
                        instance_number: Some(1),
                        transfer_syntax: None,
                        rows: None,
                        columns: None,
                        blob_key: "blob/shared".into(),
                        metadata: DicomJson::empty(),
                        created_at: None,
                    }]
                } else if query.series_uid == series_two_uid {
                    vec![
                        Instance {
                            instance_uid: SopInstanceUid::from("1.2.3.2.1"),
                            series_uid: series_two_uid.clone(),
                            study_uid: study_uid.clone(),
                            sop_class_uid: None,
                            instance_number: Some(1),
                            transfer_syntax: None,
                            rows: None,
                            columns: None,
                            blob_key: "blob/shared".into(),
                            metadata: DicomJson::empty(),
                            created_at: None,
                        },
                        Instance {
                            instance_uid: SopInstanceUid::from("1.2.3.2.2"),
                            series_uid: series_two_uid.clone(),
                            study_uid: study_uid.clone(),
                            sop_class_uid: None,
                            instance_number: Some(2),
                            transfer_syntax: None,
                            rows: None,
                            columns: None,
                            blob_key: "blob/unique".into(),
                            metadata: DicomJson::empty(),
                            created_at: None,
                        },
                    ]
                } else {
                    panic!("unexpected series UID: {}", query.series_uid);
                };
                Ok(instances)
            }
        });
        store.expect_delete_study().once().returning(|_| Ok(()));

        let mut blobs = MockBlobStr::new();
        blobs
            .expect_delete()
            .withf(|key| key == "blob/shared")
            .once()
            .returning(|_| Ok(()));
        blobs
            .expect_delete()
            .withf(|key| key == "blob/unique")
            .once()
            .returning(|_| Ok(()));

        let app = build_router(make_test_state(store, blobs));
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
