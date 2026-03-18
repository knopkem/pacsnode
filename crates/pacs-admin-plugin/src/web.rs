use std::{
    collections::{BTreeSet, HashSet},
    convert::Infallible,
    sync::Arc,
    time::Duration,
};

use askama::Template;
use async_stream::stream;
use axum::{
    extract::{Extension, Form, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Redirect, Response,
    },
    routing::{delete, get, post},
    Router,
};
use pacs_core::{
    AuditLogEntry, AuditLogQuery, DicomNode, InstanceQuery, PacsError, SeriesQuery, ServerSettings,
    Study, StudyQuery, StudyUid,
};
use pacs_dicom::supported_retrieve_transfer_syntaxes;
use pacs_dimse::DicomClient;
use pacs_plugin::{AppState, PacsEvent, PluginHealth, ResourceLevel};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use tracing::{error, warn};
use url::form_urlencoded;

use crate::runtime::{ActivityEntry, AdminRuntime};

const ADMIN_CSS: &str = include_str!("../templates/static/admin.css");
const DEFAULT_STUDY_PAGE_SIZE: u32 = 20;
const MAX_STUDY_PAGE_SIZE: u32 = 100;
const DEFAULT_AUDIT_PAGE_SIZE: u32 = 25;
const MAX_AUDIT_PAGE_SIZE: u32 = 100;
const NODE_VERIFY_TIMEOUT_SECS: u64 = 5;

