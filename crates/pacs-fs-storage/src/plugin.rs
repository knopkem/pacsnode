use std::sync::Arc;

use async_trait::async_trait;
use pacs_core::BlobStore;
use pacs_plugin::{
    BlobStorePlugin, Plugin, PluginContext, PluginError, PluginHealth, PluginManifest,
};

use crate::{FilesystemStorageConfig, FsBlobStore};

/// Compile-time plugin ID for the filesystem blob store.
pub const FS_BLOB_STORE_PLUGIN_ID: &str = "filesystem-blob-store";

/// Plugin wrapper for the filesystem-backed blob store.
#[derive(Default)]
pub struct FsBlobStorePlugin {
    store: Option<Arc<FsBlobStore>>,
}

#[async_trait]
impl Plugin for FsBlobStorePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            FS_BLOB_STORE_PLUGIN_ID,
            "Filesystem Blob Store",
            env!("CARGO_PKG_VERSION"),
        )
        .disabled_by_default()
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        let config: FilesystemStorageConfig =
            serde_json::from_value(ctx.config.clone()).map_err(|error| PluginError::Config {
                plugin_id: FS_BLOB_STORE_PLUGIN_ID.into(),
                message: error.to_string(),
            })?;

        let store = FsBlobStore::new(&config).map_err(|source| PluginError::InitFailed {
            plugin_id: FS_BLOB_STORE_PLUGIN_ID.into(),
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

impl BlobStorePlugin for FsBlobStorePlugin {
    fn blob_store(&self) -> Result<Arc<dyn BlobStore>, PluginError> {
        self.store
            .as_ref()
            .map(|store| Arc::clone(store) as Arc<dyn BlobStore>)
            .ok_or_else(|| PluginError::NotInitialized {
                plugin_id: FS_BLOB_STORE_PLUGIN_ID.into(),
                capability: "BlobStore".into(),
            })
    }
}

pacs_plugin::register_plugin!(FsBlobStorePlugin::default);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_expected_id() {
        let plugin = FsBlobStorePlugin::default();
        assert_eq!(plugin.manifest().id, FS_BLOB_STORE_PLUGIN_ID);
    }
}
