//! pacsnode audit-log plugin.
//!
//! Persists runtime events to the existing `audit_log` PostgreSQL table.

use async_trait::async_trait;
use pacs_plugin::{
    register_plugin, EventKind, EventPlugin, PacsEvent, Plugin, PluginContext, PluginError,
    PluginHealth, PluginManifest, QuerySource, ResourceLevel,
};
use serde::Deserialize;
use sqlx::{postgres::PgPoolOptions, PgPool};

/// Compile-time plugin ID for the audit logger.
pub const AUDIT_LOGGER_PLUGIN_ID: &str = "audit-logger";

const METADATA_STORE_DEPENDENCY: &str = "pg-metadata-store";

#[derive(Default)]
pub struct AuditLoggerPlugin {
    pool: Option<PgPool>,
}

#[derive(Debug, Clone, Deserialize)]
struct AuditPluginConfig {
    url: String,
    #[serde(default = "default_max_connections")]
    max_connections: u32,
}

#[derive(Debug)]
struct AuditRecord {
    user_id: Option<String>,
    action: &'static str,
    resource: &'static str,
    resource_uid: Option<String>,
    source_ip: Option<String>,
    status: &'static str,
    details: serde_json::Value,
}

fn default_max_connections() -> u32 {
    5
}

#[async_trait]
impl Plugin for AuditLoggerPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            AUDIT_LOGGER_PLUGIN_ID,
            "Audit Logger",
            env!("CARGO_PKG_VERSION"),
        )
        .with_dependencies([METADATA_STORE_DEPENDENCY])
        .disabled_by_default()
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        let config: AuditPluginConfig =
            serde_json::from_value(ctx.config.clone()).map_err(|error| PluginError::Config {
                plugin_id: AUDIT_LOGGER_PLUGIN_ID.into(),
                message: error.to_string(),
            })?;

        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .connect(&config.url)
            .await
            .map_err(|source| PluginError::InitFailed {
                plugin_id: AUDIT_LOGGER_PLUGIN_ID.into(),
                source: Box::new(source),
            })?;

        self.pool = Some(pool);
        Ok(())
    }

    async fn health(&self) -> PluginHealth {
        if self.pool.is_some() {
            PluginHealth::Healthy
        } else {
            PluginHealth::Unhealthy("plugin not initialized".into())
        }
    }

    fn as_event_plugin(&self) -> Option<&dyn EventPlugin> {
        Some(self)
    }
}

#[async_trait]
impl EventPlugin for AuditLoggerPlugin {
    fn subscriptions(&self) -> Vec<EventKind> {
        vec![
            EventKind::InstanceStored,
            EventKind::StudyComplete,
            EventKind::ResourceDeleted,
            EventKind::AssociationOpened,
            EventKind::AssociationRejected,
            EventKind::AssociationClosed,
            EventKind::QueryPerformed,
        ]
    }

