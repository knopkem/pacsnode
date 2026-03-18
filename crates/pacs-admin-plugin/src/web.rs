use std::{
    collections::{BTreeSet, HashSet},
    convert::Infallible,
    sync::Arc,
    time::Duration,
};

use argon2::{
    password_hash::{PasswordHasher, SaltString},
    Argon2,
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
use chrono::Utc;
use pacs_core::{
    AuditLogEntry, AuditLogQuery, DicomNode, InstanceQuery, NewAuditLogEntry, PacsError,
    PasswordPolicy, SeriesQuery, ServerSettings, Study, StudyQuery, StudyUid, User, UserId,
    UserQuery, UserRole,
};
use pacs_dicom::supported_retrieve_transfer_syntaxes;
use pacs_dimse::DicomClient;
use pacs_plugin::{AppState, AuthenticatedUser, PacsEvent, PluginHealth, ResourceLevel};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use tracing::{error, warn};
use url::form_urlencoded;
use uuid::Uuid;

use crate::runtime::{ActivityEntry, AdminRuntime};

const ADMIN_CSS: &str = include_str!("../templates/static/admin.css");
const DEFAULT_STUDY_PAGE_SIZE: u32 = 20;
const MAX_STUDY_PAGE_SIZE: u32 = 100;
const DEFAULT_AUDIT_PAGE_SIZE: u32 = 25;
const MAX_AUDIT_PAGE_SIZE: u32 = 100;
const DEFAULT_USER_PAGE_SIZE: u32 = 25;
const MAX_USER_PAGE_SIZE: u32 = 100;
const NODE_VERIFY_TIMEOUT_SECS: u64 = 5;
const AUTH_PLUGIN_ID: &str = "basic-auth";
const ACCESS_COOKIE_NAME: &str = "pacsnode_access_token";
const REFRESH_COOKIE_NAME: &str = "pacsnode_refresh_token";

pub(crate) fn routes(runtime: Arc<AdminRuntime>) -> Router<AppState> {
    let route_prefix = runtime.route_prefix().to_string();
    let root_path = route_prefix.clone();
    let root_slash_path = format!("{route_prefix}/");
    let system_path = format!("{route_prefix}/system");
    let studies_path = format!("{route_prefix}/studies");
    let studies_list_path = format!("{route_prefix}/studies/list");
    let study_delete_path = format!("{route_prefix}/studies/{{study_uid}}");
    let users_path = format!("{route_prefix}/users");
    let user_policy_path = format!("{route_prefix}/users/policy");
    let user_edit_path = format!("{route_prefix}/users/{{user_id}}/edit");
    let user_delete_path = format!("{route_prefix}/users/{{user_id}}");
    let nodes_path = format!("{route_prefix}/nodes");
    let logout_path = format!("{route_prefix}/logout");
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
        .route(&users_path, get(users_page).post(save_user))
        .route(&logout_path, post(admin_logout))
        .route(&user_policy_path, post(save_password_policy))
        .route(&user_edit_path, get(edit_user))
        .route(&user_delete_path, delete(delete_user))
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
    logout_path: Option<String>,
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
    logout_path: Option<String>,
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
    storage_syntax_options: Vec<StorageTransferSyntaxOptionView>,
    preferred_syntax_order: Vec<PreferredTransferSyntaxItemView>,
}

#[derive(Template)]
#[template(path = "studies.html")]
struct StudiesPageTemplate {
    page_title: &'static str,
    route_prefix: String,
    active_nav: &'static str,
    logout_path: Option<String>,
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
    logout_path: Option<String>,
    nodes_markup: String,
}

#[derive(Template)]
#[template(path = "users.html")]
struct UsersPageTemplate {
    page_title: &'static str,
    route_prefix: String,
    active_nav: &'static str,
    logout_path: Option<String>,
    users_markup: String,
}

#[derive(Template)]
#[template(path = "audit.html")]
struct AuditPageTemplate {
    page_title: &'static str,
    route_prefix: String,
    active_nav: &'static str,
    logout_path: Option<String>,
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
#[template(path = "fragments/users_panel.html")]
struct UsersPanelTemplate {
    users_path: String,
    user_policy_path: String,
    filters: UserFilterView,
    form: UserFormView,
    policy_form: PasswordPolicyFormView,
    flash: Option<FlashView>,
    rows: Vec<UserRowView>,
    has_users: bool,
    has_active_filters: bool,
    result_summary: String,
    page_summary: String,
    empty_summary: String,
    policy_summary: String,
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
    storage_transfer_syntax: String,
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
            "storage_transfer_syntax" => input.storage_transfer_syntax = value,
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
    storage_transfer_syntax: String,
    max_associations: String,
    dimse_timeout_secs: String,
}

struct TransferSyntaxOptionView {
    uid: String,
    label: String,
    is_required: bool,
    accepted_selected: bool,
}

struct StorageTransferSyntaxOptionView {
    uid: String,
    label: String,
    selected: bool,
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
struct UserFilters {
    search: Option<String>,
    role: Option<String>,
    status: Option<String>,
    page_size: Option<u32>,
}

#[derive(Clone)]
struct UserFilterView {
    search: String,
    role: String,
    status: String,
    page_size: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct UserFormInput {
    user_id: Option<String>,
    username: String,
    display_name: String,
    email: String,
    password: String,
    role: String,
    attributes_json: String,
    is_active: Option<String>,
    filter_search: Option<String>,
    filter_role: Option<String>,
    filter_status: Option<String>,
    filter_page_size: Option<String>,
}

#[derive(Clone)]
struct UserFormView {
    user_id: String,
    username: String,
    display_name: String,
    email: String,
    role: String,
    attributes_json: String,
    is_active: bool,
    form_title: String,
    submit_label: String,
    password_placeholder: String,
    password_help: String,
    password_required: bool,
    filter_search: String,
    filter_role: String,
    filter_status: String,
    filter_page_size: String,
}

struct UserRowView {
    username: String,
    display_name: String,
    email: String,
    role_label: String,
    status_label: String,
    status_class: &'static str,
    locked_until: String,
    password_changed_at: String,
    edit_href: String,
    delete_href: String,
    delete_disabled: bool,
    is_current_user: bool,
}

#[derive(Default)]
struct UsersPanelOverrides {
    policy: Option<PasswordPolicy>,
    policy_form: Option<PasswordPolicyFormView>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PasswordPolicyFormInput {
    min_length: String,
    require_uppercase: Option<String>,
    require_digit: Option<String>,
    require_special: Option<String>,
    max_failed_attempts: String,
    lockout_duration_secs: String,
    max_age_days: String,
    filter_search: Option<String>,
    filter_role: Option<String>,
    filter_status: Option<String>,
    filter_page_size: Option<String>,
}

#[derive(Clone)]
struct PasswordPolicyFormView {
    min_length: String,
    require_uppercase: bool,
    require_digit: bool,
    require_special: bool,
    max_failed_attempts: String,
    lockout_duration_secs: String,
    max_age_days: String,
    filter_search: String,
    filter_role: String,
    filter_status: String,
    filter_page_size: String,
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
    headers: HeaderMap,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

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
        logout_path: shell_logout_path(&state, runtime.route_prefix(), &headers),
        server_info: server_info_view(&state.server_info),
        stats_markup,
        recent_activity_markup,
    })
}

async fn system_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

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
        logout_path: shell_logout_path(&state, runtime.route_prefix(), &headers),
        server_info: server_info_view(&state.server_info),
        settings_markup,
        plugin_rows,
    })
}

