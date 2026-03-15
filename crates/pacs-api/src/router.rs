//! Axum router construction for the pacsnode HTTP server.

use std::time::Duration;

use axum::{
    http::StatusCode,
    routing::{delete, get},
    Router,
};
use tower_http::{
    compression::CompressionLayer, cors::CorsLayer, limit::RequestBodyLimitLayer,
    timeout::TimeoutLayer, trace::TraceLayer,
};

use crate::{
    routes::{health, qido, rest, stow, wado},
    state::AppState,
};

/// Constructs the full Axum [`Router`] with all DICOMweb and REST routes,
/// middleware layers applied, and the shared [`AppState`] wired in.
///
/// # Middleware stack (outermost → innermost)
///
/// - [`TraceLayer`] — HTTP request/response tracing
/// - [`CorsLayer`] — permissive CORS headers
/// - [`TimeoutLayer`] — 30-second request timeout
/// - [`RequestBodyLimitLayer`] — 500 MiB body size limit
/// - [`CompressionLayer`] — response compression
pub fn build_router(state: AppState) -> Router {
    Router::new()
        // ── Health / Stats ────────────────────────────────────────────────────
        .route("/health", get(health::get_health))
        .route("/statistics", get(health::get_statistics))
        .route("/system", get(health::get_system_info))
        // ── STOW-RS ───────────────────────────────────────────────────────────
        // POST /wado/studies shares the path with QIDO GET below
        .route(
            "/wado/studies",
            get(qido::search_studies).post(stow::stow_store),
        )
        // ── QIDO-RS ───────────────────────────────────────────────────────────
        .route(
            "/wado/studies/{study_uid}/series",
            get(qido::search_series),
        )
        .route(
            "/wado/studies/{study_uid}/series/{series_uid}/instances",
            get(qido::search_instances),
        )
        // ── WADO-RS metadata ──────────────────────────────────────────────────
        .route(
            "/wado/studies/{study_uid}/metadata",
            get(wado::study_metadata),
        )
        .route(
            "/wado/studies/{study_uid}/series/{series_uid}/metadata",
            get(wado::series_metadata),
        )
        .route(
            "/wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/metadata",
            get(wado::instance_metadata),
        )
        // ── WADO-RS retrieve ──────────────────────────────────────────────────
        .route("/wado/studies/{study_uid}", get(wado::retrieve_study))
        .route(
            "/wado/studies/{study_uid}/series/{series_uid}",
            get(wado::retrieve_series),
        )
        .route(
            "/wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}",
            get(wado::retrieve_instance),
        )
        // ── REST: studies ─────────────────────────────────────────────────────
        .route("/api/studies", get(rest::studies::list_studies))
        .route(
            "/api/studies/{study_uid}",
            get(rest::studies::get_study).delete(rest::studies::delete_study),
        )
        // ── REST: series ──────────────────────────────────────────────────────
        .route(
            "/api/studies/{study_uid}/series",
            get(rest::series::list_series_for_study),
        )
        .route(
            "/api/series/{series_uid}",
            get(rest::series::get_series).delete(rest::series::delete_series),
        )
        // ── REST: instances ───────────────────────────────────────────────────
        .route(
            "/api/series/{series_uid}/instances",
            get(rest::instances::list_instances_for_series),
        )
        .route(
            "/api/instances/{instance_uid}",
            get(rest::instances::get_instance).delete(rest::instances::delete_instance),
        )
        // ── REST: nodes ───────────────────────────────────────────────────────
        .route(
            "/api/nodes",
            get(rest::nodes::list_nodes).post(rest::nodes::add_node),
        )
        .route("/api/nodes/{ae_title}", delete(rest::nodes::remove_node))
        // ── Middleware ────────────────────────────────────────────────────────
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(RequestBodyLimitLayer::new(500 * 1024 * 1024))
        .layer(CompressionLayer::new())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{make_test_state, MockBlobStr, MockMetaStore};

    #[test]
    fn test_build_router_does_not_panic() {
        let state = make_test_state(MockMetaStore::new(), MockBlobStr::new());
        let _ = build_router(state);
    }
}
