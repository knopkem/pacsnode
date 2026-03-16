# pacsnode OHIF/Default Viewer Integration — Audit Summary

## EXECUTIVE SUMMARY

**Current State:** OHIF viewer integration is **planned but not implemented**. The codebase has a mature, working compile-time plugin system with clear architectural patterns for adding HTTP routes and middleware. All infrastructure needed for a viewer plugin exists; only the viewer plugin itself is missing.

**What Exists (Ready to Use):**
- ✅ Compile-time trait-based plugin system (fully operational)
- ✅ RoutePlugin capability trait for contributing HTTP routes
- ✅ MiddlewarePlugin capability trait for wrapping routers
- ✅ Route merging mechanism in PluginRegistry
- ✅ Plugin configuration system (TOML + environment variables)
- ✅ DICOMweb API fully implemented (QIDO-RS, WADO-RS, STOW-RS)
- ✅ Static file serving support via tower-http (available but not used)
- ✅ Plugin registration mechanism via `inventory` crate and `register_plugin!` macro

**What's Missing:**
- ❌ `pacs-viewer-plugin` crate (the OHIF viewer plugin itself)
- ❌ Configuration schema for viewer path and static assets
- ❌ Static file serving handler
- ❌ Default redirect to viewer (e.g., `/` → `/viewer/`)
- ❌ Tests for viewer plugin
- ❌ Documentation on deploying OHIF with pacsnode

**Key Design Decisions Already Made:**
1. Plugins are compiled-in, not dynamically loaded
2. RoutePlugin returns `Router<AppState>` that gets merged
3. MiddlewarePlugin wraps the full router in priority order (lower = outermost)
4. Configuration lives in `[plugins.<plugin-id>]` TOML section
5. Tower-http is already a dependency (though ServeDir feature not enabled yet)

---

## DETAILED AUDIT

### 1. EXISTING PLANS FOR OHIF/VIEWER

**Documents Reference OHIF Integration:**

#### `DOCS/feature-matrix.md` (Line 65)
```
| **Built-in web UI** | ❌ 🔮 | ✅ | Orthanc Explorer (basic); pacsnode plans OHIF |
| **Viewer / UI** | 10% | 70% | Rendered DICOMweb previews exist, but OHIF/static UI hosting is still missing |
```

#### `DOCS/final-plan.md` (Lines 384, 467)
```
- OAuth2 / SMART on FHIR authentication for OHIF and clinical app compatibility
- Web UI integration (OHIF viewer via DICOMweb)
```

#### `DOCS/plugin-system.md` (Lines 1490, 1625-1627)
```toml
[features]
viewer = ["dep:plugin-ohif"]
full = ["auth", "audit", "metrics", "viewer", "codec-jp2k"]

[plugins.ohif-viewer]
static_dir   = "/opt/pacsnode/viewer"
route_prefix = "/viewer"
```

**Status:** This is a **design specification** for a future Phase 4+ feature, not an implementation. The config schema is documented but the plugin doesn't exist.

---

### 2. PLUGIN SYSTEM ARCHITECTURE

#### Plugin Lifecycle (Already Implemented)

**File:** `crates/pacs-plugin/src/plugin.rs` & `crates/pacs-plugin/src/registry.rs`

The `Plugin` trait requires:
- `manifest()` — returns ID, name, version, dependencies, enabled-by-default flag
- `init(&mut self, ctx: PluginContext)` — called during startup
- `start(ctx)` — called after all plugins initialized (optional)
- `shutdown()` — called during graceful shutdown (optional)
- `health()` — periodic health checks (optional)
- Accessor methods: `as_*_plugin()` for each capability

#### Capability Traits (Already Implemented)

**File:** `crates/pacs-plugin/src/capabilities.rs`

Currently defined capabilities (7 traits):
1. **MetadataStorePlugin** — singleton, provides metadata storage backend
2. **BlobStorePlugin** — singleton, provides blob storage backend
3. **StoreScpPlugin, FindScpPlugin, GetScpPlugin, MoveScpPlugin** — DIMSE handlers
4. **RoutePlugin** — ✅ **This is what we need for viewer**
   ```rust
   pub trait RoutePlugin: Plugin {
       fn routes(&self) -> Router<AppState>;
   }
   ```
5. **MiddlewarePlugin** — ✅ **Can use for auth/logging on viewer**
   ```rust
   pub trait MiddlewarePlugin: Plugin {
       fn apply(&self, router: Router<AppState>) -> Router<AppState>;
       fn priority(&self) -> i32 { 50 }
   }
   ```
