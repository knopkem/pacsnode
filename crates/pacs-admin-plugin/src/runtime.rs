use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use chrono::{DateTime, Utc};
use pacs_core::MetadataStore;
use pacs_plugin::{PluginError, QuerySource, ResourceLevel, ServerInfo};
use serde::Deserialize;
use tokio::sync::{broadcast, RwLock};

const EVENT_CHANNEL_CAPACITY: usize = 256;
const DEFAULT_ACTIVITY_LIMIT: usize = 24;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AdminPluginConfig {
    #[serde(default = "default_route_prefix")]
    route_prefix: String,
    #[serde(default)]
    redirect_root: bool,
    #[serde(default = "default_activity_limit")]
    activity_limit: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ActivityEntry {
    pub(crate) occurred_at: DateTime<Utc>,
    pub(crate) badge: String,
    pub(crate) title: String,
    pub(crate) detail: String,
    pub(crate) tone_class: &'static str,
}

pub(crate) struct AdminRuntime {
    route_prefix: String,
    redirect_root: bool,
    activity_limit: usize,
    event_tx: broadcast::Sender<pacs_plugin::PacsEvent>,
    metadata_store: Arc<dyn MetadataStore>,
    active_associations: AtomicU64,
    recent_activity: RwLock<VecDeque<ActivityEntry>>,
}

impl AdminRuntime {
    pub(crate) fn new(
        config: AdminPluginConfig,
        _server_info: ServerInfo,
        metadata_store: Arc<dyn MetadataStore>,
    ) -> Result<Self, PluginError> {
        let route_prefix = normalize_route_prefix(&config.route_prefix).map_err(|message| {
            PluginError::Config {
                plugin_id: "admin-dashboard".into(),
                message,
            }
        })?;
        let activity_limit = config.activity_limit.max(1);
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);

        Ok(Self {
            route_prefix,
            redirect_root: config.redirect_root,
            activity_limit,
            event_tx,
            metadata_store,
            active_associations: AtomicU64::new(0),
            recent_activity: RwLock::new(VecDeque::with_capacity(activity_limit)),
        })
    }

    pub(crate) fn route_prefix(&self) -> &str {
        &self.route_prefix
    }

    pub(crate) fn redirect_root(&self) -> bool {
        self.redirect_root
    }

    pub(crate) fn metadata_store(&self) -> Arc<dyn MetadataStore> {
        Arc::clone(&self.metadata_store)
    }
    pub(crate) fn active_associations(&self) -> u64 {
        self.active_associations.load(Ordering::Relaxed)
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<pacs_plugin::PacsEvent> {
        self.event_tx.subscribe()
    }

    pub(crate) async fn recent_activity(&self) -> Vec<ActivityEntry> {
        self.recent_activity.read().await.iter().cloned().collect()
    }

    pub(crate) async fn record_event(&self, event: &pacs_plugin::PacsEvent) {
        match event {
            pacs_plugin::PacsEvent::AssociationOpened { .. } => {
                self.active_associations.fetch_add(1, Ordering::Relaxed);
            }
            pacs_plugin::PacsEvent::AssociationClosed { .. } => {
                decrement_gauge(&self.active_associations);
            }
            pacs_plugin::PacsEvent::AssociationRejected { .. }
            | pacs_plugin::PacsEvent::InstanceStored { .. }
            | pacs_plugin::PacsEvent::StudyComplete { .. }
            | pacs_plugin::PacsEvent::ResourceDeleted { .. }
            | pacs_plugin::PacsEvent::QueryPerformed { .. } => {}
        }

        self.push_activity(activity_from_event(event)).await;
        let _ = self.event_tx.send(event.clone());
    }

    async fn push_activity(&self, entry: ActivityEntry) {
        let mut activity = self.recent_activity.write().await;
        activity.push_front(entry);
        while activity.len() > self.activity_limit {
            activity.pop_back();
        }
    }
}

fn default_route_prefix() -> String {
    "/admin".into()
}

fn default_activity_limit() -> usize {
    DEFAULT_ACTIVITY_LIMIT
}

fn normalize_route_prefix(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("route_prefix cannot be empty".into());
    }

    let normalized = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };
    let normalized = if normalized.len() > 1 {
        normalized.trim_end_matches('/').to_string()
    } else {
        normalized
    };

    if normalized == "/" {
        return Err("route_prefix must not be '/'".into());
    }

    Ok(normalized)
}

