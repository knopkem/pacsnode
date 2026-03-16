use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single append-only audit log entry.
///
/// # Examples
///
/// ```
/// use chrono::{TimeZone, Utc};
/// use pacs_core::AuditLogEntry;
///
/// let entry = AuditLogEntry {
///     id: 1,
///     occurred_at: Utc.with_ymd_and_hms(2026, 3, 16, 12, 0, 0).unwrap(),
///     user_id: Some("admin".into()),
///     action: "DELETE".into(),
///     resource: "study".into(),
///     resource_uid: Some("1.2.3".into()),
///     source_ip: Some("127.0.0.1".into()),
///     status: "ok".into(),
///     details: serde_json::json!({}),
/// };
///
/// assert_eq!(entry.action, "DELETE");
/// assert_eq!(entry.resource_uid.as_deref(), Some("1.2.3"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditLogEntry {
    /// Monotonic audit row identifier.
    pub id: i64,
    /// Timestamp when the event occurred.
    pub occurred_at: DateTime<Utc>,
    /// Authenticated user ID, if the action originated from HTTP auth.
    pub user_id: Option<String>,
    /// High-level action name, such as `STORE`, `QUERY`, or `DELETE`.
    pub action: String,
    /// Resource kind, such as `study`, `instance`, or `association`.
    pub resource: String,
    /// Resource UID or identifier, when available.
    pub resource_uid: Option<String>,
    /// Source IP address, when available.
    pub source_ip: Option<String>,
    /// Outcome status, such as `ok` or `rejected`.
    pub status: String,
    /// Structured event details.
    pub details: serde_json::Value,
}

/// Search filters for retrieving audit log entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditLogQuery {
    /// Filter by authenticated user ID.
    pub user_id: Option<String>,
    /// Filter by action name.
    pub action: Option<String>,
    /// Filter by resource kind.
    pub resource: Option<String>,
    /// Filter by resource UID or identifier.
    pub resource_uid: Option<String>,
    /// Filter by source IP address.
    pub source_ip: Option<String>,
    /// Filter by status.
    pub status: Option<String>,
    /// Inclusive lower bound for the occurrence time.
    pub occurred_from: Option<DateTime<Utc>>,
    /// Inclusive upper bound for the occurrence time.
    pub occurred_to: Option<DateTime<Utc>>,
    /// Maximum number of entries to return.
    pub limit: Option<u32>,
    /// Number of entries to skip for pagination.
    pub offset: Option<u32>,
}

/// A paginated audit log search result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditLogPage {
    /// Returned audit entries.
    pub entries: Vec<AuditLogEntry>,
    /// Total number of matching rows before pagination.
    pub total: i64,
    /// Effective page size used for the query.
    pub limit: u32,
    /// Effective row offset used for the query.
    pub offset: u32,
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn audit_log_query_defaults_to_no_filters() {
        let query = AuditLogQuery::default();
        assert!(query.user_id.is_none());
        assert!(query.action.is_none());
        assert!(query.resource.is_none());
        assert!(query.resource_uid.is_none());
        assert!(query.source_ip.is_none());
        assert!(query.status.is_none());
        assert!(query.occurred_from.is_none());
        assert!(query.occurred_to.is_none());
        assert!(query.limit.is_none());
        assert!(query.offset.is_none());
    }

    #[test]
    fn audit_log_entry_serde_roundtrip() {
        let entry = AuditLogEntry {
            id: 7,
            occurred_at: Utc.with_ymd_and_hms(2026, 3, 16, 12, 0, 0).unwrap(),
            user_id: Some("admin".into()),
            action: "QUERY".into(),
            resource: "query".into(),
            resource_uid: None,
            source_ip: Some("127.0.0.1".into()),
            status: "ok".into(),
            details: serde_json::json!({
                "level": "STUDY",
                "num_results": 3,
            }),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let back: AuditLogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn audit_log_page_serde_roundtrip() {
        let page = AuditLogPage {
            entries: vec![AuditLogEntry {
                id: 1,
                occurred_at: Utc.with_ymd_and_hms(2026, 3, 16, 12, 0, 0).unwrap(),
                user_id: None,
                action: "ASSOCIATION_REJECT".into(),
                resource: "association".into(),
                resource_uid: None,
                source_ip: Some("192.0.2.10".into()),
                status: "rejected".into(),
                details: serde_json::json!({
                    "calling_ae": "UNKNOWN",
                    "reason": "calling AE title is not registered",
                }),
            }],
            total: 1,
            limit: 50,
            offset: 0,
        };

        let json = serde_json::to_string(&page).unwrap();
        let back: AuditLogPage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, page);
    }
}