async fn save_system_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
    body: String,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

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
    headers: HeaderMap,
    Query(filters): Query<StudiesFilters>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

    let results_markup = match render_studies_results_markup(&state, &runtime, &filters).await {
        Ok(markup) => markup,
        Err(status) => return error_response(status, "admin studies browser failed to render"),
    };

    render_html(&StudiesPageTemplate {
        page_title: "Studies",
        route_prefix: runtime.route_prefix().to_string(),
        active_nav: "studies",
        logout_path: shell_logout_path(&state, runtime.route_prefix(), &headers),
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
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

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
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    let actor = match require_admin(&state, user) {
        Ok(actor) => actor,
        Err(message) => return error_response(StatusCode::FORBIDDEN, message),
    };

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
            user_id: Some(actor.user_id.clone()),
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

async fn users_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filters): Query<UserFilters>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

    let users_markup = match render_users_panel_markup(
        &state,
        &runtime,
        &filters,
        UserFormView::default_with_filters(&filters),
        None,
        UsersPanelOverrides::default(),
    )
    .await
    {
        Ok(markup) => markup,
        Err(status) => return error_response(status, "admin user management failed to load"),
    };

    render_html(&UsersPageTemplate {
        page_title: "Users",
        route_prefix: runtime.route_prefix().to_string(),
        active_nav: "users",
        logout_path: shell_logout_path(&state, runtime.route_prefix(), &headers),
        users_markup,
    })
}

async fn edit_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Query(filters): Query<UserFilters>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

    let user_id = match user_id.parse::<UserId>() {
        Ok(user_id) => user_id,
        Err(error) => {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                UserFormView::default_with_filters(&filters),
                Some(FlashView {
                    title: "User load failed".into(),
                    detail: format!("Invalid user id: {error}"),
                    tone_class: "flash-warning",
                }),
                UsersPanelOverrides::default(),
            )
            .await;
        }
    };

    let loaded_user = match state.store.get_user(&user_id).await {
        Ok(loaded_user) => loaded_user,
        Err(error) => {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                UserFormView::default_with_filters(&filters),
                Some(store_error_flash("User load failed", &error)),
                UsersPanelOverrides::default(),
            )
            .await;
        }
    };

    render_users_response(
        &state,
        &runtime,
        &headers,
        &filters,
        UserFormView::from_user(&loaded_user, &filters),
        Some(FlashView {
            title: "User loaded".into(),
            detail: format!(
                "Editing {}. Leave the password field blank to keep the current password hash.",
                loaded_user.username
            ),
            tone_class: "flash-info",
        }),
        UsersPanelOverrides::default(),
    )
    .await
}

async fn save_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
    Form(input): Form<UserFormInput>,
) -> Response {
    let actor = match require_admin(&state, user) {
        Ok(actor) => actor,
        Err(message) => return error_response(StatusCode::FORBIDDEN, message),
    };
    let filters = input.to_filters();
    let form = UserFormView::from_input(&input);

    let policy = match state.store.get_password_policy().await {
        Ok(policy) => policy,
        Err(error) => {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                form,
                Some(store_error_flash("Password policy load failed", &error)),
                UsersPanelOverrides::default(),
            )
            .await;
        }
    };

    let existing_user = match input.user_id() {
        Ok(Some(user_id)) => match state.store.get_user(&user_id).await {
            Ok(user) => Some(user),
            Err(error) => {
                return render_users_response(
                    &state,
                    &runtime,
                    &headers,
                    &filters,
                    form,
                    Some(store_error_flash("User load failed", &error)),
                    UsersPanelOverrides::default(),
                )
                .await;
            }
        },
        Ok(None) => None,
        Err(flash) => {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                form,
                Some(flash),
                users_panel_overrides(Some(policy), None),
            )
            .await;
        }
    };

    let username = match input.username_value() {
        Ok(username) => username,
        Err(flash) => {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                form,
                Some(flash),
                users_panel_overrides(Some(policy), None),
            )
            .await;
        }
    };
    let role = match input.role_value() {
        Ok(role) => role,
        Err(flash) => {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                form,
                Some(flash),
                users_panel_overrides(Some(policy), None),
            )
            .await;
        }
    };
    let attributes = match input.attributes_value() {
        Ok(attributes) => attributes,
        Err(flash) => {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                form,
                Some(flash),
                users_panel_overrides(Some(policy), None),
            )
            .await;
        }
    };

    match state.store.get_user_by_username(&username).await {
        Ok(other_user) => {
            if existing_user.as_ref().map(|user| user.id) != Some(other_user.id) {
                return render_users_response(
                    &state,
                    &runtime,
                    &headers,
                    &filters,
                    form,
                    Some(validation_flash("Username is already in use.")),
                    users_panel_overrides(Some(policy), None),
                )
                .await;
            }
        }
        Err(PacsError::NotFound { .. }) => {}
        Err(error) => {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                form,
                Some(store_error_flash("Username check failed", &error)),
                users_panel_overrides(Some(policy), None),
            )
            .await;
        }
    }

    let is_active = input.is_active.is_some();
    let password = input.password.trim();
    let actor_user_id = actor.user_id.clone();
    if existing_user
        .as_ref()
        .is_some_and(|target| target.id.to_string() == actor_user_id)
        && !is_active
    {
        return render_users_response(
            &state,
            &runtime,
            &headers,
            &filters,
            form,
            Some(validation_flash("You cannot deactivate your own account.")),
            users_panel_overrides(Some(policy), None),
        )
        .await;
    }
    if existing_user
        .as_ref()
        .is_some_and(|target| target.id.to_string() == actor_user_id)
        && role != UserRole::Admin
    {
        return render_users_response(
            &state,
            &runtime,
            &headers,
            &filters,
            form,
            Some(validation_flash(
                "You cannot remove the admin role from your own account.",
            )),
            users_panel_overrides(Some(policy), None),
        )
        .await;
    }

    let mut next_user = if let Some(existing_user) = existing_user.clone() {
        let mut user = existing_user.clone();
        user.username = username;
        user.display_name = normalize_string_field(&input.display_name);
        user.email = normalize_string_field(&input.email);
        user.role = role;
        user.attributes = attributes;
        user.is_active = is_active;
        user
    } else {
        User {
            id: UserId::new(),
            username,
            display_name: normalize_string_field(&input.display_name),
            email: normalize_string_field(&input.email),
            password_hash: String::new(),
            role,
            attributes,
            is_active,
            failed_login_attempts: 0,
            locked_until: None,
            password_changed_at: None,
            created_at: None,
            updated_at: None,
        }
    };

    if let Some(existing_user) = existing_user.as_ref() {
        if existing_user.role == UserRole::Admin
            && existing_user.is_active
            && (next_user.role != UserRole::Admin || !next_user.is_active)
        {
            let active_admins = match active_admins(&state).await {
                Ok(active_admins) => active_admins,
                Err(error) => {
                    return render_users_response(
                        &state,
                        &runtime,
                        &headers,
                        &filters,
                        form,
                        Some(store_error_flash("Admin safety check failed", &error)),
                        users_panel_overrides(Some(policy), None),
                    )
                    .await;
                }
            };
            if active_admins.len() <= 1 {
                return render_users_response(
                    &state,
                    &runtime,
                    &headers,
                    &filters,
                    form,
                    Some(validation_flash(
                        "At least one active admin account must remain enabled.",
                    )),
                    users_panel_overrides(Some(policy), None),
                )
                .await;
            }
        }
    }

    let mut revoke_refresh_tokens = false;
    if password.is_empty() {
        if let Some(existing_user) = existing_user.as_ref() {
            next_user.password_hash = existing_user.password_hash.clone();
            next_user.password_changed_at = existing_user.password_changed_at;
        } else {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                form,
                Some(validation_flash(
                    "Password is required for new local users.",
                )),
                users_panel_overrides(Some(policy), None),
            )
            .await;
        }
    } else {
        if let Err(flash) = validate_password_against_policy(password, &policy) {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                form,
                Some(flash),
                users_panel_overrides(Some(policy), None),
            )
            .await;
        }
        let password_hash = match hash_local_password(password) {
            Ok(password_hash) => password_hash,
            Err(detail) => {
                return render_users_response(
                    &state,
                    &runtime,
                    &headers,
                    &filters,
                    form,
                    Some(FlashView {
                        title: "Password hashing failed".into(),
                        detail,
                        tone_class: "flash-danger",
                    }),
                    users_panel_overrides(Some(policy), None),
                )
                .await;
            }
        };
        next_user.password_hash = password_hash;
        next_user.password_changed_at = Some(Utc::now());
        next_user.failed_login_attempts = 0;
        next_user.locked_until = None;
        revoke_refresh_tokens = true;
    }

    if !next_user.is_active {
        next_user.failed_login_attempts = 0;
        next_user.locked_until = None;
        revoke_refresh_tokens = true;
    }

    if let Err(error) = state.store.store_user(&next_user).await {
        return render_users_response(
            &state,
            &runtime,
            &headers,
            &filters,
            form,
            Some(store_error_flash("User save failed", &error)),
            users_panel_overrides(Some(policy), None),
        )
        .await;
    }

    if revoke_refresh_tokens {
        if let Err(error) = state.store.revoke_refresh_tokens(&next_user.id).await {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                UserFormView::default_with_filters(&filters),
                Some(store_error_flash("Session revocation failed", &error)),
                users_panel_overrides(Some(policy), None),
            )
            .await;
        }
    }

    maybe_store_admin_audit_log(
        &state,
        &headers,
        &actor,
        if existing_user.is_some() {
            "USER_UPDATE"
        } else {
            "USER_CREATE"
        },
        "user",
        Some(next_user.id.to_string()),
        serde_json::json!({
            "actor_username": actor.username,
            "actor_role": actor.role,
            "auth_method": "local",
            "target_username": next_user.username,
            "target_role": next_user.role.as_str(),
            "target_is_active": next_user.is_active,
            "password_rotated": !password.is_empty(),
            "refresh_tokens_revoked": revoke_refresh_tokens,
        }),
    )
    .await;

    let flash = FlashView {
        title: if existing_user.is_some() {
            "User updated".into()
        } else {
            "User created".into()
        },
        detail: if revoke_refresh_tokens {
            format!(
                "{} was saved and existing refresh sessions were revoked.",
                next_user.username
            )
        } else {
            format!("{} was saved successfully.", next_user.username)
        },
        tone_class: "flash-success",
    };

    render_users_response(
        &state,
        &runtime,
        &headers,
        &filters,
        UserFormView::default_with_filters(&filters),
        Some(flash),
        users_panel_overrides(Some(policy), None),
    )
    .await
}