pub(crate) fn routes(runtime: Arc<AdminRuntime>) -> Router<AppState> {
    let route_prefix = runtime.route_prefix().to_string();
    let root_path = route_prefix.clone();
    let root_slash_path = format!("{route_prefix}/");
    let system_path = format!("{route_prefix}/system");
    let studies_path = format!("{route_prefix}/studies");
    let studies_list_path = format!("{route_prefix}/studies/list");
    let study_delete_path = format!("{route_prefix}/studies/{{study_uid}}");
    let nodes_path = format!("{route_prefix}/nodes");
    let node_edit_path = format!("{route_prefix}/nodes/{{ae_title}}/edit");
    let node_delete_path = format!("{route_prefix}/nodes/{{ae_title}}");
    let node_verify_path = format!("{route_prefix}/nodes/{{ae_title}}/verify");
    let audit_path = format!("{route_prefix}/audit");
    let audit_list_path = format!("{route_prefix}/audit/list");
    let events_path = format!("{route_prefix}/events");
    let css_path = format!("{route_prefix}/static/admin.css");

    let mut router = Router::new()
        .route(&root_path, get(dashboard_page))
        .route(&root_slash_path, get(dashboard_page))
        .route(&system_path, get(system_page).post(save_system_settings))
        .route(&studies_path, get(studies_page))
        .route(&studies_list_path, get(studies_results_fragment))
        .route(&study_delete_path, delete(delete_study))
        .route(&nodes_path, get(nodes_page).post(save_node))
        .route(&node_edit_path, get(edit_node))
        .route(&node_delete_path, delete(delete_node))
        .route(&node_verify_path, post(verify_node))
        .route(&audit_path, get(audit_page))
        .route(&audit_list_path, get(audit_results_fragment))
        .route(&events_path, get(events_stream))
        .route(&css_path, get(admin_css));

    if runtime.redirect_root() {
        let redirect_path = route_prefix.clone();
        router = router.route(
            "/",
            get(move || {
                let redirect_path = redirect_path.clone();
                async move { Redirect::temporary(&redirect_path) }
            }),
        );
    }

    router.layer(Extension(runtime))
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardPageTemplate {
    page_title: &'static str,
    route_prefix: String,
    active_nav: &'static str,
    server_info: ServerInfoView,
    stats_markup: String,
    recent_activity_markup: String,
}

#[derive(Template)]
#[template(path = "system.html")]
struct SystemPageTemplate {
    page_title: &'static str,
    route_prefix: String,
    active_nav: &'static str,
    server_info: ServerInfoView,
    settings_markup: String,
    plugin_rows: Vec<PluginHealthView>,
}

#[derive(Template)]
#[template(path = "fragments/system_settings_panel.html")]
struct SystemSettingsPanelTemplate {
    system_path: String,
    form: ServerSettingsFormView,
    flash: Option<FlashView>,
    source_label: String,
    restart_required: bool,
    syntax_options: Vec<TransferSyntaxOptionView>,
    preferred_syntax_order: Vec<PreferredTransferSyntaxItemView>,
}

#[derive(Template)]
#[template(path = "studies.html")]
struct StudiesPageTemplate {
    page_title: &'static str,
    route_prefix: String,
    active_nav: &'static str,
    studies_path: String,
    studies_results_path: String,
    filters: StudiesFilterView,
    results_markup: String,
}

#[derive(Template)]
#[template(path = "nodes.html")]
struct NodesPageTemplate {
    page_title: &'static str,
    route_prefix: String,
    active_nav: &'static str,
    nodes_markup: String,
}

#[derive(Template)]
#[template(path = "audit.html")]
struct AuditPageTemplate {
    page_title: &'static str,
    route_prefix: String,
    active_nav: &'static str,
    audit_path: String,
    audit_results_path: String,
    filters: AuditFilterView,
    results_markup: String,
}

#[derive(Template)]
#[template(path = "fragments/nodes_panel.html")]
struct NodesPanelTemplate {
    nodes_path: String,
    form: NodeFormView,
    flash: Option<FlashView>,
    rows: Vec<NodeRowView>,
    has_nodes: bool,
}

#[derive(Template)]
#[template(path = "fragments/audit_results.html")]
struct AuditResultsTemplate {
    rows: Vec<AuditRowView>,
    has_results: bool,
    has_active_filters: bool,
    result_summary: String,
    page_summary: String,
    empty_summary: String,
    page_href: String,
    has_prev: bool,
    prev_page_href: String,
    prev_results_href: String,
    has_next: bool,
    next_page_href: String,
    next_results_href: String,
}

#[derive(Template)]
#[template(path = "fragments/studies_results.html")]
struct StudiesResultsTemplate {
    rows: Vec<StudyRowView>,
    has_results: bool,
    has_active_filters: bool,
    result_summary: String,
    page_summary: String,
    empty_summary: String,
    page_href: String,
    has_prev: bool,
    prev_page_href: String,
    prev_results_href: String,
    has_next: bool,
    next_page_href: String,
    next_results_href: String,
}

#[derive(Template)]
#[template(path = "fragments/stats_cards.html")]
struct StatsCardsTemplate {
    cards: Vec<StatCardView>,
}

#[derive(Template)]
#[template(path = "fragments/recent_activity_items.html")]
struct RecentActivityItemsTemplate {
    entries: Vec<ActivityView>,
}

#[derive(Template)]
#[template(path = "fragments/toast.html")]
struct ToastTemplate {
    item: ActivityView,
}

#[derive(Clone)]
struct ServerInfoView {
    ae_title: String,
    http_port: u16,
    dicom_port: u16,
    version: String,
}

struct PluginHealthView {
    plugin_id: String,
    status_label: String,
    status_class: &'static str,
    detail: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct ServerSettingsFormInput {
    dicom_port: String,
    ae_title: String,
    ae_whitelist_enabled: Option<String>,
    #[serde(deserialize_with = "deserialize_one_or_many_strings")]
    accepted_transfer_syntaxes: Vec<String>,
    #[serde(deserialize_with = "deserialize_one_or_many_strings")]
    preferred_transfer_syntaxes: Vec<String>,
    max_associations: String,
    dimse_timeout_secs: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrManyStrings {
    One(String),
    Many(Vec<String>),
}

fn deserialize_one_or_many_strings<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<OneOrManyStrings>::deserialize(deserializer)?;
    Ok(match value {
        None => Vec::new(),
        Some(OneOrManyStrings::One(item)) => vec![item],
        Some(OneOrManyStrings::Many(items)) => items,
    })
}

fn parse_server_settings_form(body: &str) -> ServerSettingsFormInput {
    let mut input = ServerSettingsFormInput::default();

    for (key, value) in form_urlencoded::parse(body.as_bytes()) {
        let value = value.into_owned();
        match key.as_ref() {
            "dicom_port" => input.dicom_port = value,
            "ae_title" => input.ae_title = value,
            "ae_whitelist_enabled" => input.ae_whitelist_enabled = Some(value),
            "accepted_transfer_syntaxes" => input.accepted_transfer_syntaxes.push(value),
            "preferred_transfer_syntaxes" => input.preferred_transfer_syntaxes.push(value),
            "max_associations" => input.max_associations = value,
            "dimse_timeout_secs" => input.dimse_timeout_secs = value,
            _ => {}
        }
    }

    input
}

#[derive(Clone)]
struct ServerSettingsFormView {
    dicom_port: String,
    ae_title: String,
    ae_whitelist_enabled: bool,
    accepted_transfer_syntaxes: Vec<String>,
    preferred_transfer_syntaxes: Vec<String>,
    max_associations: String,
    dimse_timeout_secs: String,
}

struct TransferSyntaxOptionView {
    uid: String,
    label: String,
    is_required: bool,
    accepted_selected: bool,
}

struct PreferredTransferSyntaxItemView {
    uid: String,
    label: String,
    is_required: bool,
}

struct StatCardView {
    eyebrow: &'static str,
    value: String,
    detail: String,
}

#[derive(Clone)]
struct ActivityView {
    timestamp: String,
    badge: String,
    title: String,
    detail: String,
    tone_class: &'static str,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct StudiesFilters {
    patient_id: Option<String>,
    patient_name: Option<String>,
    study_uid: Option<String>,
    accession_number: Option<String>,
    modality: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
}

#[derive(Clone)]
struct StudiesFilterView {
    patient_id: String,
    patient_name: String,
    study_uid: String,
    accession_number: String,
    modality: String,
    page_size: u32,
}

struct StudyRowView {
    study_uid: String,
    patient_id: String,
    patient_name: String,
    study_date: String,
    accession_number: String,
    description: String,
    modalities: String,
    num_series: i32,
    num_instances: i32,
    delete_href: String,
}

struct StudiesResultsView {
    rows: Vec<StudyRowView>,
    has_results: bool,
    has_active_filters: bool,
    result_summary: String,
    page_summary: String,
    empty_summary: String,
    page_href: String,
    has_prev: bool,
    prev_page_href: String,
    prev_results_href: String,
    has_next: bool,
    next_page_href: String,
    next_results_href: String,
}

#[derive(Debug, Clone)]
struct FlashView {
    title: String,
    detail: String,
    tone_class: &'static str,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct NodeFormInput {
    ae_title: String,
    host: String,
    port: String,
    description: String,
    tls_enabled: Option<String>,
}

#[derive(Clone)]
struct NodeFormView {
    ae_title: String,
    host: String,
    port: String,
    description: String,
    tls_enabled: bool,
}

struct NodeRowView {
    ae_title: String,
    host: String,
    port: u16,
    description: String,
    tls_label: &'static str,
    tls_class: &'static str,
    delete_href: String,
    edit_href: String,
    verify_href: String,
    verification_state: &'static str,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AuditFilters {
    user_id: Option<String>,
    action: Option<String>,
    resource: Option<String>,
    resource_uid: Option<String>,
    status: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
}

#[derive(Clone)]
struct AuditFilterView {
    user_id: String,
    action: String,
    resource: String,
    resource_uid: String,
    status: String,
    page_size: u32,
}

struct AuditRowView {
    occurred_at: String,
    action: String,
    resource: String,
    resource_uid: String,
    status: String,
    status_class: &'static str,
    user_id: String,
    source_ip: String,
    details_pretty: String,
}

struct AuditResultsView {
    rows: Vec<AuditRowView>,
    has_results: bool,
    has_active_filters: bool,
    result_summary: String,
    page_summary: String,
    empty_summary: String,
    page_href: String,
    has_prev: bool,
    prev_page_href: String,
    prev_results_href: String,
    has_next: bool,
    next_page_href: String,
    next_results_href: String,
}

async fn dashboard_page(
    State(state): State<AppState>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
) -> Response {
    let stats_markup = match render_stats_markup(&runtime).await {
        Ok(markup) => markup,
        Err(status) => return error_response(status, "admin dashboard failed to query PACS state"),
    };
    let recent_activity_markup =
        match render_recent_activity_markup(runtime.recent_activity().await) {
            Ok(markup) => markup,
            Err(status) => return error_response(status, "admin dashboard failed to render"),
        };

    render_html(&DashboardPageTemplate {
        page_title: "Admin Dashboard",
        route_prefix: runtime.route_prefix().to_string(),
        active_nav: "dashboard",
        server_info: server_info_view(&state.server_info),
        stats_markup,
        recent_activity_markup,
    })
}

async fn system_page(
    State(state): State<AppState>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
) -> Response {
    let settings_markup = match render_system_settings_markup(&state, &runtime, None, None).await {
        Ok(markup) => markup,
        Err(status) => return error_response(status, "admin system settings failed to load"),
    };
    let plugin_rows = state
        .plugins
        .aggregate_health()
        .await
        .into_iter()
        .map(|(plugin_id, health)| plugin_health_view(plugin_id, health))
        .collect();

    render_html(&SystemPageTemplate {
        page_title: "System Overview",
        route_prefix: runtime.route_prefix().to_string(),
        active_nav: "system",
        server_info: server_info_view(&state.server_info),
        settings_markup,
        plugin_rows,
    })
}

async fn save_system_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    body: String,
) -> Response {
    let input = parse_server_settings_form(&body);
    let form = ServerSettingsFormView::from_input(&input);
    let settings = match input.into_settings() {
        Ok(settings) => settings,
        Err(flash) => {
            return render_system_response(&state, &runtime, &headers, Some(form), Some(flash))
                .await;
        }
    };

    if let Err(error) = state.store.upsert_server_settings(&settings).await {
        return render_system_response(
            &state,
            &runtime,
            &headers,
            Some(form),
            Some(store_error_flash("Settings update failed", &error)),
        )
        .await;
    }

    let flash = FlashView {
        title: "Settings saved".into(),
        detail: "DIMSE listener settings were persisted. Restart pacsnode to apply them to the active listener.".into(),
        tone_class: "flash-success",
    };
    render_system_response(&state, &runtime, &headers, None, Some(flash)).await
}

async fn studies_page(
    State(state): State<AppState>,
    Query(filters): Query<StudiesFilters>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
) -> Response {
    let results_markup = match render_studies_results_markup(&state, &runtime, &filters).await {
        Ok(markup) => markup,
        Err(status) => return error_response(status, "admin studies browser failed to render"),
    };

    render_html(&StudiesPageTemplate {
        page_title: "Studies",
        route_prefix: runtime.route_prefix().to_string(),
        active_nav: "studies",
        studies_path: studies_page_path(runtime.route_prefix()),
        studies_results_path: studies_results_path(runtime.route_prefix()),
        filters: StudiesFilterView::from_filters(&filters),
        results_markup,
    })
}

async fn studies_results_fragment(
    State(state): State<AppState>,
    Query(filters): Query<StudiesFilters>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
) -> Response {
    match render_studies_results_markup(&state, &runtime, &filters).await {
        Ok(markup) => html_markup_response(markup),
        Err(status) => error_response(status, "admin studies browser failed to load results"),
    }
}

async fn delete_study(
    State(state): State<AppState>,
    Path(study_uid): Path<String>,
    Query(filters): Query<StudiesFilters>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
) -> Response {
    let study_uid = StudyUid::from(study_uid.as_str());
    let blob_keys = match collect_study_blob_keys(&state, &study_uid).await {
        Ok(blob_keys) => blob_keys,
        Err(status) => {
            return error_response(status, "admin studies browser failed to collect blobs")
        }
    };

    if let Err(error) = state.store.delete_study(&study_uid).await {
        return error_response(
            pacs_error_to_status(&error),
            "admin studies browser failed to delete study",
        );
    }

    cleanup_blob_keys(&state, blob_keys).await;
    state
        .plugins
        .emit_event(PacsEvent::ResourceDeleted {
            level: ResourceLevel::Study,
            uid: study_uid.to_string(),
            user_id: None,
        })
        .await;

    let current_page = filters.page();
    let mut response_filters = filters.clone();
    let initial_markup =
        match build_studies_results_view(&state, runtime.route_prefix(), &response_filters).await {
            Ok(view) => {
                if !view.has_results && current_page > 1 {
                    response_filters = response_filters.with_page(current_page - 1);
                    None
                } else {
                    Some(render_studies_results_view(view))
                }
            }
            Err(status) => {
                return error_response(
                    status,
                    "admin studies browser failed to refresh after delete",
                )
            }
        };

    let markup = match initial_markup {
        Some(markup) => markup,
        None => match render_studies_results_markup(&state, &runtime, &response_filters).await {
            Ok(markup) => markup,
            Err(status) => {
                return error_response(
                    status,
                    "admin studies browser failed to refresh after delete",
                )
            }
        },
    };

    html_markup_response(markup)
}

async fn nodes_page(
    State(state): State<AppState>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
) -> Response {
    let nodes_markup =
        match render_nodes_panel_markup(&state, &runtime, NodeFormView::default(), None).await {
            Ok(markup) => markup,
            Err(status) => return error_response(status, "admin node management failed to load"),
        };

    render_html(&NodesPageTemplate {
        page_title: "Nodes",
        route_prefix: runtime.route_prefix().to_string(),
        active_nav: "nodes",
        nodes_markup,
    })
}

async fn edit_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(ae_title): Path<String>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
) -> Response {
    let node = match load_node(&state, &ae_title).await {
        Ok(node) => node,
        Err(error) => {
            return render_nodes_response(
                &state,
                &runtime,
                &headers,
                NodeFormView::default(),
                Some(store_error_flash("Node load failed", &error)),
            )
            .await;
        }
    };

    render_nodes_response(
        &state,
        &runtime,
        &headers,
        NodeFormView::from_node(&node),
        Some(FlashView {
            title: "Node loaded".into(),
            detail: format!(
                "Editing {}. Update the fields and save to persist changes.",
                node.ae_title
            ),
            tone_class: "flash-info",
        }),
    )
    .await
}

