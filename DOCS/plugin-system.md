# pacsnode Plugin System — Implementation Guide

> This document contains all architecture decisions, exact file locations, code
> templates, and step-by-step instructions needed to implement the plugin system.
> It is designed to be self-contained: an implementor should not need to re-derive
> any design decisions.

---

## Table of Contents

1. [Architecture Decision](#1-architecture-decision)
2. [Current Extension Points](#2-current-extension-points)
3. [Phase 1 — `pacs-plugin` Crate](#3-phase-1--pacs-plugin-crate)
4. [Phase 2 — Wrap Existing Code as Plugins](#4-phase-2--wrap-existing-code-as-plugins)
5. [Phase 3 — First Optional Plugins](#5-phase-3--first-optional-plugins)
6. [Phase 4 — Content Plugins (Future)](#6-phase-4--content-plugins)
7. [Phase 5 — Alternative Backends (Future)](#7-phase-5--alternative-backends)
8. [Built-in vs Plugin Classification](#8-built-in-vs-plugin-classification)
9. [Testing Strategy](#9-testing-strategy)
10. [Configuration Schema](#10-configuration-schema)
11. [Migration Path](#11-migration-path)

---

## 1. Architecture Decision

### Chosen Approach: Compile-Time Trait-Based Plugins

Plugins are Rust crates compiled into the pacsnode binary. They register themselves
at compile time via the `inventory` crate. The `pacs-server` binary activates plugins
based on the `[plugins]` TOML configuration.

**No dynamic loading, no FFI, no WASM (for now).**

### Rationale

| Factor | Conclusion |
|--------|-----------|
| Medical safety | Compile-time type checking > runtime FFI safety |
| Existing architecture | `Arc<dyn MetadataStore>` / `Arc<dyn BlobStore>` already trait-based DI |
| Toolkit design | `StoreServiceProvider`, `FindServiceProvider` etc. are already trait interfaces |
| Deployment model | PACS servers update in maintenance windows; hot-reload unnecessary |
| Rust ecosystem | `inventory` crate proven (used by tracing, criterion, etc.) |

### Future WASM Extension

Phase 6+ adds `wasmtime`/`extism` for sandboxed scripting (non-Rust plugins).
The `EventPlugin` trait is intentionally designed so a WASM adapter can implement it.

---

## 2. Current Extension Points

These already exist in the codebase and the plugin system formalizes them:

### 2.1 MetadataStore Trait

**File:** `crates/pacs-core/src/store/mod.rs` (lines 18–89)

```rust
#[async_trait]
pub trait MetadataStore: Send + Sync {
    async fn store_study(&self, study: &Study) -> PacsResult<()>;
    async fn store_series(&self, series: &Series) -> PacsResult<()>;
    async fn store_instance(&self, instance: &Instance) -> PacsResult<()>;
    async fn query_studies(&self, q: &StudyQuery) -> PacsResult<Vec<Study>>;
    async fn query_series(&self, q: &SeriesQuery) -> PacsResult<Vec<Series>>;
    async fn query_instances(&self, q: &InstanceQuery) -> PacsResult<Vec<Instance>>;
    async fn get_study(&self, uid: &StudyUid) -> PacsResult<Study>;
    async fn get_series(&self, uid: &SeriesUid) -> PacsResult<Series>;
    async fn get_instance(&self, uid: &SopInstanceUid) -> PacsResult<Instance>;
    async fn get_instance_metadata(&self, uid: &SopInstanceUid) -> PacsResult<DicomJson>;
    async fn delete_study(&self, uid: &StudyUid) -> PacsResult<()>;
    async fn delete_series(&self, uid: &SeriesUid) -> PacsResult<()>;
    async fn delete_instance(&self, uid: &SopInstanceUid) -> PacsResult<()>;
    async fn get_statistics(&self) -> PacsResult<PacsStatistics>;
    async fn list_nodes(&self) -> PacsResult<Vec<DicomNode>>;
    async fn upsert_node(&self, node: &DicomNode) -> PacsResult<()>;
    async fn delete_node(&self, ae_title: &str) -> PacsResult<()>;
}
```

**Current impl:** `pacs_store::PgMetadataStore` (PostgreSQL)
**Injected at:** `crates/pacs-server/src/main.rs:62` as `Arc<dyn MetadataStore>`

### 2.2 BlobStore Trait

**File:** `crates/pacs-core/src/blob/mod.rs` (lines 15–38)

```rust
#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, key: &str, data: Bytes) -> PacsResult<()>;
    async fn get(&self, key: &str) -> PacsResult<Bytes>;
    async fn delete(&self, key: &str) -> PacsResult<()>;
    async fn exists(&self, key: &str) -> PacsResult<bool>;
    async fn presigned_url(&self, key: &str, ttl_secs: u32) -> PacsResult<String>;
}
```

**Current impl:** `pacs_storage::S3BlobStore`
**Injected at:** `crates/pacs-server/src/main.rs:57` as `Arc<dyn BlobStore>`

### 2.3 DIMSE Service Provider Traits (from dicom-toolkit-rs)

**File:** `~/.cargo/git/checkouts/dicom-toolkit-rs-*/*/crates/dicom-toolkit-net/src/services/provider.rs`

```rust
pub trait StoreServiceProvider: Send + Sync + 'static {
    async fn on_store(&self, event: StoreEvent) -> StoreResult;
}
pub trait FindServiceProvider: Send + Sync + 'static {
    async fn on_find(&self, event: FindEvent) -> Vec<DataSet>;
}
pub trait GetServiceProvider: Send + Sync + 'static {
    async fn on_get(&self, event: GetEvent) -> Vec<RetrieveItem>;
}
pub trait MoveServiceProvider: Send + Sync + 'static {
    async fn on_move(&self, event: MoveEvent) -> Vec<RetrieveItem>;
}
```

**Current impls:** `PacsStoreProvider`, `PacsQueryProvider`
**File:** `crates/pacs-dimse/src/server/provider.rs` (lines 26–282)
**Wired at:** `crates/pacs-dimse/src/server/mod.rs:179–192` (per connection) and
`mod.rs:300–322` (`build_dicom_server`)

### 2.4 Axum Router + Tower Middleware

**File:** `crates/pacs-api/src/router.rs` (lines 30–111)

Routes and middleware are composed functionally. To add plugin routes:
```rust
router.merge(plugin_router)     // adds routes from plugin
router.layer(plugin_middleware)  // wraps all routes with plugin layer
```

### 2.5 AppState (shared state)

**File:** `crates/pacs-api/src/state.rs` (lines 28–35)

```rust
#[derive(Clone)]
pub struct AppState {
    pub server_info: ServerInfo,
    pub store: Arc<dyn MetadataStore>,
    pub blobs: Arc<dyn BlobStore>,
}
```

This will be extended to hold the `PluginRegistry` reference.

### 2.6 Application Wiring (main.rs)

**File:** `crates/pacs-server/src/main.rs` (lines 18–121)

Startup sequence:
1. Load `AppConfig` (line 21)
2. Init tracing (line 24)
3. Connect to PostgreSQL (lines 34–38)
4. Run migrations (lines 40–47)
5. Create `S3BlobStore` (lines 50–59)
6. Create `PgMetadataStore` (lines 62–63)
7. Build `AppState` (lines 66–75)
8. Build Axum router (line 76)
9. Build `DicomServer` (lines 91–95)
10. Run HTTP + DIMSE concurrently with graceful shutdown (lines 100–117)

The plugin system inserts between steps 6 and 7.

---

## 3. Phase 1 — `pacs-plugin` Crate

### 3.1 Create the Crate

**New file:** `crates/pacs-plugin/Cargo.toml`

```toml
[package]
name        = "pacs-plugin"
description = "pacsnode — plugin system traits, registry, and event bus"
version.workspace     = true
edition.workspace     = true
rust-version.workspace = true
license.workspace     = true

[dependencies]
pacs-core   = { workspace = true }
async-trait = { workspace = true }
axum        = { workspace = true }
bytes       = { workspace = true }
inventory   = "0.3"
serde       = { workspace = true }
serde_json  = { workspace = true }
thiserror   = { workspace = true }
tokio       = { workspace = true }
toml        = "0.8"
tracing     = { workspace = true }

[dev-dependencies]
mockall  = { workspace = true }
rstest   = { workspace = true }
tokio    = { workspace = true }
```

**Add to workspace root `Cargo.toml`:**

```toml
# In [workspace] members:
"crates/pacs-plugin",

# In [workspace.dependencies]:
pacs-plugin  = { path = "crates/pacs-plugin" }
inventory    = "0.3"
toml         = "0.8"
```

### 3.2 Plugin Trait

**New file:** `crates/pacs-plugin/src/plugin.rs`

```rust
//! Core plugin trait and lifecycle types.

use async_trait::async_trait;
use crate::error::PluginError;

/// Health status reported by a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginHealth {
    /// Plugin is operating normally.
    Healthy,
    /// Plugin is degraded but functional.
    Degraded(String),
    /// Plugin has failed and is not functional.
    Unhealthy(String),
}

/// Metadata about a plugin, returned by [`Plugin::manifest`].
#[derive(Debug, Clone)]
pub struct PluginManifest {
    /// Unique identifier (e.g., `"pg-metadata-store"`). Must be kebab-case.
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// SemVer version string.
    pub version: String,
    /// Plugin IDs that must be initialized before this one.
    pub dependencies: Vec<String>,
}

/// Core plugin trait. Every plugin implements this for lifecycle management.
///
/// # Lifecycle
///
/// 1. `manifest()` — called first to determine identity and dependencies
/// 2. `init(ctx)` — called once in dependency order with the plugin's config
/// 3. `start(ctx)` — called after ALL plugins are initialized
/// 4. (server runs)
/// 5. `shutdown()` — called in reverse dependency order during graceful shutdown
///
/// # Example
///
/// ```rust,ignore
/// use pacs_plugin::{Plugin, PluginContext, PluginManifest, PluginHealth};
///
/// pub struct MyPlugin;
///
/// #[async_trait]
/// impl Plugin for MyPlugin {
///     fn manifest(&self) -> PluginManifest {
///         PluginManifest {
///             id: "my-plugin".into(),
///             name: "My Plugin".into(),
///             version: "0.1.0".into(),
///             dependencies: vec![],
///         }
///     }
///     async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> { Ok(()) }
/// }
/// ```
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    /// Returns the plugin's identity, version, and dependencies.
    fn manifest(&self) -> PluginManifest;

    /// Initialize the plugin. Called once during startup in dependency order.
    ///
    /// Use this to validate config, open connections, and allocate resources.
    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError>;

    /// Called after ALL plugins are initialized. Plugins may now interact
    /// with each other through the registry.
    async fn start(&self, _ctx: &PluginContext) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called during graceful shutdown in reverse dependency order.
    async fn shutdown(&self) -> Result<(), PluginError> {
        Ok(())
    }

    /// Health check. The registry aggregates these for `GET /health`.
    async fn health(&self) -> PluginHealth {
        PluginHealth::Healthy
    }
}

// Forward declaration — defined in context.rs
use crate::context::PluginContext;
```

### 3.3 Capability Traits

**New file:** `crates/pacs-plugin/src/capabilities.rs`

```rust
//! Capability traits that plugins implement to provide specific services.
//!
//! A plugin declares its capabilities by implementing one or more of these
//! traits in addition to [`Plugin`].

use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use pacs_core::{BlobStore, MetadataStore};

use crate::error::PluginError;
use crate::event::PacsEvent;

/// Plugin that provides a [`MetadataStore`] implementation.
///
/// Only ONE metadata store plugin may be active. If multiple are enabled,
/// the registry returns an error during init.
pub trait MetadataStorePlugin: crate::Plugin {
    /// Returns the metadata store provided by this plugin.
    fn metadata_store(&self) -> Arc<dyn MetadataStore>;
}

/// Plugin that provides a [`BlobStore`] implementation.
///
/// Only ONE blob store plugin may be active.
pub trait BlobStorePlugin: crate::Plugin {
    /// Returns the blob store provided by this plugin.
    fn blob_store(&self) -> Arc<dyn BlobStore>;
}

/// Plugin that contributes additional HTTP routes to the Axum router.
///
/// Routes are merged in plugin dependency order. Conflicting paths cause
/// an error during startup.
///
/// # Example
///
/// ```rust,ignore
/// impl RoutePlugin for PrometheusPlugin {
///     fn routes(&self) -> Router {
///         Router::new().route("/metrics", get(handler))
///     }
/// }
/// ```
pub trait RoutePlugin: crate::Plugin {
    /// Returns an Axum router to merge into the main application.
    ///
    /// The router should NOT include state — it will be provided by the host.
    fn routes(&self) -> Router;
}

/// Plugin that provides HTTP middleware.
///
/// Middleware is applied in priority order (lower = outermost / runs first).
/// For example, an auth middleware should have priority 0 (outermost), while
/// a metrics middleware might have priority 100 (innermost).
pub trait MiddlewarePlugin: crate::Plugin {
    /// Returns a tower `Layer` to wrap the application router.
    ///
    /// Uses `Box<dyn CloneLayer>` for type erasure. The implementor returns
    /// a concrete tower-http layer wrapped in the provided adapter.
    fn apply_to(&self, router: Router) -> Router;

    /// Middleware priority. Lower values run first (outermost).
    /// Default: 50. Auth should use 0–10. Metrics: 90–100.
    fn priority(&self) -> i32 {
        50
    }
}

/// Plugin that reacts to system events (instance stored, deleted, etc.).
///
/// The event bus delivers events asynchronously. Event handlers MUST NOT
/// block or perform long-running operations — offload heavy work to a
/// `tokio::task::spawn` or channel.
///
/// # Cancellation Safety
///
/// `on_event` must be cancellation-safe. If the server shuts down while
/// an event is being processed, the future may be dropped.
#[async_trait]
pub trait EventPlugin: crate::Plugin {
    /// Which event kinds this plugin wants to receive.
    fn subscriptions(&self) -> Vec<EventKind>;

    /// Handle a single event. Must return quickly.
    async fn on_event(&self, event: &PacsEvent) -> Result<(), PluginError>;
}

/// Categories of events that plugins can subscribe to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// A new DICOM instance was stored via C-STORE or STOW-RS.
    InstanceStored,
    /// A study has been received completely (heuristic: timer-based).
    StudyComplete,
    /// A resource (study, series, or instance) was deleted.
    ResourceDeleted,
    /// A DIMSE association was opened.
    AssociationOpened,
    /// A DIMSE association was closed.
    AssociationClosed,
    /// A query (C-FIND or QIDO-RS) was performed.
    QueryPerformed,
}

/// Plugin that provides DICOM transfer syntax codecs.
///
/// Used by WADO-RS frame retrieval and server-side transcoding. Each plugin
/// registers the transfer syntax UIDs it can decode/encode.
pub trait CodecPlugin: crate::Plugin {
    /// Transfer syntax UIDs this codec supports (e.g., `"1.2.840.10008.1.2.4.90"`).
    fn supported_transfer_syntaxes(&self) -> Vec<String>;

    /// Decode compressed pixel data to raw frames.
    fn decode(
        &self,
        data: &[u8],
        transfer_syntax_uid: &str,
    ) -> Result<Vec<Vec<u8>>, PluginError>;

    /// Encode raw frames to compressed pixel data.
    fn encode(
        &self,
        frames: &[Vec<u8>],
        transfer_syntax_uid: &str,
        rows: u16,
        cols: u16,
        bits_allocated: u16,
        samples_per_pixel: u16,
    ) -> Result<Vec<u8>, PluginError>;
}

/// Plugin that provides DICOM dataset processing (e.g., anonymization, tag modification).
pub trait ProcessingPlugin: crate::Plugin {
    /// A short identifier for this processor (e.g., `"anonymize"`, `"tag-morph"`).
    fn processor_id(&self) -> &str;

    /// Process a DICOM dataset in place.
    fn process(
        &self,
        dataset: &mut dicom_toolkit_data::DataSet,
        params: &serde_json::Value,
    ) -> Result<(), PluginError>;
}
```

### 3.4 Error Type

**New file:** `crates/pacs-plugin/src/error.rs`

```rust
//! Plugin-specific error types.

use thiserror::Error;

/// Errors that plugins can return during lifecycle operations.
#[derive(Debug, Error)]
pub enum PluginError {
    /// Plugin configuration is invalid or missing.
    #[error("plugin config error ({plugin_id}): {message}")]
    Config {
        plugin_id: String,
        message: String,
    },

    /// A required dependency plugin is not available.
    #[error("missing dependency: plugin '{plugin_id}' requires '{dependency}'")]
    MissingDependency {
        plugin_id: String,
        dependency: String,
    },

    /// Plugin initialization failed.
    #[error("plugin init failed ({plugin_id}): {source}")]
    InitFailed {
        plugin_id: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A circular dependency was detected in the plugin graph.
    #[error("circular dependency detected involving: {cycle}")]
    CircularDependency {
        cycle: String,
    },

    /// Multiple plugins provide the same singleton capability.
    #[error("duplicate {capability} provider: '{first}' and '{second}'")]
    DuplicateProvider {
        capability: String,
        first: String,
        second: String,
    },

    /// The plugin encountered a runtime error while handling an event.
    #[error("plugin runtime error ({plugin_id}): {message}")]
    Runtime {
        plugin_id: String,
        message: String,
    },

    /// Wraps a generic error source.
    #[error("plugin error ({plugin_id}): {source}")]
    Other {
        plugin_id: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}
```

### 3.5 Event Bus

**New file:** `crates/pacs-plugin/src/event.rs`

```rust
//! Event system for cross-plugin communication.

use std::net::SocketAddr;
use tokio::sync::broadcast;

/// Events emitted by the pacsnode core and delivered to [`EventPlugin`] subscribers.
#[derive(Debug, Clone)]
pub enum PacsEvent {
    /// A new DICOM instance was stored.
    InstanceStored {
        study_uid: String,
        series_uid: String,
        sop_instance_uid: String,
        sop_class_uid: String,
        calling_ae: String,
    },

    /// All instances for a study have been received (heuristic/timer-based).
    StudyComplete {
        study_uid: String,
        num_series: u32,
        num_instances: u32,
    },

    /// A resource was deleted via REST API.
    ResourceDeleted {
        level: ResourceLevel,
        uid: String,
        deleted_by: Option<String>,
    },

    /// A DIMSE association was opened.
    AssociationOpened {
        calling_ae: String,
        called_ae: String,
        peer_addr: SocketAddr,
    },

    /// A DIMSE association was closed.
    AssociationClosed {
        calling_ae: String,
        duration_ms: u64,
    },

    /// A query was performed (C-FIND or QIDO-RS).
    QueryPerformed {
        level: String,
        source: QuerySource,
        num_results: usize,
    },
}

/// Whether a query came from DIMSE or DICOMweb.
#[derive(Debug, Clone)]
pub enum QuerySource {
    Dimse { calling_ae: String },
    DICOMweb { remote_addr: Option<SocketAddr> },
}

/// The DICOM resource hierarchy level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceLevel {
    Patient,
    Study,
    Series,
    Instance,
}

/// Broadcast-based event bus.
///
/// Plugins subscribe via the broadcast receiver. The bus has a bounded
/// capacity; slow consumers miss events (logged as a warning).
///
/// # Example
///
/// ```rust,ignore
/// let bus = EventBus::new(1024);
/// let mut rx = bus.subscribe();
/// bus.emit(PacsEvent::InstanceStored { /* ... */ });
/// let event = rx.recv().await.unwrap();
/// ```
pub struct EventBus {
    tx: broadcast::Sender<PacsEvent>,
}

impl EventBus {
    /// Creates a new event bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Emit an event to all subscribers. Returns the number of receivers.
    pub fn emit(&self, event: PacsEvent) -> usize {
        // send() returns Err only if there are no receivers, which is fine
        self.tx.send(event).unwrap_or(0)
    }

    /// Subscribe to events. Returns a broadcast receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<PacsEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(4096)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn event_bus_roundtrip() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();

        bus.emit(PacsEvent::InstanceStored {
            study_uid: "1.2.3".into(),
            series_uid: "4.5.6".into(),
            sop_instance_uid: "7.8.9".into(),
            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".into(),
            calling_ae: "STORESCU".into(),
        });

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, PacsEvent::InstanceStored { .. }));
    }

    #[tokio::test]
    async fn event_bus_no_receivers_is_ok() {
        let bus = EventBus::new(16);
        let count = bus.emit(PacsEvent::StudyComplete {
            study_uid: "1.2.3".into(),
            num_series: 3,
            num_instances: 45,
        });
        assert_eq!(count, 0);
    }
}
```

### 3.6 Plugin Context

**New file:** `crates/pacs-plugin/src/context.rs`

```rust
//! Plugin context — shared state provided to plugins during lifecycle.

use std::sync::Arc;

use pacs_core::{BlobStore, MetadataStore};

use crate::event::EventBus;

/// Read-only server configuration exposed to plugins.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub ae_title: String,
    pub http_port: u16,
    pub dicom_port: u16,
    pub version: &'static str,
}

/// Context passed to [`Plugin::init`] and [`Plugin::start`].
///
/// Provides access to the plugin's configuration, shared services, and
/// the event bus for emitting events.
pub struct PluginContext {
    /// The plugin's config section from `[plugins.<id>]`. Empty table if
    /// no config is provided.
    pub config: toml::Value,

    /// The metadata store (available after the MetadataStorePlugin inits).
    /// Will be `None` during init of the MetadataStorePlugin itself.
    pub metadata_store: Option<Arc<dyn MetadataStore>>,

    /// The blob store (available after the BlobStorePlugin inits).
    /// Will be `None` during init of the BlobStorePlugin itself.
    pub blob_store: Option<Arc<dyn BlobStore>>,

    /// Static server identity information.
    pub server_info: ServerInfo,

    /// Event bus for publishing / subscribing to system events.
    pub event_bus: Arc<EventBus>,
}
```

### 3.7 Plugin Registry

**New file:** `crates/pacs-plugin/src/registry.rs`

This is the most complex component. Key responsibilities:

1. **Registration:** Accept `Box<dyn Plugin>` instances
2. **Dependency resolution:** Topological sort on `manifest().dependencies`
3. **Singleton enforcement:** Only one `MetadataStorePlugin`, one `BlobStorePlugin`
4. **Init lifecycle:** Call `init()` in order, then `start()` on all
5. **Shutdown:** Call `shutdown()` in reverse order
6. **Route merging:** Collect `RoutePlugin::routes()` into single `Router`
7. **Middleware ordering:** Apply `MiddlewarePlugin`s by priority
8. **Event dispatch:** Route events to subscribed `EventPlugin`s
9. **Codec lookup:** Map transfer syntax UIDs to `CodecPlugin`s
10. **Health aggregation:** Collect `health()` from all plugins

```rust
//! Plugin registry — manages plugin lifecycle and provides service lookup.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::capabilities::*;
use crate::context::{PluginContext, ServerInfo};
use crate::error::PluginError;
use crate::event::{EventBus, PacsEvent};
use crate::plugin::{Plugin, PluginHealth, PluginManifest};

use pacs_core::{BlobStore, MetadataStore};

/// Factory function type for creating plugin instances.
pub struct PluginRegistration {
    pub create: fn() -> Box<dyn Plugin>,
}

// Enables compile-time registration via `inventory::submit!`
inventory::collect!(PluginRegistration);

/// Central plugin manager.
pub struct PluginRegistry {
    /// All registered plugins, stored in dependency-resolved order after init.
    plugins: Vec<Box<dyn Plugin>>,

    /// Quick lookup: plugin_id → index in `plugins`
    index: HashMap<String, usize>,

    /// The active metadata store (from whichever MetadataStorePlugin is enabled).
    metadata_store: Option<Arc<dyn MetadataStore>>,

    /// The active blob store.
    blob_store: Option<Arc<dyn BlobStore>>,

    /// Routes contributed by RoutePlugins.
    plugin_routes: Vec<Router>,

    /// Event subscriptions: EventKind → list of plugin indices.
    event_subs: HashMap<EventKind, Vec<usize>>,

    /// Codec lookup: transfer_syntax_uid → plugin index.
    codecs: HashMap<String, usize>,

    /// Shared event bus.
    event_bus: Arc<EventBus>,

    /// Enabled plugin IDs from config. If None, all registered plugins are enabled.
    enabled_ids: Option<Vec<String>>,
}

impl PluginRegistry {
    /// Create a new empty registry.
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            plugins: Vec::new(),
            index: HashMap::new(),
            metadata_store: None,
            blob_store: None,
            plugin_routes: Vec::new(),
            event_subs: HashMap::new(),
            codecs: HashMap::new(),
            event_bus,
            enabled_ids: None,
        }
    }

    /// Set the list of enabled plugin IDs from configuration.
    pub fn set_enabled(&mut self, ids: Vec<String>) {
        self.enabled_ids = Some(ids);
    }

    /// Register a plugin instance. Does not call init yet.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        let manifest = plugin.manifest();
        let id = manifest.id.clone();

        // Skip if not in the enabled list
        if let Some(ref enabled) = self.enabled_ids {
            if !enabled.contains(&id) {
                debug!(plugin_id = %id, "Plugin not in enabled list, skipping");
                return;
            }
        }

        info!(plugin_id = %id, version = %manifest.version, "Registering plugin");
        let idx = self.plugins.len();
        self.index.insert(id, idx);
        self.plugins.push(plugin);
    }

    /// Auto-discover and register all compile-time registered plugins.
    pub fn register_all_discovered(&mut self) {
        for reg in inventory::iter::<PluginRegistration> {
            let plugin = (reg.create)();
            self.register(plugin);
        }
    }

    /// Initialize all plugins in dependency order.
    ///
    /// # Errors
    ///
    /// Returns `PluginError::CircularDependency` if a cycle is detected.
    /// Returns `PluginError::MissingDependency` if a required plugin is absent.
    /// Returns `PluginError::DuplicateProvider` if multiple singleton plugins exist.
    pub async fn init_all(&mut self, server_info: ServerInfo, plugin_configs: &HashMap<String, toml::Value>) -> Result<(), PluginError> {
        let order = self.resolve_dependency_order()?;

        for idx in &order {
            let plugin = &mut self.plugins[*idx];
            let manifest = plugin.manifest();
            let config = plugin_configs
                .get(&manifest.id)
                .cloned()
                .unwrap_or(toml::Value::Table(toml::map::Map::new()));

            let ctx = PluginContext {
                config,
                metadata_store: self.metadata_store.clone(),
                blob_store: self.blob_store.clone(),
                server_info: server_info.clone(),
                event_bus: Arc::clone(&self.event_bus),
            };

            info!(plugin_id = %manifest.id, "Initializing plugin");
            plugin.init(&ctx).await?;

            // After init, extract capabilities from this plugin.
            // (capability detection uses Any-based downcasting or a
            // separate registration mechanism — see implementation note below)
        }

        // Call start() on all plugins
        for idx in &order {
            let plugin = &self.plugins[*idx];
            let manifest = plugin.manifest();
            let config = plugin_configs
                .get(&manifest.id)
                .cloned()
                .unwrap_or(toml::Value::Table(toml::map::Map::new()));

            let ctx = PluginContext {
                config,
                metadata_store: self.metadata_store.clone(),
                blob_store: self.blob_store.clone(),
                server_info: server_info.clone(),
                event_bus: Arc::clone(&self.event_bus),
            };
            plugin.start(&ctx).await?;
        }

        Ok(())
    }

    /// Shutdown all plugins in reverse dependency order.
    pub async fn shutdown_all(&mut self) -> Result<(), PluginError> {
        let order = self.resolve_dependency_order().unwrap_or_default();
        for idx in order.iter().rev() {
            let plugin = &self.plugins[*idx];
            let manifest = plugin.manifest();
            info!(plugin_id = %manifest.id, "Shutting down plugin");
            if let Err(e) = plugin.shutdown().await {
                error!(plugin_id = %manifest.id, error = %e, "Plugin shutdown error");
            }
        }
        Ok(())
    }

    /// Returns the active metadata store, if any MetadataStorePlugin is loaded.
    pub fn metadata_store(&self) -> Option<Arc<dyn MetadataStore>> {
        self.metadata_store.clone()
    }

    /// Returns the active blob store, if any BlobStorePlugin is loaded.
    pub fn blob_store(&self) -> Option<Arc<dyn BlobStore>> {
        self.blob_store.clone()
    }

    /// Returns a merged Axum router from all RoutePlugins.
    pub fn merged_routes(&self) -> Router {
        let mut router = Router::new();
        for r in &self.plugin_routes {
            router = router.merge(r.clone());
        }
        router
    }

    /// Emit an event to all subscribed EventPlugins.
    pub async fn emit_event(&self, event: &PacsEvent) {
        self.event_bus.emit(event.clone());
    }

    /// Aggregate health from all plugins.
    pub async fn aggregate_health(&self) -> Vec<(String, PluginHealth)> {
        let mut results = Vec::new();
        for plugin in &self.plugins {
            let manifest = plugin.manifest();
            let health = plugin.health().await;
            results.push((manifest.id, health));
        }
        results
    }

    /// Topological sort of plugins by their dependency graph.
    fn resolve_dependency_order(&self) -> Result<Vec<usize>, PluginError> {
        // Kahn's algorithm for topological sort
        let n = self.plugins.len();
        let mut in_degree = vec![0usize; n];
        let mut adj: Vec<Vec<usize>> = vec![vec![]; n];

        for (idx, plugin) in self.plugins.iter().enumerate() {
            let manifest = plugin.manifest();
            for dep in &manifest.dependencies {
                let dep_idx = self.index.get(dep).ok_or_else(|| {
                    PluginError::MissingDependency {
                        plugin_id: manifest.id.clone(),
                        dependency: dep.clone(),
                    }
                })?;
                adj[*dep_idx].push(idx);
                in_degree[idx] += 1;
            }
        }

        let mut queue: Vec<usize> = (0..n).filter(|i| in_degree[*i] == 0).collect();
        let mut order = Vec::with_capacity(n);

        while let Some(node) = queue.pop() {
            order.push(node);
            for &next in &adj[node] {
                in_degree[next] -= 1;
                if in_degree[next] == 0 {
                    queue.push(next);
                }
            }
        }

        if order.len() != n {
            let cycle_members: Vec<String> = (0..n)
                .filter(|i| in_degree[*i] > 0)
                .map(|i| self.plugins[i].manifest().id)
                .collect();
            return Err(PluginError::CircularDependency {
                cycle: cycle_members.join(" → "),
            });
        }

        Ok(order)
    }
}
```

**Implementation note — capability detection:**

Since Rust doesn't have trait upcasting stabilized for all cases, use an explicit
capability declaration approach. Each plugin wrapper struct implements a
`PluginCapabilities` helper:

```rust
/// Plugins call these methods on the registry to declare their capabilities
/// after init succeeds. This avoids the need for trait upcasting.
impl PluginRegistry {
    pub fn provide_metadata_store(&mut self, plugin_id: &str, store: Arc<dyn MetadataStore>) -> Result<(), PluginError> {
        if let Some(ref existing) = self.metadata_store {
            // Find existing provider ID for error message
            return Err(PluginError::DuplicateProvider {
                capability: "MetadataStore".into(),
                first: "existing".into(),
                second: plugin_id.into(),
            });
        }
        self.metadata_store = Some(store);
        Ok(())
    }

    pub fn provide_blob_store(&mut self, plugin_id: &str, store: Arc<dyn BlobStore>) -> Result<(), PluginError> {
        if self.blob_store.is_some() {
            return Err(PluginError::DuplicateProvider {
                capability: "BlobStore".into(),
                first: "existing".into(),
                second: plugin_id.into(),
            });
        }
        self.blob_store = Some(store);
        Ok(())
    }

    pub fn provide_routes(&mut self, routes: Router) {
        self.plugin_routes.push(routes);
    }

    pub fn subscribe_events(&mut self, plugin_idx: usize, kinds: Vec<EventKind>) {
        for kind in kinds {
            self.event_subs.entry(kind).or_default().push(plugin_idx);
        }
    }

    pub fn provide_codec(&mut self, plugin_idx: usize, syntaxes: Vec<String>) {
        for syntax in syntaxes {
            self.codecs.insert(syntax, plugin_idx);
        }
    }
}
```

### 3.8 Registration Macro

**New file:** `crates/pacs-plugin/src/macros.rs`

```rust
//! Convenience macro for compile-time plugin registration.

/// Register a plugin factory with the global plugin registry.
///
/// Call this once per plugin crate, at module scope:
///
/// ```rust,ignore
/// use pacs_plugin::register_plugin;
///
/// register_plugin!(MyPlugin::default);
/// ```
///
/// The factory function must return `Box<dyn Plugin>`. It is called once
/// during `PluginRegistry::register_all_discovered()`.
#[macro_export]
macro_rules! register_plugin {
    ($factory:expr) => {
        $crate::inventory::submit! {
            $crate::PluginRegistration {
                create: || Box::new($factory()),
            }
        }
    };
}
```

### 3.9 Crate lib.rs

**New file:** `crates/pacs-plugin/src/lib.rs`

```rust
//! pacsnode — plugin system traits, registry, and event bus.
//!
//! ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for
//! diagnostic or therapeutic purposes.
//!
//! This crate provides the extension framework for pacsnode. Plugins are
//! Rust crates that implement [`Plugin`] plus one or more capability traits
//! ([`MetadataStorePlugin`], [`BlobStorePlugin`], [`RoutePlugin`], etc.).
//!
//! # Architecture
//!
//! Plugins are compiled into the pacsnode binary and registered at compile
//! time via the [`register_plugin!`] macro (backed by the `inventory` crate).
//! The [`PluginRegistry`] manages lifecycle: dependency resolution,
//! initialization, event dispatch, and graceful shutdown.

pub mod capabilities;
pub mod context;
pub mod error;
pub mod event;
pub mod macros;
pub mod plugin;
pub mod registry;

pub use capabilities::*;
pub use context::PluginContext;
pub use error::PluginError;
pub use event::{EventBus, PacsEvent, QuerySource, ResourceLevel};
pub use plugin::{Plugin, PluginHealth, PluginManifest};
pub use registry::{PluginRegistration, PluginRegistry};

// Re-export inventory for use in register_plugin! macro
pub use inventory;
```

---

## 4. Phase 2 — Wrap Existing Code as Plugins

### 4.1 PgMetadataStorePlugin

**File:** `crates/pacs-store/src/plugin.rs` (new)

```rust
use std::sync::Arc;

use async_trait::async_trait;
use pacs_core::MetadataStore;
use pacs_plugin::{Plugin, PluginContext, PluginError, PluginHealth, PluginManifest};
use sqlx::postgres::PgPoolOptions;

use crate::PgMetadataStore;

pub struct PgMetadataStorePlugin {
    store: Option<Arc<PgMetadataStore>>,
}

impl PgMetadataStorePlugin {
    pub fn new() -> Self {
        Self { store: None }
    }
}

impl Default for PgMetadataStorePlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for PgMetadataStorePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "pg-metadata-store".into(),
            name: "PostgreSQL Metadata Store".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            dependencies: vec![],
        }
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        // Read database URL from plugin config or fall back to the
        // standard PACS_DATABASE__URL env var.
        let url = ctx.config.get("url")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| PluginError::Config {
                plugin_id: "pg-metadata-store".into(),
                message: "missing 'url' in plugin config or [database] section".into(),
            })?;

        let max_conns = ctx.config.get("max_connections")
            .and_then(|v| v.as_integer())
            .unwrap_or(20) as u32;

        let pool = PgPoolOptions::new()
            .max_connections(max_conns)
            .connect(&url)
            .await
            .map_err(|e| PluginError::InitFailed {
                plugin_id: "pg-metadata-store".into(),
                source: Box::new(e),
            })?;

        self.store = Some(Arc::new(PgMetadataStore::new(pool)));
        Ok(())
    }

    async fn health(&self) -> PluginHealth {
        // Could ping the DB here
        if self.store.is_some() {
            PluginHealth::Healthy
        } else {
            PluginHealth::Unhealthy("not initialized".into())
        }
    }
}