6. **EventPlugin** — reacts to DICOM events (InstanceStored, etc.)
7. **CodecPlugin** — handles transfer syntax encoding/decoding

---

### 3. PLUGIN REGISTRATION & ROUTE MERGING

**File:** `crates/pacs-plugin/src/registry.rs`

#### Route Merging (Lines 259–267)
```rust
pub fn merged_routes(&self) -> Router<AppState> {
    let mut router = Router::new();
    for plugin in &self.plugins {
        if let Some(route_plugin) = plugin.as_route_plugin() {
            router = router.merge(route_plugin.routes());
        }
    }
    router
}
```

#### Middleware Application (Lines 270–287)
```rust
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
```

**Priority Order (Lower wraps first, runs first on request):**
- 0: Authentication (e.g., `basic-auth` plugin)
- 50: Default/general-purpose
- 100: Metrics/observability (e.g., `prometheus-metrics` plugin)

#### Automatic Plugin Discovery
**File:** `crates/pacs-server/src/main.rs` (Lines 17–21, 37–43)
```rust
use pacs_audit_plugin as _;
use pacs_auth_plugin as _;
use pacs_metrics_plugin as _;
use pacs_storage as _;
use pacs_store as _;

let mut registry = PluginRegistry::new();
if !cfg.plugins.enabled.is_empty() {
    registry.set_enabled(cfg.plugins.enabled.clone());
}
registry
    .register_all_discovered()
    .context("failed to register compiled-in plugins")?;
```

The `inventory` crate + `register_plugin!` macro handles discovery. Each plugin must call:
```rust
register_plugin!(MyPlugin::default);
```

---

### 4. HOW PLUGIN CONTRIBUTIONS ARE WIRED

**File:** `crates/pacs-server/src/main.rs` (Lines 72–74)
```rust
let router = registry
    .apply_middleware(pacs_api::build_router_without_state().merge(registry.merged_routes()))
    .with_state(app_state);
```

**Flow:**
1. Build core DICOMweb/REST routes via `pacs_api::build_router_without_state()`
2. Merge plugin-contributed routes via `registry.merged_routes()`
3. Apply middleware plugins (in priority order) via `registry.apply_middleware()`
4. Attach application state via `.with_state(app_state)`

---

### 5. CONFIGURATION SYSTEM

**File:** `crates/pacs-server/src/config.rs`

#### Config Loading (Two-Layer)
1. **TOML file** (`config.toml`) — optional, looked up in working directory
2. **Environment variables** — override TOML, prefix `PACS_`, separator `__`

#### Plugin Configuration Structure
```rust
pub struct PluginsConfig {
    /// Optional plugin IDs to activate
    pub enabled: Vec<String>,
    /// Per-plugin config sections keyed by plugin ID
    pub configs: HashMap<String, serde_json::Value>,
}
```

#### Plugin Config Flow (Lines 127–150)
```rust
fn build_plugin_configs(cfg: &AppConfig) -> Result<HashMap<String, serde_json::Value>> {
    let mut configs = cfg.plugins.configs.clone();
    
    // Example: merge database config into pg-metadata-store
    let mut db_config = serde_json::to_value(&cfg.database)?;
    if let Some(override_value) = configs.remove("pg-metadata-store") {
        merge_json(&mut db_config, override_value);
    }
    configs.insert("pg-metadata-store".into(), db_config);
    
    Ok(configs)
}
```

Plugin receives its config via `PluginContext`:
```rust
async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
    let config: MyConfig = serde_json::from_value(ctx.config.clone())?;
    // ...
}
```

---

### 6. STATIC FILE SERVING INFRASTRUCTURE

**Current State:** Tower-http is already a workspace dependency but **ServeDir feature is NOT enabled**.

**File:** `Cargo.toml` (Line 53)
```toml
tower-http = { version = "0.6", features = ["trace", "cors", "timeout", "limit", "request-id", "compression-full"] }
```

**What's Needed to Enable Static Files:**
1. Add `"fs"` feature to tower-http (provides `ServeDir`)
2. In viewer plugin, use `tower_http::services::ServeDir` to serve OHIF assets
3. Route pattern: `Router::nest("/viewer", ServeDir::new("/opt/pacsnode/viewer"))`

**Example Pattern (from other plugins):**
```rust
use tower_http::services::ServeDir;
use axum::routing::get_service;
use axum::Router;

impl RoutePlugin for OhifViewerPlugin {
    fn routes(&self) -> Router<AppState> {
        let static_dir = self.runtime.as_ref().unwrap().static_dir.clone();
        Router::new()
            .nest_service(
                "/viewer",
                get_service(ServeDir::new(static_dir))
                    .handle_error(handle_error),
            )
    }
}
```

