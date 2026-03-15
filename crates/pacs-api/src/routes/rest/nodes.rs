//! REST handlers for DICOM node management — `GET/POST /api/nodes`, `DELETE /api/nodes/:ae_title`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use pacs_core::PacsError;

use crate::{error::ApiError, state::AppState, state::DicomNode};

/// `GET /api/nodes` — list all registered DICOM nodes.
pub async fn list_nodes(State(state): State<AppState>) -> impl IntoResponse {
    let nodes = state.nodes.read().await.clone();
    Json(nodes)
}

/// `POST /api/nodes` — register a new DICOM node.
///
/// Returns `201 Created` with the created [`DicomNode`] as JSON.
pub async fn add_node(
    State(state): State<AppState>,
    Json(node): Json<DicomNode>,
) -> impl IntoResponse {
    state.nodes.write().await.push(node.clone());
    (StatusCode::CREATED, Json(node))
}

/// `DELETE /api/nodes/:ae_title` — remove a DICOM node by AE title.
///
/// Returns `204 No Content` if found, `404` if no node has the given AE title.
pub async fn remove_node(
    State(state): State<AppState>,
    Path(ae_title): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let mut nodes = state.nodes.write().await;
    let before = nodes.len();
    nodes.retain(|n| n.ae_title != ae_title);
    if nodes.len() == before {
        return Err(ApiError(PacsError::NotFound {
            resource: "node",
            uid: ae_title,
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::{
        router::build_router,
        test_support::{make_test_state, MockBlobStr, MockMetaStore},
    };

    #[tokio::test]
    async fn test_list_nodes_returns_empty_array() {
        let app = build_router(make_test_state(MockMetaStore::new(), MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/nodes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_add_and_list_node() {
        let app = build_router(make_test_state(MockMetaStore::new(), MockBlobStr::new()));
        let node_json =
            r#"{"ae_title":"PACS1","host":"10.0.0.1","port":104,"description":null}"#;

        // Add the node
        let add_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/nodes")
                    .header("content-type", "application/json")
                    .body(Body::from(node_json))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(add_resp.status(), StatusCode::CREATED);

        // List nodes
        let list_resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/nodes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = list_resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 1);
        assert_eq!(json[0]["ae_title"], "PACS1");
    }

    #[tokio::test]
    async fn test_delete_nonexistent_node_returns_404() {
        let app = build_router(make_test_state(MockMetaStore::new(), MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/nodes/NONEXISTENT")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_add_then_delete_node() {
        let state = make_test_state(MockMetaStore::new(), MockBlobStr::new());
        let app = build_router(state);
        let node_json =
            r#"{"ae_title":"REMOTE","host":"192.168.1.2","port":11112,"description":null}"#;

        // Add node
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/nodes")
                    .header("content-type", "application/json")
                    .body(Body::from(node_json))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Delete node
        let del_resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/nodes/REMOTE")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);
    }
}
