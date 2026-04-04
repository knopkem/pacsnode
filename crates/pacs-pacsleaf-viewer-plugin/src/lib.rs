//! pacsnode pacsleaf viewer plugin.
//!
//! Serves the pacsleaf React application under a configurable route prefix and
//! exposes a generated runtime app-config payload for the frontend.

use std::{
    fs as stdfs,
    io::{Cursor, ErrorKind},
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
    RoutePlugin, ServerInfo,
};
use serde::Deserialize;
use tokio::fs;
use tower::ServiceExt;
use tower_http::services::ServeFile;
use tracing::{error, info, warn};

/// Compile-time plugin ID for the pacsleaf viewer plugin.
pub const PACSLEAF_VIEWER_PLUGIN_ID: &str = "pacsleaf-viewer";

const EMBEDDED_VIEWER_ARCHIVE: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/embedded-pacsleaf-viewer.zip"));
const EMBEDDED_VIEWER_BUNDLE_HASH: &str = env!("PACSNODE_EMBEDDED_PACSLEAF_VIEWER_BUNDLE_HASH");
const EMBEDDED_VIEWER_MARKER_FILE: &str = ".pacsnode-embedded-pacsleaf-viewer.sha256";

#[derive(Default)]
pub struct PacsleafViewerPlugin {
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
    #[serde(default = "default_generate_app_config")]
    generate_app_config: bool,
    #[serde(default = "default_provision_embedded_bundle")]
    provision_embedded_bundle: bool,
    #[serde(default = "default_streamer_url")]
    streamer_url: String,
    #[serde(default = "default_rendering_mode")]
    rendering_mode: String,
}

#[derive(Debug, Clone)]
struct ViewerRuntime {
    static_dir: PathBuf,
    route_prefix: String,
    viewer_root: String,
    redirect_root: bool,
    generate_app_config: bool,
    index_file: PathBuf,
    index_path: PathBuf,
    fallback_path: PathBuf,
    app_config_js: String,
}

enum AssetResolution {
    File(PathBuf),
    Fallback,
    GeneratedAppConfig,
    NotFound,
}