---

### 7. EXISTING PLUGIN EXAMPLES

#### `pacs-auth-plugin` (Most Complete Example)
**File:** `crates/pacs-auth-plugin/src/lib.rs`

- ✅ Implements both **RoutePlugin** and **MiddlewarePlugin**
- ✅ Routes: `/auth/login`, `/auth/refresh` (POST)
- ✅ Middleware: Validates JWT bearer tokens
- ✅ Config: Username, password hash, JWT secret, paths, TTL
- ✅ Priority: 0 (runs first on requests)

**Key Pattern:**
```rust
#[derive(Default)]
pub struct BasicAuthPlugin {
    runtime: Option<Arc<AuthRuntime>>,
}

#[async_trait]
impl Plugin for BasicAuthPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            "basic-auth",
            "Basic HTTP Auth",
            env!("CARGO_PKG_VERSION"),
        )
        .disabled_by_default()  // Optional — must be enabled in config
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        let config: AuthPluginConfig =
            serde_json::from_value(ctx.config.clone())?;
        
        // Parse, validate, build runtime state
        self.runtime = Some(Arc::new(AuthRuntime { /* ... */ }));
        Ok(())
    }

    fn as_route_plugin(&self) -> Option<&dyn RoutePlugin> {
        Some(self)
    }

    fn as_middleware_plugin(&self) -> Option<&dyn MiddlewarePlugin> {
        Some(self)
    }
}

impl RoutePlugin for BasicAuthPlugin {
    fn routes(&self) -> Router<AppState> {
        let runtime = self.runtime.as_ref().map(Arc::clone).unwrap();
        Router::new()
            .route("/auth/login", post(login_handler))
            .route("/auth/refresh", post(refresh_handler))
    }
}

impl MiddlewarePlugin for BasicAuthPlugin {
    fn apply(&self, router: Router<AppState>) -> Router<AppState> {
        router.layer(from_fn_with_state(runtime, auth_middleware))
    }
    
    fn priority(&self) -> i32 { 0 }
}

register_plugin!(BasicAuthPlugin::default);
```

#### `pacs-metrics-plugin`
**File:** `crates/pacs-metrics-plugin/src/lib.rs`

- ✅ Implements **RoutePlugin**, **EventPlugin**, **MiddlewarePlugin**
- ✅ Routes: `/metrics` (GET) — Prometheus exposition format
- ✅ Events: Tracks InstanceStored, associations, queries
- ✅ Config: Endpoint path (default `/metrics`)
- ✅ Priority: 100 (wraps other middleware, measures full pipeline)

---

### 8. CONCRETE FILES TO CHANGE FOR OHIF IMPLEMENTATION

#### New Files to Create:
1. **`crates/pacs-viewer-plugin/`** (new crate)
   - `Cargo.toml`
   - `src/lib.rs`

2. **Configuration:**
   - `config.toml.example` — add `[plugins.ohif-viewer]` section

3. **Documentation:**
   - `DOCS/ohif-viewer-integration.md` — deployment guide
   - `PLUGIN_GUIDE.md` — update with ohif-viewer example (if comprehensive)

4. **Tests (Optional but Recommended):**
   - `crates/pacs-viewer-plugin/src/lib.rs` — include `#[cfg(test)] mod tests`

#### Files to Modify:

1. **`Cargo.toml`** (workspace root, Line 15)
   ```toml
   [workspace]
   members = [
       # ... existing ...
       "crates/pacs-viewer-plugin",
   ]
   ```

2. **`Cargo.toml`** (workspace root, Lines 25–36)
   ```toml
   [workspace.dependencies]
   pacs-viewer-plugin = { path = "crates/pacs-viewer-plugin" }
   ```
   
   Also enable ServeDir feature (Line 53):
   ```toml
   tower-http = { version = "0.6", features = [
       "trace", "cors", "timeout", "limit", "request-id", "compression-full",
       "fs"  # Add this for ServeDir
   ] }
   ```

3. **`crates/pacs-server/Cargo.toml`** (Line 16+)
   ```toml
   [dependencies]
   pacs-viewer-plugin = { workspace = true }
   ```

4. **`crates/pacs-server/src/main.rs`** (Line 17–21)
   ```rust
   use pacs_viewer_plugin as _;  // Add this line
   ```

5. **`config.toml.example`** (append at end)
   ```toml
   # Example OHIF Viewer config:
   # [plugins.ohif-viewer]
   # static_dir   = "/opt/pacsnode/viewer"
   # route_prefix = "/viewer"
   # index_file   = "index.html"
   # fallback_file = "index.html"  # For SPA routing
   ```