impl pacs_plugin::MetadataStorePlugin for PgMetadataStorePlugin {
    fn metadata_store(&self) -> Arc<dyn MetadataStore> {
        self.store.clone().expect("plugin not initialized")
    }
}

// Register with the global plugin registry
pacs_plugin::register_plugin!(PgMetadataStorePlugin::default);
```

### 4.2 S3BlobStorePlugin

**File:** `crates/pacs-storage/src/plugin.rs` (new)

Same pattern as above but for `S3BlobStore`. Reads `endpoint`, `bucket`,
`access_key`, `secret_key`, `region` from `ctx.config`.

### 4.3 Refactor main.rs

**File:** `crates/pacs-server/src/main.rs`

The key change is replacing direct service construction with registry-based wiring:

```rust
// BEFORE (current, ~lines 49-76):
let blob_store: Arc<dyn BlobStore> = Arc::new(S3BlobStore::new(&storage_config)?);
let meta_store: Arc<dyn MetadataStore> = Arc::new(PgMetadataStore::new(pool));
let app_state = AppState { server_info, store: meta_store.clone(), blobs: blob_store.clone() };
let router = build_router(app_state);

// AFTER (with plugin system):
let event_bus = Arc::new(EventBus::new(4096));
let mut registry = PluginRegistry::new(Arc::clone(&event_bus));

