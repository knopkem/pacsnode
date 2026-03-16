use std::sync::Arc;

use async_trait::async_trait;
use pacs_core::{BlobStore, MetadataStore};
use pacs_plugin::{
    register_plugin, FindScpPlugin, GetScpPlugin, MoveScpPlugin, Plugin, PluginContext,
    PluginError, PluginManifest, PluginRegistry, StoreScpHandler, StoreScpPlugin,
};

use crate::server::provider::{PacsQueryProvider, PacsStoreProvider};

const METADATA_STORE_DEPENDENCY: &str = "pg-metadata-store";
const BLOB_STORE_DEPENDENCY: &str = "s3-blob-store";

/// Built-in plugin ID for the default pacsnode C-STORE SCP handler.
pub const PACS_STORE_SCP_PLUGIN_ID: &str = "pacs-store-scp";

/// Built-in plugin ID for the default pacsnode query/retrieve SCP handler.
pub const PACS_QUERY_SCP_PLUGIN_ID: &str = "pacs-query-scp";

/// Built-in plugin that exposes pacsnode's default C-STORE SCP handler.
#[derive(Default)]
pub struct PacsStoreScpPlugin {
    store: Option<Arc<dyn MetadataStore>>,
    blobs: Option<Arc<dyn BlobStore>>,
}

#[async_trait]
impl Plugin for PacsStoreScpPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            PACS_STORE_SCP_PLUGIN_ID,
            "pacsnode C-STORE SCP",
            env!("CARGO_PKG_VERSION"),
        )
        .with_dependencies([METADATA_STORE_DEPENDENCY, BLOB_STORE_DEPENDENCY])
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        self.store =
            Some(
                ctx.metadata_store
                    .clone()
                    .ok_or_else(|| PluginError::MissingDependency {
                        plugin_id: PACS_STORE_SCP_PLUGIN_ID.into(),
                        dependency: "metadata-store".into(),
                    })?,
            );
        self.blobs =
            Some(
                ctx.blob_store
                    .clone()
                    .ok_or_else(|| PluginError::MissingDependency {
                        plugin_id: PACS_STORE_SCP_PLUGIN_ID.into(),
                        dependency: "blob-store".into(),
                    })?,
            );
        Ok(())
    }

    fn as_store_scp_plugin(&self) -> Option<&dyn StoreScpPlugin> {
        Some(self)
    }
}

impl StoreScpPlugin for PacsStoreScpPlugin {
    fn store_scp_handler(
        &self,
        plugins: Arc<PluginRegistry>,
    ) -> Result<Arc<dyn StoreScpHandler>, PluginError> {
        let store = Arc::clone(self.store.as_ref().ok_or_else(|| PluginError::Runtime {
            plugin_id: PACS_STORE_SCP_PLUGIN_ID.into(),
            message: "plugin not initialized".into(),
        })?);
        let blobs = Arc::clone(self.blobs.as_ref().ok_or_else(|| PluginError::Runtime {
            plugin_id: PACS_STORE_SCP_PLUGIN_ID.into(),
            message: "plugin not initialized".into(),
        })?);

        Ok(Arc::new(PacsStoreProvider::with_plugins(
            store,
            blobs,
            Some(plugins),
        )))
    }
}

/// Built-in plugin that exposes pacsnode's default C-FIND/C-GET/C-MOVE SCP handlers.
#[derive(Default)]
pub struct PacsQueryScpPlugin {
    store: Option<Arc<dyn MetadataStore>>,
    blobs: Option<Arc<dyn BlobStore>>,
}

#[async_trait]
impl Plugin for PacsQueryScpPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            PACS_QUERY_SCP_PLUGIN_ID,
            "pacsnode Query/Retrieve SCP",
            env!("CARGO_PKG_VERSION"),
        )
        .with_dependencies([METADATA_STORE_DEPENDENCY, BLOB_STORE_DEPENDENCY])
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        self.store =
            Some(
                ctx.metadata_store
                    .clone()
                    .ok_or_else(|| PluginError::MissingDependency {
                        plugin_id: PACS_QUERY_SCP_PLUGIN_ID.into(),
                        dependency: "metadata-store".into(),
                    })?,
            );
        self.blobs =
            Some(
                ctx.blob_store
                    .clone()
                    .ok_or_else(|| PluginError::MissingDependency {
                        plugin_id: PACS_QUERY_SCP_PLUGIN_ID.into(),
                        dependency: "blob-store".into(),
                    })?,
            );
        Ok(())
    }

    fn as_find_scp_plugin(&self) -> Option<&dyn FindScpPlugin> {
        Some(self)
    }

    fn as_get_scp_plugin(&self) -> Option<&dyn GetScpPlugin> {
        Some(self)
    }

    fn as_move_scp_plugin(&self) -> Option<&dyn MoveScpPlugin> {
        Some(self)
    }
}

