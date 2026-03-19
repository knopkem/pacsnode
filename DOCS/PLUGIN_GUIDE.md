# pacsnode Plugin System Guide

pacsnode uses a **compile-time, trait-based plugin system** inspired by Caddy's module
architecture. Plugins are ordinary Rust crates that implement a set of traits, are
compiled into the final binary, and are automatically discovered at startup via the
[`inventory`](https://docs.rs/inventory) crate. There is no dynamic loading, no FFI
boundary, and no runtime ABI concerns — you get full Rust type safety and zero-cost
abstractions.

This guide explains the architecture, walks through creating new plugins step by step,
and documents every trait and type you need to know.

---

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Built-in vs Optional Plugins](#built-in-vs-optional-plugins)
3. [Plugin Lifecycle](#plugin-lifecycle)
4. [Creating a New Plugin](#creating-a-new-plugin)
5. [The Plugin Trait](#the-plugin-trait)
6. [Plugin Manifest](#plugin-manifest)
7. [Capability Traits](#capability-traits)
   - [MetadataStorePlugin](#metadatastoreplugin)
   - [BlobStorePlugin](#blobstoreplugin)
   - [RoutePlugin](#routeplugin)
   - [MiddlewarePlugin](#middlewareplugin)
   - [EventPlugin](#eventplugin)
   - [CodecPlugin](#codecplugin)
   - [ProcessingPlugin](#processingplugin)
   - [DIMSE SCP Plugins](#dimse-scp-plugins)
8. [Event System](#event-system)
9. [Plugin Context](#plugin-context)
10. [Configuration](#configuration)
11. [Error Handling](#error-handling)
12. [Testing Plugins](#testing-plugins)
13. [Reference: Existing Plugins](#reference-existing-plugins)

---

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────────┐
│                        pacsnode binary                           │
│                                                                  │
│  ┌────────────┐   ┌──────────┐   ┌────────────┐                │
│  │ pacs-core  │   │ pacs-api │   │ pacs-dimse │                │
│  │ (traits)   │   │ (axum)   │   │ (DIMSE)    │                │
│  └─────┬──────┘   └────┬─────┘   └─────┬──────┘                │
│        │               │               │                        │
│  ┌─────▼───────────────▼───────────────▼──────┐                │
│  │           Plugin Host  (pacs-plugin)        │                │
│  │                                             │                │
│  │  PluginRegistry                             │                │
│  │  ├─ MetadataStorePlugin  (singleton)        │                │
│  │  ├─ BlobStorePlugin      (singleton)        │                │
│  │  ├─ StoreScpPlugin       (singleton)        │                │
│  │  ├─ FindScpPlugin        (singleton)        │                │
│  │  ├─ RoutePlugin(s)       (merged)           │                │
│  │  ├─ MiddlewarePlugin(s)  (layered)          │                │
│  │  ├─ EventPlugin(s)       (fan-out)          │                │
│  │  ├─ CodecPlugin(s)       (by syntax UID)    │                │
│  │  └─ ProcessingPlugin(s)  (by processor ID)  │                │
│  │                                             │                │
│  │  EventBus  ←───── broadcast channel ──────► │                │
│  └─────────────────────────────────────────────┘                │
│                                                                  │
│  Built-in (always active):      Optional (enable in config):    │
│  ├─ pg-metadata-store           ├─ basic-auth                   │
│  ├─ s3-blob-store               ├─ audit-logger                 │
│  ├─ pacs-store-scp              └─ prometheus-metrics            │
│  └─ pacs-query-scp                                              │
└──────────────────────────────────────────────────────────────────┘
```

The core idea:

- Every plugin is a Rust struct that implements the `Plugin` trait.
- Each plugin declares a **manifest** (ID, name, version, dependencies, default-enabled flag).
- Plugins optionally implement one or more **capability traits** to provide routes,
  middleware, event handling, storage backends, DIMSE handlers, codecs, or processing
  pipelines.
- The `PluginRegistry` collects all plugins, resolves dependencies, initialises them
  in topological order, and wires their capabilities into the running server.
- An `EventBus` (backed by `tokio::sync::broadcast`) lets plugins react to DICOM
  events without tight coupling.

### Why Compile-Time?

Medical imaging software prioritises **correctness and safety** over hot-reload
convenience. Compile-time plugins give you:

- **Full type safety** — no FFI boundaries, no ABI mismatches.
- **Zero-cost abstractions** — trait dispatch is monomorphised or thin vtable calls.
- **Normal debugging** — standard Rust backtraces, no dlopen headaches.
- **Security** — the Rust compiler enforces memory safety across plugin boundaries.
- **Simplicity** — ~10 traits to learn, no runtime plugin loaders.

The trade-off is that adding a plugin requires a recompile. In practice, PACS servers
are updated during maintenance windows, making this an acceptable constraint.

---

## Built-in vs Optional Plugins

### Built-in Plugins

These are compiled in and **enabled by default**. They activate automatically unless
you explicitly replace them (e.g. with an alternative storage backend).

| Plugin ID | Crate | Provides |
|-----------|-------|----------|
| `pg-metadata-store` | `pacs-store` | PostgreSQL metadata storage |
| `sqlite-metadata-store` | `pacs-sqlite-store` | SQLite metadata storage |
| `s3-blob-store` | `pacs-storage` | S3/MinIO blob storage |
| `filesystem-blob-store` | `pacs-fs-storage` | Filesystem blob storage |
| `pacs-store-scp` | `pacs-dimse` | C-STORE SCP handler |
| `pacs-query-scp` | `pacs-dimse` | C-FIND / C-GET / C-MOVE SCP handlers |

### Optional Plugins

These are compiled in but **disabled by default**. Activate them by adding their ID
to `plugins.enabled` in `config.toml`:

| Plugin ID | Crate | Provides |
|-----------|-------|----------|
| `basic-auth` | `pacs-auth-plugin` | JWT authentication + login endpoints |
| `audit-logger` | `pacs-audit-plugin` | Audit trail to the active metadata store |
| `prometheus-metrics` | `pacs-metrics-plugin` | Prometheus `/metrics` endpoint + HTTP latency tracking |

```toml
# config.toml
[plugins]
enabled = ["basic-auth", "audit-logger", "prometheus-metrics"]
```

---

## Plugin Lifecycle

Every plugin goes through a well-defined lifecycle, managed by the `PluginRegistry`:

```
     ┌──────────┐
     │ Discover │  inventory::iter collects all register_plugin!() entries
     └────┬─────┘
          │
     ┌────▼─────┐
     │ Register │  registry.register() — validate ID uniqueness, record capabilities
     └────┬─────┘
          │
     ┌────▼─────────────┐
     │ Dependency Sort  │  Topological sort ensures init order respects dependencies
     └────┬─────────────┘
          │
     ┌────▼─────┐
     │   Init   │  plugin.init(ctx) — parse config, connect to services, build state
     └────┬─────┘
          │
     ┌────▼─────┐
     │  Start   │  plugin.start(ctx) — begin background work (optional)
     └────┬─────┘
          │
     ┌────▼────────────┐
     │  Running        │  Capabilities active, events flowing, health checks
     │  (server loop)  │
     └────┬────────────┘
          │  (graceful shutdown signal)
     ┌────▼──────┐
     │ Shutdown  │  plugin.shutdown() — release resources, in reverse init order
     └───────────┘
```

**Key points:**

- `init()` is called in dependency order. If plugin B depends on plugin A, A is
  initialised first and its capabilities (e.g. `metadata_store()`) are available in
  B's `PluginContext`.
- `shutdown()` is called in **reverse** order, so dependents shut down before their
  dependencies.
- The registry enforces **singleton constraints** for storage backends and DIMSE
  handlers — at most one `MetadataStorePlugin`, one `BlobStorePlugin`, etc.

---

## Creating a New Plugin

This section walks through building a plugin from scratch. We'll create an example
"webhook notifier" plugin that sends HTTP POST notifications when DICOM instances
are stored.

### Step 1: Create the Crate

```bash
cargo new --lib crates/pacs-webhook-plugin
```

Add it to the workspace in the root `Cargo.toml`:

```toml
[workspace]
members = [
    # ... existing members ...
    "crates/pacs-webhook-plugin",
]
```

### Step 2: Add Dependencies

In `crates/pacs-webhook-plugin/Cargo.toml`:

```toml
[package]
name = "pacs-webhook-plugin"
version = "0.1.0"
edition = "2021"

[dependencies]
pacs-plugin = { path = "../pacs-plugin" }
async-trait = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
tracing = { workspace = true }
reqwest = { version = "0.12", features = ["json"] }
```

### Step 3: Implement the Plugin

In `crates/pacs-webhook-plugin/src/lib.rs`:

```rust
use async_trait::async_trait;
use pacs_plugin::{
    capabilities::{EventKind, EventPlugin},
    context::PluginContext,
    error::PluginError,
    event::PacsEvent,
    plugin::{Plugin, PluginHealth, PluginManifest},
    register_plugin,
};
use serde::Deserialize;
use std::sync::Arc;
use tracing::{info, warn};

/// Plugin ID — used in config.toml and dependency declarations.
pub const WEBHOOK_PLUGIN_ID: &str = "webhook-notifier";

/// Configuration parsed from `[plugins.webhook-notifier]` in config.toml.
#[derive(Debug, Clone, Deserialize)]
struct WebhookConfig {
    /// URL to POST events to.
    url: String,
    /// Optional bearer token for the webhook endpoint.
    bearer_token: Option<String>,
}

/// Runtime state created during init().
struct WebhookRuntime {
    config: WebhookConfig,
    client: reqwest::Client,
}

/// The plugin struct. Uses Option<Arc<...>> to hold post-init state.
#[derive(Default)]
pub struct WebhookNotifierPlugin {
    runtime: Option<Arc<WebhookRuntime>>,
}

#[async_trait]
impl Plugin for WebhookNotifierPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            WEBHOOK_PLUGIN_ID,
            "Webhook Notifier",
            env!("CARGO_PKG_VERSION"),
        )
        .disabled_by_default()
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        let config: WebhookConfig =
            serde_json::from_value(ctx.config.clone()).map_err(|e| PluginError::Config {
                plugin_id: WEBHOOK_PLUGIN_ID.into(),
                message: e.to_string(),
            })?;

        info!(url = %config.url, "Webhook notifier configured");

        self.runtime = Some(Arc::new(WebhookRuntime {
            config,
            client: reqwest::Client::new(),
        }));

        Ok(())
    }

    async fn health(&self) -> PluginHealth {
        match &self.runtime {
            Some(_) => PluginHealth::Healthy,
            None => PluginHealth::Unhealthy("not initialized".into()),
        }
    }

    // Declare that this plugin provides event handling.
    fn as_event_plugin(&self) -> Option<&dyn EventPlugin> {
        Some(self)
    }
}

#[async_trait]
impl EventPlugin for WebhookNotifierPlugin {
    fn subscriptions(&self) -> Vec<EventKind> {
        vec![EventKind::InstanceStored]
    }

    async fn on_event(&self, event: &PacsEvent) -> Result<(), PluginError> {
        let rt = self.runtime.as_ref().ok_or_else(|| PluginError::NotInitialized {
            plugin_id: WEBHOOK_PLUGIN_ID.into(),
            capability: "event".into(),
        })?;

        if let PacsEvent::InstanceStored {
            study_uid,
            series_uid,
            sop_instance_uid,
            ..
        } = event
        {
            let payload = serde_json::json!({
                "event": "instance_stored",
                "study_uid": study_uid,
                "series_uid": series_uid,
                "sop_instance_uid": sop_instance_uid,
            });

            let mut req = rt.client.post(&rt.config.url).json(&payload);

            if let Some(token) = &rt.config.bearer_token {
                req = req.bearer_auth(token);
            }

            if let Err(e) = req.send().await {
                warn!(error = %e, "Failed to send webhook notification");
            }
        }

        Ok(())
    }
}

// This single line registers the plugin for automatic discovery.
register_plugin!(WebhookNotifierPlugin::default);
```

### Step 4: Wire It into the Binary

In `crates/pacs-server/Cargo.toml`, add the dependency:

```toml
[dependencies]
pacs-webhook-plugin = { path = "../pacs-webhook-plugin" }
```

In `crates/pacs-server/src/main.rs`, import the crate so the linker includes it:

```rust
use pacs_webhook_plugin as _;
```

That's it. The `register_plugin!` macro + `inventory` crate handles discovery. The
plugin will appear in the registry automatically.

### Step 5: Configure It

In `config.toml`:

```toml
[plugins]
enabled = ["webhook-notifier"]

[plugins.webhook-notifier]
url = "https://example.com/hooks/dicom"
bearer_token = "secret-token"
```

### Step 6: Test It

Build and run:

```bash
cargo build
./target/debug/pacs-server
```

The plugin will be initialised at startup and will POST to the webhook URL every time
a DICOM instance is stored via STOW-RS or C-STORE.

---

## The Plugin Trait

Every plugin must implement `Plugin`. This is the only required trait — all capability
traits are optional.

```rust
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    /// Returns the plugin's manifest (identity, dependencies, defaults).
    fn manifest(&self) -> PluginManifest;

    /// Called once during startup. Parse config, connect to services, build state.
    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError>;

    /// Called after all plugins are initialised. Start background tasks here.
    async fn start(&self, _ctx: &PluginContext) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called during graceful shutdown. Release resources, close connections.
    async fn shutdown(&self) -> Result<(), PluginError> {
        Ok(())
    }

    /// Periodic health check. Called by the /health endpoint.
    async fn health(&self) -> PluginHealth {
        PluginHealth::Healthy
    }

    // ── Capability accessors ──────────────────────────────────
    // Override the ones that match your plugin's capabilities.
    // The registry calls these to discover what each plugin provides.

    fn as_metadata_store_plugin(&self) -> Option<&dyn MetadataStorePlugin> { None }
    fn as_blob_store_plugin(&self)     -> Option<&dyn BlobStorePlugin>     { None }
    fn as_store_scp_plugin(&self)      -> Option<&dyn StoreScpPlugin>      { None }
    fn as_find_scp_plugin(&self)       -> Option<&dyn FindScpPlugin>       { None }
    fn as_get_scp_plugin(&self)        -> Option<&dyn GetScpPlugin>        { None }
    fn as_move_scp_plugin(&self)       -> Option<&dyn MoveScpPlugin>       { None }
    fn as_route_plugin(&self)          -> Option<&dyn RoutePlugin>         { None }
    fn as_middleware_plugin(&self)      -> Option<&dyn MiddlewarePlugin>    { None }
    fn as_event_plugin(&self)          -> Option<&dyn EventPlugin>         { None }
    fn as_codec_plugin(&self)          -> Option<&dyn CodecPlugin>         { None }
    fn as_processing_plugin(&self)     -> Option<&dyn ProcessingPlugin>    { None }
}
```

### Common Pattern: `Option<Arc<Runtime>>`

Most plugins follow a two-phase pattern:

1. The struct starts with `Option<Arc<Runtime>>` set to `None`.
2. `init()` parses config, builds a `Runtime` struct, and stores `Some(Arc::new(...))`.
3. Capability methods access `self.runtime` and return `PluginError::NotInitialized`
   if called before `init()`.

```rust
#[derive(Default)]
pub struct MyPlugin {
    runtime: Option<Arc<MyRuntime>>,
}

struct MyRuntime {
    // initialised state: DB pools, HTTP clients, config, etc.
}
```

---

## Plugin Manifest

The manifest is your plugin's identity card:

```rust
pub struct PluginManifest {
    pub id: String,               // Unique kebab-case identifier
    pub name: String,             // Human-readable display name
    pub version: String,          // Semantic version (e.g. "0.1.0")
    pub dependencies: Vec<String>,// Plugin IDs that must init before this one
    pub enabled_by_default: bool, // true = core plugin, false = opt-in
}
```

### Builder Methods

```rust
// Core plugin (enabled by default):
PluginManifest::new("my-plugin", "My Plugin", env!("CARGO_PKG_VERSION"))

// With dependencies:
PluginManifest::new("audit-logger", "Audit Logger", "0.1.0")
    .with_dependencies([pacs_plugin::METADATA_STORE_CAPABILITY_DEPENDENCY])

// Optional plugin (must be listed in plugins.enabled):
PluginManifest::new("basic-auth", "Basic Auth", "0.1.0")
    .disabled_by_default()

// Both:
PluginManifest::new("my-plugin", "My Plugin", "0.1.0")
    .with_dependencies([
        pacs_plugin::METADATA_STORE_CAPABILITY_DEPENDENCY,
        pacs_plugin::BLOB_STORE_CAPABILITY_DEPENDENCY,
    ])
    .disabled_by_default()
```

### ID Conventions

- Use **kebab-case**: `my-cool-plugin`, not `my_cool_plugin` or `MyCoolPlugin`.
- The ID is used as the TOML config key: `[plugins.my-cool-plugin]`.
- The ID is used in `plugins.enabled` lists and dependency declarations.
- Keep it short but descriptive.

---

## Capability Traits

Capabilities define what a plugin *provides* to the system. A plugin can implement
zero or more capability traits. The registry discovers capabilities via the
`as_*_plugin()` accessor methods.

### MetadataStorePlugin

Provides the DICOM metadata storage backend (study/series/instance index).

```rust
pub trait MetadataStorePlugin: Plugin {
    fn metadata_store(&self) -> Result<Arc<dyn MetadataStore>, PluginError>;
}
```

**Constraints:** Singleton — only one metadata store plugin can be active. If two
plugins provide this capability, the registry returns `PluginError::DuplicateProvider`.

**Example:** `pg-metadata-store` (see `crates/pacs-store/src/plugin.rs`).

### BlobStorePlugin

Provides the DICOM binary object storage backend (raw .dcm files).

```rust
pub trait BlobStorePlugin: Plugin {
    fn blob_store(&self) -> Result<Arc<dyn BlobStore>, PluginError>;
}
```

**Constraints:** Singleton — same as MetadataStorePlugin.

**Example:** `s3-blob-store` (see `crates/pacs-storage/src/plugin.rs`).

### RoutePlugin

Adds HTTP routes to the Axum router. Routes from all `RoutePlugin` implementations
are merged into the main router at startup.

```rust
pub trait RoutePlugin: Plugin {
    fn routes(&self) -> Router<AppState>;
}
```

**Use when:** Your plugin needs to expose HTTP endpoints — login pages, metrics
endpoints, viewer static files, export downloads, etc.

**Example:**

```rust
impl RoutePlugin for MyPlugin {
    fn routes(&self) -> Router<AppState> {
        let rt = self.runtime.clone().unwrap();
        Router::new()
            .route("/my-endpoint", get(handler))
            .with_state(rt)  // you can use sub-state if needed
    }
}
```

**Note:** The returned `Router<AppState>` is merged into the main server router.
You have access to `AppState` (which includes the plugin registry, metadata store,
blob store, and server info) via Axum's `State` extractor.

### MiddlewarePlugin

Applies tower middleware (e.g., auth checks, request logging, rate limiting) to the
entire HTTP router.

```rust
pub trait MiddlewarePlugin: Plugin {
    fn apply(&self, router: Router<AppState>) -> Router<AppState>;
    fn priority(&self) -> i32 { 50 }
}
```

**Ordering:** Middleware plugins are applied in **ascending priority order**. Lower
priority numbers wrap *outermost* (run first on request, last on response):

| Priority | Typical use |
|----------|-------------|
| 0 | Authentication (must run first) |
| 50 | Default / general-purpose |
| 100 | Metrics / observability (measure the full pipeline) |

**Example:** The `basic-auth` plugin uses priority `0` so it validates tokens before
any other middleware runs. The `prometheus-metrics` plugin uses priority `100` so it
measures the full request lifecycle including auth.

### EventPlugin

Reacts to DICOM lifecycle events (instances stored, queries performed, associations
opened/closed, etc.).

```rust
#[async_trait]
pub trait EventPlugin: Plugin {
    fn subscriptions(&self) -> Vec<EventKind>;
    async fn on_event(&self, event: &PacsEvent) -> Result<(), PluginError>;
}
```

**How it works:** The registry calls `on_event()` directly for all subscribed event
plugins whenever `registry.emit_event()` is called. Additionally, events are broadcast
on the `EventBus` channel for any code that holds a subscriber handle.

**Available event kinds:**

| EventKind | Fired when |
|-----------|------------|
| `InstanceStored` | A DICOM instance is stored via STOW-RS or C-STORE |
| `StudyComplete` | All instances for a study have been received |
| `ResourceDeleted` | A study, series, or instance is deleted via REST API |
| `AssociationOpened` | A DIMSE association is established |
| `AssociationClosed` | A DIMSE association is released |
| `QueryPerformed` | A QIDO-RS or C-FIND query completes |

**Example:** See `crates/pacs-audit-plugin/src/lib.rs` for a complete implementation.

### CodecPlugin

Provides DICOM transfer syntax encoding/decoding (e.g., JPEG 2000, JPEG-LS).

```rust
pub trait CodecPlugin: Plugin {
    fn supported_transfer_syntaxes(&self) -> Vec<String>;

    fn decode(
        &self,
        data: &[u8],
        transfer_syntax_uid: &str,
    ) -> Result<Vec<Vec<u8>>, PluginError>;

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
```

**Constraints:** Transfer syntax UIDs are registered as a map. Two codec plugins
cannot claim the same transfer syntax UID.

**Use when:** You need to add support for compressed transfer syntaxes like JPEG 2000
(`1.2.840.10008.1.2.4.90`), JPEG-LS (`1.2.840.10008.1.2.4.80`), etc.

### ProcessingPlugin

Provides a named data processing pipeline that operates on DICOM datasets (e.g.,
anonymisation, tag morphing, pixel redaction).

```rust
pub trait ProcessingPlugin: Plugin {
    fn processor_id(&self) -> &str;
    fn process(
        &self,
        dataset: &mut DataSet,
        params: &serde_json::Value,
    ) -> Result<(), PluginError>;
}
```

**Use when:** You need to transform DICOM data in a reusable way — anonymisation
profiles, header normalisation, pixel de-identification.

### DIMSE SCP Plugins

These provide handlers for DICOM network operations:

```rust
// Object-safe handler traits (used at runtime):
pub trait StoreScpHandler: Send + Sync {
    fn handle_store(&self, event: StoreEvent) -> BoxFuture<'_, StoreResult>;
}

pub trait FindScpHandler: Send + Sync {
    fn handle_find(&self, event: FindEvent) -> BoxFuture<'_, Vec<DataSet>>;
}

pub trait GetScpHandler: Send + Sync {
    fn handle_get(&self, event: GetEvent) -> BoxFuture<'_, Vec<RetrieveItem>>;
}

pub trait MoveScpHandler: Send + Sync {
    fn handle_move(&self, event: MoveEvent) -> BoxFuture<'_, Vec<RetrieveItem>>;
}

// Plugin capability traits (used during init):
pub trait StoreScpPlugin: Plugin {
    fn store_scp_handler(&self, plugins: Arc<PluginRegistry>)
        -> Result<Arc<dyn StoreScpHandler>, PluginError>;
}

pub trait FindScpPlugin: Plugin {
    fn find_scp_handler(&self, plugins: Arc<PluginRegistry>)
        -> Result<Arc<dyn FindScpHandler>, PluginError>;
}

// ... similarly for GetScpPlugin and MoveScpPlugin
```

**Constraints:** Singleton — only one store SCP, one find SCP, etc.

**Note:** The handler traits use `BoxFuture` (a pinned boxed future) instead of
`async fn` to maintain object safety. This is a deliberate design choice so that
the `PluginRegistry` can hold `Arc<dyn StoreScpHandler>` without leaking the
toolkit's non-object-safe async provider traits.

---

## Event System

The event system lets plugins react to system-wide DICOM lifecycle events without
direct coupling between components.

### PacsEvent

```rust
pub enum PacsEvent {
    InstanceStored {
        study_uid: String,
        series_uid: String,
        sop_instance_uid: String,
        sop_class_uid: String,
        source: String,           // e.g. "DICOMweb" or calling AE title
        user_id: Option<String>,  // from authenticated user, if auth enabled
    },
    StudyComplete {
        study_uid: String,
    },
    ResourceDeleted {
        level: ResourceLevel,     // Patient, Study, Series, or Instance
        uid: String,
        user_id: Option<String>,
    },
    AssociationOpened {
        calling_ae: String,
        peer_addr: SocketAddr,
    },
    AssociationClosed {
        calling_ae: String,
    },
    QueryPerformed {
        level: String,            // e.g. "STUDY", "SERIES"
        source: QuerySource,      // Dimse { calling_ae } or Dicomweb
        num_results: usize,
        user_id: Option<String>,
    },
}
```

### EventBus

The `EventBus` is a `tokio::sync::broadcast` channel shared across all plugins:

```rust
pub struct EventBus {
    tx: broadcast::Sender<PacsEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self;  // default: 256
    pub fn emit(&self, event: PacsEvent) -> usize;
    pub fn subscribe(&self) -> broadcast::Receiver<PacsEvent>;
}
```

**Two consumption paths:**

1. **`EventPlugin::on_event()`** — the registry calls this directly and synchronously
   (within an async context) for all subscribed plugins after each event emission.
2. **`event_bus.subscribe()`** — obtain a `broadcast::Receiver` for background tasks
   that need a stream of events.

### Emitting Events

Events are emitted by core server code (HTTP handlers, DIMSE providers) via:

```rust
// In an HTTP handler or DIMSE provider:
registry.emit_event(PacsEvent::InstanceStored {
    study_uid: "1.2.3".into(),
    series_uid: "1.2.3.4".into(),
    sop_instance_uid: "1.2.3.4.5".into(),
    sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".into(),
    source: "DICOMweb".into(),
    user_id: None,
}).await;
```

Plugins should generally **not** emit events themselves unless they represent a
genuine new system event.

---

## Plugin Context

When `init()` is called, the plugin receives a `PluginContext`:

```rust
pub struct PluginContext {
    /// Plugin-specific config from [plugins.<id>] TOML section.
    pub config: serde_json::Value,

    /// The active metadata store, if a MetadataStorePlugin has initialised.
    /// Available to plugins that depend on the metadata store plugin.
    pub metadata_store: Option<Arc<dyn MetadataStore>>,

    /// The active blob store, if a BlobStorePlugin has initialised.
    pub blob_store: Option<Arc<dyn BlobStore>>,

    /// Static server identity info.
    pub server_info: ServerInfo,

    /// Shared event bus for emitting or subscribing to events.
    pub event_bus: Arc<EventBus>,
}
```

**Dependency-aware context:** If your plugin depends on `pacs_plugin::METADATA_STORE_CAPABILITY_DEPENDENCY`, the
`ctx.metadata_store` field will be `Some(...)` by the time your `init()` runs,
because the registry resolves dependencies topologically.

### ServerInfo

Available via `ctx.server_info`:

```rust
pub struct ServerInfo {
    pub ae_title: String,     // DICOM AE title (e.g. "PACSNODE")
    pub http_port: u16,       // HTTP API port
    pub dicom_port: u16,      // DIMSE TCP port
    pub version: &'static str,// Application version
}
```

### AppState

Available in Axum handlers via `State<AppState>`:

```rust
pub struct AppState {
    pub server_info: ServerInfo,
    pub store: Arc<dyn MetadataStore>,
    pub blobs: Arc<dyn BlobStore>,
    pub plugins: Arc<PluginRegistry>,
}
```

---

## Configuration

### Enabling Plugins

In `config.toml`:

```toml
[plugins]
enabled = ["basic-auth", "audit-logger", "prometheus-metrics"]
```

Built-in plugins (`enabled_by_default: true`) are always active — you don't need to
list them. The `enabled` list is for optional plugins only.

### Per-Plugin Configuration

Each plugin receives its config as a `serde_json::Value` parsed from the
`[plugins.<id>]` TOML section:

```toml
[plugins.basic-auth]
username = "admin"
password_hash = "$argon2id$v=19$..."
jwt_secret = "your-secret-here"
token_ttl_secs = 3600
public_paths = ["/health", "/metrics"]

[plugins.audit-logger]
max_connections = 5

[plugins.prometheus-metrics]
endpoint = "/metrics"
```

**In your plugin**, deserialise with `serde_json::from_value`:

```rust
#[derive(Deserialize)]
struct MyConfig {
    url: String,
    #[serde(default = "default_timeout")]
    timeout_secs: u64,
}

fn default_timeout() -> u64 { 30 }

async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
    let config: MyConfig = serde_json::from_value(ctx.config.clone())
        .map_err(|e| PluginError::Config {
            plugin_id: "my-plugin".into(),
            message: e.to_string(),
        })?;
    // ...
}
```

### Built-in Plugin Config Merging

Built-in plugins like `pg-metadata-store`, `sqlite-metadata-store`, `s3-blob-store`,
and `filesystem-blob-store` receive their config from the core `[database]`,
`[storage]`, or `[filesystem_storage]` sections as appropriate. The server's
`build_plugin_configs()` function merges these into the plugin config map
automatically, so you only need `[plugins.<id>]` when you want to override or
extend the built-in defaults.

---

## Error Handling

All plugin errors use the `PluginError` enum:

```rust
pub enum PluginError {
    Config { plugin_id, message },       // Bad config value
    MissingDependency { plugin_id, dependency },
    CircularDependency { cycle },
    DuplicateProvider { capability, first, second },
    DuplicatePluginId { id },
    NotInitialized { plugin_id, capability },
    InitFailed { plugin_id, source },    // Wraps any error
    Runtime { plugin_id, message },      // Catch-all for runtime errors
}
```

### Converting Errors

`PluginError` implements `From<serde_json::Error>` for convenient config parsing.
For other error types, use `.map_err()`:

```rust
let pool = PgPoolOptions::new()
    .connect(&config.url)
    .await
    .map_err(|e| PluginError::InitFailed {
        plugin_id: MY_PLUGIN_ID.into(),
        source: Box::new(e),
    })?;
```

### Health Reporting

Plugins report health via the `health()` method:

```rust
pub enum PluginHealth {
    Healthy,
    Degraded(String),    // Working but not ideal
    Unhealthy(String),   // Broken
}
```

The registry aggregates health from all plugins via `aggregate_health()`, which is
exposed through the `/health` endpoint.

---

## Testing Plugins

### Unit Testing

Test your plugin logic in the same crate with `#[cfg(test)]`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use pacs_plugin::{
        context::PluginContext,
        event::EventBus,
        plugin::Plugin,
        state::ServerInfo,
    };
    use std::sync::Arc;

    fn test_context(config: serde_json::Value) -> PluginContext {
        PluginContext {
            config,
            metadata_store: None,
            blob_store: None,
            server_info: ServerInfo {
                ae_title: "TEST".into(),
                http_port: 8042,
                dicom_port: 4242,
                version: "0.0.0-test",
            },
            event_bus: Arc::new(EventBus::default()),
        }
    }

    #[tokio::test]
    async fn test_init_with_valid_config() {
        let mut plugin = MyPlugin::default();
        let ctx = test_context(serde_json::json!({
            "url": "https://example.com/webhook"
        }));
        assert!(plugin.init(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_init_with_bad_config() {
        let mut plugin = MyPlugin::default();
        let ctx = test_context(serde_json::json!({})); // missing required "url"
        let result = plugin.init(&ctx).await;
        assert!(matches!(result, Err(PluginError::Config { .. })));
    }

    #[tokio::test]
    async fn test_event_handling() {
        let mut plugin = MyPlugin::default();
        let ctx = test_context(serde_json::json!({"url": "http://localhost"}));
        plugin.init(&ctx).await.unwrap();

        let event = PacsEvent::InstanceStored {
            study_uid: "1.2.3".into(),
            series_uid: "1.2.3.4".into(),
            sop_instance_uid: "1.2.3.4.5".into(),
            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".into(),
            source: "test".into(),
            user_id: None,
        };

        let result = plugin.on_event(&event).await;
        assert!(result.is_ok());
    }
}
```

### Integration Testing

For plugins that depend on external services (databases, APIs), use `testcontainers`
in `tests/` at the crate root:

```rust
// tests/integration_test.rs
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn test_audit_with_real_postgres() {
    let pg = Postgres::default().start().await.unwrap();
    let url = format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        pg.get_host_port_ipv4(5432).await.unwrap()
    );

    let mut plugin = AuditLoggerPlugin::default();
    let ctx = test_context(serde_json::json!({ "url": url }));
    plugin.init(&ctx).await.unwrap();

    // Test event persistence...
}
```

### Running Tests

```bash
# All workspace tests (except those requiring Docker):
cargo test --workspace --all-targets --exclude pacs-store

# Specific plugin crate:
cargo test -p pacs-webhook-plugin

# With Docker-dependent integration tests:
cargo test --workspace --all-targets
```

---

## Reference: Existing Plugins

Study the built-in plugins as implementation references:

| Plugin | Crate | Capabilities | Complexity | Best reference for |
|--------|-------|-------------|------------|-------------------|
| `pg-metadata-store` | `crates/pacs-store/src/plugin.rs` | MetadataStorePlugin | Medium | Storage backend plugins |
| `s3-blob-store` | `crates/pacs-storage/src/plugin.rs` | BlobStorePlugin | Medium | Storage backend plugins |
| `pacs-store-scp` | `crates/pacs-dimse/` | StoreScpPlugin | High | DIMSE handler plugins |
| `pacs-query-scp` | `crates/pacs-dimse/` | FindScp + GetScp + MoveScp | High | DIMSE handler plugins |
| `basic-auth` | `crates/pacs-auth-plugin/src/lib.rs` | Route + Middleware | Medium | Auth / middleware plugins |
| `audit-logger` | `crates/pacs-audit-plugin/src/lib.rs` | Event | Simple | Event subscriber plugins |
| `prometheus-metrics` | `crates/pacs-metrics-plugin/src/lib.rs` | Route + Event + Middleware | Medium | Multi-capability plugins |

### Quick Reference: Plugin Checklist

When creating a new plugin, make sure you:

- [ ] Create a new crate under `crates/`
- [ ] Add `pacs-plugin` as a dependency
- [ ] Implement `Plugin` with a proper manifest (unique kebab-case ID)
- [ ] Call `register_plugin!(MyPlugin::default);` at crate root
- [ ] Override `as_*_plugin()` for each capability you provide
- [ ] Implement the corresponding capability traits
- [ ] Add the crate as a dependency in `crates/pacs-server/Cargo.toml`
- [ ] Add `use my_crate as _;` in `crates/pacs-server/src/main.rs`
- [ ] Add the workspace member in root `Cargo.toml`
- [ ] Use `disabled_by_default()` for optional plugins
- [ ] Document configuration in `config.toml.example`
- [ ] Write unit tests for init, health, and capability logic
- [ ] Write integration tests if the plugin touches external services

---

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Plugin loading | Compile-time (`inventory`) | Type safety, zero-cost, no ABI issues — critical for medical software |
| Capability discovery | `as_*_plugin()` methods | Object-safe, simple, no separate registry API per capability |
| Singleton enforcement | Registry error on duplicates | Prevents ambiguity (which metadata store to use?) |
| Event delivery | Direct call + broadcast | Guarantees event plugins process events; broadcast for background consumers |
| Middleware ordering | Explicit priority integer | Deterministic, composable, no name-based ordering headaches |
| Config format | TOML → `serde_json::Value` | TOML for human editing, JSON Value for flexible schema-free parsing |
| Error handling | `PluginError` enum with `thiserror` | Consistent, typed errors across all plugins |
| Health model | Three-state (Healthy/Degraded/Unhealthy) | Matches standard health check patterns (k8s probes, etc.) |

---

## Future: WASM Extension Points

A future phase may add WASM-based plugins (via [Extism](https://extism.org/) or
[wasmtime](https://wasmtime.dev/)) for sandboxed, untrusted third-party hooks. The
event system is designed to naturally extend to WASM guests — a WASM meta-plugin
would implement `EventPlugin` and forward events to WASM modules.

This would enable Lua/Python/JS-like scripting without sacrificing the safety of the
core plugin system.