// Set enabled list from config
if let Some(enabled) = cfg.plugins.as_ref().and_then(|p| p.enabled.as_ref()) {
    registry.set_enabled(enabled.clone());
}

// Auto-discover compiled-in plugins
registry.register_all_discovered();

// Initialize all plugins
let server_info = pacs_plugin::context::ServerInfo {
    ae_title: cfg.server.ae_title.clone(),
    http_port: cfg.server.http_port,
    dicom_port: cfg.server.dicom_port,
    version: env!("CARGO_PKG_VERSION"),
};
let plugin_configs = cfg.plugins.as_ref()
    .map(|p| p.configs.clone())
    .unwrap_or_default();
registry.init_all(server_info, &plugin_configs).await?;

// Extract services from registry
let meta_store = registry.metadata_store()
    .context("no MetadataStorePlugin loaded")?;
let blob_store = registry.blob_store()
    .context("no BlobStorePlugin loaded")?;

// Build router with plugin routes merged in
let app_state = AppState {
    server_info: /* ... */,
    store: meta_store.clone(),
    blobs: blob_store.clone(),
    event_bus: Arc::clone(&event_bus),
};
let mut router = build_router(app_state);
router = router.merge(registry.merged_routes());
// Apply middleware plugins (sorted by priority)

// ... rest of startup unchanged ...