async fn save_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    Form(input): Form<NodeFormInput>,
) -> Response {
    let form = NodeFormView::from_input(&input);
    let node = match input.into_node() {
        Ok(node) => node,
        Err(flash) => {
            return render_nodes_response(&state, &runtime, &headers, form, Some(flash)).await;
        }
    };

    if let Err(error) = state.store.upsert_node(&node).await {
        return render_nodes_response(
            &state,
            &runtime,
            &headers,
            form,
            Some(store_error_flash("Node update failed", &error)),
        )
        .await;
    }

    let flash = FlashView {
        title: "Node saved".into(),
        detail: format!(
            "{} now points to {}:{}.",
            node.ae_title, node.host, node.port
        ),
        tone_class: "flash-success",
    };
    render_nodes_response(
        &state,
        &runtime,
        &headers,
        NodeFormView::default(),
        Some(flash),
    )
    .await
}

async fn delete_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(ae_title): Path<String>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
) -> Response {
    if let Err(error) = state.store.delete_node(&ae_title).await {
        return render_nodes_response(
            &state,
            &runtime,
            &headers,
            NodeFormView::default(),
            Some(store_error_flash("Node removal failed", &error)),
        )
        .await;
    }

    let flash = FlashView {
        title: "Node removed".into(),
        detail: format!("{} was removed from the registered AE list.", ae_title),
        tone_class: "flash-warning",
    };
    render_nodes_response(
        &state,
        &runtime,
        &headers,
        NodeFormView::default(),
        Some(flash),
    )
    .await
}

async fn verify_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(ae_title): Path<String>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
) -> Response {
    let node = match load_node(&state, &ae_title).await {
        Ok(node) => node,
        Err(error) => {
            return render_nodes_response(
                &state,
                &runtime,
                &headers,
                NodeFormView::default(),
                Some(store_error_flash("Verification failed", &error)),
            )
            .await;
        }
    };

    let flash = if node.tls_enabled {
        FlashView {
            title: "Verification unavailable".into(),
            detail: format!(
                "{} is configured for TLS. The current admin verification flow only supports non-TLS C-ECHO.",
                node.ae_title
            ),
            tone_class: "flash-warning",
        }
    } else {
        let client = DicomClient::new(state.server_info.ae_title.clone(), NODE_VERIFY_TIMEOUT_SECS);
        let verify_target =
            pacs_dimse::DicomNode::new(node.ae_title.clone(), node.host.clone(), node.port);
        match client.echo(&verify_target).await {
            Ok(()) => FlashView {
                title: "Verification succeeded".into(),
                detail: format!(
                    "{} responded to C-ECHO at {}:{}.",
                    node.ae_title, node.host, node.port
                ),
                tone_class: "flash-success",
            },
            Err(error) => FlashView {
                title: "Verification failed".into(),
                detail: format!("{} did not answer C-ECHO cleanly: {}", node.ae_title, error),
                tone_class: "flash-danger",
            },
        }
    };

    render_nodes_response(
        &state,
        &runtime,
        &headers,
        NodeFormView::default(),
        Some(flash),
    )
    .await
}

async fn audit_page(
    State(state): State<AppState>,
    Query(filters): Query<AuditFilters>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
) -> Response {
    let results_markup = match render_audit_results_markup(&state, &runtime, &filters).await {
        Ok(markup) => markup,
        Err(status) => return error_response(status, "admin audit explorer failed to render"),
    };

    render_html(&AuditPageTemplate {
        page_title: "Audit Log",
        route_prefix: runtime.route_prefix().to_string(),
        active_nav: "audit",
        audit_path: audit_page_path(runtime.route_prefix()),
        audit_results_path: audit_results_path(runtime.route_prefix()),
        filters: AuditFilterView::from_filters(&filters),
        results_markup,
    })
}

async fn audit_results_fragment(
    State(state): State<AppState>,
    Query(filters): Query<AuditFilters>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
) -> Response {
    match render_audit_results_markup(&state, &runtime, &filters).await {
        Ok(markup) => html_markup_response(markup),
        Err(status) => error_response(status, "admin audit explorer failed to load results"),
    }
}

async fn admin_css() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        ADMIN_CSS,
    )
        .into_response()
}

