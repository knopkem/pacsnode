use std::sync::Arc;

use async_trait::async_trait;
use pacs_core::MetadataStore;
use pacs_plugin::{
    MetadataStorePlugin, Plugin, PluginContext, PluginError, PluginHealth, PluginManifest,
};
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;

use crate::PgMetadataStore;

/// Compile-time plugin ID for the PostgreSQL metadata store.
pub const PG_METADATA_STORE_PLUGIN_ID: &str = "pg-metadata-store";

/// Plugin wrapper for the PostgreSQL metadata store.
#[derive(Default)]
pub struct PgMetadataStorePlugin {
    store: Option<Arc<PgMetadataStore>>,
}

#[derive(Debug, Deserialize)]
struct PgMetadataStorePluginConfig {
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

#[async_trait]
impl Plugin for PgMetadataStorePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            PG_METADATA_STORE_PLUGIN_ID,
            "PostgreSQL Metadata Store",
            env!("CARGO_PKG_VERSION"),
        )
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        let config: PgMetadataStorePluginConfig = serde_json::from_value(ctx.config.clone())
            .map_err(|error| PluginError::Config {
                plugin_id: PG_METADATA_STORE_PLUGIN_ID.into(),
                message: error.to_string(),
            })?;

        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .connect(&config.url)
            .await
            .map_err(|source| PluginError::InitFailed {
                plugin_id: PG_METADATA_STORE_PLUGIN_ID.into(),
                source: Box::new(source),
            })?;

        if config.run_migrations {
            sqlx::migrate!("../../migrations")
                .run(&pool)
                .await
                .map_err(|source| PluginError::InitFailed {
                    plugin_id: PG_METADATA_STORE_PLUGIN_ID.into(),
                    source: Box::new(source),
                })?;
        }

        self.store = Some(Arc::new(PgMetadataStore::new(pool)));
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

impl MetadataStorePlugin for PgMetadataStorePlugin {
    fn metadata_store(&self) -> Result<Arc<dyn MetadataStore>, PluginError> {
        self.store
            .as_ref()
            .map(|store| Arc::clone(store) as Arc<dyn MetadataStore>)
            .ok_or_else(|| PluginError::NotInitialized {
                plugin_id: PG_METADATA_STORE_PLUGIN_ID.into(),
                capability: "MetadataStore".into(),
            })
    }
}

pacs_plugin::register_plugin!(PgMetadataStorePlugin::default);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_expected_id() {
        let plugin = PgMetadataStorePlugin::default();
        assert_eq!(plugin.manifest().id, PG_METADATA_STORE_PLUGIN_ID);
    }
}