pub(crate) fn activity_from_event(event: &pacs_plugin::PacsEvent) -> ActivityEntry {
    let occurred_at = Utc::now();
    match event {
        pacs_plugin::PacsEvent::InstanceStored {
            study_uid,
            sop_instance_uid,
            source,
            ..
        } => ActivityEntry {
            occurred_at,
            badge: "STORE".into(),
            title: format!("Instance received from {source}"),
            detail: format!(
                "Study {} • Instance {}",
                shorten_uid(study_uid),
                shorten_uid(sop_instance_uid)
            ),
            tone_class: "tone-success",
        },
        pacs_plugin::PacsEvent::StudyComplete { study_uid } => ActivityEntry {
            occurred_at,
            badge: "STUDY".into(),
            title: "Study transfer completed".into(),
            detail: format!("Study {} is complete", shorten_uid(study_uid)),
            tone_class: "tone-success",
        },
        pacs_plugin::PacsEvent::ResourceDeleted { level, uid, .. } => ActivityEntry {
            occurred_at,
            badge: "DELETE".into(),
            title: format!("{} deleted", resource_level_label(*level)),
            detail: format!("UID {}", shorten_uid(uid)),
            tone_class: "tone-warning",
        },
        pacs_plugin::PacsEvent::AssociationOpened {
            calling_ae,
            peer_addr,
        } => ActivityEntry {
            occurred_at,
            badge: "DIMSE".into(),
            title: format!("Association opened by {calling_ae}"),
            detail: peer_addr.to_string(),
            tone_class: "tone-info",
        },
        pacs_plugin::PacsEvent::AssociationRejected {
            calling_ae, reason, ..
        } => ActivityEntry {
            occurred_at,
            badge: "DIMSE".into(),
            title: format!("Association rejected for {calling_ae}"),
            detail: reason.clone(),
            tone_class: "tone-danger",
        },
        pacs_plugin::PacsEvent::AssociationClosed { calling_ae } => ActivityEntry {
            occurred_at,
            badge: "DIMSE".into(),
            title: format!("Association closed for {calling_ae}"),
            detail: "Peer disconnected cleanly".into(),
            tone_class: "tone-muted",
        },
        pacs_plugin::PacsEvent::QueryPerformed {
            level,
            source,
            num_results,
            ..
        } => ActivityEntry {
            occurred_at,
            badge: "QUERY".into(),
            title: format!("{level} query completed"),
            detail: format!(
                "{} result(s) via {}",
                num_results,
                query_source_label(source)
            ),
            tone_class: "tone-info",
        },
    }
}

fn query_source_label(source: &QuerySource) -> &'static str {
    match source {
        QuerySource::Dimse { .. } => "DIMSE",
        QuerySource::Dicomweb => "DICOMweb",
    }
}

fn resource_level_label(level: ResourceLevel) -> &'static str {
    match level {
        ResourceLevel::Patient => "Patient",
        ResourceLevel::Study => "Study",
        ResourceLevel::Series => "Series",
        ResourceLevel::Instance => "Instance",
    }
}

fn shorten_uid(value: &str) -> String {
    const EDGE: usize = 10;
    if value.len() <= EDGE * 2 + 3 {
        return value.to_string();
    }
    format!("{}...{}", &value[..EDGE], &value[value.len() - EDGE..])
}