async fn events_stream(
    Extension(runtime): Extension<Arc<AdminRuntime>>,
) -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
    let mut rx = runtime.subscribe();
    let runtime_for_stream = Arc::clone(&runtime);

    let stream = stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if should_emit_stats(&event) {
                        match render_stats_markup(&runtime_for_stream).await {
                            Ok(markup) => {
                                yield Ok(Event::default().event("stats").data(markup));
                            }
                            Err(status) => {
                                error!(?status, "failed to render admin stats event");
                            }
                        }
                    }

                    let activity_item = activity_view_from_entry(crate::runtime::activity_from_event(&event));
                    match (RecentActivityItemsTemplate { entries: vec![activity_item.clone()] }).render() {
                        Ok(markup) => {
                            yield Ok(Event::default().event("activity").data(markup));
                        }
                        Err(error) => {
                            error!(error = %error, "failed to render admin activity fragment");
                        }
                    }

                    match (ToastTemplate { item: activity_item }).render() {
                        Ok(markup) => {
                            yield Ok(Event::default().event("toast").data(markup));
                        }
                        Err(error) => {
                            error!(error = %error, "failed to render admin toast fragment");
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!(skipped, "admin dashboard SSE receiver lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}

fn should_emit_stats(event: &PacsEvent) -> bool {
    matches!(
        event,
        PacsEvent::InstanceStored { .. }
            | PacsEvent::StudyComplete { .. }
            | PacsEvent::ResourceDeleted { .. }
            | PacsEvent::AssociationOpened { .. }
            | PacsEvent::AssociationClosed { .. }
    )
}

async fn render_stats_markup(runtime: &AdminRuntime) -> Result<String, StatusCode> {
    let stats = runtime
        .metadata_store()
        .get_statistics()
        .await
        .map_err(internal_store_error)?;

    StatsCardsTemplate {
        cards: vec![
            StatCardView {
                eyebrow: "Studies",
                value: stats.num_studies.to_string(),
                detail: format!("{} series indexed", stats.num_series),
            },
            StatCardView {
                eyebrow: "Instances",
                value: stats.num_instances.to_string(),
                detail: "Stored across active studies".into(),
            },
            StatCardView {
                eyebrow: "Storage",
                value: format_bytes(stats.disk_usage_bytes),
                detail: "Metadata + binary payload footprint".into(),
            },
            StatCardView {
                eyebrow: "Associations",
                value: runtime.active_associations().to_string(),
                detail: "Current DIMSE sessions".into(),
            },
        ],
    }
    .render()
    .map_err(internal_render_error)
}

fn render_recent_activity_markup(entries: Vec<ActivityEntry>) -> Result<String, StatusCode> {
    RecentActivityItemsTemplate {
        entries: entries.into_iter().map(activity_view_from_entry).collect(),
    }
    .render()
    .map_err(internal_render_error)
}

async fn render_studies_results_markup(
    state: &AppState,
    runtime: &AdminRuntime,
    filters: &StudiesFilters,
) -> Result<String, StatusCode> {
    let view = build_studies_results_view(state, runtime.route_prefix(), filters).await?;
    Ok(render_studies_results_view(view))
}

async fn render_system_settings_markup(
    state: &AppState,
    runtime: &AdminRuntime,
    form_override: Option<ServerSettingsFormView>,
    flash: Option<FlashView>,
) -> Result<String, StatusCode> {
    let persisted_settings = load_saved_server_settings(state).await?;
    let form =
        form_override.unwrap_or_else(|| ServerSettingsFormView::from_settings(&persisted_settings));
    let syntax_options = transfer_syntax_option_views(&form.accepted_transfer_syntaxes);
    let preferred_syntax_order =
        preferred_transfer_syntax_item_views(&form.preferred_transfer_syntaxes);

    SystemSettingsPanelTemplate {
        system_path: format!("{}/system", runtime.route_prefix()),
        form,
        flash,
        source_label: if persisted_settings == state.server_settings {
            "Active DIMSE configuration".into()
        } else {
            "Saved for next restart".into()
        },
        restart_required: persisted_settings != state.server_settings,
        syntax_options,
        preferred_syntax_order,
    }
    .render()
    .map_err(internal_render_error)
}

async fn build_studies_results_view(
    state: &AppState,
    route_prefix: &str,
    filters: &StudiesFilters,
) -> Result<StudiesResultsView, StatusCode> {
    let page = filters.page();
    let page_size = filters.page_size();
    let offset = filters.offset();
    let fetch_limit = page_size.saturating_add(1);
    let mut studies = state
        .store
        .query_studies(&filters.to_store_query(fetch_limit))
        .await
        .map_err(internal_store_error)?;
    let has_next = studies.len() as u32 > page_size;
    if has_next {
        studies.truncate(page_size as usize);
    }

    let has_results = !studies.is_empty();
    let start_index = if has_results { offset + 1 } else { 0 };
    let end_index = offset + studies.len() as u32;
    let has_prev = page > 1;

    let page_href = path_with_query(
        &studies_page_path(route_prefix),
        &filters.with_page(page).to_query_string(),
    );
    let prev_page_href = path_with_query(
        &studies_page_path(route_prefix),
        &filters.with_page(page.saturating_sub(1)).to_query_string(),
    );
    let prev_results_href = path_with_query(
        &studies_results_path(route_prefix),
        &filters.with_page(page.saturating_sub(1)).to_query_string(),
    );
    let next_page_href = path_with_query(
        &studies_page_path(route_prefix),
        &filters.with_page(page + 1).to_query_string(),
    );
    let next_results_href = path_with_query(
        &studies_results_path(route_prefix),
        &filters.with_page(page + 1).to_query_string(),
    );
    let query_string = filters.with_page(page).to_query_string();

    Ok(StudiesResultsView {
        rows: studies
            .into_iter()
            .map(|study| study_row_view(route_prefix, &query_string, study))
            .collect(),
        has_results,
        has_active_filters: filters.has_active_filters(),
        result_summary: if has_results {
            format!("Showing studies {start_index}-{end_index}")
        } else {
            "No studies found".into()
        },
        page_summary: format!("Page {page} | {page_size} per page"),
        empty_summary: if filters.has_active_filters() {
            "Adjust the filters and try again. Only explicit study fields are queried here.".into()
        } else {
            "No studies are currently indexed in the active metadata store.".into()
        },
        page_href,
        has_prev,
        prev_page_href,
        prev_results_href,
        has_next,
        next_page_href,
        next_results_href,
    })
}

fn render_studies_results_view(view: StudiesResultsView) -> String {
    StudiesResultsTemplate {
        rows: view.rows,
        has_results: view.has_results,
        has_active_filters: view.has_active_filters,
        result_summary: view.result_summary,
        page_summary: view.page_summary,
        empty_summary: view.empty_summary,
        page_href: view.page_href,
        has_prev: view.has_prev,
        prev_page_href: view.prev_page_href,
        prev_results_href: view.prev_results_href,
        has_next: view.has_next,
        next_page_href: view.next_page_href,
        next_results_href: view.next_results_href,
    }
    .render()
    .unwrap_or_else(|error| {
        error!(error = %error, "failed to render admin studies results fragment");
        "<section id=\"studies-results\" class=\"panel\"><p>Failed to render studies results.</p></section>".into()
    })
}

async fn render_nodes_panel_markup(
    state: &AppState,
    runtime: &AdminRuntime,
    form: NodeFormView,
    flash: Option<FlashView>,
) -> Result<String, StatusCode> {
    let nodes = state
        .store
        .list_nodes()
        .await
        .map_err(internal_store_error)?;
    let rows = nodes
        .into_iter()
        .map(|node| node_row_view(runtime.route_prefix(), node))
        .collect::<Vec<_>>();

    NodesPanelTemplate {
        nodes_path: format!("{}/nodes", runtime.route_prefix()),
        form,
        flash,
        has_nodes: !rows.is_empty(),
        rows,
    }
    .render()
    .map_err(internal_render_error)
}

async fn render_system_response(
    state: &AppState,
    runtime: &AdminRuntime,
    headers: &HeaderMap,
    form_override: Option<ServerSettingsFormView>,
    flash: Option<FlashView>,
) -> Response {
    let settings_markup =
        match render_system_settings_markup(state, runtime, form_override, flash).await {
            Ok(markup) => markup,
            Err(status) => return error_response(status, "admin system settings failed to render"),
        };

    if is_htmx_request(headers) {
        html_markup_response(settings_markup)
    } else {
        let plugin_rows = state
            .plugins
            .aggregate_health()
            .await
            .into_iter()
            .map(|(plugin_id, health)| plugin_health_view(plugin_id, health))
            .collect();

        render_html(&SystemPageTemplate {
            page_title: "System Overview",
            route_prefix: runtime.route_prefix().to_string(),
            active_nav: "system",
            server_info: server_info_view(&state.server_info),
            settings_markup,
            plugin_rows,
        })
    }
}

async fn render_audit_results_markup(
    state: &AppState,
    runtime: &AdminRuntime,
    filters: &AuditFilters,
) -> Result<String, StatusCode> {
    let view = build_audit_results_view(state, runtime.route_prefix(), filters).await?;
    AuditResultsTemplate {
        rows: view.rows,
        has_results: view.has_results,
        has_active_filters: view.has_active_filters,
        result_summary: view.result_summary,
        page_summary: view.page_summary,
        empty_summary: view.empty_summary,
        page_href: view.page_href,
        has_prev: view.has_prev,
        prev_page_href: view.prev_page_href,
        prev_results_href: view.prev_results_href,
        has_next: view.has_next,
        next_page_href: view.next_page_href,
        next_results_href: view.next_results_href,
    }
    .render()
    .map_err(internal_render_error)
}

async fn build_audit_results_view(
    state: &AppState,
    route_prefix: &str,
    filters: &AuditFilters,
) -> Result<AuditResultsView, StatusCode> {
    let page_size = filters.page_size();
    let query = filters.to_store_query();
    let result_page = state
        .store
        .search_audit_logs(&query)
        .await
        .map_err(internal_store_error)?;
    let has_results = !result_page.entries.is_empty();
    let has_prev = result_page.offset > 0;
    let has_next =
        (result_page.offset as i64 + result_page.entries.len() as i64) < result_page.total;
    let current_page = (result_page.offset / result_page.limit) + 1;
    let start_index = if has_results {
        result_page.offset + 1
    } else {
        0
    };
    let end_index = result_page.offset + result_page.entries.len() as u32;

    let page_href = path_with_query(
        &audit_page_path(route_prefix),
        &filters.with_page(current_page).to_query_string(),
    );
    let prev_page_href = path_with_query(
        &audit_page_path(route_prefix),
        &filters
            .with_page(current_page.saturating_sub(1))
            .to_query_string(),
    );
    let prev_results_href = path_with_query(
        &audit_results_path(route_prefix),
        &filters
            .with_page(current_page.saturating_sub(1))
            .to_query_string(),
    );
    let next_page_href = path_with_query(
        &audit_page_path(route_prefix),
        &filters.with_page(current_page + 1).to_query_string(),
    );
    let next_results_href = path_with_query(
        &audit_results_path(route_prefix),
        &filters.with_page(current_page + 1).to_query_string(),
    );

    Ok(AuditResultsView {
        rows: result_page
            .entries
            .into_iter()
            .map(audit_row_view)
            .collect(),
        has_results,
        has_active_filters: filters.has_active_filters(),
        result_summary: if has_results {
            format!(
                "Showing audit rows {start_index}-{end_index} of {}",
                result_page.total
            )
        } else {
            "No audit rows found".into()
        },
        page_summary: format!("Page {current_page} | {} per page", page_size),
        empty_summary: if filters.has_active_filters() {
            "No audit rows match the active filters.".into()
        } else {
            "No audit rows have been written by the active metadata store yet.".into()
        },
        page_href,
        has_prev,
        prev_page_href,
        prev_results_href,
        has_next,
        next_page_href,
        next_results_href,
    })
}

async fn render_nodes_response(
    state: &AppState,
    runtime: &AdminRuntime,
    headers: &HeaderMap,
    form: NodeFormView,
    flash: Option<FlashView>,
) -> Response {
    let markup = match render_nodes_panel_markup(state, runtime, form, flash).await {
        Ok(markup) => markup,
        Err(status) => return error_response(status, "admin node management failed to render"),
    };

    if is_htmx_request(headers) {
        html_markup_response(markup)
    } else {
        render_html(&NodesPageTemplate {
            page_title: "Nodes",
            route_prefix: runtime.route_prefix().to_string(),
            active_nav: "nodes",
            nodes_markup: markup,
        })
    }
}

fn activity_view_from_entry(entry: ActivityEntry) -> ActivityView {
    ActivityView {
        timestamp: entry
            .occurred_at
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string(),
        badge: entry.badge,
        title: entry.title,
        detail: entry.detail,
        tone_class: entry.tone_class,
    }
}

fn server_info_view(server_info: &pacs_plugin::ServerInfo) -> ServerInfoView {
    ServerInfoView {
        ae_title: server_info.ae_title.clone(),
        http_port: server_info.http_port,
        dicom_port: server_info.dicom_port,
        version: server_info.version.to_string(),
    }
}

fn plugin_health_view(plugin_id: String, health: PluginHealth) -> PluginHealthView {
    match health {
        PluginHealth::Healthy => PluginHealthView {
            plugin_id,
            status_label: "Healthy".into(),
            status_class: "status-healthy",
            detail: "Operating normally".into(),
        },
        PluginHealth::Degraded(detail) => PluginHealthView {
            plugin_id,
            status_label: "Degraded".into(),
            status_class: "status-degraded",
            detail,
        },
        PluginHealth::Unhealthy(detail) => PluginHealthView {
            plugin_id,
            status_label: "Unhealthy".into(),
            status_class: "status-unhealthy",
            detail,
        },
    }
}

fn study_row_view(route_prefix: &str, query_string: &str, study: Study) -> StudyRowView {
    StudyRowView {
        study_uid: study.study_uid.to_string(),
        patient_id: fallback_string(study.patient_id),
        patient_name: fallback_string(study.patient_name),
        study_date: study
            .study_date
            .map(|value| value.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "-".into()),
        accession_number: fallback_string(study.accession_number),
        description: fallback_string(study.description),
        modalities: if study.modalities.is_empty() {
            "-".into()
        } else {
            study.modalities.join(", ")
        },
        num_series: study.num_series,
        num_instances: study.num_instances,
        delete_href: path_with_query(
            &format!("{route_prefix}/studies/{}", study.study_uid),
            query_string,
        ),
    }
}

fn node_row_view(route_prefix: &str, node: DicomNode) -> NodeRowView {
    let ae_title = node.ae_title;
    NodeRowView {
        delete_href: format!("{route_prefix}/nodes/{ae_title}"),
        edit_href: format!("{route_prefix}/nodes/{ae_title}/edit"),
        verify_href: format!("{route_prefix}/nodes/{ae_title}/verify"),
        verification_state: if node.tls_enabled { "TLS" } else { "C-ECHO" },
        tls_label: if node.tls_enabled { "Enabled" } else { "Off" },
        tls_class: if node.tls_enabled {
            "status-pill status-degraded"
        } else {
            "status-pill tone-muted"
        },
        description: fallback_string(node.description),
        ae_title,
        host: node.host,
        port: node.port,
    }
}

fn audit_row_view(entry: AuditLogEntry) -> AuditRowView {
    AuditRowView {
        occurred_at: entry
            .occurred_at
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string(),
        action: entry.action,
        resource: entry.resource,
        resource_uid: fallback_string(entry.resource_uid),
        status_class: audit_status_class(&entry.status),
        status: entry.status,
        user_id: fallback_string(entry.user_id),
        source_ip: fallback_string(entry.source_ip),
        details_pretty: pretty_json(&entry.details),
    }
}

fn audit_status_class(status: &str) -> &'static str {
    match status {
        "ok" => "status-healthy",
        "rejected" => "status-unhealthy",
        _ => "status-degraded",
    }
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

async fn load_node(state: &AppState, ae_title: &str) -> Result<DicomNode, PacsError> {
    let nodes = state.store.list_nodes().await?;
    nodes
        .into_iter()
        .find(|node| node.ae_title == ae_title)
        .ok_or_else(|| PacsError::NotFound {
            resource: "node",
            uid: ae_title.to_string(),
        })
}

async fn load_saved_server_settings(state: &AppState) -> Result<ServerSettings, StatusCode> {
    state
        .store
        .get_server_settings()
        .await
        .map_err(internal_store_error)
        .map(|settings| settings.unwrap_or_else(|| state.server_settings.clone()))
}

fn store_error_flash(title: &str, error: &PacsError) -> FlashView {
    FlashView {
        title: title.into(),
        detail: error.to_string(),
        tone_class: "flash-danger",
    }
}

fn is_htmx_request(headers: &HeaderMap) -> bool {
    headers
        .get("HX-Request")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn fallback_string(value: Option<String>) -> String {
    value
        .filter(|item| !item.trim().is_empty())
        .unwrap_or_else(|| "-".into())
}

fn transfer_syntax_option_views(accepted_values: &[String]) -> Vec<TransferSyntaxOptionView> {
    let accepted = accepted_values.iter().cloned().collect::<HashSet<_>>();

    supported_retrieve_transfer_syntaxes()
        .iter()
        .map(|uid| TransferSyntaxOptionView {
            uid: (*uid).to_string(),
            label: transfer_syntax_label(uid).to_string(),
            is_required: *uid == REQUIRED_ACCEPTED_TRANSFER_SYNTAX_UID,
            accepted_selected: accepted.contains(*uid),
        })
        .collect()
}

const REQUIRED_ACCEPTED_TRANSFER_SYNTAX_UID: &str = "1.2.840.10008.1.2";

fn transfer_syntax_label(uid: &str) -> String {
    match uid {
        "1.2.840.10008.1.2" => "Implicit VR Little Endian".into(),
        "1.2.840.10008.1.2.1" => "Explicit VR Little Endian".into(),
        "1.2.840.10008.1.2.1.99" => "Deflated Explicit VR Little Endian".into(),
        "1.2.840.10008.1.2.2" => "Explicit VR Big Endian".into(),
        "1.2.840.10008.1.2.5" => "RLE Lossless".into(),
        "1.2.840.10008.1.2.4.50" => "JPEG Baseline (Process 1)".into(),
        "1.2.840.10008.1.2.4.51" => "JPEG Extended (Processes 2/4)".into(),
        "1.2.840.10008.1.2.4.57" => "JPEG Lossless (Process 14)".into(),
        "1.2.840.10008.1.2.4.70" => "JPEG Lossless First-Order Prediction".into(),
        "1.2.840.10008.1.2.4.80" => "JPEG-LS Lossless".into(),
        "1.2.840.10008.1.2.4.90" => "JPEG 2000 Lossless Only".into(),
        "1.2.840.10008.1.2.4.91" => "JPEG 2000".into(),
        _ => uid.to_string(),
    }
}

fn supported_transfer_syntax_uid_set() -> HashSet<String> {
    supported_retrieve_transfer_syntaxes()
        .iter()
        .map(|uid| (*uid).to_string())
        .collect()
}

fn all_supported_transfer_syntax_uids() -> Vec<String> {
    supported_retrieve_transfer_syntaxes()
        .iter()
        .map(|uid| (*uid).to_string())
        .collect()
}

fn normalize_syntax_selection(values: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() && !deduped.iter().any(|existing| existing == trimmed) {
            deduped.push(trimmed.to_string());
        }
    }
    deduped
}

fn normalize_accepted_transfer_syntax_selection(values: Vec<String>) -> Vec<String> {
    let mut accepted = normalize_syntax_selection(values);
    if !accepted
        .iter()
        .any(|uid| uid == REQUIRED_ACCEPTED_TRANSFER_SYNTAX_UID)
    {
        accepted.insert(0, REQUIRED_ACCEPTED_TRANSFER_SYNTAX_UID.to_string());
    }
    accepted
}

fn accepted_transfer_syntaxes_for_form(
    accept_all_transfer_syntaxes: bool,
    accepted_transfer_syntaxes: Vec<String>,
) -> Vec<String> {
    if accept_all_transfer_syntaxes {
        all_supported_transfer_syntax_uids()
    } else {
        normalize_accepted_transfer_syntax_selection(accepted_transfer_syntaxes)
    }
}

fn preferred_transfer_syntaxes_for_form(
    accepted_transfer_syntaxes: &[String],
    preferred_transfer_syntaxes: Vec<String>,
) -> Vec<String> {
    let accepted_lookup = accepted_transfer_syntaxes
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut ordered = Vec::new();

    for uid in normalize_syntax_selection(preferred_transfer_syntaxes) {
        if accepted_lookup.contains(uid.as_str())
            && !ordered.iter().any(|existing| existing == &uid)
        {
            ordered.push(uid);
        }
    }

    for uid in accepted_transfer_syntaxes {
        if !ordered.iter().any(|existing| existing == uid) {
            ordered.push(uid.clone());
        }
    }

    ordered
}

fn preferred_transfer_syntax_item_views(values: &[String]) -> Vec<PreferredTransferSyntaxItemView> {
    values
        .iter()
        .map(|uid| PreferredTransferSyntaxItemView {
            uid: uid.clone(),
            label: transfer_syntax_label(uid),
            is_required: uid == REQUIRED_ACCEPTED_TRANSFER_SYNTAX_UID,
        })
        .collect()
}

fn first_unsupported_syntax<'a>(
    values: &'a [String],
    supported: &HashSet<String>,
) -> Option<&'a str> {
    values
        .iter()
        .find(|value| !supported.contains(value.as_str()))
        .map(String::as_str)
}

fn studies_page_path(route_prefix: &str) -> String {
    format!("{route_prefix}/studies")
}

fn studies_results_path(route_prefix: &str) -> String {
    format!("{route_prefix}/studies/list")
}

fn audit_page_path(route_prefix: &str) -> String {
    format!("{route_prefix}/audit")
}

fn audit_results_path(route_prefix: &str) -> String {
    format!("{route_prefix}/audit/list")
}

fn path_with_query(path: &str, query_string: &str) -> String {
    if query_string.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{query_string}")
    }
}