// On shutdown:
registry.shutdown_all().await?;
```

### 4.4 Update AppState

**File:** `crates/pacs-api/src/state.rs`

Add `event_bus` field:

```rust
#[derive(Clone)]
pub struct AppState {
    pub server_info: ServerInfo,
    pub store: Arc<dyn MetadataStore>,
    pub blobs: Arc<dyn BlobStore>,
    pub event_bus: Arc<pacs_plugin::EventBus>,  // NEW
}
```

### 4.5 Emit Events from Existing Code

**File:** `crates/pacs-dimse/src/server/provider.rs`

At the end of `PacsStoreProvider::on_store()` (after line 202), emit:

```rust
// After successful store, emit event
if let Some(bus) = self.event_bus.as_ref() {
    bus.emit(PacsEvent::InstanceStored {
        study_uid: study_uid_str.clone(),
        series_uid: series_uid_str.clone(),
        sop_instance_uid: sop_instance_uid_str.clone(),
        sop_class_uid: event.sop_class_uid.clone(),
        calling_ae: event.calling_ae.clone(),
    });
}
```

Add `event_bus: Option<Arc<EventBus>>` to `PacsStoreProvider` and `PacsQueryProvider`.

Similarly, emit `ResourceDeleted` in the REST delete handlers
(`crates/pacs-api/src/routes/rest/studies.rs`, `series.rs`, `instances.rs`).

### 4.6 Add `[plugins]` Config Section

**File:** `crates/pacs-server/src/config.rs`

```rust
/// Plugin system configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PluginsConfig {
    /// List of plugin IDs to enable. If absent, all compiled-in plugins activate.
    pub enabled: Option<Vec<String>>,
    /// Per-plugin configuration sections, keyed by plugin ID.
    #[serde(flatten)]
    pub configs: HashMap<String, toml::Value>,
}

