//! pacsnode Prometheus metrics plugin.
//!
//! Tracks core PACS events and exposes them in Prometheus exposition format.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

use async_trait::async_trait;
use axum::{
    body::Body,
    extract::State as AxumState,
    http::{header, Request, StatusCode},
    middleware::{from_fn_with_state, Next},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use pacs_plugin::{
    register_plugin, AppState, EventKind, EventPlugin, MiddlewarePlugin, PacsEvent, Plugin,
    PluginContext, PluginError, PluginHealth, PluginManifest, QuerySource, RoutePlugin,
};
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::warn;

/// Compile-time plugin ID for the Prometheus metrics plugin.
pub const PROMETHEUS_METRICS_PLUGIN_ID: &str = "prometheus-metrics";

const HTTP_DURATION_BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

#[derive(Default)]
pub struct PrometheusMetricsPlugin {
    runtime: Option<Arc<MetricsRuntime>>,
}

#[derive(Debug, Clone, Deserialize)]
struct MetricsPluginConfig {
    #[serde(default = "default_endpoint")]
    endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct QueryMetricKey {
    level: String,
    source: String,
}

struct Histogram {
    bucket_hits: [u64; HTTP_DURATION_BUCKETS.len()],
    total_count: u64,
    total_sum: f64,
}

struct HistogramSnapshot {
    bucket_hits: [u64; HTTP_DURATION_BUCKETS.len()],
    total_count: u64,
    total_sum: f64,
}

struct MetricsRuntime {
    endpoint: String,
    instances_stored_total: AtomicU64,
    associations_active: AtomicU64,
    query_totals: Mutex<HashMap<QueryMetricKey, u64>>,
    http_duration: Mutex<Histogram>,
}

fn default_endpoint() -> String {
    "/metrics".into()
}

#[async_trait]
impl Plugin for PrometheusMetricsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            PROMETHEUS_METRICS_PLUGIN_ID,
            "Prometheus Metrics",
            env!("CARGO_PKG_VERSION"),
        )
        .disabled_by_default()
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        let config: MetricsPluginConfig =
            serde_json::from_value(ctx.config.clone()).map_err(|error| PluginError::Config {
                plugin_id: PROMETHEUS_METRICS_PLUGIN_ID.into(),
                message: error.to_string(),
            })?;
        let endpoint = normalize_path(&config.endpoint).map_err(|message| PluginError::Config {
            plugin_id: PROMETHEUS_METRICS_PLUGIN_ID.into(),
            message,
        })?;

        self.runtime = Some(Arc::new(MetricsRuntime {
            endpoint,
            instances_stored_total: AtomicU64::new(0),
            associations_active: AtomicU64::new(0),
            query_totals: Mutex::new(HashMap::new()),
            http_duration: Mutex::new(Histogram::default()),
        }));
        Ok(())
    }

    async fn health(&self) -> PluginHealth {
        if self.runtime.is_some() {
            PluginHealth::Healthy
        } else {
            PluginHealth::Unhealthy("plugin not initialized".into())
        }
    }

    fn as_route_plugin(&self) -> Option<&dyn RoutePlugin> {
        Some(self)
    }
    fn as_event_plugin(&self) -> Option<&dyn EventPlugin> {
        Some(self)
    }
    fn as_middleware_plugin(&self) -> Option<&dyn MiddlewarePlugin> {
        Some(self)
    }
}

impl RoutePlugin for PrometheusMetricsPlugin {
    fn routes(&self) -> Router<AppState> {
        let Some(runtime) = self.runtime.as_ref().map(Arc::clone) else {
            warn!(
                plugin_id = PROMETHEUS_METRICS_PLUGIN_ID,
                "Metrics routes requested before init"
            );
            return Router::new();
        };
        let endpoint = runtime.endpoint.clone();
        Router::new().route(
            &endpoint,
            get({
                let runtime = Arc::clone(&runtime);
                move || metrics_handler(Arc::clone(&runtime))
            }),
        )
    }
}

#[async_trait]
impl EventPlugin for PrometheusMetricsPlugin {
    fn subscriptions(&self) -> Vec<EventKind> {
        vec![
            EventKind::InstanceStored,
            EventKind::AssociationOpened,
            EventKind::AssociationClosed,
            EventKind::QueryPerformed,
        ]
    }