async fn save_password_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
    Form(input): Form<PasswordPolicyFormInput>,
) -> Response {
    let actor = match require_admin(&state, user) {
        Ok(actor) => actor,
        Err(message) => return error_response(StatusCode::FORBIDDEN, message),
    };
    let filters = input.to_filters();
    let policy_form = PasswordPolicyFormView::from_input(&input);
    let policy = match input.into_policy() {
        Ok(policy) => policy,
        Err(flash) => {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                UserFormView::default_with_filters(&filters),
                Some(flash),
                users_panel_overrides(None, Some(policy_form)),
            )
            .await;
        }
    };

    if let Err(error) = state.store.upsert_password_policy(&policy).await {
        return render_users_response(
            &state,
            &runtime,
            &headers,
            &filters,
            UserFormView::default_with_filters(&filters),
            Some(store_error_flash("Password policy save failed", &error)),
            users_panel_overrides(Some(policy.clone()), Some(policy_form)),
        )
        .await;
    }

    maybe_store_admin_audit_log(
        &state,
        &headers,
        &actor,
        "PASSWORD_POLICY_UPDATE",
        "password_policy",
        None,
        serde_json::json!({
            "actor_username": actor.username,
            "actor_role": actor.role,
            "auth_method": "local",
            "min_length": policy.min_length,
            "require_uppercase": policy.require_uppercase,
            "require_digit": policy.require_digit,
            "require_special": policy.require_special,
            "max_failed_attempts": policy.max_failed_attempts,
            "lockout_duration_secs": policy.lockout_duration_secs,
            "max_age_days": policy.max_age_days,
        }),
    )
    .await;

    render_users_response(
        &state,
        &runtime,
        &headers,
        &filters,
        UserFormView::default_with_filters(&filters),
        Some(FlashView {
            title: "Password policy updated".into(),
            detail: "New local password rules were saved and apply to future password changes immediately. Existing password hashes remain unchanged.".into(),
            tone_class: "flash-success",
        }),
        users_panel_overrides(
            Some(policy.clone()),
            Some(PasswordPolicyFormView::from_policy(&policy, &filters)),
        ),
    )
    .await
}

async fn delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Query(filters): Query<UserFilters>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    let actor = match require_admin(&state, user) {
        Ok(actor) => actor,
        Err(message) => return error_response(StatusCode::FORBIDDEN, message),
    };
    let user_id = match user_id.parse::<UserId>() {
        Ok(user_id) => user_id,
        Err(error) => {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                UserFormView::default_with_filters(&filters),
                Some(FlashView {
                    title: "User removal failed".into(),
                    detail: format!("Invalid user id: {error}"),
                    tone_class: "flash-warning",
                }),
                UsersPanelOverrides::default(),
            )
            .await;
        }
    };

    let target_user = match state.store.get_user(&user_id).await {
        Ok(target_user) => target_user,
        Err(error) => {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                UserFormView::default_with_filters(&filters),
                Some(store_error_flash("User removal failed", &error)),
                UsersPanelOverrides::default(),
            )
            .await;
        }
    };

    if target_user.id.to_string() == actor.user_id {
        return render_users_response(
            &state,
            &runtime,
            &headers,
            &filters,
            UserFormView::default_with_filters(&filters),
            Some(validation_flash("You cannot delete your own account.")),
            UsersPanelOverrides::default(),
        )
        .await;
    }

    if target_user.role == UserRole::Admin && target_user.is_active {
        let active_admins = match active_admins(&state).await {
            Ok(active_admins) => active_admins,
            Err(error) => {
                return render_users_response(
                    &state,
                    &runtime,
                    &headers,
                    &filters,
                    UserFormView::default_with_filters(&filters),
                    Some(store_error_flash("Admin safety check failed", &error)),
                    UsersPanelOverrides::default(),
                )
                .await;
            }
        };
        if active_admins.len() <= 1 {
            return render_users_response(
                &state,
                &runtime,
                &headers,
                &filters,
                UserFormView::default_with_filters(&filters),
                Some(validation_flash(
                    "At least one active admin account must remain enabled.",
                )),
                UsersPanelOverrides::default(),
            )
            .await;
        }
    }

    if let Err(error) = state.store.revoke_refresh_tokens(&target_user.id).await {
        return render_users_response(
            &state,
            &runtime,
            &headers,
            &filters,
            UserFormView::default_with_filters(&filters),
            Some(store_error_flash("Session revocation failed", &error)),
            UsersPanelOverrides::default(),
        )
        .await;
    }

    if let Err(error) = state.store.delete_user(&target_user.id).await {
        return render_users_response(
            &state,
            &runtime,
            &headers,
            &filters,
            UserFormView::default_with_filters(&filters),
            Some(store_error_flash("User removal failed", &error)),
            UsersPanelOverrides::default(),
        )
        .await;
    }

    maybe_store_admin_audit_log(
        &state,
        &headers,
        &actor,
        "USER_DELETE",
        "user",
        Some(target_user.id.to_string()),
        serde_json::json!({
            "actor_username": actor.username,
            "actor_role": actor.role,
            "auth_method": "local",
            "target_username": target_user.username,
            "target_role": target_user.role.as_str(),
            "refresh_tokens_revoked": true,
        }),
    )
    .await;

    render_users_response(
        &state,
        &runtime,
        &headers,
        &filters,
        UserFormView::default_with_filters(&filters),
        Some(FlashView {
            title: "User deleted".into(),
            detail: format!(
                "{} was removed and existing refresh sessions were revoked.",
                target_user.username
            ),
            tone_class: "flash-warning",
        }),
        UsersPanelOverrides::default(),
    )
    .await
}

