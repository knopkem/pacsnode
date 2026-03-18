use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::Router;
use tracing::{debug, info, warn};

use crate::{
    capabilities::{
        EventKind, FindScpHandler, GetScpHandler, MiddlewarePlugin, MoveScpHandler, StoreScpHandler,
    },
    context::PluginContext,
    error::PluginError,
    event::{EventBus, PacsEvent},
    plugin::{Plugin, PluginHealth},
    state::{AppState, ServerInfo},
};
use pacs_core::{BlobStore, MetadataStore};

/// Compile-time plugin registration record.
pub struct PluginRegistration {
    /// Factory that constructs the plugin instance.
    pub create: fn() -> Box<dyn Plugin>,
}

inventory::collect!(PluginRegistration);

/// Runtime registry for all compiled-in pacsnode plugins.
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
    plugin_ids: HashMap<String, usize>,
    event_subscribers: HashMap<EventKind, Vec<usize>>,
    codec_plugins: HashMap<String, usize>,
    enabled_ids: Option<HashSet<String>>,
    metadata_store: Option<(String, Arc<dyn MetadataStore>)>,
    blob_store: Option<(String, Arc<dyn BlobStore>)>,
    store_scp_plugin: Option<(String, usize)>,
    find_scp_plugin: Option<(String, usize)>,
    get_scp_plugin: Option<(String, usize)>,
    move_scp_plugin: Option<(String, usize)>,
    event_bus: Arc<EventBus>,
}

