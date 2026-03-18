use std::sync::Arc;
use std::{path::PathBuf, str::FromStr};

use async_trait::async_trait;
use pacs_core::MetadataStore;
use pacs_plugin::{
    MetadataStorePlugin, Plugin, PluginContext, PluginError, PluginHealth, PluginManifest,
};
use serde::Deserialize;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::SqliteMetadataStore;

/// Compile-time plugin ID for the SQLite metadata store.
pub const SQLITE_METADATA_STORE_PLUGIN_ID: &str = "sqlite-metadata-store";

/// Plugin wrapper for the SQLite metadata store.
#[derive(Default)]
pub struct SqliteMetadataStorePlugin {
    store: Option<Arc<SqliteMetadataStore>>,
}

#[derive(Debug, Clone, Deserialize)]
struct SqliteMetadataStorePluginConfig {
    url: String,
    #[serde(default = "default_max_connections")]
    max_connections: u32,
    #[serde(default = "default_true")]
    run_migrations: bool,
}

fn default_max_connections() -> u32 {
    20
}

fn default_true() -> bool {
    true
}

fn sqlite_file_path(url: &str) -> Option<PathBuf> {
    let path = url.trim().strip_prefix("sqlite://")?;
    if path.is_empty() || path == ":memory:" {
        return None;
    }
    Some(PathBuf::from(path))
}

fn ensure_sqlite_parent_directory(url: &str) -> Result<(), std::io::Error> {
    let Some(path) = sqlite_file_path(url) else {
        return Ok(());
    };
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(parent)
}

#[async_trait]
impl Plugin for SqliteMetadataStorePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            SQLITE_METADATA_STORE_PLUGIN_ID,
            "SQLite Metadata Store",
            env!("CARGO_PKG_VERSION"),
        )
        .disabled_by_default()
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        let config: SqliteMetadataStorePluginConfig = serde_json::from_value(ctx.config.clone())
            .map_err(|error| PluginError::Config {
                plugin_id: SQLITE_METADATA_STORE_PLUGIN_ID.into(),
                message: error.to_string(),
            })?;

        ensure_sqlite_parent_directory(&config.url).map_err(|source| PluginError::InitFailed {
            plugin_id: SQLITE_METADATA_STORE_PLUGIN_ID.into(),
            source: Box::new(source),
        })?;

        let options = SqliteConnectOptions::from_str(&config.url)
            .map_err(|source| PluginError::InitFailed {
                plugin_id: SQLITE_METADATA_STORE_PLUGIN_ID.into(),
                source: Box::new(source),
            })?
            .create_if_missing(true)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(config.max_connections)
            .connect_with(options)
            .await
            .map_err(|source| PluginError::InitFailed {
                plugin_id: SQLITE_METADATA_STORE_PLUGIN_ID.into(),
                source: Box::new(source),
            })?;

        if config.run_migrations {
            sqlx::migrate!("./migrations")
                .run(&pool)
                .await
                .map_err(|source| PluginError::InitFailed {
                    plugin_id: SQLITE_METADATA_STORE_PLUGIN_ID.into(),
                    source: Box::new(source),
                })?;
        }

        self.store = Some(Arc::new(SqliteMetadataStore::new(pool)));
        Ok(())
    }

    async fn health(&self) -> PluginHealth {
        if self.store.is_some() {
            PluginHealth::Healthy
        } else {
            PluginHealth::Unhealthy("plugin not initialized".into())
        }
    }

    fn as_metadata_store_plugin(&self) -> Option<&dyn MetadataStorePlugin> {
        Some(self)
    }
}

impl MetadataStorePlugin for SqliteMetadataStorePlugin {
    fn metadata_store(&self) -> Result<Arc<dyn MetadataStore>, PluginError> {
        self.store
            .as_ref()
            .map(|store| Arc::clone(store) as Arc<dyn MetadataStore>)
            .ok_or_else(|| PluginError::NotInitialized {
                plugin_id: SQLITE_METADATA_STORE_PLUGIN_ID.into(),
                capability: "MetadataStore".into(),
            })
    }
}

pacs_plugin::register_plugin!(SqliteMetadataStorePlugin::default);

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pacs_plugin::{EventBus, ServerInfo};
    use tempfile::TempDir;

    use super::*;

    fn test_context(tempdir: &TempDir) -> PluginContext {
        let path = tempdir.path().join("metadata.db");
        PluginContext {
            config: serde_json::json!({
                "url": format!("sqlite://{}", path.display()),
                "max_connections": 1,
                "run_migrations": true,
            }),
            metadata_store: None,
            blob_store: None,
            server_info: ServerInfo {
                ae_title: "PACSNODE".into(),
                http_port: 8042,
                dicom_port: 4242,
                version: "test",
            },
            event_bus: Arc::new(EventBus::default()),
        }
    }

    #[test]
    fn manifest_has_expected_id() {
        let plugin = SqliteMetadataStorePlugin::default();
        assert_eq!(plugin.manifest().id, SQLITE_METADATA_STORE_PLUGIN_ID);
    }

    #[tokio::test]
    async fn init_creates_metadata_store() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut plugin = SqliteMetadataStorePlugin::default();
        plugin
            .init(&test_context(&tempdir))
            .await
            .expect("sqlite plugin should initialize");

        let store = plugin.metadata_store().expect("metadata store");
        let stats = store.get_statistics().await.expect("stats");
        assert_eq!(stats.num_studies, 0);
    }

    #[tokio::test]
    async fn init_creates_missing_parent_directory_for_sqlite_db() {
        let tempdir = TempDir::new().expect("tempdir");
        let db_path = tempdir.path().join("data/metadata/metadata.db");
        let mut plugin = SqliteMetadataStorePlugin::default();
        let ctx = PluginContext {
            config: serde_json::json!({
                "url": format!("sqlite://{}", db_path.display()),
                "max_connections": 1,
                "run_migrations": true,
            }),
            metadata_store: None,
            blob_store: None,
            server_info: ServerInfo {
                ae_title: "PACSNODE".into(),
                http_port: 8042,
                dicom_port: 4242,
                version: "test",
            },
            event_bus: Arc::new(EventBus::default()),
        };

        plugin
            .init(&ctx)
            .await
            .expect("sqlite plugin should create parent directories");

        assert!(db_path.parent().unwrap().is_dir());
        assert!(db_path.is_file());
    }
}
