use std::sync::Arc;

use async_trait::async_trait;
use pacs_core::BlobStore;
use pacs_plugin::{
    BlobStorePlugin, Plugin, PluginContext, PluginError, PluginHealth, PluginManifest,
};

use crate::{S3BlobStore, StorageConfig};

/// Compile-time plugin ID for the S3-compatible blob store.
pub const S3_BLOB_STORE_PLUGIN_ID: &str = "s3-blob-store";

/// Plugin wrapper for the S3-compatible blob store.
#[derive(Default)]
pub struct S3BlobStorePlugin {
    store: Option<Arc<S3BlobStore>>,
}

#[async_trait]
impl Plugin for S3BlobStorePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            S3_BLOB_STORE_PLUGIN_ID,
            "S3 Blob Store",
            env!("CARGO_PKG_VERSION"),
        )
        .disabled_by_default()
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        let config: StorageConfig =
            serde_json::from_value(ctx.config.clone()).map_err(|error| PluginError::Config {
                plugin_id: S3_BLOB_STORE_PLUGIN_ID.into(),
                message: error.to_string(),
            })?;

        let store = S3BlobStore::new(&config).map_err(|source| PluginError::InitFailed {
            plugin_id: S3_BLOB_STORE_PLUGIN_ID.into(),
            source: Box::new(source),
        })?;
        self.store = Some(Arc::new(store));
        Ok(())
    }

    async fn health(&self) -> PluginHealth {
        if self.store.is_some() {
            PluginHealth::Healthy
        } else {
            PluginHealth::Unhealthy("plugin not initialized".into())
        }
    }

    fn as_blob_store_plugin(&self) -> Option<&dyn BlobStorePlugin> {
        Some(self)
    }
}

impl BlobStorePlugin for S3BlobStorePlugin {
    fn blob_store(&self) -> Result<Arc<dyn BlobStore>, PluginError> {
        self.store
            .as_ref()
            .map(|store| Arc::clone(store) as Arc<dyn BlobStore>)
            .ok_or_else(|| PluginError::NotInitialized {
                plugin_id: S3_BLOB_STORE_PLUGIN_ID.into(),
                capability: "BlobStore".into(),
            })
    }
}

pacs_plugin::register_plugin!(S3BlobStorePlugin::default);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_expected_id() {
        let plugin = S3BlobStorePlugin::default();
        assert_eq!(plugin.manifest().id, S3_BLOB_STORE_PLUGIN_ID);
    }
}