// Add to AppConfig:
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub storage: StorageConfig,
    pub logging: LoggingConfig,
    pub plugins: Option<PluginsConfig>,  // NEW
}
```

---

## 5. Phase 3 — First Optional Plugins

### 5.1 plugin-auth

**New crate:** `plugins/plugin-auth/`

Implements `MiddlewarePlugin` with priority 0 (outermost). Provides:
- JWT token validation via `jsonwebtoken` (already in workspace deps)
- Password hashing via `argon2` (already in workspace deps)
- Login endpoint: `POST /auth/login`
- Token refresh: `POST /auth/refresh`
- Middleware that validates `Authorization: Bearer <token>` on all routes
  except `/health`, `/auth/login`, and configurable public paths.

Dependencies: `pacs-plugin`, `jsonwebtoken`, `argon2`, `axum`, `tower`

### 5.2 plugin-audit

**New crate:** `plugins/plugin-audit/`

Implements `EventPlugin`. Subscribes to all event kinds. Writes to the
existing `audit_log` table (migration `0003_audit_log.sql`). Maps:
- `InstanceStored` → `INSERT INTO audit_log (action, ...)`
- `ResourceDeleted` → `INSERT INTO audit_log (action, ...)`
- `QueryPerformed` → `INSERT INTO audit_log (action, ...)`

Depends on: `pg-metadata-store` (needs DB access via `PluginContext`)

### 5.3 plugin-metrics

**New crate:** `plugins/plugin-metrics/`

Implements `RoutePlugin` + `EventPlugin`. Adds `/metrics` endpoint returning
Prometheus exposition format. Tracks:
- `pacsnode_instances_stored_total` (counter)
- `pacsnode_queries_total` (counter, labels: level, source)
- `pacsnode_associations_active` (gauge)
- `pacsnode_http_request_duration_seconds` (histogram, via middleware)

Dependencies: `metrics`, `metrics-exporter-prometheus`

---

## 6. Phase 4 — Content Plugins

### 6.1 plugin-jp2k

Wire `dicom_toolkit_codec::Jp2kCodec` as a `CodecPlugin`. Supports:
- `1.2.840.10008.1.2.4.90` (JPEG 2000 Lossless)
- `1.2.840.10008.1.2.4.91` (JPEG 2000 Lossy)

### 6.2 plugin-anonymize

`ProcessingPlugin` implementing DICOM PS3.15 Annex E de-identification profiles:
- Basic Profile
- Retain Longitudinal Temporal Information
- Clean Pixel Data Option

### 6.3 plugin-forward

`EventPlugin` subscribing to `InstanceStored`. Rule engine:
```toml
[plugins.auto-forward]
rules = [
    { modality = "CT", destination = "AI_SERVER" },
    { study_description = "*CHEST*", destination = "LUNG_CAD" },
]
```

### 6.4 plugin-export

`RoutePlugin` adding:
- `GET /api/studies/{uid}/archive` → ZIP with DICOMDIR
- `GET /api/series/{uid}/archive` → ZIP

---

## 7. Phase 5 — Alternative Backends

### 7.1 plugin-fs

`BlobStorePlugin` storing DICOM files on local filesystem.
Path layout: `{base_dir}/{study_uid}/{series_uid}/{instance_uid}.dcm`

For single-machine deployments without S3 infrastructure.

### 7.2 plugin-sqlite

`MetadataStorePlugin` backed by SQLite via `sqlx`.
For development, testing, and small-site deployments.

---

## 8. Built-in vs Plugin Classification

### Always Built-In (core crate code)

| Component | Crate | Rationale |
|-----------|-------|-----------|
| DIMSE TCP/PDU engine | pacs-dimse | Protocol foundation |
| Association negotiation | pacs-dimse | Protocol foundation |
| DICOMweb route definitions | pacs-api | Core API contract |
| REST route definitions | pacs-api | Core API contract |
| Plugin trait + registry | pacs-plugin | Meta: plugins can't load themselves |
| Event bus | pacs-plugin | Infrastructure for plugin communication |
| Config system | pacs-server | Must exist before plugins init |
| Domain types (Study, Series, etc.) | pacs-core | Shared vocabulary |
| Error types | pacs-core | Shared vocabulary |
| DICOM tag dictionary | pacs-dicom | Static reference data |
| Tracing setup | pacs-server | Cross-cutting, all code needs it |
| Graceful shutdown | pacs-server | Coordinates plugin shutdown |
| Health endpoint | pacs-api | Always available for orchestrators |

### Default Plugins (compiled in, replaceable)

| Plugin | Replaces | Alternative |
|--------|----------|-------------|
| `pg-metadata-store` | Direct `PgMetadataStore::new()` | `sqlite-store` |
| `s3-blob-store` | Direct `S3BlobStore::new()` | `fs-store` |
| `pacs-store-scp` | Direct `PacsStoreProvider::new()` | Custom store handler |
| `pacs-query-scp` | Direct `PacsQueryProvider::new()` | Custom query handler |

### Optional (feature flag gated)

See [Phase 3–5](#5-phase-3--first-optional-plugins) above. Feature flags in
workspace `Cargo.toml`:

```toml
[workspace.features]
default    = ["pg-store", "s3-store"]
pg-store   = ["dep:pacs-store"]
s3-store   = ["dep:pacs-storage"]
auth       = ["dep:plugin-auth"]
audit      = ["dep:plugin-audit"]
metrics    = ["dep:plugin-metrics"]
viewer     = ["dep:plugin-ohif"]
codec-jp2k = ["dep:plugin-jp2k"]
full       = ["auth", "audit", "metrics", "viewer", "codec-jp2k"]
```

---

## 9. Testing Strategy

### Unit Tests (per plugin crate)

Each plugin crate has `#[cfg(test)] mod tests` covering:
- `manifest()` returns expected id/version/deps
- `init()` succeeds with valid config
- `init()` returns `PluginError::Config` with invalid/missing config
- `health()` returns `Healthy` after init, `Unhealthy` before
- Capability methods return expected values after init