async fn collect_study_blob_keys(
    state: &AppState,
    study_uid: &StudyUid,
) -> Result<BTreeSet<String>, StatusCode> {
    let series = state
        .store
        .query_series(&SeriesQuery {
            study_uid: study_uid.clone(),
            series_uid: None,
            modality: None,
            series_number: None,
            limit: None,
            offset: None,
        })
        .await
        .map_err(internal_store_error)?;

    let mut blob_keys = BTreeSet::new();
    for series in series {
        let instances = state
            .store
            .query_instances(&InstanceQuery {
                series_uid: series.series_uid,
                instance_uid: None,
                sop_class_uid: None,
                instance_number: None,
                limit: None,
                offset: None,
            })
            .await
            .map_err(internal_store_error)?;
        blob_keys.extend(instances.into_iter().map(|instance| instance.blob_key));
    }

    Ok(blob_keys)
}

async fn cleanup_blob_keys(state: &AppState, blob_keys: BTreeSet<String>) {
    for blob_key in blob_keys {
        match state.blobs.delete(&blob_key).await {
            Ok(()) | Err(PacsError::NotFound { .. }) => {}
            Err(error) => {
                warn!(blob_key = %blob_key, error = %error, "failed to delete blob after study removal");
            }
        }
    }
}

fn internal_store_error(error: pacs_core::PacsError) -> StatusCode {
    error!(error = %error, "admin dashboard store call failed");
    pacs_error_to_status(&error)
}

