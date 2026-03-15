//! pacsnode server entry point.
//!
//! ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for
//! diagnostic or therapeutic purposes.

use std::sync::Arc;

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

mod config;

use config::{AppConfig, LogFormat};

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

    // ── Database ──────────────────────────────────────────────────────────────
    let pool = PgPoolOptions::new()
        .max_connections(cfg.database.max_connections)
        .connect(&cfg.database.url)
        .await
        .context("failed to connect to PostgreSQL")?;

    if cfg.database.run_migrations {
        info!("running database migrations");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .context("database migration failed")?;
        info!("migrations complete");
    }

    // ── Blob store ────────────────────────────────────────────────────────────
    let storage_config = pacs_storage::StorageConfig {
        endpoint:   cfg.storage.endpoint.clone(),
        bucket:     cfg.storage.bucket.clone(),
        access_key: cfg.storage.access_key.clone(),
        secret_key: cfg.storage.secret_key.clone(),
        region:     cfg.storage.region.clone(),
    };
    let blob_store: Arc<dyn pacs_core::BlobStore> = Arc::new(
        pacs_storage::S3BlobStore::new(&storage_config)
            .context("failed to build S3 blob store")?,
    );

    // ── Metadata store ────────────────────────────────────────────────────────
    let meta_store: Arc<dyn pacs_core::MetadataStore> =
        Arc::new(pacs_store::PgMetadataStore::new(pool.clone()));

    // ── HTTP server ───────────────────────────────────────────────────────────
    let app_state = pacs_api::AppState {
        server_info: pacs_api::ServerInfo {
            ae_title: cfg.server.ae_title.clone(),
            http_port: cfg.server.http_port,
            dicom_port: cfg.server.dicom_port,
            version: env!("CARGO_PKG_VERSION"),
        },
        store: meta_store.clone(),
        blobs: blob_store.clone(),
        nodes: Arc::new(tokio::sync::RwLock::new(vec![])),
    };
    let router = pacs_api::build_router(app_state);

    let http_addr = format!("0.0.0.0:{}", cfg.server.http_port);
    let http_listener = TcpListener::bind(&http_addr)
        .await
        .with_context(|| format!("failed to bind HTTP port {}", cfg.server.http_port))?;
    info!(addr = %http_addr, "HTTP server listening");

    // ── DIMSE server ──────────────────────────────────────────────────────────
    let dimse_config = pacs_dimse::DimseConfig {
        ae_title:         cfg.server.ae_title.clone(),
        port:             cfg.server.dicom_port,
        max_associations: cfg.server.max_associations,
        timeout_secs:     cfg.server.dimse_timeout_secs,
    };
    let dicom_server = Arc::new(pacs_dimse::DicomServer::new(
        dimse_config,
        meta_store,
        blob_store,
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

    info!("pacsnode shut down");
    Ok(())
}

/// Initialise the global [`tracing`] subscriber.
fn init_tracing(level: &str, format: &LogFormat) {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    match format {
        LogFormat::Json => {
            fmt().json().with_env_filter(filter).with_target(true).init();
        }
        LogFormat::Pretty => {
            fmt().pretty().with_env_filter(filter).with_target(true).init();
        }
    }
}

/// Resolves when SIGINT / Ctrl+C is received.
async fn ctrl_c_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
}