async fn nodes_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

    let nodes_markup =
        match render_nodes_panel_markup(&state, &runtime, NodeFormView::default(), None).await {
            Ok(markup) => markup,
            Err(status) => return error_response(status, "admin node management failed to load"),
        };

    render_html(&NodesPageTemplate {
        page_title: "Nodes",
        route_prefix: runtime.route_prefix().to_string(),
        active_nav: "nodes",
        logout_path: shell_logout_path(&state, runtime.route_prefix(), &headers),
        nodes_markup,
    })
}

async fn edit_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(ae_title): Path<String>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

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
    user: Option<Extension<AuthenticatedUser>>,
    Form(input): Form<NodeFormInput>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

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
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

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
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

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
    headers: HeaderMap,
    Query(filters): Query<AuditFilters>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

    let results_markup = match render_audit_results_markup(&state, &runtime, &filters).await {
        Ok(markup) => markup,
        Err(status) => return error_response(status, "admin audit explorer failed to render"),
    };

    render_html(&AuditPageTemplate {
        page_title: "Audit Log",
        route_prefix: runtime.route_prefix().to_string(),
        active_nav: "audit",
        logout_path: shell_logout_path(&state, runtime.route_prefix(), &headers),
        audit_path: audit_page_path(runtime.route_prefix()),
        audit_results_path: audit_results_path(runtime.route_prefix()),
        filters: AuditFilterView::from_filters(&filters),
        results_markup,
    })
}

async fn admin_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    let actor = match require_admin(&state, user) {
        Ok(actor) => actor,
        Err(message) => return error_response(StatusCode::FORBIDDEN, message),
    };

    if state.plugins.has_plugin(AUTH_PLUGIN_ID) {
        if let Ok(user_id) = actor.user_id.parse::<UserId>() {
            if let Err(error) = state.store.revoke_refresh_tokens(&user_id).await {
                return error_response(
                    pacs_error_to_status(&error),
                    "admin logout failed to revoke active sessions",
                );
            }
        }
    }

    let mut response = Redirect::to(runtime.route_prefix()).into_response();
    clear_auth_cookies(
        response.headers_mut(),
        request_uses_secure_transport(&headers),
    );
    response
}

async fn audit_results_fragment(
    State(state): State<AppState>,
    Query(filters): Query<AuditFilters>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

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
    State(state): State<AppState>,
    Extension(runtime): Extension<Arc<AdminRuntime>>,
    user: Option<Extension<AuthenticatedUser>>,
) -> Response {
    if let Err(message) = require_admin(&state, user) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

    let mut rx = runtime.subscribe();
    let runtime_for_stream = Arc::clone(&runtime);

    let stream = stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if should_emit_stats(&event) {
                        match render_stats_markup(&runtime_for_stream).await {
                            Ok(markup) => {
                                yield Ok::<Event, Infallible>(Event::default().event("stats").data(markup));
                            }
                            Err(status) => {
                                error!(?status, "failed to render admin stats event");
                            }
                        }
                    }

                    let activity_item = activity_view_from_entry(crate::runtime::activity_from_event(&event));
                    match (RecentActivityItemsTemplate { entries: vec![activity_item.clone()] }).render() {
                        Ok(markup) => {
                            yield Ok::<Event, Infallible>(Event::default().event("activity").data(markup));
                        }
                        Err(error) => {
                            error!(error = %error, "failed to render admin activity fragment");
                        }
                    }

                    match (ToastTemplate { item: activity_item }).render() {
                        Ok(markup) => {
                            yield Ok::<Event, Infallible>(Event::default().event("toast").data(markup));
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

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response()
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
    let storage_syntax_options =
        storage_transfer_syntax_option_views(form.storage_transfer_syntax.as_str());
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
        storage_syntax_options,
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

async fn render_users_panel_markup(
    state: &AppState,
    runtime: &AdminRuntime,
    filters: &UserFilters,
    form: UserFormView,
    flash: Option<FlashView>,
    overrides: UsersPanelOverrides,
) -> Result<String, StatusCode> {
    let UsersPanelOverrides {
        policy,
        policy_form,
    } = overrides;

    let policy = match policy {
        Some(policy) => policy,
        None => state
            .store
            .get_password_policy()
            .await
            .map_err(internal_store_error)?,
    };
    let users = state
        .store
        .query_users(&filters.to_store_query())
        .await
        .map_err(internal_store_error)?;
    let rows = users
        .into_iter()
        .map(|user| user_row_view(runtime.route_prefix(), filters, user, &form.user_id))
        .collect::<Vec<_>>();
    let row_count = rows.len();
    let policy_form =
        policy_form.unwrap_or_else(|| PasswordPolicyFormView::from_policy(&policy, filters));

    UsersPanelTemplate {
        users_path: users_page_path(runtime.route_prefix()),
        user_policy_path: users_policy_path(runtime.route_prefix()),
        filters: UserFilterView::from_filters(filters),
        form,
        policy_form,
        flash,
        has_users: !rows.is_empty(),
        rows,
        has_active_filters: filters.has_active_filters(),
        result_summary: format!("Showing {} local user(s)", row_count),
        page_summary: format!("Up to {} rows", filters.page_size()),
        empty_summary: if filters.has_active_filters() {
            "Adjust the filters and try again.".into()
        } else {
            "Create the next local operator account here.".into()
        },
        policy_summary: password_policy_summary(&policy),
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
            logout_path: shell_logout_path(state, runtime.route_prefix(), headers),
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
            logout_path: shell_logout_path(state, runtime.route_prefix(), headers),
            nodes_markup: markup,
        })
    }
}

async fn render_users_response(
    state: &AppState,
    runtime: &AdminRuntime,
    headers: &HeaderMap,
    filters: &UserFilters,
    form: UserFormView,
    flash: Option<FlashView>,
    overrides: UsersPanelOverrides,
) -> Response {
    let markup =
        match render_users_panel_markup(state, runtime, filters, form, flash, overrides).await {
            Ok(markup) => markup,
            Err(status) => return error_response(status, "admin user management failed to render"),
        };

    if is_htmx_request(headers) {
        html_markup_response(markup)
    } else {
        render_html(&UsersPageTemplate {
            page_title: "Users",
            route_prefix: runtime.route_prefix().to_string(),
            active_nav: "users",
            logout_path: shell_logout_path(state, runtime.route_prefix(), headers),
            users_markup: markup,
        })
    }
}

fn shell_logout_path(state: &AppState, route_prefix: &str, headers: &HeaderMap) -> Option<String> {
    (state.plugins.has_plugin(AUTH_PLUGIN_ID) && has_local_auth_cookie(headers))
        .then(|| format!("{route_prefix}/logout"))
}

fn has_local_auth_cookie(headers: &HeaderMap) -> bool {
    cookie_value(headers, ACCESS_COOKIE_NAME).is_some()
        || cookie_value(headers, REFRESH_COOKIE_NAME).is_some()
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| {
            raw.split(';').find_map(|entry| {
                let (cookie_name, cookie_value) = entry.trim().split_once('=')?;
                (cookie_name == name).then(|| cookie_value.to_string())
            })
        })
}

