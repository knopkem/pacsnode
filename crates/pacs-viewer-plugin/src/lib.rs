//! pacsnode OHIF/static viewer plugin.
//!
//! Serves a static web viewer build such as OHIF under a configurable route
//! prefix and redirects `/` to that viewer entry point when enabled.

use std::{
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use axum::{
    extract::{Path as AxumPath, Request},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use pacs_plugin::{
    register_plugin, AppState, Plugin, PluginContext, PluginError, PluginHealth, PluginManifest,
    RoutePlugin,
};
use serde::Deserialize;
use tokio::fs;
use tower::ServiceExt;
use tower_http::services::ServeFile;
use tracing::{error, warn};

/// Compile-time plugin ID for the built-in OHIF/static viewer plugin.
pub const OHIF_VIEWER_PLUGIN_ID: &str = "ohif-viewer";

/// Serves a static viewer build under a configurable route prefix.
///
/// # Example
///
/// ```rust
/// use pacs_viewer_plugin::OhifViewerPlugin;
///
/// let plugin = OhifViewerPlugin::default();
/// let manifest = pacs_plugin::Plugin::manifest(&plugin);
/// assert_eq!(manifest.id, "ohif-viewer");
/// ```
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
    #[serde(default = "default_redirect_root")]
    redirect_root: bool,
    #[serde(default = "default_index_file")]
    index_file: String,
    #[serde(default = "default_fallback_file")]
    fallback_file: String,
}

#[derive(Debug, Clone)]
struct ViewerRuntime {
    static_dir: PathBuf,
    route_prefix: String,
    viewer_root: String,
    redirect_root: bool,
    index_file: PathBuf,
    index_path: PathBuf,
    fallback_path: PathBuf,
}

enum AssetResolution {
    File(PathBuf),
    Fallback,
    NotFound,
}

fn default_static_dir() -> String {
    "/opt/pacsnode/viewer".into()
}

fn default_route_prefix() -> String {
    "/viewer".into()
}

fn default_index_file() -> String {
    "index.html".into()
}

fn default_redirect_root() -> bool {
    true
}

fn default_fallback_file() -> String {
    "index.html".into()
}

#[async_trait]
impl Plugin for OhifViewerPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            OHIF_VIEWER_PLUGIN_ID,
            "OHIF Viewer",
            env!("CARGO_PKG_VERSION"),
        )
        .disabled_by_default()
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        let config: ViewerPluginConfig =
            serde_json::from_value(ctx.config.clone()).map_err(|error| PluginError::Config {
                plugin_id: OHIF_VIEWER_PLUGIN_ID.into(),
                message: error.to_string(),
            })?;
        let runtime =
            ViewerRuntime::build(config)
                .await
                .map_err(|message| PluginError::Config {
                    plugin_id: OHIF_VIEWER_PLUGIN_ID.into(),
                    message,
                })?;
        self.runtime = Some(Arc::new(runtime));
        Ok(())
    }

    async fn health(&self) -> PluginHealth {
        let Some(runtime) = &self.runtime else {
            return PluginHealth::Unhealthy("plugin not initialized".into());
        };
        match runtime.validate_layout().await {
            Ok(()) => PluginHealth::Healthy,
            Err(message) => PluginHealth::Unhealthy(message),
        }
    }

    fn as_route_plugin(&self) -> Option<&dyn RoutePlugin> {
        Some(self)
    }
}

impl RoutePlugin for OhifViewerPlugin {
    fn routes(&self) -> Router<AppState> {
        let Some(runtime) = self.runtime.as_ref().map(Arc::clone) else {
            warn!(
                plugin_id = OHIF_VIEWER_PLUGIN_ID,
                "Viewer routes requested before init"
            );
            return Router::new();
        };

        let viewer_root = runtime.viewer_root.clone();
        let route_prefix = runtime.route_prefix.clone();
        let viewer_root_route = viewer_root.clone();
        let mut router = Router::new()
            .route(
                "/assets/{*path}",
                get({
                    let runtime = Arc::clone(&runtime);
                    move |AxumPath(path): AxumPath<String>, request| {
                        serve_root_asset_alias_request(
                            Arc::clone(&runtime),
                            format!("assets/{path}"),
                            request,
                        )
                    }
                }),
            )
            .route(
                "/{file}",
                get({
                    let runtime = Arc::clone(&runtime);
                    move |AxumPath(file): AxumPath<String>, request| {
                        serve_root_asset_alias_request(Arc::clone(&runtime), file, request)
                    }
                }),
            )
            .route(
                &route_prefix,
                get({
                    let viewer_root = viewer_root.clone();
                    move || async move { Redirect::temporary(&viewer_root) }
                }),
            )
            .route(
                &viewer_root_route,
                get({
                    let runtime = Arc::clone(&runtime);
                    move |request| serve_viewer_request(Arc::clone(&runtime), None, request)
                }),
            )
            .route(
                &format!("{route_prefix}/{{*path}}"),
                get({
                    let runtime = Arc::clone(&runtime);
                    move |AxumPath(path), request| {
                        serve_viewer_request(Arc::clone(&runtime), Some(path), request)
                    }
                }),
            );

        if runtime.redirect_root {
            router = router.route(
                "/",
                get({
                    let viewer_root = viewer_root.clone();
                    move || async move { Redirect::temporary(&viewer_root) }
                }),
            );
        }

        router
    }
}

