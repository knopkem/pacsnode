//! pacsnode admin dashboard plugin.
//!
//! Provides a server-rendered operations UI backed by Askama templates,
//! HTMX, and server-sent events.

mod import;
mod runtime;
mod web;

use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use pacs_plugin::{
    register_plugin, AppState, EventKind, EventPlugin, Plugin, PluginContext, PluginError,
    PluginHealth, PluginManifest, RoutePlugin, METADATA_STORE_CAPABILITY_DEPENDENCY,
};
use runtime::{AdminPluginConfig, AdminRuntime};
use tracing::warn;

pub const ADMIN_DASHBOARD_PLUGIN_ID: &str = "admin-dashboard";

#[derive(Default)]
struct AdminDashboardPlugin {
    runtime: Option<Arc<AdminRuntime>>,
}

#[async_trait]
impl Plugin for AdminDashboardPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            ADMIN_DASHBOARD_PLUGIN_ID,
            "Admin Dashboard",
            env!("CARGO_PKG_VERSION"),
        )
        .with_dependencies([METADATA_STORE_CAPABILITY_DEPENDENCY])
        .disabled_by_default()
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        let config: AdminPluginConfig =
            serde_json::from_value(ctx.config.clone()).map_err(|error| PluginError::Config {
                plugin_id: ADMIN_DASHBOARD_PLUGIN_ID.into(),
                message: error.to_string(),
            })?;

        let metadata_store = ctx.metadata_store.as_ref().map(Arc::clone).ok_or_else(|| {
            PluginError::MissingDependency {
                plugin_id: ADMIN_DASHBOARD_PLUGIN_ID.into(),
                dependency: METADATA_STORE_CAPABILITY_DEPENDENCY.into(),
            }
        })?;

        self.runtime = Some(Arc::new(AdminRuntime::new(
            config,
            ctx.server_info.clone(),
            metadata_store,
        )?));
        Ok(())
    }

    async fn health(&self) -> PluginHealth {
        if self.runtime.is_some() {
            PluginHealth::Healthy
        } else {
            PluginHealth::Unhealthy("plugin not initialized".into())
        }
    }

    fn as_route_plugin(&self) -> Option<&dyn RoutePlugin> {
        Some(self)
    }

    fn as_event_plugin(&self) -> Option<&dyn EventPlugin> {
        Some(self)
    }
}

impl RoutePlugin for AdminDashboardPlugin {
    fn routes(&self) -> Router<AppState> {
        let Some(runtime) = self.runtime.as_ref().map(Arc::clone) else {
            warn!(
                plugin_id = ADMIN_DASHBOARD_PLUGIN_ID,
                "Admin dashboard routes requested before init"
            );
            return Router::new();
        };

        web::routes(runtime)
    }
}

#[async_trait]
impl EventPlugin for AdminDashboardPlugin {
    fn subscriptions(&self) -> Vec<EventKind> {
        vec![
            EventKind::InstanceStored,
            EventKind::StudyComplete,
            EventKind::ResourceDeleted,
            EventKind::AssociationOpened,
            EventKind::AssociationRejected,
            EventKind::AssociationClosed,
            EventKind::QueryPerformed,
        ]
    }

    async fn on_event(&self, event: &pacs_plugin::PacsEvent) -> Result<(), PluginError> {
        let Some(runtime) = &self.runtime else {
            return Err(PluginError::NotInitialized {
                plugin_id: ADMIN_DASHBOARD_PLUGIN_ID.into(),
                capability: "EventPlugin".into(),
            });
        };

        runtime.record_event(event).await;
        Ok(())
    }
}

register_plugin!(AdminDashboardPlugin::default);
