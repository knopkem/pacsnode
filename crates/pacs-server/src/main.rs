//! pacsnode server entry point.
//!
//! ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for
//! diagnostic or therapeutic purposes.

use std::{collections::HashMap, sync::Arc};

use anyhow::{anyhow, Context, Result};
use pacs_plugin::{PluginRegistry, ServerInfo};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

mod config;

use config::{AppConfig, LogFormat};
use pacs_audit_plugin as _;
use pacs_auth_plugin as _;
use pacs_metrics_plugin as _;
use pacs_storage as _;
use pacs_store as _;

#[tokio::main]
async fn main() -> Result<()> {
    // ── Config ────────────────────────────────────────────────────────────────
    let cfg = AppConfig::load().context("failed to load configuration")?;

    // ── Tracing ───────────────────────────────────────────────────────────────
    init_tracing(&cfg.logging.level, &cfg.logging.format);

    info!(
        http_port  = cfg.server.http_port,
        dicom_port = cfg.server.dicom_port,
        ae_title   = %cfg.server.ae_title,
        "pacsnode starting"
    );
    let mut registry = PluginRegistry::new();
    if !cfg.plugins.enabled.is_empty() {
        registry.set_enabled(cfg.plugins.enabled.clone());
    }
    registry
        .register_all_discovered()
        .context("failed to register compiled-in plugins")?;

    let server_info = ServerInfo {
        ae_title: cfg.server.ae_title.clone(),
        http_port: cfg.server.http_port,
        dicom_port: cfg.server.dicom_port,
        version: env!("CARGO_PKG_VERSION"),
    };
    let plugin_configs = build_plugin_configs(&cfg)?;
    registry
        .init_all(server_info.clone(), &plugin_configs)
        .await
        .context("failed to initialize plugins")?;
    let registry = Arc::new(registry);

    let meta_store = registry
        .metadata_store()
        .ok_or_else(|| anyhow!("no MetadataStore plugin is active"))?;
    let blob_store = registry
        .blob_store()
        .ok_or_else(|| anyhow!("no BlobStore plugin is active"))?;

    // ── HTTP server ───────────────────────────────────────────────────────────
    let app_state = pacs_api::AppState {
        server_info: server_info.clone(),
        store: meta_store.clone(),
        blobs: blob_store.clone(),
        plugins: Arc::clone(&registry),
    };
    let router = registry
        .apply_middleware(pacs_api::build_router_without_state().merge(registry.merged_routes()))
        .with_state(app_state);

    let http_addr = format!("0.0.0.0:{}", cfg.server.http_port);
    let http_listener = TcpListener::bind(&http_addr)
        .await
        .with_context(|| format!("failed to bind HTTP port {}", cfg.server.http_port))?;
    info!(addr = %http_addr, "HTTP server listening");

    // ── DIMSE server ──────────────────────────────────────────────────────────
    let dimse_config = pacs_dimse::DimseConfig {
        ae_title: cfg.server.ae_title.clone(),
        port: cfg.server.dicom_port,
        max_associations: cfg.server.max_associations,
        timeout_secs: cfg.server.dimse_timeout_secs,
    };
    let dicom_server = Arc::new(pacs_dimse::DicomServer::with_plugins(
        dimse_config,
        meta_store,
        blob_store,
        Some(Arc::clone(&registry)),
    ));
    let shutdown_token = CancellationToken::new();
    let shutdown_token2 = shutdown_token.clone();

    // ── Start both servers concurrently ───────────────────────────────────────
    tokio::select! {
        result = axum::serve(http_listener, router)
            .with_graceful_shutdown(ctrl_c_signal()) =>
        {
            if let Err(e) = result {
                warn!(error = %e, "HTTP server exited with error");
            }
        }
        result = dicom_server.serve(shutdown_token) => {
            if let Err(e) = result {
                warn!(error = %e, "DIMSE server exited with error");
            }
        }
        _ = ctrl_c_signal() => {
            info!("shutdown signal received");
            shutdown_token2.cancel();
        }
    }

    registry
        .shutdown_all()
        .await
        .context("failed to shut down plugins")?;

    info!("pacsnode shut down");
    Ok(())
}

fn build_plugin_configs(cfg: &AppConfig) -> Result<HashMap<String, serde_json::Value>> {
    let mut configs = cfg.plugins.configs.clone();

    let mut db_config = serde_json::to_value(&cfg.database).context("serialize database config")?;
    if let Some(override_value) = configs.remove("pg-metadata-store") {
        merge_json(&mut db_config, override_value);
    }
    configs.insert("pg-metadata-store".into(), db_config);

    let mut storage_config =
        serde_json::to_value(&cfg.storage).context("serialize storage config")?;
    if let Some(override_value) = configs.remove("s3-blob-store") {
        merge_json(&mut storage_config, override_value);
    }
    configs.insert("s3-blob-store".into(), storage_config);

    let mut audit_config = serde_json::to_value(&cfg.database).context("serialize audit config")?;
    if let Some(override_value) = configs.remove("audit-logger") {
        merge_json(&mut audit_config, override_value);
    }
    configs.insert("audit-logger".into(), audit_config);

    Ok(configs)
}

fn merge_json(base: &mut serde_json::Value, overlay: serde_json::Value) {
    match (base, overlay) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                match base_map.get_mut(&key) {
                    Some(existing) => merge_json(existing, value),
                    None => {
                        base_map.insert(key, value);
                    }
                }
            }
        }
        (base_slot, overlay_value) => *base_slot = overlay_value,
    }
}

/// Initialise the global [`tracing`] subscriber.
fn init_tracing(level: &str, format: &LogFormat) {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    match format {
        LogFormat::Json => {
            fmt()
                .json()
                .with_env_filter(filter)
                .with_target(true)
                .init();
        }
        LogFormat::Pretty => {
            fmt()
                .pretty()
                .with_env_filter(filter)
                .with_target(true)
                .init();
        }
    }
}

/// Resolves when SIGINT / Ctrl+C is received.
async fn ctrl_c_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
}