### Registry Tests (pacs-plugin crate)

```rust
#[cfg(test)]
mod tests {
    // Test dependency resolution
    fn test_dependency_order_simple();
    fn test_dependency_order_diamond();
    fn test_circular_dependency_detected();
    fn test_missing_dependency_detected();

    // Test singleton enforcement
    fn test_duplicate_metadata_store_rejected();
    fn test_duplicate_blob_store_rejected();

    // Test lifecycle
    async fn test_init_called_in_order();
    async fn test_shutdown_called_in_reverse_order();
    async fn test_start_called_after_all_inits();

    // Test event dispatch
    async fn test_event_delivered_to_subscribers();
    async fn test_event_not_delivered_to_non_subscribers();

    // Test route merging
    fn test_routes_merged_from_plugins();

    // Test health aggregation
    async fn test_health_aggregates_all_plugins();
}
```

### Integration Tests

After Phase 2 refactor, ALL existing tests must pass unchanged:
```bash
cargo test --workspace --all-targets   # 253+ tests
sh scripts/smoke-test.sh               # 24 checks
```

### Plugin-Specific Integration Tests

Each plugin crate should have integration tests in `tests/`:
- `plugin-auth`: Test auth middleware blocks unauthenticated, passes authenticated
- `plugin-audit`: Test events trigger audit log inserts (testcontainers Postgres)
- `plugin-metrics`: Test `/metrics` endpoint returns Prometheus format