    async fn on_event(&self, event: &PacsEvent) -> Result<(), PluginError> {
        let Some(pool) = &self.pool else {
            return Err(PluginError::NotInitialized {
                plugin_id: AUDIT_LOGGER_PLUGIN_ID.into(),
                capability: "EventPlugin".into(),
            });
        };

        let Some(record) = audit_record_from_event(event) else {
            return Ok(());
        };

        sqlx::query(
            "INSERT INTO audit_log (user_id, action, resource, resource_uid, source_ip, status, details) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(record.user_id)
        .bind(record.action)
        .bind(record.resource)
        .bind(record.resource_uid)
        .bind(record.source_ip)
        .bind(record.status)
        .bind(record.details)
        .execute(pool)
        .await
        .map_err(|source| PluginError::Runtime {
            plugin_id: AUDIT_LOGGER_PLUGIN_ID.into(),
            message: source.to_string(),
        })?;

        Ok(())
    }
}

fn audit_record_from_event(event: &PacsEvent) -> Option<AuditRecord> {
    match event {
        PacsEvent::InstanceStored {
            study_uid,
            series_uid,
            sop_instance_uid,
            sop_class_uid,
            source,
            user_id,
        } => Some(AuditRecord {
            user_id: user_id.clone(),
            action: "STORE",
            resource: "instance",
            resource_uid: Some(sop_instance_uid.clone()),
            source_ip: None,
            status: "ok",
            details: serde_json::json!({
                "study_uid": study_uid,
                "series_uid": series_uid,
                "sop_class_uid": sop_class_uid,
                "source": source,
            }),
        }),
        PacsEvent::StudyComplete { study_uid } => Some(AuditRecord {
            user_id: None,
            action: "STUDY_COMPLETE",
            resource: "study",
            resource_uid: Some(study_uid.clone()),
            source_ip: None,
            status: "ok",
            details: serde_json::json!({}),
        }),
        PacsEvent::ResourceDeleted {
            level,
            uid,
            user_id,
        } => Some(AuditRecord {
            user_id: user_id.clone(),
            action: "DELETE",
            resource: resource_name(*level),
            resource_uid: Some(uid.clone()),
            source_ip: None,
            status: "ok",
            details: serde_json::json!({}),
        }),
        PacsEvent::AssociationOpened {
            calling_ae,
            peer_addr,
        } => Some(AuditRecord {
            user_id: None,
            action: "ASSOCIATION_OPEN",
            resource: "association",
            resource_uid: None,
            source_ip: Some(peer_addr.ip().to_string()),
            status: "ok",
            details: serde_json::json!({
                "calling_ae": calling_ae,
            }),
        }),
        PacsEvent::AssociationRejected {
            calling_ae,
            peer_addr,
            reason,
        } => Some(AuditRecord {
            user_id: None,
            action: "ASSOCIATION_REJECT",
            resource: "association",
            resource_uid: None,
            source_ip: Some(peer_addr.ip().to_string()),
            status: "rejected",
            details: serde_json::json!({
                "calling_ae": calling_ae,
                "reason": reason,
            }),
        }),
        PacsEvent::AssociationClosed { calling_ae } => Some(AuditRecord {
            user_id: None,
            action: "ASSOCIATION_CLOSE",
            resource: "association",
            resource_uid: None,
            source_ip: None,
            status: "ok",
            details: serde_json::json!({
                "calling_ae": calling_ae,
            }),
        }),
        PacsEvent::QueryPerformed {
            level,
            source,
            num_results,
            user_id,
        } => Some(AuditRecord {
            user_id: user_id.clone(),
            action: "QUERY",
            resource: "query",
            resource_uid: None,
            source_ip: None,
            status: "ok",
            details: serde_json::json!({
                "level": level,
                "source": query_source_json(source),
                "num_results": num_results,
            }),
        }),
    }
}

fn resource_name(level: ResourceLevel) -> &'static str {
    match level {
        ResourceLevel::Patient => "patient",
        ResourceLevel::Study => "study",
        ResourceLevel::Series => "series",
        ResourceLevel::Instance => "instance",
    }
}

fn query_source_json(source: &QuerySource) -> serde_json::Value {
    match source {
        QuerySource::Dimse { calling_ae } => serde_json::json!({
            "kind": "dimse",
            "calling_ae": calling_ae,
        }),
        QuerySource::Dicomweb => serde_json::json!({
            "kind": "dicomweb",
        }),
    }
}

register_plugin!(AuditLoggerPlugin::default);

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::*;

    #[test]
    fn manifest_has_expected_id() {
        let plugin = AuditLoggerPlugin::default();
        assert_eq!(plugin.manifest().id, AUDIT_LOGGER_PLUGIN_ID);
        assert!(!plugin.manifest().enabled_by_default);
    }

    #[test]
    fn maps_instance_store_event_to_audit_record() {
        let record = audit_record_from_event(&PacsEvent::InstanceStored {
            study_uid: "1.2.3".into(),
            series_uid: "4.5.6".into(),
            sop_instance_uid: "7.8.9".into(),
            sop_class_uid: "1.2.840".into(),
            source: "STOW-RS".into(),
            user_id: Some("admin".into()),
        })
        .unwrap();
        assert_eq!(record.action, "STORE");
        assert_eq!(record.resource, "instance");
        assert_eq!(record.user_id.as_deref(), Some("admin"));
        assert_eq!(record.resource_uid.as_deref(), Some("7.8.9"));
    }

    #[test]
    fn maps_query_event_to_audit_record() {
        let record = audit_record_from_event(&PacsEvent::QueryPerformed {
            level: "STUDY".into(),
            source: QuerySource::Dimse {
                calling_ae: "FINDSCU".into(),
            },
            num_results: 3,
            user_id: None,
        })
        .unwrap();
        assert_eq!(record.action, "QUERY");
        assert_eq!(record.resource, "query");
        assert_eq!(record.details["source"]["kind"], "dimse");
        assert_eq!(record.details["num_results"], 3);
    }

    #[test]
    fn maps_association_open_to_source_ip() {
        let record = audit_record_from_event(&PacsEvent::AssociationOpened {
            calling_ae: "FINDSCU".into(),
            peer_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4242),
        })
        .unwrap();
        assert_eq!(record.action, "ASSOCIATION_OPEN");
        assert_eq!(record.source_ip.as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn maps_association_reject_to_rejected_status() {
        let record = audit_record_from_event(&PacsEvent::AssociationRejected {
            calling_ae: "UNKNOWN".into(),
            peer_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4242),
            reason: "calling AE title is not registered".into(),
        })
        .unwrap();

        assert_eq!(record.action, "ASSOCIATION_REJECT");
        assert_eq!(record.status, "rejected");
        assert_eq!(
            record.details["reason"],
            serde_json::json!("calling AE title is not registered")
        );
    }
}
