//! Audit-log SQL helpers: browse, search, and fetch by ID.

use chrono::{DateTime, Utc};
use pacs_core::{AuditLogEntry, AuditLogPage, AuditLogQuery, PacsResult};
use sqlx::{PgPool, Postgres, QueryBuilder};

use crate::error::{map_db_err, map_store_err};

/// Raw database row returned by audit-log `SELECT` queries.
#[derive(sqlx::FromRow)]
struct AuditLogRow {
    id: i64,
    occurred_at: DateTime<Utc>,
    user_id: Option<String>,
    action: String,
    resource: String,
    resource_uid: Option<String>,
    source_ip: Option<String>,
    status: String,
    details: Option<serde_json::Value>,
}

#[derive(sqlx::FromRow)]
struct AuditCountRow {
    total: i64,
}

impl From<AuditLogRow> for AuditLogEntry {
    fn from(row: AuditLogRow) -> Self {
        Self {
            id: row.id,
            occurred_at: row.occurred_at,
            user_id: row.user_id,
            action: row.action,
            resource: row.resource,
            resource_uid: row.resource_uid,
            source_ip: row.source_ip,
            status: row.status,
            details: row.details.unwrap_or_else(|| serde_json::json!({})),
        }
    }
}

const SELECT_COLS: &str = r#"
    SELECT id, occurred_at, user_id, action, resource, resource_uid, source_ip, status, details
    FROM audit_log
"#;

/// Searches the append-only audit log with pagination.
pub(crate) async fn search(pool: &PgPool, query: &AuditLogQuery) -> PacsResult<AuditLogPage> {
    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);

    let mut count_qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT COUNT(*)::BIGINT AS total FROM audit_log",
    );
    push_filters(&mut count_qb, query);
    let total = count_qb
        .build_query_as::<AuditCountRow>()
        .fetch_one(pool)
        .await
        .map_err(map_store_err)?
        .total;

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(SELECT_COLS);
    push_filters(&mut qb, query);
    qb.push(" ORDER BY occurred_at DESC, id DESC LIMIT ");
    qb.push_bind(i64::from(limit));
    qb.push(" OFFSET ");
    qb.push_bind(i64::from(offset));

    let entries = qb
        .build_query_as::<AuditLogRow>()
        .fetch_all(pool)
        .await
        .map_err(map_store_err)?
        .into_iter()
        .map(AuditLogEntry::from)
        .collect();

    Ok(AuditLogPage {
        entries,
        total,
        limit,
        offset,
    })
}

/// Retrieves a single audit-log row by numeric ID.
pub(crate) async fn get(pool: &PgPool, id: i64) -> PacsResult<AuditLogEntry> {
    sqlx::query_as::<_, AuditLogRow>(&format!("{SELECT_COLS} WHERE id = $1"))
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|error| map_db_err(error, "audit_log", &id.to_string()))
        .map(AuditLogEntry::from)
}

fn push_filters(qb: &mut QueryBuilder<'_, Postgres>, query: &AuditLogQuery) {
    qb.push(" WHERE 1=1");

    if let Some(user_id) = &query.user_id {
        qb.push(" AND user_id = ");
        qb.push_bind(user_id.clone());
    }

    if let Some(action) = &query.action {
        qb.push(" AND action ILIKE ");
        qb.push_bind(action.clone());
    }

    if let Some(resource) = &query.resource {
        qb.push(" AND resource ILIKE ");
        qb.push_bind(resource.clone());
    }

    if let Some(resource_uid) = &query.resource_uid {
        qb.push(" AND resource_uid = ");
        qb.push_bind(resource_uid.clone());
    }

    if let Some(source_ip) = &query.source_ip {
        qb.push(" AND source_ip = ");
        qb.push_bind(source_ip.clone());
    }

    if let Some(status) = &query.status {
        qb.push(" AND status ILIKE ");
        qb.push_bind(status.clone());
    }

    if let Some(occurred_from) = query.occurred_from {
        qb.push(" AND occurred_at >= ");
        qb.push_bind(occurred_from);
    }

    if let Some(occurred_to) = query.occurred_to {
        qb.push(" AND occurred_at <= ");
        qb.push_bind(occurred_to);
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn make_row() -> AuditLogRow {
        AuditLogRow {
            id: 5,
            occurred_at: Utc.with_ymd_and_hms(2026, 3, 16, 12, 0, 0).unwrap(),
            user_id: Some("admin".into()),
            action: "DELETE".into(),
            resource: "study".into(),
            resource_uid: Some("1.2.3".into()),
            source_ip: Some("127.0.0.1".into()),
            status: "ok".into(),
            details: Some(serde_json::json!({
                "calling_ae": "SCU1",
            })),
        }
    }

    #[test]
    fn audit_row_converts_to_domain_entry() {
        let entry = AuditLogEntry::from(make_row());
        assert_eq!(entry.id, 5);
        assert_eq!(entry.action, "DELETE");
        assert_eq!(entry.resource, "study");
        assert_eq!(entry.resource_uid.as_deref(), Some("1.2.3"));
        assert_eq!(entry.details["calling_ae"], "SCU1");
    }

    #[test]
    fn missing_details_becomes_empty_object() {
        let mut row = make_row();
        row.details = None;
        let entry = AuditLogEntry::from(row);
        assert_eq!(entry.details, serde_json::json!({}));
    }
}