6. **`PLUGIN_GUIDE.md`** (optional, but recommended)
   - Add OHIF viewer as a "Real-World Example" under "Creating a New Plugin"

---

## 9. SAFEST IMPLEMENTATION APPROACH

### Phase 1: Create Viewer Plugin (Minimal MVP)

1. **Create the plugin crate:**
   ```bash
   mkdir -p crates/pacs-viewer-plugin/src
   ```

2. **`crates/pacs-viewer-plugin/Cargo.toml`:**
   ```toml
   [package]
   name = "pacs-viewer-plugin"
   version.workspace = true
   edition.workspace = true
   rust-version.workspace = true
   license.workspace = true

   [dependencies]
   pacs-plugin = { workspace = true }
   axum = { workspace = true }
   tower-http = { workspace = true, features = ["fs"] }
   serde = { workspace = true }
   tracing = { workspace = true }
   async-trait = { workspace = true }
   ```

3. **`crates/pacs-viewer-plugin/src/lib.rs`:**
   ```rust
   use std::sync::Arc;
   use async_trait::async_trait;
   use axum::{routing::get_service, Router};
   use pacs_plugin::{
       register_plugin, AppState, Plugin, PluginContext, PluginError,
       PluginHealth, PluginManifest, RoutePlugin,
   };
   use serde::Deserialize;
   use tower_http::services::{ServeDir, ServeFile};
   use tracing::warn;

   pub const VIEWER_PLUGIN_ID: &str = "ohif-viewer";

   #[derive(Default)]
   pub struct OhifViewerPlugin {
       runtime: Option<Arc<ViewerRuntime>>,
   }

   #[derive(Debug, Clone, Deserialize)]
   struct ViewerPluginConfig {
       #[serde(default = "default_static_dir")]
       static_dir: String,
       #[serde(default = "default_route_prefix")]
       route_prefix: String,
       #[serde(default = "default_index_file")]
       index_file: String,
   }

   #[derive(Debug, Clone)]
   struct ViewerRuntime {
       static_dir: String,
       route_prefix: String,
       index_file: String,
   }

   fn default_static_dir() -> String {
       "/opt/pacsnode/viewer".to_string()
   }

   fn default_route_prefix() -> String {
       "/viewer".to_string()
   }

   fn default_index_file() -> String {
       "index.html".to_string()
   }

   #[async_trait]
   impl Plugin for OhifViewerPlugin {
       fn manifest(&self) -> PluginManifest {
           PluginManifest::new(
               VIEWER_PLUGIN_ID,
               "OHIF Viewer",
               env!("CARGO_PKG_VERSION"),
           )
           .disabled_by_default()
       }

       async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
           let config: ViewerPluginConfig =
               serde_json::from_value(ctx.config.clone()).map_err(|error| {
                   PluginError::Config {
                       plugin_id: VIEWER_PLUGIN_ID.into(),
                       message: error.to_string(),
                   }
               })?;

           // Validate that static_dir exists
           if !std::path::Path::new(&config.static_dir).exists() {
               return Err(PluginError::Config {
                   plugin_id: VIEWER_PLUGIN_ID.into(),
                   message: format!(
                       "static_dir does not exist: {}",
                       config.static_dir
                   ),
               });
           }

           self.runtime = Some(Arc::new(ViewerRuntime {
               static_dir: config.static_dir,
               route_prefix: config.route_prefix,
               index_file: config.index_file,
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
   }

   impl RoutePlugin for OhifViewerPlugin {
       fn routes(&self) -> Router<AppState> {
           let Some(runtime) = self.runtime.as_ref().map(Arc::clone) else {
               warn!(plugin_id = VIEWER_PLUGIN_ID, "viewer plugin routes requested before init");
               return Router::new();
           };

           let static_dir = runtime.static_dir.clone();
           let route_prefix = runtime.route_prefix.clone();

           Router::new().nest_service(
               &route_prefix,
               ServeDir::new(static_dir),
           )
       }
   }

   register_plugin!(OhifViewerPlugin::default);

   #[cfg(test)]
   mod tests {
       use super::*;

       #[test]
       fn test_manifest() {
           let plugin = OhifViewerPlugin::default();
           let manifest = plugin.manifest();
           assert_eq!(manifest.id, "ohif-viewer");
           assert!(!manifest.enabled_by_default);
       }

       #[tokio::test]
       async fn test_init_fails_with_nonexistent_dir() {
           let mut plugin = OhifViewerPlugin::default();
           let ctx = PluginContext {
               config: serde_json::json!({
                   "static_dir": "/nonexistent/path"
               }),
               server_info: pacs_plugin::ServerInfo {
                   ae_title: "TEST".into(),
                   http_port: 8042,
                   dicom_port: 4242,
                   version: "test",
               },
           };

           let result = plugin.init(&ctx).await;
           assert!(result.is_err());
       }
   }
   ```