fn pacs_error_to_status(error: &PacsError) -> StatusCode {
    match error {
        PacsError::NotFound { .. } => StatusCode::NOT_FOUND,
        PacsError::InvalidUid(_) | PacsError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
        PacsError::NotAcceptable(_) => StatusCode::NOT_ACCEPTABLE,
        PacsError::UnsupportedMediaType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        PacsError::Store(_)
        | PacsError::Blob(_)
        | PacsError::DicomParse(_)
        | PacsError::Config(_)
        | PacsError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn internal_render_error(error: askama::Error) -> StatusCode {
    error!(error = %error, "admin dashboard template rendering failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

fn render_html<T: Template>(template: &T) -> Response {
    match template.render() {
        Ok(body) => html_markup_response(body),
        Err(error) => error_response(
            internal_render_error(error),
            "admin dashboard failed to render",
        ),
    }
}

fn html_markup_response(body: String) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store, max-age=0"),
        ],
        body,
    )
        .into_response()
}

fn error_response(status: StatusCode, message: &'static str) -> Response {
    (status, message).into_response()
}

fn format_bytes(bytes: i64) -> String {
    let mut size = bytes.max(0) as f64;
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut unit_index = 0;
    while size >= 1024.0 && unit_index < units.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", size as i64, units[unit_index])
    } else {
        format!("{size:.1} {}", units[unit_index])
    }
}

impl StudiesFilters {
    fn page(&self) -> u32 {
        self.page.unwrap_or(1).max(1)
    }

    fn page_size(&self) -> u32 {
        self.page_size
            .unwrap_or(DEFAULT_STUDY_PAGE_SIZE)
            .clamp(1, MAX_STUDY_PAGE_SIZE)
    }

    fn offset(&self) -> u32 {
        self.page()
            .saturating_sub(1)
            .saturating_mul(self.page_size())
    }

    fn with_page(&self, page: u32) -> Self {
        let mut next = self.clone();
        next.page = Some(page.max(1));
        next
    }

    fn has_active_filters(&self) -> bool {
        [
            self.patient_id.as_deref(),
            self.patient_name.as_deref(),
            self.study_uid.as_deref(),
            self.accession_number.as_deref(),
            self.modality.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| !value.trim().is_empty())
    }

    fn to_store_query(&self, limit: u32) -> StudyQuery {
        StudyQuery {
            patient_id: normalized_filter(&self.patient_id),
            patient_name: normalized_patient_name(&self.patient_name),
            study_date_from: None,
            study_date_to: None,
            accession_number: normalized_filter(&self.accession_number),
            study_uid: normalized_filter(&self.study_uid).map(StudyUid::from),
            modality: normalized_filter(&self.modality).map(|value| value.to_uppercase()),
            limit: Some(limit),
            offset: Some(self.offset()),
            include_fields: Vec::new(),
            fuzzy_matching: false,
        }
    }

    fn to_query_string(&self) -> String {
        let mut serializer = form_urlencoded::Serializer::new(String::new());
        if let Some(value) = normalized_filter(&self.patient_id) {
            serializer.append_pair("patient_id", &value);
        }
        if let Some(value) = normalized_filter(&self.patient_name) {
            serializer.append_pair("patient_name", &value);
        }
        if let Some(value) = normalized_filter(&self.study_uid) {
            serializer.append_pair("study_uid", &value);
        }
        if let Some(value) = normalized_filter(&self.accession_number) {
            serializer.append_pair("accession_number", &value);
        }
        if let Some(value) = normalized_filter(&self.modality) {
            serializer.append_pair("modality", &value);
        }
        serializer.append_pair("page", &self.page().to_string());
        serializer.append_pair("page_size", &self.page_size().to_string());
        serializer.finish()
    }
}

impl StudiesFilterView {
    fn from_filters(filters: &StudiesFilters) -> Self {
        Self {
            patient_id: filters
                .patient_id
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            patient_name: filters
                .patient_name
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            study_uid: filters
                .study_uid
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            accession_number: filters
                .accession_number
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            modality: filters
                .modality
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            page_size: filters.page_size(),
        }
    }
}

impl NodeFormInput {
    fn into_node(self) -> Result<DicomNode, FlashView> {
        let ae_title = self.ae_title.trim().to_string();
        if ae_title.is_empty() {
            return Err(validation_flash("AE title is required."));
        }
        if ae_title.len() > 16 {
            return Err(validation_flash("AE title must be 16 characters or fewer."));
        }

        let host = self.host.trim().to_string();
        if host.is_empty() {
            return Err(validation_flash("Host is required."));
        }

        let port = self
            .port
            .trim()
            .parse::<u16>()
            .map_err(|_| validation_flash("Port must be a valid TCP port."))?;
        if port == 0 {
            return Err(validation_flash("Port must be greater than zero."));
        }

        let description = self.description.trim();

        Ok(DicomNode {
            ae_title,
            host,
            port,
            description: (!description.is_empty()).then(|| description.to_string()),
            tls_enabled: self.tls_enabled.is_some(),
        })
    }
}