fn clear_auth_cookies(headers: &mut HeaderMap, secure: bool) {
    append_set_cookie(headers, clear_cookie(ACCESS_COOKIE_NAME, secure));
    append_set_cookie(headers, clear_cookie(REFRESH_COOKIE_NAME, secure));
}

fn append_set_cookie(headers: &mut HeaderMap, value: String) {
    match axum::http::HeaderValue::from_str(&value) {
        Ok(value) => {
            headers.append(header::SET_COOKIE, value);
        }
        Err(error) => {
            error!(error = %error, "failed to encode admin logout cookie header");
        }
    }
}

fn clear_cookie(name: &str, secure: bool) -> String {
    let secure_suffix = if secure { "; Secure" } else { "" };
    format!("{name}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax{secure_suffix}")
}

fn request_uses_secure_transport(headers: &HeaderMap) -> bool {
    forwarded_proto(headers).is_some_and(|proto| proto.eq_ignore_ascii_case("https"))
        || headers
            .get("X-Forwarded-Ssl")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("on"))
}

fn forwarded_proto(headers: &HeaderMap) -> Option<&str> {
    if let Some(proto) = headers
        .get("X-Forwarded-Proto")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(proto);
    }

    headers
        .get("Forwarded")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value.split(';').find_map(|segment| {
                let (key, forwarded_value) = segment.trim().split_once('=')?;
                key.eq_ignore_ascii_case("proto")
                    .then(|| forwarded_value.trim_matches('"').trim())
            })
        })
        .filter(|value| !value.is_empty())
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