impl PluginRegistry {
    /// Creates a new empty plugin registry.
    ///
    /// # Example
    ///
    /// ```rust
    /// use pacs_plugin::PluginRegistry;
    ///
    /// let registry = PluginRegistry::new();
    /// assert!(registry.metadata_store().is_none());
    /// ```
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            plugin_ids: HashMap::new(),
            event_subscribers: HashMap::new(),
            codec_plugins: HashMap::new(),
            enabled_ids: None,
            metadata_store: None,
            blob_store: None,
            store_scp_plugin: None,
            find_scp_plugin: None,
            get_scp_plugin: None,
            move_scp_plugin: None,
            event_bus: Arc::new(EventBus::default()),
        }
    }

    /// Limits runtime activation to the given plugin IDs.
    pub fn set_enabled<I>(&mut self, ids: I)
    where
        I: IntoIterator<Item = String>,
    {
        self.enabled_ids = Some(ids.into_iter().collect());
    }

    /// Returns the shared event bus.
    pub fn event_bus(&self) -> Arc<EventBus> {
        Arc::clone(&self.event_bus)
    }

    /// Registers a plugin instance.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) -> Result<(), PluginError> {
        let manifest = plugin.manifest();
        if let Some(enabled) = &self.enabled_ids {
            if !manifest.enabled_by_default && !enabled.contains(&manifest.id) {
                debug!(plugin_id = %manifest.id, "Skipping disabled plugin");
                return Ok(());
            }
        } else if !manifest.enabled_by_default {
            debug!(plugin_id = %manifest.id, "Skipping plugin disabled by default");
            return Ok(());
        }

        if self.plugin_ids.contains_key(&manifest.id) {
            return Err(PluginError::DuplicatePluginId { id: manifest.id });
        }

        let idx = self.plugins.len();
        self.plugin_ids.insert(manifest.id.clone(), idx);
        self.plugins.push(plugin);
        Ok(())
    }

    /// Registers all plugins discovered through the `inventory` registry.
    pub fn register_all_discovered(&mut self) -> Result<(), PluginError> {
        for registration in inventory::iter::<PluginRegistration> {
            self.register((registration.create)())?;
        }
        Ok(())
    }

    /// Initializes and starts all registered plugins in dependency order.
    pub async fn init_all(
        &mut self,
        server_info: ServerInfo,
        plugin_configs: &HashMap<String, serde_json::Value>,
    ) -> Result<(), PluginError> {
        let order = self.resolve_dependency_order()?;

        for &idx in &order {
            let manifest = self.plugins[idx].manifest();
            let ctx = PluginContext {
                config: plugin_configs
                    .get(&manifest.id)
                    .cloned()
                    .unwrap_or(serde_json::Value::Object(Default::default())),
                metadata_store: self
                    .metadata_store
                    .as_ref()
                    .map(|(_, store)| Arc::clone(store)),
                blob_store: self.blob_store.as_ref().map(|(_, store)| Arc::clone(store)),
                server_info: server_info.clone(),
                event_bus: Arc::clone(&self.event_bus),
            };

            info!(plugin_id = %manifest.id, "Initializing plugin");
            self.plugins[idx].init(&ctx).await?;
            self.register_capabilities(idx)?;
        }

        for &idx in &order {
            let manifest = self.plugins[idx].manifest();
            let ctx = PluginContext {
                config: plugin_configs
                    .get(&manifest.id)
                    .cloned()
                    .unwrap_or(serde_json::Value::Object(Default::default())),
                metadata_store: self
                    .metadata_store
                    .as_ref()
                    .map(|(_, store)| Arc::clone(store)),
                blob_store: self.blob_store.as_ref().map(|(_, store)| Arc::clone(store)),
                server_info: server_info.clone(),
                event_bus: Arc::clone(&self.event_bus),
            };

            info!(plugin_id = %manifest.id, "Starting plugin");
            self.plugins[idx].start(&ctx).await?;
        }

        Ok(())
    }

    /// Shuts down all plugins in reverse dependency order.
    pub async fn shutdown_all(&self) -> Result<(), PluginError> {
        let order = self.resolve_dependency_order()?;
        for &idx in order.iter().rev() {
            let manifest = self.plugins[idx].manifest();
            info!(plugin_id = %manifest.id, "Shutting down plugin");
            self.plugins[idx].shutdown().await?;
        }
        Ok(())
    }

    /// Returns the active metadata store.
    pub fn metadata_store(&self) -> Option<Arc<dyn MetadataStore>> {
        self.metadata_store
            .as_ref()
            .map(|(_, store)| Arc::clone(store))
    }

    /// Returns the active blob store.
    pub fn blob_store(&self) -> Option<Arc<dyn BlobStore>> {
        self.blob_store.as_ref().map(|(_, store)| Arc::clone(store))
    }

    /// Returns the active C-STORE SCP handler, if any.
    pub fn store_scp_handler(
        self: &Arc<Self>,
    ) -> Result<Option<Arc<dyn StoreScpHandler>>, PluginError> {
        match self.store_scp_plugin.as_ref() {
            Some((_, idx)) => self.plugins[*idx]
                .as_store_scp_plugin()
                .ok_or_else(|| PluginError::Runtime {
                    plugin_id: self.plugins[*idx].manifest().id,
                    message: "store SCP capability missing from registered plugin".into(),
                })?
                .store_scp_handler(Arc::clone(self))
                .map(Some),
            None => Ok(None),
        }
    }

    /// Returns the active C-FIND SCP handler, if any.
    pub fn find_scp_handler(
        self: &Arc<Self>,
    ) -> Result<Option<Arc<dyn FindScpHandler>>, PluginError> {
        match self.find_scp_plugin.as_ref() {
            Some((_, idx)) => self.plugins[*idx]
                .as_find_scp_plugin()
                .ok_or_else(|| PluginError::Runtime {
                    plugin_id: self.plugins[*idx].manifest().id,
                    message: "find SCP capability missing from registered plugin".into(),
                })?
                .find_scp_handler(Arc::clone(self))
                .map(Some),
            None => Ok(None),
        }
    }

    /// Returns the active C-GET SCP handler, if any.
    pub fn get_scp_handler(
        self: &Arc<Self>,
    ) -> Result<Option<Arc<dyn GetScpHandler>>, PluginError> {
        match self.get_scp_plugin.as_ref() {
            Some((_, idx)) => self.plugins[*idx]
                .as_get_scp_plugin()
                .ok_or_else(|| PluginError::Runtime {
                    plugin_id: self.plugins[*idx].manifest().id,
                    message: "get SCP capability missing from registered plugin".into(),
                })?
                .get_scp_handler(Arc::clone(self))
                .map(Some),
            None => Ok(None),
        }
    }

    /// Returns the active C-MOVE SCP handler, if any.
    pub fn move_scp_handler(
        self: &Arc<Self>,
    ) -> Result<Option<Arc<dyn MoveScpHandler>>, PluginError> {
        match self.move_scp_plugin.as_ref() {
            Some((_, idx)) => self.plugins[*idx]
                .as_move_scp_plugin()
                .ok_or_else(|| PluginError::Runtime {
                    plugin_id: self.plugins[*idx].manifest().id,
                    message: "move SCP capability missing from registered plugin".into(),
                })?
                .move_scp_handler(Arc::clone(self))
                .map(Some),
            None => Ok(None),
        }
    }

    /// Returns the merged router contributed by all route plugins.
    pub fn merged_routes(&self) -> Router<AppState> {
        let mut router = Router::new();
        for plugin in &self.plugins {
            if let Some(route_plugin) = plugin.as_route_plugin() {
                router = router.merge(route_plugin.routes());
            }
        }
        router
    }

    /// Applies middleware plugins to the router in priority order.
    pub fn apply_middleware(&self, mut router: Router<AppState>) -> Router<AppState> {
        let mut middleware_plugins: Vec<(i32, &dyn MiddlewarePlugin)> = self
            .plugins
            .iter()
            .filter_map(|plugin| {
                plugin
                    .as_middleware_plugin()
                    .map(|middleware| (middleware.priority(), middleware))
            })
            .collect();
        middleware_plugins.sort_by_key(|(priority, _)| *priority);

        for (_, middleware) in middleware_plugins {
            router = middleware.apply(router);
        }

        router
    }

    /// Emits an event to subscribed plugins and the shared event bus.
    pub async fn emit_event(&self, event: PacsEvent) {
        self.event_bus.emit(event.clone());
        if let Some(subscribers) = self.event_subscribers.get(&event.kind()) {
            for &idx in subscribers {
                if let Some(event_plugin) = self.plugins[idx].as_event_plugin() {
                    let manifest = self.plugins[idx].manifest();
                    if let Err(error) = event_plugin.on_event(&event).await {
                        warn!(
                            plugin_id = %manifest.id,
                            error = %error,
                            event_kind = ?event.kind(),
                            "Plugin event handler failed"
                        );
                    }
                }
            }
        }
    }

    /// Aggregates health from all registered plugins.
    pub async fn aggregate_health(&self) -> Vec<(String, PluginHealth)> {
        let mut results = Vec::with_capacity(self.plugins.len());
        for plugin in &self.plugins {
            let manifest = plugin.manifest();
            results.push((manifest.id, plugin.health().await));
        }
        results
    }

    fn register_capabilities(&mut self, idx: usize) -> Result<(), PluginError> {
        let manifest = self.plugins[idx].manifest();

        if let Some(metadata_plugin) = self.plugins[idx].as_metadata_store_plugin() {
            let store = metadata_plugin.metadata_store()?;
            if let Some((existing, _)) = &self.metadata_store {
                return Err(PluginError::DuplicateProvider {
                    capability: "MetadataStore".into(),
                    first: existing.clone(),
                    second: manifest.id,
                });
            }
            self.metadata_store = Some((manifest.id.clone(), store));
        }

        if let Some(blob_plugin) = self.plugins[idx].as_blob_store_plugin() {
            let store = blob_plugin.blob_store()?;
            if let Some((existing, _)) = &self.blob_store {
                return Err(PluginError::DuplicateProvider {
                    capability: "BlobStore".into(),
                    first: existing.clone(),
                    second: manifest.id,
                });
            }
            self.blob_store = Some((manifest.id.clone(), store));
        }

        if self.plugins[idx].as_store_scp_plugin().is_some() {
            if let Some((existing, _)) = &self.store_scp_plugin {
                return Err(PluginError::DuplicateProvider {
                    capability: "StoreScp".into(),
                    first: existing.clone(),
                    second: manifest.id.clone(),
                });
            }
            self.store_scp_plugin = Some((manifest.id.clone(), idx));
        }

        if self.plugins[idx].as_find_scp_plugin().is_some() {
            if let Some((existing, _)) = &self.find_scp_plugin {
                return Err(PluginError::DuplicateProvider {
                    capability: "FindScp".into(),
                    first: existing.clone(),
                    second: manifest.id.clone(),
                });
            }
            self.find_scp_plugin = Some((manifest.id.clone(), idx));
        }

        if self.plugins[idx].as_get_scp_plugin().is_some() {
            if let Some((existing, _)) = &self.get_scp_plugin {
                return Err(PluginError::DuplicateProvider {
                    capability: "GetScp".into(),
                    first: existing.clone(),
                    second: manifest.id.clone(),
                });
            }
            self.get_scp_plugin = Some((manifest.id.clone(), idx));
        }

        if self.plugins[idx].as_move_scp_plugin().is_some() {
            if let Some((existing, _)) = &self.move_scp_plugin {
                return Err(PluginError::DuplicateProvider {
                    capability: "MoveScp".into(),
                    first: existing.clone(),
                    second: manifest.id.clone(),
                });
            }
            self.move_scp_plugin = Some((manifest.id.clone(), idx));
        }

        if let Some(event_plugin) = self.plugins[idx].as_event_plugin() {
            for kind in event_plugin.subscriptions() {
                self.event_subscribers.entry(kind).or_default().push(idx);
            }
        }

        if let Some(codec_plugin) = self.plugins[idx].as_codec_plugin() {
            for syntax in codec_plugin.supported_transfer_syntaxes() {
                if let Some(existing_idx) = self.codec_plugins.insert(syntax.clone(), idx) {
                    let existing = self.plugins[existing_idx].manifest();
                    return Err(PluginError::DuplicateProvider {
                        capability: format!("Codec({syntax})"),
                        first: existing.id,
                        second: manifest.id,
                    });
                }
            }
        }

        Ok(())
    }

    fn resolve_dependency_order(&self) -> Result<Vec<usize>, PluginError> {
        let mut indegree = vec![0usize; self.plugins.len()];
        let mut graph: Vec<Vec<usize>> = vec![Vec::new(); self.plugins.len()];

        for (idx, plugin) in self.plugins.iter().enumerate() {
            let manifest = plugin.manifest();
            for dependency in manifest.dependencies {
                let dep_indices =
                    self.resolve_dependency_indices(&dependency)
                        .ok_or_else(|| PluginError::MissingDependency {
                            plugin_id: manifest.id.clone(),
                            dependency,
                        })?;

                for dep_idx in dep_indices {
                    graph[dep_idx].push(idx);
                    indegree[idx] += 1;
                }
            }
        }

        let mut ready: Vec<usize> = indegree
            .iter()
            .enumerate()
            .filter_map(|(idx, degree)| (*degree == 0).then_some(idx))
            .collect();
        let mut order = Vec::with_capacity(self.plugins.len());

        while let Some(node) = ready.pop() {
            order.push(node);
            for &next in &graph[node] {
                indegree[next] -= 1;
                if indegree[next] == 0 {
                    ready.push(next);
                }
            }
        }

        if order.len() != self.plugins.len() {
            let cycle = indegree
                .iter()
                .enumerate()
                .filter(|(_, degree)| **degree > 0)
                .map(|(idx, _)| self.plugins[idx].manifest().id)
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(PluginError::CircularDependency { cycle });
        }

        Ok(order)
    }

    fn resolve_dependency_indices(&self, dependency: &str) -> Option<Vec<usize>> {
        let matches = match dependency {
            crate::METADATA_STORE_CAPABILITY_DEPENDENCY => self
                .plugins
                .iter()
                .enumerate()
                .filter_map(|(idx, plugin)| {
                    plugin.as_metadata_store_plugin().is_some().then_some(idx)
                })
                .collect::<Vec<_>>(),
            crate::BLOB_STORE_CAPABILITY_DEPENDENCY => self
                .plugins
                .iter()
                .enumerate()
                .filter_map(|(idx, plugin)| plugin.as_blob_store_plugin().is_some().then_some(idx))
                .collect::<Vec<_>>(),
            _ => self
                .plugin_ids
                .get(dependency)
                .copied()
                .into_iter()
                .collect::<Vec<_>>(),
        };

        (!matches.is_empty()).then_some(matches)
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use bytes::Bytes;
    use pacs_core::{
        AuditLogEntry, AuditLogPage, AuditLogQuery, BlobStore, DicomJson, DicomNode, Instance,
        InstanceQuery, MetadataStore, NewAuditLogEntry, PacsError, PacsResult, PacsStatistics,
        Series, SeriesQuery, SeriesUid, ServerSettings, SopInstanceUid, Study, StudyQuery,
        StudyUid,
    };
    use tokio::sync::Mutex;

    use super::*;
    use crate::{
        capabilities::{BlobStorePlugin, EventKind, EventPlugin, MetadataStorePlugin},
        Plugin, PluginContext, PluginManifest,
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

    struct OrderPlugin {
        id: &'static str,
        deps: Vec<String>,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl OrderPlugin {
        fn new(id: &'static str, deps: Vec<String>, events: Arc<Mutex<Vec<String>>>) -> Self {
            Self { id, deps, events }
        }
    }

    #[async_trait]
    impl Plugin for OrderPlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new(self.id, self.id, "0.1.0").with_dependencies(self.deps.clone())
        }

        async fn init(&mut self, _ctx: &PluginContext) -> Result<(), PluginError> {
            self.events.lock().await.push(format!("init:{}", self.id));
            Ok(())
        }

        async fn start(&self, _ctx: &PluginContext) -> Result<(), PluginError> {
            self.events.lock().await.push(format!("start:{}", self.id));
            Ok(())
        }

        async fn shutdown(&self) -> Result<(), PluginError> {
            self.events
                .lock()
                .await
                .push(format!("shutdown:{}", self.id));
            Ok(())
        }
    }

    struct MetadataPlugin {
        store: Arc<dyn MetadataStore>,
    }

    impl Default for MetadataPlugin {
        fn default() -> Self {
            Self {
                store: Arc::new(NoopMetadataStore),
            }
        }
    }

    #[async_trait]
    impl Plugin for MetadataPlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new("meta", "meta", "0.1.0")
        }

        async fn init(&mut self, _ctx: &PluginContext) -> Result<(), PluginError> {
            Ok(())
        }

        fn as_metadata_store_plugin(&self) -> Option<&dyn MetadataStorePlugin> {
            Some(self)
        }
    }

    impl MetadataStorePlugin for MetadataPlugin {
        fn metadata_store(&self) -> Result<Arc<dyn MetadataStore>, PluginError> {
            Ok(Arc::clone(&self.store))
        }
    }

    struct BlobPlugin {
        store: Arc<dyn BlobStore>,
    }

    impl Default for BlobPlugin {
        fn default() -> Self {
            Self {
                store: Arc::new(NoopBlobStore),
            }
        }
    }

    #[async_trait]
    impl Plugin for BlobPlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new("blob", "blob", "0.1.0")
        }

        async fn init(&mut self, _ctx: &PluginContext) -> Result<(), PluginError> {
            Ok(())
        }

        fn as_blob_store_plugin(&self) -> Option<&dyn BlobStorePlugin> {
            Some(self)
        }
    }

    impl BlobStorePlugin for BlobPlugin {
        fn blob_store(&self) -> Result<Arc<dyn BlobStore>, PluginError> {
            Ok(Arc::clone(&self.store))
        }
    }

    struct EventRecorder {
        seen: Arc<Mutex<Vec<EventKind>>>,
    }

    #[async_trait]
    impl Plugin for EventRecorder {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new("event-recorder", "Event Recorder", "0.1.0")
        }

        async fn init(&mut self, _ctx: &PluginContext) -> Result<(), PluginError> {
            Ok(())
        }

        fn as_event_plugin(&self) -> Option<&dyn EventPlugin> {
            Some(self)
        }
    }

    #[async_trait]
    impl EventPlugin for EventRecorder {
        fn subscriptions(&self) -> Vec<EventKind> {
            vec![EventKind::InstanceStored]
        }

        async fn on_event(&self, event: &PacsEvent) -> Result<(), PluginError> {
            self.seen.lock().await.push(event.kind());
            Ok(())
        }
    }

    #[tokio::test]
    async fn init_start_and_shutdown_follow_dependency_order() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(OrderPlugin::new(
                "base",
                vec![],
                Arc::clone(&events),
            )))
            .unwrap();
        registry
            .register(Box::new(OrderPlugin::new(
                "child",
                vec!["base".into()],
                Arc::clone(&events),
            )))
            .unwrap();

        registry
            .init_all(
                ServerInfo {
                    ae_title: "PACSNODE".into(),
                    http_port: 8042,
                    dicom_port: 4242,
                    version: "0.1.0",
                },
                &HashMap::new(),
            )
            .await
            .unwrap();
        registry.shutdown_all().await.unwrap();

        let events = events.lock().await.clone();
        assert_eq!(
            events,
            vec![
                "init:base",
                "init:child",
                "start:base",
                "start:child",
                "shutdown:child",
                "shutdown:base",
            ]
        );
    }

    #[tokio::test]
    async fn resolves_singleton_store_capabilities() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(MetadataPlugin::default()))
            .unwrap();
        registry.register(Box::new(BlobPlugin::default())).unwrap();
        registry
            .init_all(
                ServerInfo {
                    ae_title: "PACSNODE".into(),
                    http_port: 8042,
                    dicom_port: 4242,
                    version: "0.1.0",
                },
                &HashMap::new(),
            )
            .await
            .unwrap();

        assert!(registry.metadata_store().is_some());
        assert!(registry.blob_store().is_some());
    }

    #[tokio::test]
    async fn capability_dependencies_resolve_against_provider_plugins() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(MetadataPlugin::default()))
            .unwrap();
        registry.register(Box::new(BlobPlugin::default())).unwrap();
        registry
            .register(Box::new(OrderPlugin::new(
                "dependent",
                vec![
                    crate::METADATA_STORE_CAPABILITY_DEPENDENCY.into(),
                    crate::BLOB_STORE_CAPABILITY_DEPENDENCY.into(),
                ],
                Arc::clone(&events),
            )))
            .unwrap();

        registry
            .init_all(
                ServerInfo {
                    ae_title: "PACSNODE".into(),
                    http_port: 8042,
                    dicom_port: 4242,
                    version: "0.1.0",
                },
                &HashMap::new(),
            )
            .await
            .unwrap();

        let events = events.lock().await.clone();
        assert_eq!(events, vec!["init:dependent", "start:dependent"]);
        assert!(registry.metadata_store().is_some());
        assert!(registry.blob_store().is_some());
    }

    #[tokio::test]
    async fn emits_events_to_subscribers() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(EventRecorder {
                seen: Arc::clone(&seen),
            }))
            .unwrap();
        registry
            .init_all(
                ServerInfo {
                    ae_title: "PACSNODE".into(),
                    http_port: 8042,
                    dicom_port: 4242,
                    version: "0.1.0",
                },
                &HashMap::new(),
            )
            .await
            .unwrap();

        registry
            .emit_event(PacsEvent::InstanceStored {
                study_uid: "1.2.3".into(),
                series_uid: "4.5.6".into(),
                sop_instance_uid: "7.8.9".into(),
                sop_class_uid: "1.2".into(),
                source: "TEST".into(),
                user_id: Some("admin".into()),
            })
            .await;

        assert_eq!(seen.lock().await.as_slice(), &[EventKind::InstanceStored]);
    }

    struct OptionalPlugin;

    #[async_trait]
    impl Plugin for OptionalPlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new("optional-plugin", "Optional Plugin", "0.1.0").disabled_by_default()
        }

        async fn init(&mut self, _ctx: &PluginContext) -> Result<(), PluginError> {
            Ok(())
        }
    }

    #[test]
    fn skips_plugins_disabled_by_default_unless_enabled() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(OptionalPlugin)).unwrap();
        assert!(registry.resolve_dependency_order().unwrap().is_empty());

        let mut enabled_registry = PluginRegistry::new();
        enabled_registry.set_enabled(["optional-plugin".to_string()]);
        enabled_registry.register(Box::new(OptionalPlugin)).unwrap();
        assert_eq!(
            enabled_registry.resolve_dependency_order().unwrap().len(),
            1
        );
    }
}