fn default_static_dir() -> String {
    "./web/pacsleaf-viewer/dist".into()
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

fn default_generate_app_config() -> bool {
    true
}

fn default_provision_embedded_bundle() -> bool {
    true
}

#[async_trait]
impl Plugin for PacsleafViewerPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            PACSLEAF_VIEWER_PLUGIN_ID,
            "Pacsleaf Viewer",
            env!("CARGO_PKG_VERSION"),
        )
        .disabled_by_default()
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        let config: ViewerPluginConfig =
            serde_json::from_value(ctx.config.clone()).map_err(|error| PluginError::Config {
                plugin_id: PACSLEAF_VIEWER_PLUGIN_ID.into(),
                message: error.to_string(),
            })?;
        let runtime = ViewerRuntime::build(config, &ctx.server_info)
            .await
            .map_err(|message| PluginError::Config {
                plugin_id: PACSLEAF_VIEWER_PLUGIN_ID.into(),
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

impl RoutePlugin for PacsleafViewerPlugin {
    fn routes(&self) -> Router<AppState> {
        let Some(runtime) = self.runtime.as_ref().map(Arc::clone) else {
            warn!(
                plugin_id = PACSLEAF_VIEWER_PLUGIN_ID,
                "Viewer routes requested before init"
            );
            return Router::new();
        };

        let viewer_root = runtime.viewer_root.clone();
        let route_prefix = runtime.route_prefix.clone();
        let viewer_root_route = viewer_root.clone();
        let mut router = Router::new()
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
    async fn build(config: ViewerPluginConfig, server_info: &ServerInfo) -> Result<Self, String> {
        let static_dir = normalize_static_dir(&config.static_dir)?;
        let route_prefix = normalize_route_prefix(&config.route_prefix)?;
        let index_file = normalize_relative_asset_path(&config.index_file, "index_file")?;
        let fallback_file = normalize_relative_asset_path(&config.fallback_file, "fallback_file")?;

        if config.provision_embedded_bundle {
            provision_embedded_viewer_bundle(&static_dir, &index_file).await?;
        }

        let runtime = Self {
            static_dir,
            route_prefix: route_prefix.clone(),
            viewer_root: format!("{route_prefix}/"),
            redirect_root: config.redirect_root,
            generate_app_config: config.generate_app_config,
            index_file,
            index_path: PathBuf::new(),
            fallback_path: PathBuf::new(),
            app_config_js: build_app_config_js(
                &route_prefix,
                server_info,
                &config.streamer_url,
                &config.rendering_mode,
            ),
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

        if self.generate_app_config && is_generated_app_config_path(requested_path) {
            return Ok(AssetResolution::GeneratedAppConfig);
        }

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
                plugin_id = PACSLEAF_VIEWER_PLUGIN_ID,
                error = %error,
                "Failed to resolve pacsleaf viewer asset"
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    match resolution {
        AssetResolution::File(path) => serve_file(runtime.as_ref(), path, request).await,
        AssetResolution::Fallback => {
            serve_file(runtime.as_ref(), runtime.fallback_path.clone(), request).await
        }
        AssetResolution::GeneratedAppConfig => serve_generated_app_config(runtime.as_ref()),
        AssetResolution::NotFound => StatusCode::NOT_FOUND.into_response(),
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

async fn serve_file(runtime: &ViewerRuntime, path: PathBuf, request: Request) -> Response {
    if is_html_file(&path) {
        return serve_html_file(runtime, &path).await;
    }

    match ServeFile::new(path).oneshot(request).await {
        Ok(response) => response.into_response(),
        Err(error) => match error {},
    }
}

fn serve_generated_app_config(runtime: &ViewerRuntime) -> Response {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        runtime.app_config_js.clone(),
    )
        .into_response()
}

async fn serve_html_file(runtime: &ViewerRuntime, path: &Path) -> Response {
    match fs::read(path).await {
        Ok(bytes) => {
            let contents = String::from_utf8_lossy(&bytes);
            Html(rewrite_html_asset_paths(&contents, &runtime.route_prefix)).into_response()
        }
        Err(error) => {
            error!(
                plugin_id = PACSLEAF_VIEWER_PLUGIN_ID,
                path = %path.display(),
                error = %error,
                "Failed to read pacsleaf viewer HTML file"
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

async fn provision_embedded_viewer_bundle(
    static_dir: &Path,
    index_file: &Path,
) -> Result<(), String> {
    let static_dir = static_dir.to_path_buf();
    let index_file = index_file.to_path_buf();

    tokio::task::spawn_blocking(move || {
        provision_embedded_viewer_bundle_sync(&static_dir, &index_file)
    })
    .await
    .map_err(|error| format!("failed to provision embedded pacsleaf viewer bundle: {error}"))?
}

fn provision_embedded_viewer_bundle_sync(
    static_dir: &Path,
    index_file: &Path,
) -> Result<(), String> {
    match determine_embedded_viewer_provision_action(static_dir, index_file)? {
        EmbeddedViewerProvisionAction::Skip => Ok(()),
        EmbeddedViewerProvisionAction::Provision { clear_existing } => {
            stdfs::create_dir_all(static_dir).map_err(|error| {
                format!(
                    "failed to create embedded viewer directory {}: {error}",
                    static_dir.display()
                )
            })?;

            if clear_existing {
                clear_directory_contents(static_dir)?;
            }

            extract_embedded_viewer_bundle(static_dir)?;
            info!(
                plugin_id = PACSLEAF_VIEWER_PLUGIN_ID,
                path = %static_dir.display(),
                "Provisioned embedded pacsleaf viewer bundle"
            );
            Ok(())
        }
    }
}

enum EmbeddedViewerProvisionAction {
    Skip,
    Provision { clear_existing: bool },
}

fn determine_embedded_viewer_provision_action(
    static_dir: &Path,
    index_file: &Path,
) -> Result<EmbeddedViewerProvisionAction, String> {
    match stdfs::metadata(static_dir) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(format!(
                "static_dir is not a directory: {}",
                static_dir.display()
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(EmbeddedViewerProvisionAction::Provision {
                clear_existing: false,
            });
        }
        Err(error) => {
            return Err(format!(
                "failed to read static_dir {}: {error}",
                static_dir.display()
            ));
        }
    }

    let index_path = static_dir.join(index_file);
    let marker_path = static_dir.join(EMBEDDED_VIEWER_MARKER_FILE);
    if let Some(hash) = read_embedded_viewer_marker(&marker_path)? {
        if hash == EMBEDDED_VIEWER_BUNDLE_HASH && index_path.is_file() {
            return Ok(EmbeddedViewerProvisionAction::Skip);
        }
        return Ok(EmbeddedViewerProvisionAction::Provision {
            clear_existing: true,
        });
    }

    if directory_has_only_placeholder_entries(static_dir)? {
        return Ok(EmbeddedViewerProvisionAction::Provision {
            clear_existing: false,
        });
    }

    Ok(EmbeddedViewerProvisionAction::Skip)
}

fn read_embedded_viewer_marker(path: &Path) -> Result<Option<String>, String> {
    match stdfs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents.trim().to_string())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "failed to read embedded viewer marker {}: {error}",
            path.display()
        )),
    }
}

fn directory_has_only_placeholder_entries(path: &Path) -> Result<bool, String> {
    let entries = stdfs::read_dir(path)
        .map_err(|error| format!("failed to read static_dir {}: {error}", path.display()))?;

    for entry in entries {
        let entry = entry
            .map_err(|error| format!("failed to inspect static_dir {}: {error}", path.display()))?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if file_name.starts_with('.') {
            continue;
        }
        return Ok(false);
    }

    Ok(true)
}

fn clear_directory_contents(path: &Path) -> Result<(), String> {
    let entries = stdfs::read_dir(path)
        .map_err(|error| format!("failed to read static_dir {}: {error}", path.display()))?;

    for entry in entries {
        let entry = entry
            .map_err(|error| format!("failed to inspect static_dir {}: {error}", path.display()))?;
        let entry_path = entry.path();
        let metadata = entry
            .metadata()
            .map_err(|error| format!("failed to inspect {}: {error}", entry_path.display()))?;
        if metadata.is_dir() {
            stdfs::remove_dir_all(&entry_path)
                .map_err(|error| format!("failed to remove {}: {error}", entry_path.display()))?;
        } else {
            stdfs::remove_file(&entry_path)
                .map_err(|error| format!("failed to remove {}: {error}", entry_path.display()))?;
        }
    }

    Ok(())
}

fn extract_embedded_viewer_bundle(static_dir: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(EMBEDDED_VIEWER_ARCHIVE))
        .map_err(|error| format!("failed to read embedded viewer archive: {error}"))?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            format!("failed to read embedded viewer archive entry {index}: {error}")
        })?;
        let relative_path = entry.enclosed_name().ok_or_else(|| {
            format!(
                "embedded viewer archive entry has an unsafe path: {}",
                entry.name()
            )
        })?;
        let output_path = static_dir.join(relative_path);

        if entry.is_dir() {
            stdfs::create_dir_all(&output_path)
                .map_err(|error| format!("failed to create {}: {error}", output_path.display()))?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            stdfs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }

        let mut output = stdfs::File::create(&output_path)
            .map_err(|error| format!("failed to create {}: {error}", output_path.display()))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| format!("failed to write {}: {error}", output_path.display()))?;
    }

    stdfs::write(
        static_dir.join(EMBEDDED_VIEWER_MARKER_FILE),
        format!("{EMBEDDED_VIEWER_BUNDLE_HASH}\n"),
    )
    .map_err(|error| {
        format!(
            "failed to write embedded viewer marker in {}: {error}",
            static_dir.display()
        )
    })?;

    Ok(())
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

fn is_generated_app_config_path(path: &str) -> bool {
    path == "app-config.js"
}

fn default_streamer_url() -> String {
    std::env::var("PACSNODE_PACSLEAF_STREAMER_URL")
        .or_else(|_| std::env::var("PACSLEAF_STREAMER_URL"))
        .unwrap_or_else(|_| "http://localhost:43120".into())
}

fn default_rendering_mode() -> String {
    std::env::var("PACSNODE_PACSLEAF_RENDERING_MODE")
        .unwrap_or_else(|_| "streaming".into())
}

fn build_app_config_js(
    route_prefix: &str,
    server_info: &ServerInfo,
    streamer_url: &str,
    rendering_mode: &str,
) -> String {
    let mode = match rendering_mode {
        "client" | "streaming" => rendering_mode,
        _ => "streaming",
    };

    let config = serde_json::json!({
        "appName": "pacsleaf",
        "routerBasename": route_prefix,
        "routes": {
            "studies": "/studies",
            "viewer": "/viewer/:studyUid",
            "settings": "/settings"
        },
        "dicomweb": {
            "qidoRoot": "/wado",
            "wadoRoot": "/wado",
            "wadoUriRoot": "/wado"
        },
        "restApiRoot": "/api",
        "rendering": {
            "defaultMode": mode
        },
        "streaming": {
            "defaultUrl": streamer_url,
            "defaultQuality": "balanced"
        },
        "viewer": {
            "autoSelectFirstSeries": true,
            "showMetadataRail": true
        },
        "server": {
            "aeTitle": server_info.ae_title,
            "httpPort": server_info.http_port,
            "dicomPort": server_info.dicom_port,
            "version": server_info.version
        }
    });

    format!(
        "window.__PACSLEAF_CONFIG__ = {};\n",
        serde_json::to_string_pretty(&config)
            .expect("generated pacsleaf app config is serializable")
    )
}

register_plugin!(PacsleafViewerPlugin::default);

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use super::*;
    use axum::http::HeaderValue;

    static NEXT_TEST_DIR: AtomicUsize = AtomicUsize::new(0);

    struct TestViewerDir {
        root: PathBuf,
    }

    impl TestViewerDir {
        fn new() -> Self {
            let id = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("test-artifacts")
                .join(format!("pacsleaf-viewer-plugin-{id}"));
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

    fn test_server_info() -> ServerInfo {
        ServerInfo {
            ae_title: "PACSNODE".into(),
            http_port: 8042,
            dicom_port: 4242,
            version: "test",
        }
    }

    fn plugin_context(config: serde_json::Value) -> PluginContext {
        PluginContext {
            config,
            metadata_store: None,
            blob_store: None,
            server_info: test_server_info(),
            event_bus: Arc::new(pacs_plugin::EventBus::default()),
        }
    }

    #[test]
    fn manifest_has_expected_id() {
        let plugin = PacsleafViewerPlugin::default();
        let manifest = plugin.manifest();
        assert_eq!(manifest.id, PACSLEAF_VIEWER_PLUGIN_ID);
        assert!(!manifest.enabled_by_default);
    }

    #[tokio::test]
    async fn init_rejects_missing_static_dir_when_provisioning_is_disabled() {
        let mut plugin = PacsleafViewerPlugin::default();
        let dir = TestViewerDir::new();
        let missing_dir = dir.root().join("missing-viewer");
        let error = plugin
            .init(&plugin_context(serde_json::json!({
                "static_dir": missing_dir,
                "provision_embedded_bundle": false,
            })))
            .await
            .unwrap_err();
        assert!(matches!(error, PluginError::Config { .. }));
    }

    #[tokio::test]
    async fn init_provisions_embedded_bundle_into_missing_static_dir() {
        let mut plugin = PacsleafViewerPlugin::default();
        let dir = TestViewerDir::new();
        let static_dir = dir.root().join("embedded-viewer");

        plugin
            .init(&plugin_context(serde_json::json!({
                "static_dir": static_dir,
            })))
            .await
            .expect("embedded viewer bundle should provision");

        assert!(static_dir.join("index.html").is_file());
        assert_eq!(
            stdfs::read_to_string(static_dir.join(EMBEDDED_VIEWER_MARKER_FILE))
                .unwrap()
                .trim(),
            EMBEDDED_VIEWER_BUNDLE_HASH
        );
    }

    #[tokio::test]
    async fn init_rejects_root_route_prefix() {
        let dir = TestViewerDir::new();
        dir.write("index.html", "<html>viewer</html>");
        let mut plugin = PacsleafViewerPlugin::default();
        let error = plugin
            .init(&plugin_context(serde_json::json!({
                "static_dir": dir.root(),
                "route_prefix": "/",
                "provision_embedded_bundle": false,
            })))
            .await
            .unwrap_err();
        assert!(matches!(error, PluginError::Config { .. }));
    }

    #[tokio::test]
    async fn runtime_resolves_root_request_to_index_file() {
        let dir = TestViewerDir::new();
        dir.write("index.html", "<html>viewer</html>");
        let runtime = ViewerRuntime::build(
            ViewerPluginConfig {
                static_dir: dir.root().display().to_string(),
                route_prefix: "/viewer".into(),
                redirect_root: false,
                index_file: "index.html".into(),
                fallback_file: "index.html".into(),
                generate_app_config: true,
                provision_embedded_bundle: false,
                streamer_url: default_streamer_url(),
                rendering_mode: default_rendering_mode(),
            },
            &test_server_info(),
        )
        .await
        .unwrap();

        let resolution = runtime
            .resolve_asset(None, &HeaderMap::new())
            .await
            .unwrap();
        assert!(matches!(resolution, AssetResolution::File(path) if path == runtime.index_path));
    }

    #[tokio::test]
    async fn runtime_falls_back_to_index_for_spa_navigation() {
        let dir = TestViewerDir::new();
        dir.write("index.html", "<html>viewer</html>");
        let runtime = ViewerRuntime::build(
            ViewerPluginConfig {
                static_dir: dir.root().display().to_string(),
                route_prefix: "/viewer".into(),
                redirect_root: false,
                index_file: "index.html".into(),
                fallback_file: "index.html".into(),
                generate_app_config: true,
                provision_embedded_bundle: false,
                streamer_url: default_streamer_url(),
                rendering_mode: default_rendering_mode(),
            },
            &test_server_info(),
        )
        .await
        .unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("text/html"));
        let resolution = runtime
            .resolve_asset(Some("viewer/1.2.840.10008"), &headers)
            .await
            .unwrap();
        assert!(matches!(resolution, AssetResolution::Fallback));
    }

    #[tokio::test]
    async fn runtime_exposes_generated_app_config() {
        let dir = TestViewerDir::new();
        dir.write("index.html", "<html>viewer</html>");
        let runtime = ViewerRuntime::build(
            ViewerPluginConfig {
                static_dir: dir.root().display().to_string(),
                route_prefix: "/viewer".into(),
                redirect_root: false,
                index_file: "index.html".into(),
                fallback_file: "index.html".into(),
                generate_app_config: true,
                provision_embedded_bundle: false,
                streamer_url: default_streamer_url(),
                rendering_mode: default_rendering_mode(),
            },
            &test_server_info(),
        )
        .await
        .unwrap();

        let resolution = runtime
            .resolve_asset(Some("app-config.js"), &HeaderMap::new())
            .await
            .unwrap();
        assert!(matches!(resolution, AssetResolution::GeneratedAppConfig));
        assert!(runtime.app_config_js.contains("__PACSLEAF_CONFIG__"));
        assert!(runtime
            .app_config_js
            .contains(r#""routerBasename": "/viewer""#));
        assert!(runtime.app_config_js.contains(r#""qidoRoot": "/wado""#));
        assert!(runtime
            .app_config_js
            .contains(r#""viewer": "/viewer/:studyUid""#));
    }

    #[test]
    fn html_rewriting_prefixes_root_assets_for_plugin_route() {
        let contents = r#"<link href="/assets/index.css"><script src="/app-config.js"></script>"#;
        let rewritten = rewrite_html_asset_paths(contents, "/viewer");
        assert!(rewritten.contains(r#"/viewer/assets/index.css"#));
        assert!(rewritten.contains(r#"/viewer/app-config.js"#));
    }

    #[test]
    fn generated_app_config_contains_expected_routes() {
        let app_config = build_app_config_js(
            "/viewer",
            &test_server_info(),
            "http://localhost:43120",
            "streaming",
        );
        assert!(app_config.contains("__PACSLEAF_CONFIG__"));
        assert!(app_config.contains(r#""studies": "/studies""#));
        assert!(app_config.contains(r#""settings": "/settings""#));
        assert!(app_config.contains(r#""httpPort": 8042"#));
    }
}