    async fn on_event(&self, event: &PacsEvent) -> Result<(), PluginError> {
        let Some(runtime) = &self.runtime else {
            return Err(PluginError::NotInitialized {
                plugin_id: PROMETHEUS_METRICS_PLUGIN_ID.into(),
                capability: "EventPlugin".into(),
            });
        };

        match event {
            PacsEvent::InstanceStored { .. } => {
                runtime
                    .instances_stored_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            PacsEvent::AssociationOpened { .. } => {
                runtime.associations_active.fetch_add(1, Ordering::Relaxed);
            }
            PacsEvent::AssociationRejected { .. } => {}
            PacsEvent::AssociationClosed { .. } => decrement_gauge(&runtime.associations_active),
            PacsEvent::QueryPerformed { level, source, .. } => {
                let mut query_totals = runtime.query_totals.lock().await;
                *query_totals
                    .entry(QueryMetricKey {
                        level: level.clone(),
                        source: query_source_label(source).into(),
                    })
                    .or_insert(0) += 1;
            }
            PacsEvent::StudyComplete { .. } | PacsEvent::ResourceDeleted { .. } => {}
        }

        Ok(())
    }
}

impl MiddlewarePlugin for PrometheusMetricsPlugin {
    fn apply(&self, router: Router<AppState>) -> Router<AppState> {
        let Some(runtime) = self.runtime.as_ref().map(Arc::clone) else {
            warn!(
                plugin_id = PROMETHEUS_METRICS_PLUGIN_ID,
                "Metrics middleware requested before init"
            );
            return router;
        };
        router.layer(from_fn_with_state(runtime, metrics_middleware))
    }

    fn priority(&self) -> i32 {
        100
    }
}

impl MetricsRuntime {
    async fn record_http_duration(&self, seconds: f64) {
        self.http_duration.lock().await.observe(seconds);
    }

    async fn render_prometheus(&self) -> String {
        let mut output = String::new();
        output
            .push_str("# HELP pacsnode_instances_stored_total Total number of instances stored.\n");
        output.push_str("# TYPE pacsnode_instances_stored_total counter\n");
        output.push_str(&format!(
            "pacsnode_instances_stored_total {}\n\n",
            self.instances_stored_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP pacsnode_queries_total Total number of queries performed.\n");
        output.push_str("# TYPE pacsnode_queries_total counter\n");
        let mut query_metrics = self
            .query_totals
            .lock()
            .await
            .iter()
            .map(|(key, value)| (key.clone(), *value))
            .collect::<Vec<_>>();
        query_metrics.sort_by(|(left, _), (right, _)| {
            left.level
                .cmp(&right.level)
                .then(left.source.cmp(&right.source))
        });
        for (key, value) in query_metrics {
            output.push_str(&format!(
                "pacsnode_queries_total{{level=\"{}\",source=\"{}\"}} {}\n",
                key.level, key.source, value
            ));
        }
        output.push('\n');

        output.push_str("# HELP pacsnode_associations_active Active DIMSE associations.\n");
        output.push_str("# TYPE pacsnode_associations_active gauge\n");
        output.push_str(&format!(
            "pacsnode_associations_active {}\n\n",
            self.associations_active.load(Ordering::Relaxed)
        ));

        output.push_str(
            "# HELP pacsnode_http_request_duration_seconds HTTP request duration in seconds.\n",
        );
        output.push_str("# TYPE pacsnode_http_request_duration_seconds histogram\n");
        let histogram = self.http_duration.lock().await.snapshot();
        let mut cumulative = 0_u64;
        for (index, bucket) in HTTP_DURATION_BUCKETS.iter().enumerate() {
            cumulative += histogram.bucket_hits[index];
            output.push_str(&format!(
                "pacsnode_http_request_duration_seconds_bucket{{le=\"{}\"}} {}\n",
                bucket, cumulative
            ));
        }
        output.push_str(&format!(
            "pacsnode_http_request_duration_seconds_bucket{{le=\"+Inf\"}} {}\n",
            histogram.total_count
        ));
        output.push_str(&format!(
            "pacsnode_http_request_duration_seconds_sum {}\n",
            histogram.total_sum
        ));
        output.push_str(&format!(
            "pacsnode_http_request_duration_seconds_count {}\n",
            histogram.total_count
        ));

        output
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            bucket_hits: [0; HTTP_DURATION_BUCKETS.len()],
            total_count: 0,
            total_sum: 0.0,
        }
    }
}

impl Histogram {
    fn observe(&mut self, seconds: f64) {
        self.total_count += 1;
        self.total_sum += seconds;
        if let Some(index) = HTTP_DURATION_BUCKETS
            .iter()
            .position(|bucket| seconds <= *bucket)
        {
            self.bucket_hits[index] += 1;
        }
    }

    fn snapshot(&self) -> HistogramSnapshot {
        HistogramSnapshot {
            bucket_hits: self.bucket_hits,
            total_count: self.total_count,
            total_sum: self.total_sum,
        }
    }
}

async fn metrics_handler(runtime: Arc<MetricsRuntime>) -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        runtime.render_prometheus().await,
    )
        .into_response()
}

async fn metrics_middleware(
    AxumState(runtime): AxumState<Arc<MetricsRuntime>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let started = Instant::now();
    let response = next.run(request).await;
    runtime
        .record_http_duration(started.elapsed().as_secs_f64())
        .await;
    response
}

fn normalize_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path configuration cannot be empty".into());
    }
    let normalized = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };
    if normalized.len() > 1 {
        Ok(normalized.trim_end_matches('/').to_string())
    } else {
        Ok(normalized)
    }
}

