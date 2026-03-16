//! REST handlers for browsing and searching the append-only audit log.

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use pacs_core::{AuditLogQuery, PacsError};
use serde::Deserialize;

use crate::{error::ApiError, state::AppState};

#[derive(Debug, Deserialize)]
pub(crate) struct AuditLogQueryParams {
    user_id: Option<String>,
    action: Option<String>,
    resource: Option<String>,
    resource_uid: Option<String>,
    source_ip: Option<String>,
    status: Option<String>,
    since: Option<String>,
    until: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

/// `GET /api/audit/logs` — search the append-only audit log.
pub(crate) async fn search_audit_logs(
    State(state): State<AppState>,
    Query(params): Query<AuditLogQueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let query = parse_audit_log_query(params)?;
    let page = state.store.search_audit_logs(&query).await?;
    Ok(Json(page))
}

/// `GET /api/audit/logs/{id}` — fetch a single audit log entry by numeric ID.
pub(crate) async fn get_audit_log(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    if id <= 0 {
        return Err(PacsError::InvalidRequest("audit log id must be positive".into()).into());
    }

    let entry = state.store.get_audit_log(id).await?;
    Ok(Json(entry))
}

fn parse_audit_log_query(params: AuditLogQueryParams) -> Result<AuditLogQuery, ApiError> {
    if params.limit == Some(0) {
        return Err(PacsError::InvalidRequest("limit must be greater than zero".into()).into());
    }

    Ok(AuditLogQuery {
        user_id: params.user_id,
        action: params.action,
        resource: params.resource,
        resource_uid: params.resource_uid,
        source_ip: params.source_ip,
        status: params.status,
        occurred_from: parse_optional_timestamp("since", params.since)?,
        occurred_to: parse_optional_timestamp("until", params.until)?,
        limit: params.limit,
        offset: params.offset,
    })
}

fn parse_optional_timestamp(
    name: &str,
    value: Option<String>,
) -> Result<Option<DateTime<Utc>>, ApiError> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|parsed| parsed.with_timezone(&Utc))
                .map_err(|_| {
                    PacsError::InvalidRequest(format!("{name} must be an RFC 3339 timestamp"))
                })
        })
        .transpose()
        .map_err(ApiError::from)
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use chrono::{TimeZone, Utc};
    use http_body_util::BodyExt;
    use pacs_core::{AuditLogEntry, AuditLogPage};
    use tower::ServiceExt;

    use crate::{
        router::build_router,
        test_support::{make_test_state, MockBlobStr, MockMetaStore},
    };

    fn sample_entry(id: i64) -> AuditLogEntry {
        AuditLogEntry {
            id,
            occurred_at: Utc.with_ymd_and_hms(2026, 3, 16, 12, 0, 0).unwrap(),
            user_id: Some("admin".into()),
            action: "QUERY".into(),
            resource: "query".into(),
            resource_uid: Some("1.2.3".into()),
            source_ip: Some("127.0.0.1".into()),
            status: "ok".into(),
            details: serde_json::json!({
                "level": "STUDY",
                "num_results": 2,
            }),
        }
    }

    #[tokio::test]
    async fn test_search_audit_logs_returns_page() {
        let mut store = MockMetaStore::new();
        store
            .expect_search_audit_logs()
            .once()
            .withf(|query| {
                query.action.as_deref() == Some("QUERY")
                    && query.limit == Some(10)
                    && query.offset == Some(5)
                    && query.occurred_from.is_some()
            })
            .returning(|_| {
                Ok(AuditLogPage {
                    entries: vec![sample_entry(7)],
                    total: 1,
                    limit: 10,
                    offset: 5,
                })
            });

        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(
                        "/api/audit/logs?action=QUERY&limit=10&offset=5&since=2026-03-16T00:00:00Z",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 1);
        assert_eq!(json["entries"][0]["id"], 7);
    }

    #[tokio::test]
    async fn test_search_audit_logs_rejects_invalid_since() {
        let app = build_router(make_test_state(MockMetaStore::new(), MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/audit/logs?since=not-a-timestamp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_audit_log_returns_entry() {
        let mut store = MockMetaStore::new();
        store
            .expect_get_audit_log()
            .once()
            .withf(|id| *id == 7)
            .returning(|_| Ok(sample_entry(7)));

        let app = build_router(make_test_state(store, MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/audit/logs/7")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"], 7);
        assert_eq!(json["action"], "QUERY");
    }

    #[tokio::test]
    async fn test_get_audit_log_rejects_non_positive_id() {
        let app = build_router(make_test_state(MockMetaStore::new(), MockBlobStr::new()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/audit/logs/0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