impl NodeFormView {
    fn from_input(input: &NodeFormInput) -> Self {
        Self {
            ae_title: input.ae_title.trim().to_string(),
            host: input.host.trim().to_string(),
            port: input.port.trim().to_string(),
            description: input.description.trim().to_string(),
            tls_enabled: input.tls_enabled.is_some(),
        }
    }

    fn from_node(node: &DicomNode) -> Self {
        Self {
            ae_title: node.ae_title.clone(),
            host: node.host.clone(),
            port: node.port.to_string(),
            description: node.description.clone().unwrap_or_default(),
            tls_enabled: node.tls_enabled,
        }
    }
}

impl ServerSettingsFormInput {
    fn into_settings(self) -> Result<ServerSettings, FlashView> {
        let supported_uids = supported_transfer_syntax_uid_set();
        let dicom_port = self
            .dicom_port
            .trim()
            .parse::<u16>()
            .map_err(|_| validation_flash("DICOM port must be a valid TCP port."))?;
        if dicom_port == 0 {
            return Err(validation_flash("DICOM port must be greater than zero."));
        }

        let ae_title = self.ae_title.trim().to_string();
        if ae_title.is_empty() {
            return Err(validation_flash("AE title is required."));
        }
        if ae_title.len() > 16 {
            return Err(validation_flash("AE title must be 16 characters or fewer."));
        }

        let max_associations = self
            .max_associations
            .trim()
            .parse::<usize>()
            .map_err(|_| validation_flash("Max associations must be a positive integer."))?;
        if max_associations == 0 {
            return Err(validation_flash(
                "Max associations must be greater than zero.",
            ));
        }

        let dimse_timeout_secs = self
            .dimse_timeout_secs
            .trim()
            .parse::<u64>()
            .map_err(|_| validation_flash("DIMSE timeout must be a positive integer."))?;
        if dimse_timeout_secs == 0 {
            return Err(validation_flash("DIMSE timeout must be greater than zero."));
        }

        let accepted_transfer_syntaxes =
            normalize_accepted_transfer_syntax_selection(self.accepted_transfer_syntaxes);
        let submitted_preferred_transfer_syntaxes =
            normalize_syntax_selection(self.preferred_transfer_syntaxes);
        let accept_all_transfer_syntaxes =
            supported_retrieve_transfer_syntaxes().iter().all(|uid| {
                accepted_transfer_syntaxes
                    .iter()
                    .any(|selected| selected == uid)
            });
        let preferred_transfer_syntaxes = preferred_transfer_syntaxes_for_form(
            &accepted_transfer_syntaxes,
            submitted_preferred_transfer_syntaxes.clone(),
        );

        if let Some(invalid) =
            first_unsupported_syntax(&accepted_transfer_syntaxes, &supported_uids)
        {
            return Err(validation_flash(&format!(
                "Unsupported accepted transfer syntax: {invalid}"
            )));
        }
        if let Some(invalid) =
            first_unsupported_syntax(&submitted_preferred_transfer_syntaxes, &supported_uids)
        {
            return Err(validation_flash(&format!(
                "Unsupported preferred transfer syntax: {invalid}"
            )));
        }

        Ok(ServerSettings {
            dicom_port,
            ae_title,
            ae_whitelist_enabled: self.ae_whitelist_enabled.is_some(),
            accept_all_transfer_syntaxes,
            accepted_transfer_syntaxes,
            preferred_transfer_syntaxes,
            max_associations,
            dimse_timeout_secs,
        })
    }
}

impl ServerSettingsFormView {
    fn from_input(input: &ServerSettingsFormInput) -> Self {
        let accepted_transfer_syntaxes =
            accepted_transfer_syntaxes_for_form(false, input.accepted_transfer_syntaxes.clone());
        Self {
            dicom_port: input.dicom_port.trim().to_string(),
            ae_title: input.ae_title.trim().to_string(),
            ae_whitelist_enabled: input.ae_whitelist_enabled.is_some(),
            accepted_transfer_syntaxes: accepted_transfer_syntaxes.clone(),
            preferred_transfer_syntaxes: preferred_transfer_syntaxes_for_form(
                &accepted_transfer_syntaxes,
                input.preferred_transfer_syntaxes.clone(),
            ),
            max_associations: input.max_associations.trim().to_string(),
            dimse_timeout_secs: input.dimse_timeout_secs.trim().to_string(),
        }
    }

    fn from_settings(settings: &ServerSettings) -> Self {
        let accepted_transfer_syntaxes = accepted_transfer_syntaxes_for_form(
            settings.accept_all_transfer_syntaxes,
            settings.accepted_transfer_syntaxes.clone(),
        );
        Self {
            dicom_port: settings.dicom_port.to_string(),
            ae_title: settings.ae_title.clone(),
            ae_whitelist_enabled: settings.ae_whitelist_enabled,
            accepted_transfer_syntaxes: accepted_transfer_syntaxes.clone(),
            preferred_transfer_syntaxes: preferred_transfer_syntaxes_for_form(
                &accepted_transfer_syntaxes,
                settings.preferred_transfer_syntaxes.clone(),
            ),
            max_associations: settings.max_associations.to_string(),
            dimse_timeout_secs: settings.dimse_timeout_secs.to_string(),
        }
    }
}

impl AuditFilters {
    fn page(&self) -> u32 {
        self.page.unwrap_or(1).max(1)
    }

    fn page_size(&self) -> u32 {
        self.page_size
            .unwrap_or(DEFAULT_AUDIT_PAGE_SIZE)
            .clamp(1, MAX_AUDIT_PAGE_SIZE)
    }

    fn offset(&self) -> u32 {
        self.page()
            .saturating_sub(1)
            .saturating_mul(self.page_size())
    }

    fn with_page(&self, page: u32) -> Self {
        let mut next = self.clone();
        next.page = Some(page.max(1));
        next
    }

    fn has_active_filters(&self) -> bool {
        [
            self.user_id.as_deref(),
            self.action.as_deref(),
            self.resource.as_deref(),
            self.resource_uid.as_deref(),
            self.status.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| !value.trim().is_empty())
    }

    fn to_store_query(&self) -> AuditLogQuery {
        AuditLogQuery {
            user_id: normalized_filter(&self.user_id),
            action: normalized_filter(&self.action).map(|value| value.to_uppercase()),
            resource: normalized_filter(&self.resource).map(|value| value.to_lowercase()),
            resource_uid: normalized_filter(&self.resource_uid),
            source_ip: None,
            status: normalized_filter(&self.status).map(|value| value.to_lowercase()),
            occurred_from: None,
            occurred_to: None,
            limit: Some(self.page_size()),
            offset: Some(self.offset()),
        }
    }

    fn to_query_string(&self) -> String {
        let mut serializer = form_urlencoded::Serializer::new(String::new());
        if let Some(value) = normalized_filter(&self.user_id) {
            serializer.append_pair("user_id", &value);
        }
        if let Some(value) = normalized_filter(&self.action) {
            serializer.append_pair("action", &value);
        }
        if let Some(value) = normalized_filter(&self.resource) {
            serializer.append_pair("resource", &value);
        }
        if let Some(value) = normalized_filter(&self.resource_uid) {
            serializer.append_pair("resource_uid", &value);
        }
        if let Some(value) = normalized_filter(&self.status) {
            serializer.append_pair("status", &value);
        }
        serializer.append_pair("page", &self.page().to_string());
        serializer.append_pair("page_size", &self.page_size().to_string());
        serializer.finish()
    }
}

impl AuditFilterView {
    fn from_filters(filters: &AuditFilters) -> Self {
        Self {
            user_id: filters
                .user_id
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            action: filters
                .action
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            resource: filters
                .resource
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            resource_uid: filters
                .resource_uid
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            status: filters
                .status
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            page_size: filters.page_size(),
        }
    }
}

impl Default for NodeFormView {
    fn default() -> Self {
        Self {
            ae_title: String::new(),
            host: String::new(),
            port: "104".into(),
            description: String::new(),
            tls_enabled: false,
        }
    }
}

fn validation_flash(detail: &str) -> FlashView {
    FlashView {
        title: "Validation failed".into(),
        detail: detail.into(),
        tone_class: "flash-warning",
    }
}