fn decrement_gauge(counter: &AtomicU64) {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        if current == 0 {
            return;
        }
        match counter.compare_exchange(current, current - 1, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(observed) => current = observed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use bytes::Bytes;
    use pacs_core::{
        AuditLogEntry, AuditLogPage, AuditLogQuery, BlobStore, DicomJson, DicomNode, Instance,
        InstanceQuery, MetadataStore, NewAuditLogEntry, PacsResult, PacsStatistics, Series,
        SeriesQuery, SeriesUid, ServerSettings, SopInstanceUid, Study, StudyQuery, StudyUid,
    };

    #[derive(Default)]
    struct NoopMetadataStore;

    #[async_trait]
    impl MetadataStore for NoopMetadataStore {
        async fn store_study(&self, _study: &Study) -> PacsResult<()> {
            Ok(())
        }
        async fn store_series(&self, _series: &Series) -> PacsResult<()> {
            Ok(())
        }
        async fn store_instance(&self, _instance: &Instance) -> PacsResult<()> {
            Ok(())
        }
        async fn query_studies(&self, _q: &StudyQuery) -> PacsResult<Vec<Study>> {
            Ok(vec![])
        }
        async fn query_series(&self, _q: &SeriesQuery) -> PacsResult<Vec<Series>> {
            Ok(vec![])
        }
        async fn query_instances(&self, _q: &InstanceQuery) -> PacsResult<Vec<Instance>> {
            Ok(vec![])
        }
        async fn get_study(&self, _uid: &StudyUid) -> PacsResult<Study> {
            unreachable!()
        }
        async fn get_series(&self, _uid: &SeriesUid) -> PacsResult<Series> {
            unreachable!()
        }
        async fn get_instance(&self, _uid: &SopInstanceUid) -> PacsResult<Instance> {
            unreachable!()
        }
        async fn get_instance_metadata(&self, _uid: &SopInstanceUid) -> PacsResult<DicomJson> {
            unreachable!()
        }
        async fn delete_study(&self, _uid: &StudyUid) -> PacsResult<()> {
            Ok(())
        }
        async fn delete_series(&self, _uid: &SeriesUid) -> PacsResult<()> {
            Ok(())
        }
        async fn delete_instance(&self, _uid: &SopInstanceUid) -> PacsResult<()> {
            Ok(())
        }
        async fn get_statistics(&self) -> PacsResult<PacsStatistics> {
            Ok(PacsStatistics {
                num_studies: 0,
                num_series: 0,
                num_instances: 0,
                disk_usage_bytes: 0,
            })
        }
        async fn list_nodes(&self) -> PacsResult<Vec<DicomNode>> {
            Ok(vec![])
        }
        async fn upsert_node(&self, _node: &DicomNode) -> PacsResult<()> {
            Ok(())
        }
        async fn delete_node(&self, _ae_title: &str) -> PacsResult<()> {
            Ok(())
        }
        async fn get_server_settings(&self) -> PacsResult<Option<ServerSettings>> {
            Ok(None)
        }
        async fn upsert_server_settings(&self, _settings: &ServerSettings) -> PacsResult<()> {
            Ok(())
        }
        async fn search_audit_logs(&self, _q: &AuditLogQuery) -> PacsResult<AuditLogPage> {
            Ok(AuditLogPage {
                entries: vec![],
                total: 0,
                limit: 10,
                offset: 0,
            })
        }
        async fn get_audit_log(&self, _id: i64) -> PacsResult<AuditLogEntry> {
            unreachable!()
        }
        async fn store_audit_log(&self, _entry: &NewAuditLogEntry) -> PacsResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct NoopBlobStore;

    #[async_trait]
    impl BlobStore for NoopBlobStore {
        async fn put(&self, _key: &str, _data: Bytes) -> PacsResult<()> {
            Ok(())
        }
        async fn get(&self, _key: &str) -> PacsResult<Bytes> {
            unreachable!()
        }
        async fn delete(&self, _key: &str) -> PacsResult<()> {
            Ok(())
        }
        async fn exists(&self, _key: &str) -> PacsResult<bool> {
            Ok(false)
        }
        async fn presigned_url(&self, _key: &str, _ttl_secs: u32) -> PacsResult<String> {
            Ok(String::new())
        }
    }

    #[test]
    fn route_prefix_is_normalized() {
        assert_eq!(normalize_route_prefix("admin").unwrap(), "/admin");
        assert_eq!(normalize_route_prefix("/admin/").unwrap(), "/admin");
        assert!(normalize_route_prefix("/").is_err());
    }

    #[tokio::test]
    async fn recent_activity_is_bounded() {
        let runtime = AdminRuntime::new(
            AdminPluginConfig {
                route_prefix: "/admin".into(),
                redirect_root: false,
                activity_limit: 2,
            },
            ServerInfo {
                ae_title: "PACSNODE".into(),
                http_port: 8042,
                dicom_port: 4242,
                version: "0.1.0",
            },
            Arc::new(NoopMetadataStore),
        )
        .unwrap();

        runtime
            .record_event(&pacs_plugin::PacsEvent::StudyComplete {
                study_uid: "1.2.3".into(),
            })
            .await;
        runtime
            .record_event(&pacs_plugin::PacsEvent::StudyComplete {
                study_uid: "1.2.4".into(),
            })
            .await;
        runtime
            .record_event(&pacs_plugin::PacsEvent::StudyComplete {
                study_uid: "1.2.5".into(),
            })
            .await;

        let activity = runtime.recent_activity().await;
        assert_eq!(activity.len(), 2);
        assert!(activity[0].detail.contains("1.2.5"));
    }

    #[test]
    fn noop_blob_store_is_constructible() {
        let _ = NoopBlobStore;
    }
}
