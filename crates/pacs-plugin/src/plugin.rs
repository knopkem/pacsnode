use async_trait::async_trait;

use crate::{
    capabilities::{
        BlobStorePlugin, CodecPlugin, EventPlugin, FindScpPlugin, GetScpPlugin,
        MetadataStorePlugin, MiddlewarePlugin, MoveScpPlugin, ProcessingPlugin, RoutePlugin,
        StoreScpPlugin,
    },
    context::PluginContext,
    error::PluginError,
};

/// Health reported by a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginHealth {
    /// Plugin is operating normally.
    Healthy,
    /// Plugin is operating but degraded.
    Degraded(String),
    /// Plugin failed and is unavailable.
    Unhealthy(String),
}

/// Static metadata describing a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    /// Unique plugin identifier in kebab-case.
    pub id: String,
    /// Human-friendly display name.
    pub name: String,
    /// Semantic version string.
    pub version: String,
    /// Plugin IDs that must be initialized before this one.
    pub dependencies: Vec<String>,
    /// Whether this plugin is active when no explicit enable-list is supplied.
    pub enabled_by_default: bool,
}

impl PluginManifest {
    /// Creates a manifest without dependencies.
    ///
    /// # Example
    ///
    /// ```rust
    /// use pacs_plugin::PluginManifest;
    ///
    /// let manifest = PluginManifest::new("test-plugin", "Test Plugin", "1.0.0");
    /// assert_eq!(manifest.id, "test-plugin");
    /// assert!(manifest.dependencies.is_empty());
    /// ```
    pub fn new(id: impl Into<String>, name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            version: version.into(),
            dependencies: Vec::new(),
            enabled_by_default: true,
        }
    }

    /// Attaches dependencies to the manifest.
    pub fn with_dependencies(
        mut self,
        dependencies: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.dependencies = dependencies.into_iter().map(Into::into).collect();
        self
    }

    /// Marks the plugin as disabled unless it is explicitly enabled in config.
    pub fn disabled_by_default(mut self) -> Self {
        self.enabled_by_default = false;
        self
    }
}

/// Core trait implemented by every pacsnode plugin.
///
/// The default capability accessors return `None`. Plugins override the ones
/// they support so the registry can discover them after initialization.
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    /// Returns static metadata describing the plugin.
    fn manifest(&self) -> PluginManifest;

    /// Performs plugin initialization.
    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError>;

    /// Starts the plugin after all plugins have been initialized.
    async fn start(&self, _ctx: &PluginContext) -> Result<(), PluginError> {
        Ok(())
    }

    /// Shuts the plugin down gracefully.
    async fn shutdown(&self) -> Result<(), PluginError> {
        Ok(())
    }

    /// Returns the current plugin health.
    async fn health(&self) -> PluginHealth {
        PluginHealth::Healthy
    }

    /// Returns the metadata-store capability if implemented.
    fn as_metadata_store_plugin(&self) -> Option<&dyn MetadataStorePlugin> {
        None
    }

    /// Returns the blob-store capability if implemented.
    fn as_blob_store_plugin(&self) -> Option<&dyn BlobStorePlugin> {
        None
    }

    /// Returns the DIMSE C-STORE SCP capability if implemented.
    fn as_store_scp_plugin(&self) -> Option<&dyn StoreScpPlugin> {
        None
    }

    /// Returns the DIMSE C-FIND SCP capability if implemented.
    fn as_find_scp_plugin(&self) -> Option<&dyn FindScpPlugin> {
        None
    }

    /// Returns the DIMSE C-GET SCP capability if implemented.
    fn as_get_scp_plugin(&self) -> Option<&dyn GetScpPlugin> {
        None
    }

    /// Returns the DIMSE C-MOVE SCP capability if implemented.
    fn as_move_scp_plugin(&self) -> Option<&dyn MoveScpPlugin> {
        None
    }

    /// Returns the route capability if implemented.
    fn as_route_plugin(&self) -> Option<&dyn RoutePlugin> {
        None
    }

    /// Returns the middleware capability if implemented.
    fn as_middleware_plugin(&self) -> Option<&dyn MiddlewarePlugin> {
        None
    }

    /// Returns the event capability if implemented.
    fn as_event_plugin(&self) -> Option<&dyn EventPlugin> {
        None
    }

    /// Returns the codec capability if implemented.
    fn as_codec_plugin(&self) -> Option<&dyn CodecPlugin> {
        None
    }

    /// Returns the processing capability if implemented.
    fn as_processing_plugin(&self) -> Option<&dyn ProcessingPlugin> {
        None
    }
}