impl ViewerRuntime {
    async fn build(config: ViewerPluginConfig) -> Result<Self, String> {
        let static_dir = normalize_static_dir(&config.static_dir)?;
        let route_prefix = normalize_route_prefix(&config.route_prefix)?;
        let index_file = normalize_relative_asset_path(&config.index_file, "index_file")?;
        let fallback_file = normalize_relative_asset_path(&config.fallback_file, "fallback_file")?;

        let runtime = Self {
            static_dir,
            route_prefix: route_prefix.clone(),
            viewer_root: format!("{route_prefix}/"),
            redirect_root: config.redirect_root,
            index_path: PathBuf::new(),
            index_file,
            fallback_path: PathBuf::new(),
        };

        runtime.finish_build(fallback_file).await
    }

    async fn finish_build(mut self, fallback_file: PathBuf) -> Result<Self, String> {
        self.validate_directory().await?;
        self.index_path = self.static_dir.join(&self.index_file);
        self.fallback_path = self.static_dir.join(&fallback_file);
        validate_file(&self.index_path, "index_file").await?;
        validate_file(&self.fallback_path, "fallback_file").await?;
        Ok(self)
    }

    async fn validate_layout(&self) -> Result<(), String> {
        self.validate_directory().await?;
        validate_file(&self.index_path, "index_file").await?;
        validate_file(&self.fallback_path, "fallback_file").await?;
        Ok(())
    }

    async fn validate_directory(&self) -> Result<(), String> {
        match fs::metadata(&self.static_dir).await {
            Ok(metadata) if metadata.is_dir() => Ok(()),
            Ok(_) => Err(format!(
                "static_dir is not a directory: {}",
                self.static_dir.display()
            )),
            Err(error) if error.kind() == ErrorKind::NotFound => Err(format!(
                "static_dir does not exist: {}",
                self.static_dir.display()
            )),
            Err(error) => Err(format!(
                "failed to read static_dir {}: {error}",
                self.static_dir.display()
            )),
        }
    }

    async fn resolve_asset(
        &self,
        requested_path: Option<&str>,
        headers: &HeaderMap,
    ) -> Result<AssetResolution, std::io::Error> {
        let Some(requested_path) = requested_path.filter(|path| !path.is_empty()) else {
            return Ok(AssetResolution::File(self.index_path.clone()));
        };

        let Some(relative_path) = sanitize_relative_request_path(requested_path) else {
            return Ok(AssetResolution::NotFound);
        };
        let candidate = self.static_dir.join(relative_path);

        if let Some(file_path) = resolve_existing_asset(&candidate, &self.index_file).await? {
            return Ok(AssetResolution::File(file_path));
        }

        if accepts_html(headers) {
            Ok(AssetResolution::Fallback)
        } else {
            Ok(AssetResolution::NotFound)
        }
    }
}