### Phase 2: Wire into Binary

1. Modify `Cargo.toml` (workspace root)
2. Modify `crates/pacs-server/Cargo.toml`
3. Modify `crates/pacs-server/src/main.rs`
4. Add config to `config.toml.example`

### Phase 3: Deploy & Test

1. Build OHIF distribution (npm build from https://github.com/OHIF/Viewers)
2. Place built assets at deployment path (e.g., `/opt/pacsnode/viewer`)
3. Enable in `config.toml`:
   ```toml
   [plugins]
   enabled = ["ohif-viewer"]

   [plugins.ohif-viewer]
   static_dir = "/opt/pacsnode/viewer/build"
   route_prefix = "/viewer"
   ```
4. Restart pacsnode
5. Visit `http://localhost:8042/viewer`

### Phase 4: SPA Routing (Optional Enhancement)

If OHIF uses client-side routing (which it does), add fallback to index.html:

```rust
// In viewer plugin, use tower_http::services::ServeDir with fallback
// This requires enhancement to tower-http or a custom middleware

// Alternative: Wrap ServeDir with a middleware that serves index.html 
// for 404s on HTML extension misses (e.g., /viewer/studies → index.html)
```

---

## 10. SUMMARY TABLE

| Aspect | Current State | Files | Action |
|--------|---------------|-------|--------|
| **Plugin system** | ✅ Implemented | `crates/pacs-plugin/` | None needed |
| **Route merging** | ✅ Implemented | `crates/pacs-plugin/src/registry.rs:259` | None needed |
| **Middleware wrapping** | ✅ Implemented | `crates/pacs-plugin/src/registry.rs:270` | None needed |
| **Plugin registration** | ✅ Implemented | `crates/pacs-server/src/main.rs:17` | Add `use pacs_viewer_plugin as _;` |
| **Static file serving (tower-http)** | ✅ Available (not enabled) | `Cargo.toml:53` | Enable `"fs"` feature |
| **Config system** | ✅ Implemented | `crates/pacs-server/src/config.rs` | None needed |
| **DICOMweb API** | ✅ Complete | `crates/pacs-api/` | None needed |
| **OHIF viewer plugin** | ❌ Missing | NEW: `crates/pacs-viewer-plugin/` | **CREATE** |
| **Viewer plugin config example** | ❌ Missing | `config.toml.example` | **ADD** `[plugins.ohif-viewer]` |
| **Viewer deployment docs** | ❌ Missing | NEW: `DOCS/ohif-viewer-integration.md` | **CREATE** |

---

## 11. RISKS & MITIGATIONS

| Risk | Mitigation |
|------|-----------|
| SPA routing (client-side `/viewer/studies` → server returns index.html) | Use tower-http ServeDir with fallback middleware, or implement custom handler |
| OHIF requires specific API endpoints not implemented (e.g., auth) | OHIF v3+ is fully DICOMweb-compliant; existing QIDO/WADO/STOW endpoints sufficient for default build |
| Plugin path conflicts with existing routes | Use unique prefix (e.g., `/viewer`) instead of root `/` |
| Static assets not found at runtime | Validate `static_dir` existence in `init()` ✅ |
| CORS issues between OHIF (at `/viewer`) and API (at `/wado`, `/api`) | Existing `CorsLayer::permissive()` in router already allows all origins |
| Auth plugin (if enabled) blocks viewer | Add viewer paths to `public_paths` in basic-auth config, or exempt RoutePlugin paths from middleware |

---

## 12. RECOMMENDED NEXT STEPS

1. **Create `crates/pacs-viewer-plugin/` crate** with RoutePlugin implementation
2. **Enable `tower-http` `"fs"` feature** in workspace Cargo.toml
3. **Wire viewer plugin** into pacs-server binary
4. **Add config example** to `config.toml.example`
5. **Test with OHIF build** — download or build OHIF v3+ distribution
6. **Document deployment** in new `DOCS/ohif-viewer-integration.md`
7. **Optional:** Implement SPA routing fallback for client-side navigation
8. **Optional:** Add health check that verifies static assets on disk

This approach is **safe, non-invasive, and follows all existing patterns** in the codebase.
