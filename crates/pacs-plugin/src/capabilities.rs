use std::{future::Future, pin::Pin, sync::Arc};

use async_trait::async_trait;
use axum::Router;
use dicom_toolkit_data::DataSet;
use dicom_toolkit_net::services::provider::{
    FindEvent, GetEvent, MoveEvent, RetrieveItem, StoreEvent, StoreResult,
};
use pacs_core::{BlobStore, MetadataStore};

use crate::{error::PluginError, event::PacsEvent, plugin::Plugin, state::AppState};

/// Plugin capability for providing the active metadata store.
pub trait MetadataStorePlugin: Plugin {
    /// Returns the metadata store once the plugin is initialized.
    fn metadata_store(&self) -> Result<Arc<dyn MetadataStore>, PluginError>;
}

/// Plugin capability for providing the active blob store.
pub trait BlobStorePlugin: Plugin {
    /// Returns the blob store once the plugin is initialized.
    fn blob_store(&self) -> Result<Arc<dyn BlobStore>, PluginError>;
}

/// Object-safe boxed future type used by DIMSE handler capabilities.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Object-safe handler for C-STORE SCP callbacks.
pub trait StoreScpHandler: Send + Sync {
    /// Handles a C-STORE request.
    fn handle_store(&self, event: StoreEvent) -> BoxFuture<'_, StoreResult>;
}

/// Object-safe handler for C-FIND SCP callbacks.
pub trait FindScpHandler: Send + Sync {
    /// Handles a C-FIND request.
    fn handle_find(&self, event: FindEvent) -> BoxFuture<'_, Vec<DataSet>>;
}

/// Object-safe handler for C-GET SCP callbacks.
pub trait GetScpHandler: Send + Sync {
    /// Handles a C-GET request.
    fn handle_get(&self, event: GetEvent) -> BoxFuture<'_, Vec<RetrieveItem>>;
}

/// Object-safe handler for C-MOVE SCP callbacks.
pub trait MoveScpHandler: Send + Sync {
    /// Handles a C-MOVE request.
    fn handle_move(&self, event: MoveEvent) -> BoxFuture<'_, Vec<RetrieveItem>>;
}

/// Plugin capability for providing a C-STORE SCP handler.
pub trait StoreScpPlugin: Plugin {
    /// Creates the handler used by the DIMSE server.
    fn store_scp_handler(
        &self,
        plugins: Arc<crate::PluginRegistry>,
    ) -> Result<Arc<dyn StoreScpHandler>, PluginError>;
}

/// Plugin capability for providing a C-FIND SCP handler.
pub trait FindScpPlugin: Plugin {
    /// Creates the handler used by the DIMSE server.
    fn find_scp_handler(
        &self,
        plugins: Arc<crate::PluginRegistry>,
    ) -> Result<Arc<dyn FindScpHandler>, PluginError>;
}

/// Plugin capability for providing a C-GET SCP handler.
pub trait GetScpPlugin: Plugin {
    /// Creates the handler used by the DIMSE server.
    fn get_scp_handler(
        &self,
        plugins: Arc<crate::PluginRegistry>,
    ) -> Result<Arc<dyn GetScpHandler>, PluginError>;
}

/// Plugin capability for providing a C-MOVE SCP handler.
pub trait MoveScpPlugin: Plugin {
    /// Creates the handler used by the DIMSE server.
    fn move_scp_handler(
        &self,
        plugins: Arc<crate::PluginRegistry>,
    ) -> Result<Arc<dyn MoveScpHandler>, PluginError>;
}

/// Plugin capability for contributing additional HTTP routes.
pub trait RoutePlugin: Plugin {
    /// Returns a router that will be merged into the main application router.
    fn routes(&self) -> Router<AppState>;
}

/// Plugin capability for wrapping the HTTP router with middleware.
pub trait MiddlewarePlugin: Plugin {
    /// Applies the middleware to the router and returns the wrapped router.
    fn apply(&self, router: Router<AppState>) -> Router<AppState>;

    /// Middleware priority. Lower values wrap the router first.
    fn priority(&self) -> i32 {
        50
    }
}

/// Event categories that plugins can subscribe to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// A new instance was persisted.
    InstanceStored,
    /// A study has been fully received.
    StudyComplete,
    /// A resource was deleted.
    ResourceDeleted,
    /// A DIMSE association was accepted.
    AssociationOpened,
    /// A DIMSE association was rejected after negotiation.
    AssociationRejected,
    /// A DIMSE association ended.
    AssociationClosed,
    /// A QIDO-RS or C-FIND query was executed.
    QueryPerformed,
}

/// Plugin capability for reacting to emitted system events.
#[async_trait]
pub trait EventPlugin: Plugin {
    /// Returns the event kinds this plugin wants to receive.
    fn subscriptions(&self) -> Vec<EventKind>;

    /// Handles a single event notification.
    async fn on_event(&self, event: &PacsEvent) -> Result<(), PluginError>;
}

/// Plugin capability for providing transfer-syntax codecs.
pub trait CodecPlugin: Plugin {
    /// Transfer-syntax UIDs supported by the plugin.
    fn supported_transfer_syntaxes(&self) -> Vec<String>;

    /// Decodes compressed frames into uncompressed frame buffers.
    fn decode(&self, data: &[u8], transfer_syntax_uid: &str) -> Result<Vec<Vec<u8>>, PluginError>;

    /// Encodes uncompressed frames into the requested transfer syntax.
    fn encode(
        &self,
        frames: &[Vec<u8>],
        transfer_syntax_uid: &str,
        rows: u16,
        cols: u16,
        bits_allocated: u16,
        samples_per_pixel: u16,
    ) -> Result<Vec<u8>, PluginError>;
}

/// Plugin capability for processing DICOM datasets in-place.
pub trait ProcessingPlugin: Plugin {
    /// Returns the unique processor ID used to select this processor.
    fn processor_id(&self) -> &str;

    /// Applies the processing step to the dataset.
    fn process(&self, dataset: &mut DataSet, params: &serde_json::Value)
        -> Result<(), PluginError>;
}