fn query_source_label(source: &QuerySource) -> &'static str {
    match source {
        QuerySource::Dimse { .. } => "dimse",
        QuerySource::Dicomweb => "dicomweb",
    }
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

register_plugin!(PrometheusMetricsPlugin::default);

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::{body::to_bytes, http::Request, routing::get};
    use bytes::Bytes;
    use pacs_core::{
        AuditLogEntry, AuditLogPage, AuditLogQuery, BlobStore, DicomJson, DicomNode, Instance,
        InstanceQuery, MetadataStore, PacsError, PacsResult, PacsStatistics, Series, SeriesQuery,
        SeriesUid, SopInstanceUid, Study, StudyQuery, StudyUid,
    };
    use tower::ServiceExt;

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
        async fn get_study(&self, uid: &StudyUid) -> PacsResult<Study> {
            Err(PacsError::NotFound {
                resource: "study",
                uid: uid.to_string(),
            })
        }
        async fn get_series(&self, uid: &SeriesUid) -> PacsResult<Series> {
            Err(PacsError::NotFound {
                resource: "series",
                uid: uid.to_string(),
            })
        }
        async fn get_instance(&self, uid: &SopInstanceUid) -> PacsResult<Instance> {
            Err(PacsError::NotFound {
                resource: "instance",
                uid: uid.to_string(),
            })
        }
        async fn get_instance_metadata(&self, uid: &SopInstanceUid) -> PacsResult<DicomJson> {
            Err(PacsError::NotFound {
                resource: "instance",
                uid: uid.to_string(),
            })
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
        async fn search_audit_logs(&self, _q: &AuditLogQuery) -> PacsResult<AuditLogPage> {
            Ok(AuditLogPage {
                entries: vec![],
                total: 0,
                limit: 100,
                offset: 0,
            })
        }
        async fn get_audit_log(&self, _id: i64) -> PacsResult<AuditLogEntry> {
            Err(PacsError::NotFound {
                resource: "audit_log",
                uid: "0".into(),
            })
        }
    }

    #[derive(Default)]
    struct NoopBlobStore;

    #[async_trait]
    impl BlobStore for NoopBlobStore {
        async fn put(&self, _key: &str, _data: Bytes) -> PacsResult<()> {
            Ok(())
        }
        async fn get(&self, key: &str) -> PacsResult<Bytes> {
            Err(PacsError::NotFound {
                resource: "blob",
                uid: key.to_string(),
            })
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

    fn plugin_context() -> PluginContext {
        PluginContext {
            config: serde_json::json!({}),
            metadata_store: Some(Arc::new(NoopMetadataStore)),
            blob_store: Some(Arc::new(NoopBlobStore)),
            server_info: pacs_plugin::ServerInfo {
                ae_title: "PACSNODE".into(),
                http_port: 8042,
                dicom_port: 4242,
                version: "test",
            },
            event_bus: Arc::new(pacs_plugin::EventBus::default()),
        }
    }

    fn app_state() -> AppState {
        AppState {
            server_info: pacs_plugin::ServerInfo {
                ae_title: "PACSNODE".into(),
                http_port: 8042,
                dicom_port: 4242,
                version: "test",
            },
            store: Arc::new(NoopMetadataStore),
            blobs: Arc::new(NoopBlobStore),
            plugins: Arc::new(pacs_plugin::PluginRegistry::new()),
        }
    }

    #[tokio::test]
    async fn metrics_endpoint_renders_prometheus_format() {
        let mut plugin = PrometheusMetricsPlugin::default();
        plugin.init(&plugin_context()).await.unwrap();
        plugin
            .on_event(&PacsEvent::InstanceStored {
                study_uid: "1.2.3".into(),
                series_uid: "4.5.6".into(),
                sop_instance_uid: "7.8.9".into(),
                sop_class_uid: "1.2.840".into(),
                source: "STOW-RS".into(),
                user_id: Some("admin".into()),
            })
            .await
            .unwrap();
        plugin
            .on_event(&PacsEvent::QueryPerformed {
                level: "STUDY".into(),
                source: QuerySource::Dicomweb,
                num_results: 2,
                user_id: None,
            })
            .await
            .unwrap();

        let app = plugin.routes().with_state(app_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("pacsnode_instances_stored_total 1"));
        assert!(text.contains("pacsnode_queries_total{level=\"STUDY\",source=\"dicomweb\"} 1"));
    }

    #[tokio::test]
    async fn metrics_middleware_records_request_duration() {
        let mut plugin = PrometheusMetricsPlugin::default();
        plugin.init(&plugin_context()).await.unwrap();
        let app = plugin
            .apply(
                Router::new()
                    .route("/hello", get(|| async { StatusCode::OK }))
                    .merge(plugin.routes()),
            )
            .with_state(app_state());

        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/hello")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let metrics_response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(metrics_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("pacsnode_http_request_duration_seconds_count 1"));
    }
}