fn normalized_filter(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalized_patient_name(value: &Option<String>) -> Option<String> {
    normalized_filter(value).map(|value| {
        if value.contains('*') {
            value
        } else {
            format!("{value}*")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_are_formatted_human_readably() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn stats_events_are_scoped() {
        assert!(should_emit_stats(&PacsEvent::StudyComplete {
            study_uid: "1.2.3".into(),
        }));
        assert!(!should_emit_stats(&PacsEvent::QueryPerformed {
            level: "STUDY".into(),
            source: pacs_plugin::QuerySource::Dicomweb,
            num_results: 4,
            user_id: None,
        }));
    }

    #[test]
    fn studies_filters_default_and_clamp_pagination() {
        let filters = StudiesFilters {
            page: Some(0),
            page_size: Some(500),
            ..StudiesFilters::default()
        };

        assert_eq!(filters.page(), 1);
        assert_eq!(filters.page_size(), MAX_STUDY_PAGE_SIZE);
        assert_eq!(filters.offset(), 0);
    }

    #[test]
    fn studies_filters_build_store_query_with_wildcard_name() {
        let filters = StudiesFilters {
            patient_name: Some("DOE".into()),
            modality: Some("ct".into()),
            page: Some(2),
            page_size: Some(25),
            ..StudiesFilters::default()
        };

        let query = filters.to_store_query(26);
        assert_eq!(query.patient_name.as_deref(), Some("DOE*"));
        assert_eq!(query.modality.as_deref(), Some("CT"));
        assert_eq!(query.limit, Some(26));
        assert_eq!(query.offset, Some(25));
    }

    #[test]
    fn studies_filters_encode_query_string() {
        let filters = StudiesFilters {
            patient_name: Some("Jane Doe".into()),
            study_uid: Some("1.2.3".into()),
            page: Some(3),
            page_size: Some(10),
            ..StudiesFilters::default()
        };

        let query_string = filters.to_query_string();
        assert!(query_string.contains("patient_name=Jane+Doe"));
        assert!(query_string.contains("study_uid=1.2.3"));
        assert!(query_string.contains("page=3"));
        assert!(query_string.contains("page_size=10"));
    }

    #[test]
    fn server_settings_form_defaults_missing_syntax_lists() {
        let input: ServerSettingsFormInput = serde_json::from_value(serde_json::json!({
            "dicom_port": "11112",
            "ae_title": "PACSUI",
            "max_associations": "32",
            "dimse_timeout_secs": "45"
        }))
        .unwrap();

        assert!(input.accepted_transfer_syntaxes.is_empty());
        assert!(input.preferred_transfer_syntaxes.is_empty());
    }

    #[test]
    fn server_settings_form_accepts_single_syntax_value() {
        let input: ServerSettingsFormInput = serde_json::from_value(serde_json::json!({
            "dicom_port": "11112",
            "ae_title": "PACSUI",
            "accepted_transfer_syntaxes": "1.2.840.10008.1.2.1",
            "preferred_transfer_syntaxes": "1.2.840.10008.1.2.4.50",
            "max_associations": "32",
            "dimse_timeout_secs": "45"
        }))
        .unwrap();

        assert_eq!(
            input.accepted_transfer_syntaxes,
            vec!["1.2.840.10008.1.2.1"]
        );
        assert_eq!(
            input.preferred_transfer_syntaxes,
            vec!["1.2.840.10008.1.2.4.50"]
        );
    }

    #[test]
    fn parse_server_settings_form_collects_repeated_checkbox_keys() {
        let input = parse_server_settings_form(
            "dicom_port=11113&ae_title=PACSUI&accepted_transfer_syntaxes=1.2.840.10008.1.2.2&accepted_transfer_syntaxes=1.2.840.10008.1.2.1.99&preferred_transfer_syntaxes=1.2.840.10008.1.2.2&preferred_transfer_syntaxes=1.2.840.10008.1.2.1.99&max_associations=33&dimse_timeout_secs=46",
        );

        assert_eq!(input.dicom_port, "11113");
        assert_eq!(input.ae_title, "PACSUI");
        assert_eq!(
            input.accepted_transfer_syntaxes,
            vec!["1.2.840.10008.1.2.2", "1.2.840.10008.1.2.1.99"]
        );
        assert_eq!(
            input.preferred_transfer_syntaxes,
            vec!["1.2.840.10008.1.2.2", "1.2.840.10008.1.2.1.99"]
        );
    }

    #[test]
    fn server_settings_form_forces_required_implicit_transfer_syntax() {
        let settings = ServerSettingsFormInput {
            dicom_port: "11112".into(),
            ae_title: "PACSUI".into(),
            ae_whitelist_enabled: None,
            accepted_transfer_syntaxes: vec!["1.2.840.10008.1.2.1".into()],
            preferred_transfer_syntaxes: vec!["1.2.840.10008.1.2.1".into()],
            max_associations: "32".into(),
            dimse_timeout_secs: "45".into(),
        }
        .into_settings()
        .unwrap();

        assert_eq!(
            settings.accepted_transfer_syntaxes,
            vec!["1.2.840.10008.1.2", "1.2.840.10008.1.2.1"]
        );
        assert!(!settings.accept_all_transfer_syntaxes);
    }

    #[test]
    fn server_settings_form_marks_full_selection_as_accept_all() {
        let settings = ServerSettingsFormInput {
            dicom_port: "11112".into(),
            ae_title: "PACSUI".into(),
            ae_whitelist_enabled: None,
            accepted_transfer_syntaxes: all_supported_transfer_syntax_uids(),
            preferred_transfer_syntaxes: vec!["1.2.840.10008.1.2.1".into()],
            max_associations: "32".into(),
            dimse_timeout_secs: "45".into(),
        }
        .into_settings()
        .unwrap();

        assert!(settings.accept_all_transfer_syntaxes);
    }

    #[test]
    fn server_settings_form_uses_preferred_inputs_only_for_order() {
        let settings = ServerSettingsFormInput {
            dicom_port: "11112".into(),
            ae_title: "PACSUI".into(),
            ae_whitelist_enabled: None,
            accepted_transfer_syntaxes: vec![
                "1.2.840.10008.1.2.1".into(),
                "1.2.840.10008.1.2.2".into(),
                "1.2.840.10008.1.2.1.99".into(),
            ],
            preferred_transfer_syntaxes: vec![
                "1.2.840.10008.1.2.2".into(),
                "1.2.840.10008.1.2".into(),
            ],
            max_associations: "32".into(),
            dimse_timeout_secs: "45".into(),
        }
        .into_settings()
        .unwrap();

        assert_eq!(
            settings.preferred_transfer_syntaxes,
            vec![
                "1.2.840.10008.1.2.2",
                "1.2.840.10008.1.2",
                "1.2.840.10008.1.2.1",
                "1.2.840.10008.1.2.1.99",
            ]
        );
    }

    #[test]
    fn node_form_input_validates_required_fields() {
        let input = NodeFormInput {
            ae_title: " ".into(),
            host: "10.0.0.1".into(),
            port: "104".into(),
            description: String::new(),
            tls_enabled: None,
        };

        assert_eq!(input.into_node().unwrap_err().title, "Validation failed");
    }

    #[test]
    fn node_form_input_parses_valid_node() {
        let input = NodeFormInput {
            ae_title: "REMOTE_AE".into(),
            host: "10.0.0.4".into(),
            port: "11112".into(),
            description: "Secondary PACS".into(),
            tls_enabled: Some("on".into()),
        };

        let node = input.into_node().unwrap();
        assert_eq!(node.ae_title, "REMOTE_AE");
        assert_eq!(node.host, "10.0.0.4");
        assert_eq!(node.port, 11112);
        assert_eq!(node.description.as_deref(), Some("Secondary PACS"));
        assert!(node.tls_enabled);
    }

    #[test]
    fn audit_filters_build_store_query() {
        let filters = AuditFilters {
            action: Some("query".into()),
            resource: Some("Study".into()),
            status: Some("OK".into()),
            page: Some(2),
            page_size: Some(50),
            ..AuditFilters::default()
        };

        let query = filters.to_store_query();
        assert_eq!(query.action.as_deref(), Some("QUERY"));
        assert_eq!(query.resource.as_deref(), Some("study"));
        assert_eq!(query.status.as_deref(), Some("ok"));
        assert_eq!(query.limit, Some(50));
        assert_eq!(query.offset, Some(50));
    }

    #[test]
    fn audit_filters_encode_query_string() {
        let filters = AuditFilters {
            user_id: Some("admin".into()),
            resource_uid: Some("1.2.3".into()),
            page: Some(3),
            page_size: Some(10),
            ..AuditFilters::default()
        };

        let query_string = filters.to_query_string();
        assert!(query_string.contains("user_id=admin"));
        assert!(query_string.contains("resource_uid=1.2.3"));
        assert!(query_string.contains("page=3"));
        assert!(query_string.contains("page_size=10"));
    }
}