---

## 10. Configuration Schema

### Complete Example config.toml

```toml
[server]
http_port       = 8042
dicom_port      = 4242
ae_title        = "PACSNODE"
max_associations = 64
dimse_timeout_secs = 30

[database]
url             = "postgres://pacsnode:secret@localhost/pacsnode"
max_connections = 20
run_migrations  = true

[storage]
endpoint   = "http://localhost:9000"
bucket     = "dicom"
access_key = "minio_user"
secret_key = "minio_pass"
region     = "us-east-1"

[logging]
level  = "info"
format = "json"

# ── Plugin Configuration ──────────────────────────────────────────

[plugins]
# If specified, only these plugins are activated.
# If omitted, all compiled-in plugins are activated.
enabled = [
    "pg-metadata-store",
    "s3-blob-store",
    "basic-auth",
    "audit-logger",
    "prometheus-metrics",
]

# Per-plugin config goes in [plugins.<id>] sections.
# These are passed to the plugin's init() as ctx.config.

[plugins.pg-metadata-store]
# Override database settings (defaults to [database] section values)
# url = "postgres://..."
# max_connections = 20

[plugins.s3-blob-store]
# Override storage settings (defaults to [storage] section values)
# endpoint = "http://..."

[plugins.basic-auth]
jwt_secret     = "${PACS_JWT_SECRET}"
token_ttl      = "8h"
refresh_ttl    = "30d"
public_paths   = ["/health", "/metrics"]
# admin bootstrap credentials (first run only)
admin_username = "admin"
admin_password = "${PACS_ADMIN_PASSWORD}"

[plugins.audit-logger]
# No additional config needed — uses the metadata store's DB connection

[plugins.prometheus-metrics]
endpoint = "/metrics"

[plugins.ohif-viewer]
static_dir   = "/opt/pacsnode/viewer"
route_prefix = "/viewer"

[plugins.auto-forward]
[[plugins.auto-forward.rules]]
modality    = "CT"
destination = "AI_SERVER"

[[plugins.auto-forward.rules]]
modality    = "MR"
destination = "AI_SERVER"
```

