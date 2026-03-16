//! pacsnode — plugin traits, registry, and event dispatch.
//!
//! ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for
//! diagnostic or therapeutic purposes.
//!
//! This crate provides the compile-time plugin system for pacsnode. Plugins are
//! regular Rust crates that implement [`Plugin`] and optionally one or more
//! capability traits such as [`MetadataStorePlugin`] or [`EventPlugin`].
//!
//! # Example
//!
//! ```rust,ignore
//! use pacs_plugin::{Plugin, PluginContext, PluginManifest, register_plugin};
//!
//! #[derive(Default)]
//! struct ExamplePlugin;
//!
//! #[async_trait::async_trait]
//! impl Plugin for ExamplePlugin {
//!     fn manifest(&self) -> PluginManifest {
//!         PluginManifest::new("example-plugin", "Example Plugin", env!("CARGO_PKG_VERSION"))
//!     }
//!
//!     async fn init(&mut self, _ctx: &PluginContext) -> Result<(), pacs_plugin::PluginError> {
//!         Ok(())
//!     }
//! }
//!
//! register_plugin!(ExamplePlugin::default);
//! ```

mod auth;
mod capabilities;
mod context;
mod error;
mod event;
mod plugin;
mod registry;
mod state;

pub use auth::AuthenticatedUser;
pub use capabilities::{
    BlobStorePlugin, BoxFuture, CodecPlugin, EventKind, EventPlugin, FindScpHandler, FindScpPlugin,
    GetScpHandler, GetScpPlugin, MetadataStorePlugin, MiddlewarePlugin, MoveScpHandler,
    MoveScpPlugin, ProcessingPlugin, RoutePlugin, StoreScpHandler, StoreScpPlugin,
};
pub use context::PluginContext;
pub use error::PluginError;
pub use event::{EventBus, PacsEvent, QuerySource, ResourceLevel};
pub use plugin::{Plugin, PluginHealth, PluginManifest};
pub use registry::{PluginRegistration, PluginRegistry};
pub use state::{AppState, ServerInfo};

/// Re-export of `inventory` so the registration macro works in dependent crates.
pub use inventory;

/// Registers a plugin factory for compile-time discovery.
///
/// The factory must be a zero-argument function or constructor that returns the
/// concrete plugin type. The macro wraps it in a `Box<dyn Plugin>`.
#[macro_export]
macro_rules! register_plugin {
    ($factory:path) => {
        $crate::inventory::submit! {
            $crate::PluginRegistration {
                create: || Box::new($factory()),
            }
        }
    };
}