impl PacsQueryScpPlugin {
    fn build_provider(
        &self,
        plugins: Arc<PluginRegistry>,
    ) -> Result<PacsQueryProvider, PluginError> {
        let store = Arc::clone(self.store.as_ref().ok_or_else(|| PluginError::Runtime {
            plugin_id: PACS_QUERY_SCP_PLUGIN_ID.into(),
            message: "plugin not initialized".into(),
        })?);
        let blobs = Arc::clone(self.blobs.as_ref().ok_or_else(|| PluginError::Runtime {
            plugin_id: PACS_QUERY_SCP_PLUGIN_ID.into(),
            message: "plugin not initialized".into(),
        })?);

        Ok(PacsQueryProvider::with_plugins(store, blobs, Some(plugins)))
    }
}

impl FindScpPlugin for PacsQueryScpPlugin {
    fn find_scp_handler(
        &self,
        plugins: Arc<PluginRegistry>,
    ) -> Result<Arc<dyn pacs_plugin::FindScpHandler>, PluginError> {
        Ok(Arc::new(self.build_provider(plugins)?))
    }
}

impl GetScpPlugin for PacsQueryScpPlugin {
    fn get_scp_handler(
        &self,
        plugins: Arc<PluginRegistry>,
    ) -> Result<Arc<dyn pacs_plugin::GetScpHandler>, PluginError> {
        Ok(Arc::new(self.build_provider(plugins)?))
    }
}

impl MoveScpPlugin for PacsQueryScpPlugin {
    fn move_scp_handler(
        &self,
        plugins: Arc<PluginRegistry>,
    ) -> Result<Arc<dyn pacs_plugin::MoveScpHandler>, PluginError> {
        Ok(Arc::new(self.build_provider(plugins)?))
    }
}

register_plugin!(PacsStoreScpPlugin::default);
register_plugin!(PacsQueryScpPlugin::default);

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use pacs_core::{
        AuditLogEntry, AuditLogPage, AuditLogQuery, BlobStore, DicomJson, DicomNode, Instance,
        InstanceQuery, MetadataStore, PacsError, PacsResult, PacsStatistics, Series, SeriesQuery,
        SeriesUid, SopInstanceUid, Study, StudyQuery, StudyUid,
    };
    use pacs_plugin::{EventBus, ServerInfo};

    use super::*;

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
    }

    #[derive(Default)]
    struct NoopBlobStore;

    #[async_trait]
    impl BlobStore for NoopBlobStore {
        async fn put(&self, _key: &str, _data: Bytes) -> PacsResult<()> {
            Ok(())
        }

        async fn get(&self, _key: &str) -> PacsResult<Bytes> {
            Ok(Bytes::new())
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

    fn plugin_context() -> PluginContext {
        PluginContext {
            config: Default::default(),
            metadata_store: Some(Arc::new(NoopMetadataStore)),
            blob_store: Some(Arc::new(NoopBlobStore)),
            server_info: ServerInfo {
                ae_title: "PACSNODE".into(),
                http_port: 8042,
                dicom_port: 4242,
                version: "test",
            },
            event_bus: Arc::new(EventBus::default()),
        }
    }

    #[tokio::test]
    async fn store_plugin_initializes_and_creates_handler() {
        let mut plugin = PacsStoreScpPlugin::default();
        plugin
            .init(&plugin_context())
            .await
            .expect("store plugin should initialize");

        let handler = plugin
            .store_scp_handler(Arc::new(PluginRegistry::new()))
            .expect("store handler should be created");

        let _ = handler;
    }

    #[tokio::test]
    async fn query_plugin_initializes_and_creates_all_handlers() {
        let mut plugin = PacsQueryScpPlugin::default();
        plugin
            .init(&plugin_context())
            .await
            .expect("query plugin should initialize");

        let registry = Arc::new(PluginRegistry::new());
        let _ = plugin
            .find_scp_handler(Arc::clone(&registry))
            .expect("find handler should be created");
        let _ = plugin
            .get_scp_handler(Arc::clone(&registry))
            .expect("get handler should be created");
        let _ = plugin
            .move_scp_handler(registry)
            .expect("move handler should be created");
    }
}