---

## 11. Migration Path

### Backward Compatibility

The refactor MUST NOT change any external behavior:
- All HTTP endpoints remain identical
- All DIMSE behavior remains identical
- Config format is additive (old configs work; `[plugins]` is optional)
- Docker image works without config changes (defaults activate pg + s3 plugins)

### Step-by-Step Migration

1. Create `pacs-plugin` crate with all traits/registry/eventbus
2. Add `pacs-plugin` as dependency to `pacs-store`, `pacs-storage`, `pacs-dimse`, `pacs-api`, `pacs-server`
3. Create plugin wrapper in each crate (`plugin.rs` module)
4. Update `main.rs` to use `PluginRegistry`
5. Run full test suite — must be green
6. Add feature flags for optional plugins
7. Create first optional plugin crates

### Rollback Plan

If Phase 2 causes issues, the `main.rs` can always bypass the registry
and wire services directly (as it does today). The plugin trait definitions
don't affect existing code.

---

## Appendix: Dependency Graph After Plugin System

```
pacs-core (domain types, traits)
    ↑
pacs-plugin (Plugin trait, registry, event bus)
    ↑                    ↑                ↑
pacs-store           pacs-storage      pacs-dimse        pacs-api
(PgMetadata +        (S3Blob +         (DIMSE +          (router +
 plugin wrapper)      plugin wrapper)   providers)        handlers)
    ↑                    ↑                ↑                ↑
    └────────────────────┴────────────────┴────────────────┘
                                |
                          pacs-server
                         (main.rs wiring)
                                |
            ┌───────────────────┼──────────────────┐
            ↓                   ↓                  ↓
      plugin-auth         plugin-audit       plugin-metrics
      (optional)          (optional)         (optional)
```