async fn serve_viewer_request(
    runtime: Arc<ViewerRuntime>,
    requested_path: Option<String>,
    request: Request,
) -> Response {
    let resolution = match runtime
        .resolve_asset(requested_path.as_deref(), request.headers())
        .await
    {
        Ok(resolution) => resolution,
        Err(error) => {
            error!(
                plugin_id = OHIF_VIEWER_PLUGIN_ID,
                error = %error,
                "Failed to resolve viewer asset"
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    match resolution {
        AssetResolution::File(path) => serve_file(runtime.as_ref(), path, request).await,
        AssetResolution::Fallback => {
            serve_file(runtime.as_ref(), runtime.fallback_path.clone(), request).await
        }
        AssetResolution::NotFound => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve_root_asset_alias_request(
    runtime: Arc<ViewerRuntime>,
    requested_path: String,
    request: Request,
) -> Response {
    let request_path = format!("/{requested_path}");
    if !looks_like_static_asset_path(&request_path) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let Some(relative_path) = sanitize_relative_request_path(&requested_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let candidate = runtime.static_dir.join(relative_path);

    match resolve_existing_file(&candidate).await {
        Ok(Some(file_path)) => serve_file(runtime.as_ref(), file_path, request).await,
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            error!(
                plugin_id = OHIF_VIEWER_PLUGIN_ID,
                path = %candidate.display(),
                error = %error,
                "Failed to resolve root viewer asset alias"
            );
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn resolve_existing_asset(
    candidate: &Path,
    index_file: &Path,
) -> Result<Option<PathBuf>, std::io::Error> {
    match fs::metadata(candidate).await {
        Ok(metadata) if metadata.is_file() => Ok(Some(candidate.to_path_buf())),
        Ok(metadata) if metadata.is_dir() => {
            let index_candidate = candidate.join(index_file);
            match fs::metadata(&index_candidate).await {
                Ok(metadata) if metadata.is_file() => Ok(Some(index_candidate)),
                Ok(_) => Ok(None),
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        }
        Ok(_) => Ok(None),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

async fn resolve_existing_file(candidate: &Path) -> Result<Option<PathBuf>, std::io::Error> {
    match fs::metadata(candidate).await {
        Ok(metadata) if metadata.is_file() => Ok(Some(candidate.to_path_buf())),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

async fn serve_file(runtime: &ViewerRuntime, path: PathBuf, request: Request) -> Response {
    if is_html_file(&path) {
        return serve_html_file(runtime, &path).await;
    }

    match ServeFile::new(path).oneshot(request).await {
        Ok(response) => response.into_response(),
        Err(error) => match error {},
    }
}

async fn serve_html_file(runtime: &ViewerRuntime, path: &Path) -> Response {
    match fs::read(path).await {
        Ok(bytes) => {
            let contents = String::from_utf8_lossy(&bytes);
            Html(rewrite_html_asset_paths(&contents, &runtime.route_prefix)).into_response()
        }
        Err(error) => {
            error!(
                plugin_id = OHIF_VIEWER_PLUGIN_ID,
                path = %path.display(),
                error = %error,
                "Failed to read viewer HTML file"
            );
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn normalize_static_dir(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("static_dir cannot be empty".into());
    }
    Ok(PathBuf::from(trimmed))
}

fn normalize_route_prefix(path: &str) -> Result<String, String> {
    let normalized = normalize_path(path)?;
    if normalized == "/" {
        return Err("route_prefix must not be '/'".into());
    }
    Ok(normalized)
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

fn normalize_relative_asset_path(path: &str, field_name: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} cannot be empty"));
    }

    let mut normalized = PathBuf::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "{field_name} must stay within static_dir: {trimmed}"
                ));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(format!("{field_name} cannot be empty"));
    }

    Ok(normalized)
}

fn sanitize_relative_request_path(path: &str) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(normalized)
}

async fn validate_file(path: &Path, field_name: &str) -> Result<(), String> {
    match fs::metadata(path).await {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(format!("{field_name} is not a file: {}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            Err(format!("{field_name} does not exist: {}", path.display()))
        }
        Err(error) => Err(format!(
            "failed to read {field_name} {}: {error}",
            path.display()
        )),
    }
}

fn accepts_html(headers: &HeaderMap) -> bool {
    let Some(accept) = headers.get(header::ACCEPT) else {
        return true;
    };
    let Ok(accept) = accept.to_str() else {
        return false;
    };
    accept.contains("text/html") || accept.contains("application/xhtml+xml")
}

fn is_html_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "html" | "htm"))
}

fn rewrite_html_asset_paths(contents: &str, route_prefix: &str) -> String {
    let mut rewritten = contents.to_string();
    for (prefix, terminator) in [
        ("href=\"", '"'),
        ("href='", '\''),
        ("src=\"", '"'),
        ("src='", '\''),
        ("content=\"", '"'),
        ("content='", '\''),
    ] {
        rewritten = rewrite_attribute_values(&rewritten, prefix, terminator, route_prefix);
    }
    rewritten
}

fn rewrite_attribute_values(
    input: &str,
    attribute_prefix: &str,
    terminator: char,
    route_prefix: &str,
) -> String {
    let mut output = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start) = remaining.find(attribute_prefix) {
        let (before, after_start) = remaining.split_at(start);
        output.push_str(before);
        output.push_str(attribute_prefix);

        let value_start = &after_start[attribute_prefix.len()..];
        let Some(end) = value_start.find(terminator) else {
            output.push_str(value_start);
            return output;
        };

        let (value, after_value) = value_start.split_at(end);
        output.push_str(&rewrite_root_absolute_url(value, route_prefix));
        remaining = after_value;
    }

    output.push_str(remaining);
    output
}

fn rewrite_root_absolute_url(value: &str, route_prefix: &str) -> String {
    if route_prefix == "/"
        || !value.starts_with('/')
        || value.starts_with("//")
        || !looks_like_static_asset_path(value)
        || value == route_prefix
        || value == format!("{route_prefix}/")
        || value.starts_with(&format!("{route_prefix}/"))
    {
        return value.to_string();
    }

    format!("{route_prefix}{value}")
}

fn looks_like_static_asset_path(value: &str) -> bool {
    let clean_path = value.split(['?', '#']).next().unwrap_or(value);
    clean_path.starts_with("/assets/")
        || clean_path
            .rsplit('/')
            .next()
            .is_some_and(|segment| segment.contains('.'))
}

register_plugin!(OhifViewerPlugin::default);

#[cfg(test)]
mod tests {
    use std::{
        fs as stdfs,
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use super::*;
    use async_trait::async_trait;
    use axum::{body::to_bytes, body::Body, http::Request};
    use bytes::Bytes;
    use pacs_core::{
        AuditLogEntry, AuditLogPage, AuditLogQuery, BlobStore, DicomJson, DicomNode, Instance,
        InstanceQuery, MetadataStore, PacsError, PacsResult, PacsStatistics, Series, SeriesQuery,
        SeriesUid, SopInstanceUid, Study, StudyQuery, StudyUid,
    };
    use tower::ServiceExt;

    static NEXT_TEST_DIR: AtomicUsize = AtomicUsize::new(0);

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

        async fn store_audit_log(&self, _entry: &pacs_core::NewAuditLogEntry) -> PacsResult<()> {
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

    struct TestViewerDir {
        root: PathBuf,
    }

    impl TestViewerDir {
        fn new() -> Self {
            let id = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!("pacsnode-viewer-plugin-{id}"));
            let _ = stdfs::remove_dir_all(&root);
            stdfs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn root(&self) -> &Path {
            &self.root
        }

        fn write(&self, relative_path: &str, contents: &str) {
            let path = self.root.join(relative_path);
            if let Some(parent) = path.parent() {
                stdfs::create_dir_all(parent).unwrap();
            }
            stdfs::write(path, contents).unwrap();
        }
    }

    impl Drop for TestViewerDir {
        fn drop(&mut self) {
            let _ = stdfs::remove_dir_all(&self.root);
        }
    }

    fn plugin_context(config: serde_json::Value) -> PluginContext {
        PluginContext {
            config,
            metadata_store: None,
            blob_store: None,
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

    async fn init_plugin(dir: &TestViewerDir) -> OhifViewerPlugin {
        let mut plugin = OhifViewerPlugin::default();
        plugin
            .init(&plugin_context(serde_json::json!({
                "static_dir": dir.root(),
            })))
            .await
            .unwrap();
        plugin
    }

    #[test]
    fn manifest_has_expected_id() {
        let plugin = OhifViewerPlugin::default();
        let manifest = plugin.manifest();
        assert_eq!(manifest.id, OHIF_VIEWER_PLUGIN_ID);
        assert!(!manifest.enabled_by_default);
    }

    #[tokio::test]
    async fn init_rejects_missing_static_dir() {
        let mut plugin = OhifViewerPlugin::default();
        let error = plugin
            .init(&plugin_context(serde_json::json!({
                "static_dir": "/path/that/does/not/exist",
            })))
            .await
            .unwrap_err();
        assert!(matches!(error, PluginError::Config { .. }));
    }

    #[tokio::test]
    async fn init_rejects_root_route_prefix() {
        let dir = TestViewerDir::new();
        dir.write("index.html", "<html>viewer</html>");
        let mut plugin = OhifViewerPlugin::default();
        let error = plugin
            .init(&plugin_context(serde_json::json!({
                "static_dir": dir.root(),
                "route_prefix": "/",
            })))
            .await
            .unwrap_err();
        assert!(matches!(error, PluginError::Config { .. }));
    }

    #[tokio::test]
    async fn root_redirects_to_viewer_prefix() {
        let dir = TestViewerDir::new();
        dir.write("index.html", "<html>viewer</html>");
        let plugin = init_plugin(&dir).await;
        let app = plugin.routes().with_state(app_state());

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/viewer/"
        );
    }

    #[tokio::test]
    async fn redirect_root_can_be_disabled() {
        let dir = TestViewerDir::new();
        dir.write("index.html", "<html>viewer</html>");

        let mut plugin = OhifViewerPlugin::default();
        plugin
            .init(&plugin_context(serde_json::json!({
                "static_dir": dir.root(),
                "redirect_root": false,
            })))
            .await
            .unwrap();
        let app = plugin.routes().with_state(app_state());

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn viewer_root_serves_index_file() {
        let dir = TestViewerDir::new();
        dir.write("index.html", "<html>viewer</html>");
        let plugin = init_plugin(&dir).await;
        let app = plugin.routes().with_state(app_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/viewer/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            "<html>viewer</html>"
        );
    }

    #[tokio::test]
    async fn spa_navigation_serves_fallback_file() {
        let dir = TestViewerDir::new();
        dir.write("index.html", "<html>viewer</html>");
        let plugin = init_plugin(&dir).await;
        let app = plugin.routes().with_state(app_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/viewer/studies/1.2.840.10008")
                    .header(header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            "<html>viewer</html>"
        );
    }

    #[tokio::test]
    async fn missing_asset_returns_not_found() {
        let dir = TestViewerDir::new();
        dir.write("index.html", "<html>viewer</html>");
        let plugin = init_plugin(&dir).await;
        let app = plugin.routes().with_state(app_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/viewer/missing.js")
                    .header(header::ACCEPT, "*/*")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn root_asset_alias_serves_generated_bundle_chunk() {
        let dir = TestViewerDir::new();
        dir.write("index.html", "<html>viewer</html>");
        dir.write(
            "6409.bundle.573c619db7f5fd651882.js",
            "console.log('chunk');",
        );
        let plugin = init_plugin(&dir).await;
        let app = plugin.routes().with_state(app_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/6409.bundle.573c619db7f5fd651882.js")
                    .header(header::ACCEPT, "*/*")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            "console.log('chunk');"
        );
    }

    #[tokio::test]
    async fn root_asset_alias_serves_assets_subdirectory_files() {
        let dir = TestViewerDir::new();
        dir.write("index.html", "<html>viewer</html>");
        dir.write("assets/android-chrome-144x144.png", "png-bytes");
        let plugin = init_plugin(&dir).await;
        let app = plugin.routes().with_state(app_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/assets/android-chrome-144x144.png")
                    .header(header::ACCEPT, "*/*")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.as_ref(), b"png-bytes");
    }

    #[tokio::test]
    async fn viewer_root_rewrites_root_absolute_shell_assets_to_route_prefix() {
        let dir = TestViewerDir::new();
        dir.write(
            "index.html",
            r#"<html>
<head>
  <link rel="manifest" href="/manifest.json">
  <link rel="icon" href="/assets/favicon.ico">
  <link rel="stylesheet" href="/app.bundle.css">
</head>
<body>
  <script src="/app-config.js"></script>
  <script src="/app.bundle.js"></script>
</body>
</html>"#,
        );
        let plugin = init_plugin(&dir).await;
        let app = plugin.routes().with_state(app_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/viewer/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains(r#"href="/viewer/manifest.json""#));
        assert!(body.contains(r#"href="/viewer/assets/favicon.ico""#));
        assert!(body.contains(r#"href="/viewer/app.bundle.css""#));
        assert!(body.contains(r#"src="/viewer/app-config.js""#));
        assert!(body.contains(r#"src="/viewer/app.bundle.js""#));
    }

    #[tokio::test]
    async fn viewer_root_preserves_non_asset_root_paths_and_existing_prefixes() {
        let dir = TestViewerDir::new();
        dir.write(
            "index.html",
            r#"<html>
<body>
  <a href="/dicom-web/studies">api</a>
  <script src="/viewer/app.bundle.js"></script>
</body>
</html>"#,
        );
        let plugin = init_plugin(&dir).await;
        let app = plugin.routes().with_state(app_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/viewer/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains(r#"href="/dicom-web/studies""#));
        assert!(body.contains(r#"src="/viewer/app.bundle.js""#));
        assert!(!body.contains(r#"src="/viewer/viewer/app.bundle.js""#));
    }
}