fn user_row_view(
    route_prefix: &str,
    filters: &UserFilters,
    user: User,
    current_form_user_id: &str,
) -> UserRowView {
    let query_string = filters.to_query_string();
    let is_current_user = current_form_user_id == user.id.to_string();
    UserRowView {
        username: user.username,
        display_name: fallback_string(user.display_name),
        email: fallback_string(user.email),
        role_label: display_user_role(user.role).into(),
        status_label: if user.is_active {
            "Active".into()
        } else {
            "Inactive".into()
        },
        status_class: if user.is_active {
            "status-healthy"
        } else {
            "status-unhealthy"
        },
        locked_until: user
            .locked_until
            .map(|value| value.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "-".into()),
        password_changed_at: user
            .password_changed_at
            .map(|value| value.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "-".into()),
        edit_href: path_with_query(
            &format!("{route_prefix}/users/{}/edit", user.id),
            &query_string,
        ),
        delete_href: path_with_query(&format!("{route_prefix}/users/{}", user.id), &query_string),
        delete_disabled: is_current_user,
        is_current_user,
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

async fn active_admins(state: &AppState) -> Result<Vec<User>, PacsError> {
    state
        .store
        .query_users(&UserQuery {
            search: None,
            role: Some(UserRole::Admin),
            is_active: Some(true),
            limit: Some(MAX_USER_PAGE_SIZE),
            offset: Some(0),
        })
        .await
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

fn require_admin(
    state: &AppState,
    user: Option<Extension<AuthenticatedUser>>,
) -> Result<AuthenticatedUser, &'static str> {
    if !state.plugins.has_plugin(AUTH_PLUGIN_ID) {
        return Ok(auth_disabled_admin_user());
    }

    match user {
        Some(Extension(user)) if user.role == UserRole::Admin.as_str() => Ok(user),
        Some(_) => Err("admin access requires an account with the admin role"),
        None => Err("admin access requires authentication"),
    }
}

fn auth_disabled_admin_user() -> AuthenticatedUser {
    AuthenticatedUser::new(
        "auth-disabled-admin",
        "local-admin",
        UserRole::Admin.as_str(),
        serde_json::json!({"auth_disabled": true}),
    )
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

fn normalize_string_field(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn password_policy_summary(policy: &PasswordPolicy) -> String {
    let mut requirements = vec![format!("minimum {} characters", policy.min_length)];
    if policy.require_uppercase {
        requirements.push("one uppercase letter".into());
    }
    if policy.require_digit {
        requirements.push("one numeric digit".into());
    }
    if policy.require_special {
        requirements.push("one special character".into());
    }

    let max_age_summary = policy
        .max_age_days
        .map(|days| format!(" Passwords expire after {days} day(s)."))
        .unwrap_or_default();

    format!(
        "Passwords must include {}. Accounts lock after {} failed attempts for {} seconds.",
        requirements.join(", "),
        policy.max_failed_attempts,
        policy.lockout_duration_secs
    ) + &max_age_summary
}

async fn maybe_store_admin_audit_log(
    state: &AppState,
    headers: &HeaderMap,
    actor: &AuthenticatedUser,
    action: &'static str,
    resource: &'static str,
    resource_uid: Option<String>,
    details: Value,
) {
    let entry = NewAuditLogEntry {
        user_id: Some(actor.user_id.clone()),
        action: action.into(),
        resource: resource.into(),
        resource_uid,
        source_ip: request_source_ip(headers),
        status: "ok".into(),
        details,
    };

    if let Err(error) = state.store.store_audit_log(&entry).await {
        warn!(action, resource, error = %error, "failed to persist admin audit log");
    }
}

fn request_source_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn validate_password_against_policy(
    password: &str,
    policy: &PasswordPolicy,
) -> Result<(), FlashView> {
    if password.chars().count() < policy.min_length as usize {
        return Err(validation_flash(&format!(
            "Password must be at least {} characters long.",
            policy.min_length
        )));
    }
    if policy.require_uppercase && !password.chars().any(|ch| ch.is_ascii_uppercase()) {
        return Err(validation_flash(
            "Password must include at least one uppercase letter.",
        ));
    }
    if policy.require_digit && !password.chars().any(|ch| ch.is_ascii_digit()) {
        return Err(validation_flash(
            "Password must include at least one numeric digit.",
        ));
    }
    if policy.require_special && !password.chars().any(|ch| !ch.is_ascii_alphanumeric()) {
        return Err(validation_flash(
            "Password must include at least one special character.",
        ));
    }
    Ok(())
}

fn hash_local_password(password: &str) -> Result<String, String> {
    let salt =
        SaltString::encode_b64(Uuid::new_v4().as_bytes()).map_err(|error| error.to_string())?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| error.to_string())
}

fn display_user_role(role: UserRole) -> &'static str {
    match role {
        UserRole::Admin => "Admin",
        UserRole::Radiologist => "Radiologist",
        UserRole::Technologist => "Technologist",
        UserRole::Viewer => "Viewer",
        UserRole::Uploader => "Uploader",
    }
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
        "1.2.840.10008.1.2.4.201" => "HTJ2K Lossless Only".into(),
        "1.2.840.10008.1.2.4.202" => "HTJ2K".into(),
        _ => uid.to_string(),
    }
}

fn storage_transfer_syntax_option_views(
    selected_value: &str,
) -> Vec<StorageTransferSyntaxOptionView> {
    let mut options = vec![StorageTransferSyntaxOptionView {
        uid: String::new(),
        label: "Store as received (no recoding)".into(),
        selected: selected_value.trim().is_empty(),
    }];

    options.extend(supported_retrieve_transfer_syntaxes().iter().map(|uid| {
        StorageTransferSyntaxOptionView {
            uid: (*uid).to_string(),
            label: transfer_syntax_label(uid),
            selected: selected_value == *uid,
        }
    }));

    options
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

fn users_page_path(route_prefix: &str) -> String {
    format!("{route_prefix}/users")
}

fn users_policy_path(route_prefix: &str) -> String {
    format!("{route_prefix}/users/policy")
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
        PacsError::Forbidden(_) => StatusCode::FORBIDDEN,
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

impl UserFilters {
    fn page_size(&self) -> u32 {
        self.page_size
            .unwrap_or(DEFAULT_USER_PAGE_SIZE)
            .clamp(1, MAX_USER_PAGE_SIZE)
    }

    fn has_active_filters(&self) -> bool {
        [
            self.search.as_deref(),
            self.role.as_deref(),
            self.status.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| !value.trim().is_empty())
    }

    fn to_store_query(&self) -> UserQuery {
        UserQuery {
            search: normalized_filter(&self.search),
            role: normalized_filter(&self.role)
                .map(|value| value.parse())
                .transpose()
                .unwrap_or(None),
            is_active: match normalized_filter(&self.status).as_deref() {
                Some("active") => Some(true),
                Some("inactive") => Some(false),
                _ => None,
            },
            limit: Some(self.page_size()),
            offset: Some(0),
        }
    }

    fn to_query_string(&self) -> String {
        let mut serializer = form_urlencoded::Serializer::new(String::new());
        if let Some(value) = normalized_filter(&self.search) {
            serializer.append_pair("search", &value);
        }
        if let Some(value) = normalized_filter(&self.role) {
            serializer.append_pair("role", &value);
        }
        if let Some(value) = normalized_filter(&self.status) {
            serializer.append_pair("status", &value);
        }
        serializer.append_pair("page_size", &self.page_size().to_string());
        serializer.finish()
    }
}

impl UserFilterView {
    fn from_filters(filters: &UserFilters) -> Self {
        Self {
            search: filters
                .search
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            role: filters
                .role
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

impl UserFormInput {
    fn user_id(&self) -> Result<Option<UserId>, FlashView> {
        self.user_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                value
                    .parse::<UserId>()
                    .map_err(|error| validation_flash(&format!("Invalid user id: {error}")))
            })
            .transpose()
    }

    fn username_value(&self) -> Result<String, FlashView> {
        let username = self.username.trim();
        if username.is_empty() {
            return Err(validation_flash("Username is required."));
        }
        if !username
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '@'))
        {
            return Err(validation_flash(
                "Username may contain only ASCII letters, digits, '.', '_', '-', and '@'.",
            ));
        }
        Ok(username.to_string())
    }

    fn role_value(&self) -> Result<UserRole, FlashView> {
        self.role
            .trim()
            .parse::<UserRole>()
            .map_err(|_| validation_flash("Role must be one of the supported local user roles."))
    }

    fn attributes_value(&self) -> Result<Value, FlashView> {
        let raw = self.attributes_json.trim();
        if raw.is_empty() {
            return Ok(Value::Object(Default::default()));
        }
        let parsed: Value = serde_json::from_str(raw)
            .map_err(|error| validation_flash(&format!("Attributes JSON is invalid: {error}")))?;
        if !parsed.is_object() {
            return Err(validation_flash("Attributes JSON must be an object."));
        }
        Ok(parsed)
    }

    fn to_filters(&self) -> UserFilters {
        UserFilters {
            search: self.filter_search.clone(),
            role: self.filter_role.clone(),
            status: self.filter_status.clone(),
            page_size: self
                .filter_page_size
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .and_then(|value| value.parse::<u32>().ok()),
        }
    }
}

impl PasswordPolicyFormInput {
    fn to_filters(&self) -> UserFilters {
        UserFilters {
            search: self.filter_search.clone(),
            role: self.filter_role.clone(),
            status: self.filter_status.clone(),
            page_size: self
                .filter_page_size
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .and_then(|value| value.parse::<u32>().ok()),
        }
    }

    fn into_policy(self) -> Result<PasswordPolicy, FlashView> {
        let min_length =
            self.min_length.trim().parse::<u32>().map_err(|_| {
                validation_flash("Minimum password length must be a positive integer.")
            })?;
        if min_length < 8 {
            return Err(validation_flash(
                "Minimum password length must be at least 8 characters.",
            ));
        }

        let max_failed_attempts = self
            .max_failed_attempts
            .trim()
            .parse::<u32>()
            .map_err(|_| validation_flash("Max failed attempts must be a positive integer."))?;
        if max_failed_attempts == 0 {
            return Err(validation_flash(
                "Max failed attempts must be greater than zero.",
            ));
        }

        let lockout_duration_secs = self
            .lockout_duration_secs
            .trim()
            .parse::<u32>()
            .map_err(|_| validation_flash("Lockout duration must be a positive integer."))?;
        if lockout_duration_secs == 0 {
            return Err(validation_flash(
                "Lockout duration must be greater than zero.",
            ));
        }

        let max_age_days = if self.max_age_days.trim().is_empty() {
            None
        } else {
            Some(self.max_age_days.trim().parse::<u32>().map_err(|_| {
                validation_flash(
                    "Password max age must be empty or a positive integer number of days.",
                )
            })?)
        };
        if max_age_days == Some(0) {
            return Err(validation_flash(
                "Password max age must be empty or greater than zero days.",
            ));
        }

        Ok(PasswordPolicy {
            min_length,
            require_uppercase: self.require_uppercase.is_some(),
            require_digit: self.require_digit.is_some(),
            require_special: self.require_special.is_some(),
            max_failed_attempts,
            lockout_duration_secs,
            max_age_days,
        })
    }
}

impl PasswordPolicyFormView {
    fn from_input(input: &PasswordPolicyFormInput) -> Self {
        let filters = input.to_filters();
        Self {
            min_length: input.min_length.trim().to_string(),
            require_uppercase: input.require_uppercase.is_some(),
            require_digit: input.require_digit.is_some(),
            require_special: input.require_special.is_some(),
            max_failed_attempts: input.max_failed_attempts.trim().to_string(),
            lockout_duration_secs: input.lockout_duration_secs.trim().to_string(),
            max_age_days: input.max_age_days.trim().to_string(),
            filter_search: filters.search.as_deref().unwrap_or_default().to_string(),
            filter_role: filters.role.as_deref().unwrap_or_default().to_string(),
            filter_status: filters.status.as_deref().unwrap_or_default().to_string(),
            filter_page_size: filters.page_size().to_string(),
        }
    }

    fn from_policy(policy: &PasswordPolicy, filters: &UserFilters) -> Self {
        Self {
            min_length: policy.min_length.to_string(),
            require_uppercase: policy.require_uppercase,
            require_digit: policy.require_digit,
            require_special: policy.require_special,
            max_failed_attempts: policy.max_failed_attempts.to_string(),
            lockout_duration_secs: policy.lockout_duration_secs.to_string(),
            max_age_days: policy
                .max_age_days
                .map(|days| days.to_string())
                .unwrap_or_default(),
            filter_search: filters.search.as_deref().unwrap_or_default().to_string(),
            filter_role: filters.role.as_deref().unwrap_or_default().to_string(),
            filter_status: filters.status.as_deref().unwrap_or_default().to_string(),
            filter_page_size: filters.page_size().to_string(),
        }
    }
}

impl UserFormView {
    fn default_with_filters(filters: &UserFilters) -> Self {
        Self {
            user_id: String::new(),
            username: String::new(),
            display_name: String::new(),
            email: String::new(),
            role: UserRole::Viewer.as_str().into(),
            attributes_json: "{}".into(),
            is_active: true,
            form_title: "Create a local user".into(),
            submit_label: "Create user".into(),
            password_placeholder: "Temporary password".into(),
            password_help: "New local users receive a freshly hashed password immediately.".into(),
            password_required: true,
            filter_search: filters.search.as_deref().unwrap_or_default().to_string(),
            filter_role: filters.role.as_deref().unwrap_or_default().to_string(),
            filter_status: filters.status.as_deref().unwrap_or_default().to_string(),
            filter_page_size: filters.page_size().to_string(),
        }
    }

    fn from_input(input: &UserFormInput) -> Self {
        let filters = input.to_filters();
        let is_editing = input
            .user_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        Self {
            user_id: input
                .user_id
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            username: input.username.trim().to_string(),
            display_name: input.display_name.trim().to_string(),
            email: input.email.trim().to_string(),
            role: input.role.trim().to_string(),
            attributes_json: if input.attributes_json.trim().is_empty() {
                "{}".into()
            } else {
                input.attributes_json.trim().to_string()
            },
            is_active: input.is_active.is_some(),
            form_title: if is_editing {
                "Update a local user".into()
            } else {
                "Create a local user".into()
            },
            submit_label: if is_editing {
                "Save user".into()
            } else {
                "Create user".into()
            },
            password_placeholder: if is_editing {
                "Leave blank to keep current password".into()
            } else {
                "Temporary password".into()
            },
            password_help: if is_editing {
                "Set a new password only when you want to rotate credentials immediately.".into()
            } else {
                "The password must satisfy the active local password policy.".into()
            },
            password_required: !is_editing,
            filter_search: filters.search.as_deref().unwrap_or_default().to_string(),
            filter_role: filters.role.as_deref().unwrap_or_default().to_string(),
            filter_status: filters.status.as_deref().unwrap_or_default().to_string(),
            filter_page_size: filters.page_size().to_string(),
        }
    }

    fn from_user(user: &User, filters: &UserFilters) -> Self {
        Self {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            display_name: user.display_name.clone().unwrap_or_default(),
            email: user.email.clone().unwrap_or_default(),
            role: user.role.as_str().into(),
            attributes_json: serde_json::to_string(&user.attributes)
                .unwrap_or_else(|_| "{}".into()),
            is_active: user.is_active,
            form_title: format!("Update {}", user.username),
            submit_label: "Save user".into(),
            password_placeholder: "Leave blank to keep current password".into(),
            password_help: "Provide a new password only when rotating credentials.".into(),
            password_required: false,
            filter_search: filters.search.as_deref().unwrap_or_default().to_string(),
            filter_role: filters.role.as_deref().unwrap_or_default().to_string(),
            filter_status: filters.status.as_deref().unwrap_or_default().to_string(),
            filter_page_size: filters.page_size().to_string(),
        }
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
        let storage_transfer_syntax = self.storage_transfer_syntax.trim().to_string();
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
        if !storage_transfer_syntax.is_empty()
            && !supported_uids.contains(storage_transfer_syntax.as_str())
        {
            return Err(validation_flash(&format!(
                "Unsupported storage transfer syntax: {}",
                storage_transfer_syntax
            )));
        }

        Ok(ServerSettings {
            dicom_port,
            ae_title,
            ae_whitelist_enabled: self.ae_whitelist_enabled.is_some(),
            accept_all_transfer_syntaxes,
            accepted_transfer_syntaxes,
            preferred_transfer_syntaxes,
            storage_transfer_syntax: (!storage_transfer_syntax.is_empty())
                .then_some(storage_transfer_syntax),
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
            storage_transfer_syntax: input.storage_transfer_syntax.trim().to_string(),
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
            storage_transfer_syntax: settings.storage_transfer_syntax.clone().unwrap_or_default(),
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

fn users_panel_overrides(
    policy: Option<PasswordPolicy>,
    policy_form: Option<PasswordPolicyFormView>,
) -> UsersPanelOverrides {
    UsersPanelOverrides {
        policy,
        policy_form,
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
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::{
        body::to_bytes,
        body::Body,
        http::{Request, StatusCode},
        Extension, Router,
    };
    use bytes::Bytes;
    use pacs_core::{
        AuditLogEntry, AuditLogPage, AuditLogQuery, BlobStore, DicomJson, DicomNode, Instance,
        InstanceQuery, MetadataStore, NewAuditLogEntry, PacsResult, PacsStatistics, PasswordPolicy,
        RefreshToken, Series, SeriesQuery, SeriesUid, ServerSettings, SopInstanceUid, Study,
        StudyQuery, StudyUid, User, UserId, UserQuery,
    };
    use pacs_plugin::{AuthenticatedUser, Plugin, PluginManifest, PluginRegistry, ServerInfo};
    use tower::ServiceExt;

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
        async fn get_study(&self, _uid: &StudyUid) -> PacsResult<Study> {
            unreachable!()
        }
        async fn get_series(&self, _uid: &SeriesUid) -> PacsResult<Series> {
            unreachable!()
        }
        async fn get_instance(&self, _uid: &SopInstanceUid) -> PacsResult<Instance> {
            unreachable!()
        }
        async fn get_instance_metadata(&self, _uid: &SopInstanceUid) -> PacsResult<DicomJson> {
            unreachable!()
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
        async fn store_user(&self, _user: &User) -> PacsResult<()> {
            Ok(())
        }
        async fn get_user(&self, id: &UserId) -> PacsResult<User> {
            Err(PacsError::NotFound {
                resource: "user",
                uid: id.to_string(),
            })
        }
        async fn get_user_by_username(&self, username: &str) -> PacsResult<User> {
            Err(PacsError::NotFound {
                resource: "user",
                uid: username.to_string(),
            })
        }
        async fn query_users(&self, _q: &UserQuery) -> PacsResult<Vec<User>> {
            Ok(vec![])
        }
        async fn delete_user(&self, id: &UserId) -> PacsResult<()> {
            Err(PacsError::NotFound {
                resource: "user",
                uid: id.to_string(),
            })
        }
        async fn store_refresh_token(&self, _token: &RefreshToken) -> PacsResult<()> {
            Ok(())
        }
        async fn get_refresh_token(&self, token_hash: &str) -> PacsResult<RefreshToken> {
            Err(PacsError::NotFound {
                resource: "refresh_token",
                uid: token_hash.to_string(),
            })
        }
        async fn revoke_refresh_tokens(&self, _user_id: &UserId) -> PacsResult<()> {
            Ok(())
        }
        async fn get_password_policy(&self) -> PacsResult<PasswordPolicy> {
            Ok(PasswordPolicy::default())
        }
        async fn upsert_password_policy(&self, _policy: &PasswordPolicy) -> PacsResult<()> {
            Ok(())
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
        async fn get_server_settings(&self) -> PacsResult<Option<ServerSettings>> {
            Ok(None)
        }
        async fn upsert_server_settings(&self, _settings: &ServerSettings) -> PacsResult<()> {
            Ok(())
        }
        async fn search_audit_logs(&self, _q: &AuditLogQuery) -> PacsResult<AuditLogPage> {
            Ok(AuditLogPage {
                entries: vec![],
                total: 0,
                limit: 10,
                offset: 0,
            })
        }
        async fn get_audit_log(&self, _id: i64) -> PacsResult<AuditLogEntry> {
            unreachable!()
        }
        async fn store_audit_log(&self, _entry: &NewAuditLogEntry) -> PacsResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct NoopBlobStore;

    struct TestAuthPlugin;

    #[async_trait]
    impl Plugin for TestAuthPlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new(AUTH_PLUGIN_ID, "Test Auth", "0.1.0")
        }

        async fn init(
            &mut self,
            _ctx: &pacs_plugin::PluginContext,
        ) -> Result<(), pacs_plugin::PluginError> {
            Ok(())
        }
    }

    #[async_trait]
    impl BlobStore for NoopBlobStore {
        async fn put(&self, _key: &str, _data: Bytes) -> PacsResult<()> {
            Ok(())
        }
        async fn get(&self, _key: &str) -> PacsResult<Bytes> {
            unreachable!()
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

    fn admin_user(role: UserRole) -> AuthenticatedUser {
        AuthenticatedUser::new("1", "alice", role.as_str(), serde_json::json!({}))
    }

    fn test_admin_app(user: Option<AuthenticatedUser>, auth_enabled: bool) -> Router {
        let store: Arc<dyn MetadataStore> = Arc::new(NoopMetadataStore);
        let mut registry = PluginRegistry::new();
        if auth_enabled {
            registry.register(Box::new(TestAuthPlugin)).unwrap();
        }
        let state = AppState {
            server_info: ServerInfo {
                ae_title: "TESTPACS".into(),
                http_port: 8042,
                dicom_port: 4242,
                version: env!("CARGO_PKG_VERSION"),
            },
            server_settings: ServerSettings::default(),
            store: Arc::clone(&store),
            blobs: Arc::new(NoopBlobStore),
            plugins: Arc::new(registry),
        };
        let runtime = Arc::new(
            AdminRuntime::new(
                serde_json::from_value(serde_json::json!({
                    "route_prefix": "/admin",
                    "redirect_root": false,
                    "activity_limit": 24
                }))
                .unwrap(),
                state.server_info.clone(),
                store,
            )
            .unwrap(),
        );

        let router = routes(runtime);
        let router = if let Some(user) = user {
            router.layer(Extension(user))
        } else {
            router
        };

        router.with_state(state)
    }

    #[tokio::test]
    async fn dashboard_allows_access_when_auth_is_disabled() {
        let resp = test_admin_app(None, false)
            .oneshot(
                Request::builder()
                    .uri("/admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn dashboard_requires_admin_authentication_when_auth_is_enabled() {
        let resp = test_admin_app(None, true)
            .oneshot(
                Request::builder()
                    .uri("/admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn system_page_forbids_non_admin_users() {
        let resp = test_admin_app(Some(admin_user(UserRole::Viewer)), true)
            .oneshot(
                Request::builder()
                    .uri("/admin/system")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn system_page_allows_admin_users() {
        let resp = test_admin_app(Some(admin_user(UserRole::Admin)), true)
            .oneshot(
                Request::builder()
                    .uri("/admin/system")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn events_stream_forbids_non_admin_users() {
        let resp = test_admin_app(Some(admin_user(UserRole::Viewer)), true)
            .oneshot(
                Request::builder()
                    .uri("/admin/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_shell_shows_logout_button_for_local_cookie_sessions() {
        let resp = test_admin_app(Some(admin_user(UserRole::Admin)), true)
            .oneshot(
                Request::builder()
                    .uri("/admin")
                    .header(header::COOKIE, "pacsnode_access_token=test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("action=\"/admin/logout\""));
        assert!(html.contains("Sign out"));
    }

    #[tokio::test]
    async fn admin_logout_clears_local_auth_cookies() {
        let resp = test_admin_app(Some(admin_user(UserRole::Admin)), true)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/logout")
                    .header(
                        header::COOKIE,
                        "pacsnode_access_token=test-token; pacsnode_refresh_token=refresh-token",
                    )
                    .header("X-Forwarded-Proto", "https")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(resp.headers()[header::LOCATION], "/admin");
        let cookies = resp
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect::<Vec<_>>();
        assert!(cookies.iter().any(|cookie| {
            cookie.starts_with("pacsnode_access_token=")
                && cookie.contains("Max-Age=0")
                && cookie.contains("; Secure")
        }));
        assert!(cookies.iter().any(|cookie| {
            cookie.starts_with("pacsnode_refresh_token=")
                && cookie.contains("Max-Age=0")
                && cookie.contains("; Secure")
        }));
    }

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
            storage_transfer_syntax: String::new(),
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
            storage_transfer_syntax: String::new(),
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
            storage_transfer_syntax: "1.2.840.10008.1.2.4.90".into(),
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
        assert_eq!(
            settings.storage_transfer_syntax.as_deref(),
            Some("1.2.840.10008.1.2.4.90")
        );
    }

    #[test]
    fn server_settings_form_accepts_store_as_received() {
        let settings = ServerSettingsFormInput {
            dicom_port: "11112".into(),
            ae_title: "PACSUI".into(),
            ae_whitelist_enabled: None,
            accepted_transfer_syntaxes: vec!["1.2.840.10008.1.2.1".into()],
            preferred_transfer_syntaxes: vec!["1.2.840.10008.1.2.1".into()],
            storage_transfer_syntax: String::new(),
            max_associations: "32".into(),
            dimse_timeout_secs: "45".into(),
        }
        .into_settings()
        .unwrap();

        assert!(settings.storage_transfer_syntax.is_none());
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
    fn user_filters_build_store_query() {
        let filters = UserFilters {
            search: Some("alice".into()),
            role: Some("admin".into()),
            status: Some("active".into()),
            page_size: Some(50),
        };

        let query = filters.to_store_query();
        assert_eq!(query.search.as_deref(), Some("alice"));
        assert_eq!(query.role, Some(UserRole::Admin));
        assert_eq!(query.is_active, Some(true));
        assert_eq!(query.limit, Some(50));
    }

    #[test]
    fn user_form_requires_json_object_attributes() {
        let input = UserFormInput {
            attributes_json: "[1,2,3]".into(),
            ..UserFormInput::default()
        };

        assert_eq!(
            input.attributes_value().unwrap_err().title,
            "Validation failed"
        );
    }

    #[test]
    fn password_policy_validation_enforces_requirements() {
        let policy = PasswordPolicy {
            min_length: 12,
            require_uppercase: true,
            require_digit: true,
            require_special: true,
            max_failed_attempts: 5,
            lockout_duration_secs: 900,
            max_age_days: None,
        };

        assert!(validate_password_against_policy("short", &policy).is_err());
        assert!(validate_password_against_policy("lowercaseonly1!", &policy).is_err());
        assert!(validate_password_against_policy("ValidPassword1!", &policy).is_ok());
    }

    #[test]
    fn password_policy_form_parses_valid_values() {
        let policy = PasswordPolicyFormInput {
            min_length: "14".into(),
            require_uppercase: Some("on".into()),
            require_digit: Some("on".into()),
            require_special: None,
            max_failed_attempts: "6".into(),
            lockout_duration_secs: "1200".into(),
            max_age_days: "90".into(),
            filter_search: None,
            filter_role: None,
            filter_status: None,
            filter_page_size: None,
        }
        .into_policy()
        .unwrap();

        assert_eq!(policy.min_length, 14);
        assert!(policy.require_uppercase);
        assert!(policy.require_digit);
        assert!(!policy.require_special);
        assert_eq!(policy.max_failed_attempts, 6);
        assert_eq!(policy.lockout_duration_secs, 1200);
        assert_eq!(policy.max_age_days, Some(90));
    }

    #[test]
    fn password_policy_form_rejects_short_minimum_length() {
        let error = PasswordPolicyFormInput {
            min_length: "6".into(),
            require_uppercase: Some("on".into()),
            require_digit: Some("on".into()),
            require_special: None,
            max_failed_attempts: "5".into(),
            lockout_duration_secs: "900".into(),
            max_age_days: String::new(),
            filter_search: None,
            filter_role: None,
            filter_status: None,
            filter_page_size: None,
        }
        .into_policy()
        .unwrap_err();

        assert_eq!(error.title, "Validation failed");
    }

    #[test]
    fn password_policy_summary_mentions_max_age_when_enabled() {
        let summary = password_policy_summary(&PasswordPolicy {
            min_length: 12,
            require_uppercase: true,
            require_digit: true,
            require_special: false,
            max_failed_attempts: 5,
            lockout_duration_secs: 900,
            max_age_days: Some(60),
        });

        assert!(summary.contains("expire after 60 day(s)"));
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
