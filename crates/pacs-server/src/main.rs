//! pacsnode server entry point.
//!
//! ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for
//! diagnostic or therapeutic purposes.

#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
compile_error!("Enable at least one metadata backend feature (`postgres` or `sqlite`).");

#[cfg(not(any(feature = "s3", feature = "filesystem")))]
compile_error!("Enable at least one blob backend feature (`s3` or `filesystem`).");

use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use chrono::Utc;
use pacs_audit_plugin::AUDIT_LOGGER_PLUGIN_ID;
use pacs_auth_plugin::BASIC_AUTH_PLUGIN_ID;
use pacs_core::{
    DicomNode, MetadataStore, PasswordPolicy, ServerSettings, User, UserId, UserQuery, UserRole,
};
#[cfg(feature = "filesystem")]
use pacs_fs_storage::{self as _, FS_BLOB_STORE_PLUGIN_ID};
use pacs_plugin::{PluginRegistry, ServerInfo};
#[cfg(feature = "sqlite")]
use pacs_sqlite_store::{self as _, SQLITE_METADATA_STORE_PLUGIN_ID};
#[cfg(feature = "s3")]
use pacs_storage::{self as _, S3_BLOB_STORE_PLUGIN_ID};
#[cfg(feature = "postgres")]
use pacs_store::{self as _, PG_METADATA_STORE_PLUGIN_ID};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

mod config;

use config::{AppConfig, GeneratedConfigProfile, LogFormat};
use pacs_admin_plugin as _;
use pacs_audit_plugin as _;
use pacs_auth_plugin as _;
use pacs_metrics_plugin as _;
use pacs_viewer_plugin as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BackendSelection {
    metadata_plugin_id: &'static str,
    blob_plugin_id: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    RunServer,
    GenerateConfig {
        profile: GeneratedConfigProfile,
        output: Option<PathBuf>,
        force: bool,
    },
    CreateAdmin {
        username: String,
        display_name: Option<String>,
        email: Option<String>,
    },
    PrintHelp,
}

#[tokio::main]
async fn main() -> Result<()> {
    match parse_command(std::env::args().skip(1).collect())? {
        Command::RunServer => {}
        Command::GenerateConfig {
            profile,
            output,
            force,
        } => {
            write_generated_config(profile, output.as_deref(), force)?;
            return Ok(());
        }
        Command::CreateAdmin {
            username,
            display_name,
            email,
        } => {
            let cfg = AppConfig::load().context("failed to load configuration")?;
            init_tracing(&cfg.logging.level, &cfg.logging.format);
            create_bootstrap_admin(&cfg, username, display_name, email).await?;
            return Ok(());
        }
        Command::PrintHelp => {
            print!("{}", usage_text());
            return Ok(());
        }
    }

    // ── Config ────────────────────────────────────────────────────────────────
    let cfg = AppConfig::load().context("failed to load configuration")?;
    let backend_selection = select_backend_plugins(&cfg)?;

    // ── Tracing ───────────────────────────────────────────────────────────────
    init_tracing(&cfg.logging.level, &cfg.logging.format);

    info!(
        http_port  = cfg.server.http_port,
        dicom_port = cfg.server.dicom_port,
        ae_title   = %cfg.server.ae_title,
        "pacsnode starting"
    );
    let (enabled_plugins, audit_auto_enabled) = effective_enabled_plugins(&cfg, backend_selection);
    if audit_auto_enabled {
        info!(
            plugin_id = AUDIT_LOGGER_PLUGIN_ID,
            secured_by = BASIC_AUTH_PLUGIN_ID,
            "Auto-enabling audit logging for secured deployment"
        );
    }
    let mut registry = PluginRegistry::new();
    registry.set_enabled(enabled_plugins);
    registry
        .register_all_discovered()
        .context("failed to register compiled-in plugins")?;

    let bootstrap_server_settings = cfg.server.bootstrap_server_settings();
    let init_server_info = ServerInfo {
        ae_title: bootstrap_server_settings.ae_title.clone(),
        http_port: cfg.server.http_port,
        dicom_port: bootstrap_server_settings.dicom_port,
        version: env!("CARGO_PKG_VERSION"),
    };
    let plugin_configs = build_plugin_configs(&cfg, backend_selection)?;
    registry
        .init_all(init_server_info, &plugin_configs)
        .await
        .context("failed to initialize plugins")?;
    let registry = Arc::new(registry);

    let meta_store = registry
        .metadata_store()
        .ok_or_else(|| anyhow!("no MetadataStore plugin is active"))?;
    let blob_store = registry
        .blob_store()
        .ok_or_else(|| anyhow!("no BlobStore plugin is active"))?;
    let runtime_server_settings =
        load_or_bootstrap_server_settings(meta_store.as_ref(), &cfg.server).await?;
    let server_info = ServerInfo {
        ae_title: runtime_server_settings.ae_title.clone(),
        http_port: cfg.server.http_port,
        dicom_port: runtime_server_settings.dicom_port,
        version: env!("CARGO_PKG_VERSION"),
    };
    bootstrap_configured_nodes(meta_store.as_ref(), &cfg.nodes).await?;

    // ── HTTP server ───────────────────────────────────────────────────────────
    let app_state = pacs_api::AppState {
        server_info: server_info.clone(),
        server_settings: runtime_server_settings.clone(),
        store: meta_store.clone(),
        blobs: blob_store.clone(),
        plugins: Arc::clone(&registry),
    };
    let router = registry
        .apply_middleware(pacs_api::build_router_without_state().merge(registry.merged_routes()))
        .with_state(app_state);

    let http_addr = format!("0.0.0.0:{}", cfg.server.http_port);
    let http_listener = TcpListener::bind(&http_addr)
        .await
        .with_context(|| format!("failed to bind HTTP port {}", cfg.server.http_port))?;
    info!(addr = %http_addr, "HTTP server listening");

    // ── DIMSE server ──────────────────────────────────────────────────────────
    let dimse_config = pacs_dimse::DimseConfig {
        ae_title: runtime_server_settings.ae_title.clone(),
        port: runtime_server_settings.dicom_port,
        ae_whitelist_enabled: runtime_server_settings.ae_whitelist_enabled,
        accept_all_transfer_syntaxes: runtime_server_settings.accept_all_transfer_syntaxes,
        accepted_transfer_syntaxes: runtime_server_settings.accepted_transfer_syntaxes.clone(),
        preferred_transfer_syntaxes: runtime_server_settings.preferred_transfer_syntaxes.clone(),
        max_associations: runtime_server_settings.max_associations,
        timeout_secs: runtime_server_settings.dimse_timeout_secs,
    };
    let dicom_server = Arc::new(pacs_dimse::DicomServer::with_plugins(
        dimse_config,
        meta_store,
        blob_store,
        Some(Arc::clone(&registry)),
    ));
    let app_shutdown = CancellationToken::new();
    let dimse_shutdown = CancellationToken::new();
    let signal_shutdown = app_shutdown.clone();
    let signal_dimse_shutdown = dimse_shutdown.clone();

    let signal_task = tokio::spawn(async move {
        ctrl_c_signal().await;
        info!("shutdown signal received");
        signal_dimse_shutdown.cancel();
        signal_shutdown.cancel();
    });

    let dimse_task = tokio::spawn(async move {
        if let Err(error) = dicom_server.serve(dimse_shutdown).await {
            warn!(error = %error, "DIMSE server exited with error; HTTP/admin will remain available");
        }
    });

    // ── Serve HTTP independently so admin recovery remains available even when
    // ── DIMSE startup fails (for example after persisting an occupied port).
    if let Err(error) = axum::serve(http_listener, router)
        .with_graceful_shutdown(app_shutdown.clone().cancelled_owned())
        .await
    {
        warn!(error = %error, "HTTP server exited with error");
    }

    app_shutdown.cancel();
    signal_task.abort();
    let _ = dimse_task.await;

    registry
        .shutdown_all()
        .await
        .context("failed to shut down plugins")?;

    info!("pacsnode shut down");
    Ok(())
}

fn usage_text() -> &'static str {
    "Usage:\n  pacsnode\n  pacsnode generate-config <standalone|production> [--output <path>] [--force]\n  pacsnode create-admin [--username <name>] [--display-name <name>] [--email <email>]\n  pacsnode -h|--help\n"
}

fn parse_command(args: Vec<String>) -> Result<Command> {
    if args.is_empty() {
        return Ok(Command::RunServer);
    }

    match args[0].as_str() {
        "-h" | "--help" => Ok(Command::PrintHelp),
        "generate-config" => {
            let mut profile = None;
            let mut output = None;
            let mut force = false;
            let mut idx = 1;

            while idx < args.len() {
                match args[idx].as_str() {
                    "--output" => {
                        idx += 1;
                        if idx >= args.len() {
                            return Err(anyhow!("--output requires a file path"));
                        }
                        output = Some(PathBuf::from(&args[idx]));
                    }
                    "--force" => force = true,
                    value if value.starts_with('-') => {
                        return Err(anyhow!("unknown option: {value}\n\n{}", usage_text()));
                    }
                    value => {
                        if profile.is_some() {
                            return Err(anyhow!(
                                "unexpected extra argument: {value}\n\n{}",
                                usage_text()
                            ));
                        }
                        profile = GeneratedConfigProfile::parse(value);
                        if profile.is_none() {
                            return Err(anyhow!(
                                "unknown config profile: {value}\nexpected `standalone` or `production`"
                            ));
                        }
                    }
                }
                idx += 1;
            }

            let profile =
                profile.ok_or_else(|| anyhow!("missing config profile\n\n{}", usage_text()))?;
            Ok(Command::GenerateConfig {
                profile,
                output,
                force,
            })
        }
        "create-admin" => {
            let mut username = String::from("admin");
            let mut display_name = None;
            let mut email = None;
            let mut idx = 1;

            while idx < args.len() {
                match args[idx].as_str() {
                    "--username" => {
                        idx += 1;
                        if idx >= args.len() {
                            return Err(anyhow!("--username requires a value"));
                        }
                        username = args[idx].clone();
                    }
                    "--display-name" => {
                        idx += 1;
                        if idx >= args.len() {
                            return Err(anyhow!("--display-name requires a value"));
                        }
                        display_name = Some(args[idx].clone());
                    }
                    "--email" => {
                        idx += 1;
                        if idx >= args.len() {
                            return Err(anyhow!("--email requires a value"));
                        }
                        email = Some(args[idx].clone());
                    }
                    value if value.starts_with('-') => {
                        return Err(anyhow!("unknown option: {value}\n\n{}", usage_text()));
                    }
                    value => {
                        return Err(anyhow!(
                            "unexpected extra argument: {value}\n\n{}",
                            usage_text()
                        ));
                    }
                }
                idx += 1;
            }

            Ok(Command::CreateAdmin {
                username,
                display_name,
                email,
            })
        }
        other => Err(anyhow!("unknown command: {other}\n\n{}", usage_text())),
    }
}

fn write_generated_config(
    profile: GeneratedConfigProfile,
    output: Option<&Path>,
    force: bool,
) -> Result<()> {
    let rendered = profile.render();

    if let Some(path) = output {
        if path.exists() && !force {
            return Err(anyhow!(
                "refusing to overwrite existing file {} (use --force)",
                path.display()
            ));
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create parent directory {}", parent.display())
                })?;
            }
        }
        fs::write(path, rendered).with_context(|| format!("failed to write {}", path.display()))?;
        println!("Wrote {} config to {}", profile.as_str(), path.display());
    } else {
        print!("{rendered}");
    }

    Ok(())
}

fn select_backend_plugins(cfg: &AppConfig) -> Result<BackendSelection> {
    Ok(BackendSelection {
        metadata_plugin_id: select_metadata_plugin_id(cfg)?,
        blob_plugin_id: select_blob_plugin_id(cfg)?,
    })
}

fn build_metadata_plugin_config(
    cfg: &AppConfig,
    metadata_plugin_id: &'static str,
) -> Result<serde_json::Value> {
    let mut db_config = cfg
        .database
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .context("serialize metadata database config")?
        .unwrap_or_else(empty_object);
    if let Some(override_value) = cfg.plugins.configs.get(metadata_plugin_id).cloned() {
        merge_json(&mut db_config, override_value);
    }
    ensure_non_empty_config(
        &db_config,
        "database",
        "database config is required for the selected metadata backend",
    )?;
    Ok(db_config)
}

async fn init_metadata_store_for_command(
    cfg: &AppConfig,
) -> Result<(PluginRegistry, Arc<dyn MetadataStore>)> {
    let metadata_plugin_id = select_metadata_plugin_id(cfg)?;
    let mut registry = PluginRegistry::new();
    registry.set_enabled(vec![metadata_plugin_id.into()]);
    registry
        .register_all_discovered()
        .context("failed to register compiled-in plugins")?;

    let mut plugin_configs = HashMap::new();
    plugin_configs.insert(
        metadata_plugin_id.into(),
        build_metadata_plugin_config(cfg, metadata_plugin_id)?,
    );

    let bootstrap_server_settings = cfg.server.bootstrap_server_settings();
    registry
        .init_all(
            ServerInfo {
                ae_title: bootstrap_server_settings.ae_title,
                http_port: cfg.server.http_port,
                dicom_port: bootstrap_server_settings.dicom_port,
                version: env!("CARGO_PKG_VERSION"),
            },
            &plugin_configs,
        )
        .await
        .context("failed to initialize metadata backend")?;

    let store = registry
        .metadata_store()
        .ok_or_else(|| anyhow!("no MetadataStore plugin is active"))?;
    Ok((registry, store))
}

async fn create_bootstrap_admin(
    cfg: &AppConfig,
    username: String,
    display_name: Option<String>,
    email: Option<String>,
) -> Result<()> {
    let username = validate_bootstrap_username(&username)?;
    let display_name = normalize_optional_field(display_name);
    let email = normalize_optional_field(email);
    let (registry, store) = init_metadata_store_for_command(cfg).await?;

    let operation = async {
        let existing_users = store
            .query_users(&UserQuery {
                limit: Some(1),
                ..UserQuery::default()
            })
            .await
            .context("failed to query existing users")?;

        if !existing_users.is_empty() {
            return Err(anyhow!(
                "refusing to create bootstrap admin because local users already exist"
            ));
        }

        let policy = store
            .get_password_policy()
            .await
            .context("failed to load password policy")?;
        let password = generate_bootstrap_password(&policy);
        let password_hash = hash_password(&password)?;
        let user = User {
            id: UserId::new(),
            username: username.clone(),
            display_name,
            email,
            password_hash,
            role: UserRole::Admin,
            attributes: serde_json::json!({"bootstrap_admin": true}),
            is_active: true,
            failed_login_attempts: 0,
            locked_until: None,
            password_changed_at: Some(Utc::now()),
            created_at: None,
            updated_at: None,
        };

        store
            .store_user(&user)
            .await
            .with_context(|| format!("failed to store bootstrap admin {}", user.username))?;

        println!("Bootstrap admin created.");
        println!("Username: {}", user.username);
        println!("Password: {}", password);
        println!("Role: {}", user.role);
        println!("User ID: {}", user.id);
        println!("Rotate this password immediately after the first login.");

        Ok(())
    }
    .await;

    let shutdown_result = registry
        .shutdown_all()
        .await
        .context("failed to shut down metadata backend after create-admin");

    operation?;
    shutdown_result?;
    Ok(())
}

fn validate_bootstrap_username(username: &str) -> Result<String> {
    let trimmed = username.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("bootstrap username must not be empty"));
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '@'))
    {
        return Err(anyhow!(
            "bootstrap username may contain only ASCII letters, digits, '.', '_', '-', and '@'"
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_optional_field(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| anyhow!("failed to hash bootstrap password: {error}"))
}

fn generate_bootstrap_password(policy: &PasswordPolicy) -> String {
    const LOWERCASE: &[u8] = b"abcdefghijkmnopqrstuvwxyz";
    const UPPERCASE: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
    const DIGITS: &[u8] = b"23456789";
    const SPECIAL: &[u8] = b"!@#$%^&*-_";
    const ALL: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#$%^&*-_";

    let mut rng = OsRng;
    let target_len = usize::max(policy.min_length as usize, 20);
    let mut chars = Vec::with_capacity(target_len);

    chars.push(random_password_char(LOWERCASE, &mut rng));
    if policy.require_uppercase {
        chars.push(random_password_char(UPPERCASE, &mut rng));
    }
    if policy.require_digit {
        chars.push(random_password_char(DIGITS, &mut rng));
    }
    if policy.require_special {
        chars.push(random_password_char(SPECIAL, &mut rng));
    }

    while chars.len() < target_len {
        chars.push(random_password_char(ALL, &mut rng));
    }

    for idx in (1..chars.len()).rev() {
        let swap_idx =
            (argon2::password_hash::rand_core::RngCore::next_u32(&mut rng) as usize) % (idx + 1);
        chars.swap(idx, swap_idx);
    }

    chars.into_iter().collect()
}

fn random_password_char(charset: &[u8], rng: &mut argon2::password_hash::rand_core::OsRng) -> char {
    let idx = (argon2::password_hash::rand_core::RngCore::next_u32(rng) as usize) % charset.len();
    charset[idx] as char
}

fn select_metadata_plugin_id(cfg: &AppConfig) -> Result<&'static str> {
    let database = cfg
        .database
        .as_ref()
        .ok_or_else(|| anyhow!("database config is required"))?;
    let url = database.url.trim();

    if url.starts_with("sqlite://") {
        #[cfg(feature = "sqlite")]
        {
            return Ok(SQLITE_METADATA_STORE_PLUGIN_ID);
        }
        #[cfg(not(feature = "sqlite"))]
        {
            return Err(anyhow!(
                "database.url uses sqlite:// but this binary was built without the sqlite backend"
            ));
        }
    }

    if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        #[cfg(feature = "postgres")]
        {
            return Ok(PG_METADATA_STORE_PLUGIN_ID);
        }
        #[cfg(not(feature = "postgres"))]
        {
            return Err(anyhow!(
                "database.url uses postgres:// but this binary was built without the postgres backend"
            ));
        }
    }

    Err(anyhow!(
        "database.url must start with either `sqlite://`, `postgres://`, or `postgresql://`"
    ))
}

fn select_blob_plugin_id(cfg: &AppConfig) -> Result<&'static str> {
    match (cfg.storage.as_ref(), cfg.filesystem_storage.as_ref()) {
        (Some(_), None) => {
            #[cfg(feature = "s3")]
            {
                Ok(S3_BLOB_STORE_PLUGIN_ID)
            }
            #[cfg(not(feature = "s3"))]
            {
                Err(anyhow!(
                    "`storage` is configured but this binary was built without the s3 backend"
                ))
            }
        }
        (None, Some(_)) => {
            #[cfg(feature = "filesystem")]
            {
                Ok(FS_BLOB_STORE_PLUGIN_ID)
            }
            #[cfg(not(feature = "filesystem"))]
            {
                Err(anyhow!(
                    "`filesystem_storage` is configured but this binary was built without the filesystem backend"
                ))
            }
        }
        (Some(_), Some(_)) => Err(anyhow!(
            "configure only one blob backend: either `[storage]` or `[filesystem_storage]`"
        )),
        (None, None) => Err(anyhow!(
            "blob storage config is required: set either `[storage]` or `[filesystem_storage]`"
        )),
    }
}

fn build_plugin_configs(
    cfg: &AppConfig,
    backend_selection: BackendSelection,
) -> Result<HashMap<String, serde_json::Value>> {
    let mut configs = cfg.plugins.configs.clone();

    match backend_selection.metadata_plugin_id {
        #[cfg(feature = "postgres")]
        PG_METADATA_STORE_PLUGIN_ID => {
            let mut db_config = cfg
                .database
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .context("serialize postgres database config")?
                .unwrap_or_else(empty_object);
            if let Some(override_value) = configs.remove(PG_METADATA_STORE_PLUGIN_ID) {
                merge_json(&mut db_config, override_value);
            }
            ensure_non_empty_config(
                &db_config,
                "database",
                "database config is required for the postgres metadata backend",
            )?;
            configs.insert(PG_METADATA_STORE_PLUGIN_ID.into(), db_config);
        }
        #[cfg(feature = "sqlite")]
        SQLITE_METADATA_STORE_PLUGIN_ID => {
            let mut db_config = cfg
                .database
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .context("serialize sqlite database config")?
                .unwrap_or_else(empty_object);
            if let Some(override_value) = configs.remove(SQLITE_METADATA_STORE_PLUGIN_ID) {
                merge_json(&mut db_config, override_value);
            }
            ensure_non_empty_config(
                &db_config,
                "database",
                "database config is required for the sqlite metadata backend",
            )?;
            configs.insert(SQLITE_METADATA_STORE_PLUGIN_ID.into(), db_config);
        }
        _ => unreachable!("unsupported metadata backend selection"),
    }

    match backend_selection.blob_plugin_id {
        #[cfg(feature = "s3")]
        S3_BLOB_STORE_PLUGIN_ID => {
            let mut storage_config = cfg
                .storage
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .context("serialize storage config")?
                .unwrap_or_else(empty_object);
            if let Some(override_value) = configs.remove(S3_BLOB_STORE_PLUGIN_ID) {
                merge_json(&mut storage_config, override_value);
            }
            ensure_non_empty_config(
                &storage_config,
                "storage",
                "storage config is required for the s3 blob backend",
            )?;
            configs.insert(S3_BLOB_STORE_PLUGIN_ID.into(), storage_config);
        }
        #[cfg(feature = "filesystem")]
        FS_BLOB_STORE_PLUGIN_ID => {
            let mut storage_config = cfg
                .filesystem_storage
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .context("serialize filesystem storage config")?
                .unwrap_or_else(empty_object);
            if let Some(override_value) = configs.remove(FS_BLOB_STORE_PLUGIN_ID) {
                merge_json(&mut storage_config, override_value);
            }
            ensure_non_empty_config(
                &storage_config,
                "filesystem_storage",
                "filesystem_storage config is required for the filesystem blob backend",
            )?;
            configs.insert(FS_BLOB_STORE_PLUGIN_ID.into(), storage_config);
        }
        _ => unreachable!("unsupported blob backend selection"),
    }

    let audit_config = configs
        .remove(AUDIT_LOGGER_PLUGIN_ID)
        .unwrap_or_else(empty_object);
    configs.insert(AUDIT_LOGGER_PLUGIN_ID.into(), audit_config);

    Ok(configs)
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(Default::default())
}

fn ensure_non_empty_config(config: &serde_json::Value, section: &str, message: &str) -> Result<()> {
    if matches!(config, serde_json::Value::Object(map) if map.is_empty()) {
        return Err(anyhow!(
            "{message} (missing `{section}` or plugin override)"
        ));
    }

    Ok(())
}

fn merge_json(base: &mut serde_json::Value, overlay: serde_json::Value) {
    match (base, overlay) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                match base_map.get_mut(&key) {
                    Some(existing) => merge_json(existing, value),
                    None => {
                        base_map.insert(key, value);
                    }
                }
            }
        }
        (base_slot, overlay_value) => *base_slot = overlay_value,
    }
}

async fn load_or_bootstrap_server_settings(
    store: &dyn MetadataStore,
    server_config: &config::ServerConfig,
) -> Result<ServerSettings> {
    if let Some(settings) = store
        .get_server_settings()
        .await
        .context("failed to load persisted DIMSE server settings")?
    {
        return Ok(settings);
    }

    let settings = server_config.bootstrap_server_settings();
    store
        .upsert_server_settings(&settings)
        .await
        .context("failed to bootstrap persisted DIMSE server settings")?;
    info!(
        dicom_port = settings.dicom_port,
        ae_title = %settings.ae_title,
        "Bootstrapped persisted DIMSE settings from config/defaults"
    );
    Ok(settings)
}

async fn bootstrap_configured_nodes(store: &dyn MetadataStore, nodes: &[DicomNode]) -> Result<()> {
    validate_configured_nodes(nodes)?;

    if nodes.is_empty() {
        return Ok(());
    }

    for node in nodes {
        store
            .upsert_node(node)
            .await
            .with_context(|| format!("failed to upsert configured DICOM node {}", node.ae_title))?;
    }

    info!(
        count = nodes.len(),
        "Upserted configured DICOM nodes from configuration"
    );
    Ok(())
}

fn validate_configured_nodes(nodes: &[DicomNode]) -> Result<()> {
    let mut seen = BTreeSet::new();

    for node in nodes {
        let ae_title = node.ae_title.trim();
        if ae_title.is_empty() {
            return Err(anyhow!("configured DICOM node AE title must not be empty"));
        }
        if !seen.insert(ae_title) {
            return Err(anyhow!(
                "duplicate configured DICOM node AE title: {ae_title}"
            ));
        }
    }

    Ok(())
}

/// Initialise the global [`tracing`] subscriber.
fn init_tracing(level: &str, format: &LogFormat) {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    match format {
        LogFormat::Json => {
            fmt()
                .json()
                .with_env_filter(filter)
                .with_target(true)
                .init();
        }
        LogFormat::Pretty => {
            fmt()
                .pretty()
                .with_env_filter(filter)
                .with_target(true)
                .init();
        }
    }
}

/// Resolves when SIGINT / Ctrl+C is received.
async fn ctrl_c_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
}

fn effective_enabled_plugins(
    cfg: &AppConfig,
    backend_selection: BackendSelection,
) -> (Vec<String>, bool) {
    let mut enabled: BTreeSet<String> = cfg.plugins.enabled.iter().cloned().collect();
    enabled.insert(backend_selection.metadata_plugin_id.into());
    enabled.insert(backend_selection.blob_plugin_id.into());
    let audit_auto_enabled = enabled.contains(BASIC_AUTH_PLUGIN_ID)
        && !enabled.contains(AUDIT_LOGGER_PLUGIN_ID)
        && audit_auto_enable_in_secure_deployments(cfg);

    if audit_auto_enabled {
        enabled.insert(AUDIT_LOGGER_PLUGIN_ID.into());
    }

    (enabled.into_iter().collect(), audit_auto_enabled)
}

fn audit_auto_enable_in_secure_deployments(cfg: &AppConfig) -> bool {
    cfg.plugins
        .configs
        .get(AUDIT_LOGGER_PLUGIN_ID)
        .and_then(|config| config.get("auto_enable_in_secure_deployments"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        DatabaseConfig, FilesystemStorageConfig, LoggingConfig, PluginsConfig, ServerConfig,
        StorageConfig,
    };

    #[cfg(feature = "postgres")]
    fn default_test_database_url() -> &'static str {
        "postgres://u:p@localhost/pacs"
    }

    #[cfg(all(not(feature = "postgres"), feature = "sqlite"))]
    fn default_test_database_url() -> &'static str {
        "sqlite://./data/pacsnode.db"
    }

    #[cfg(feature = "s3")]
    fn default_test_storage_config() -> Option<StorageConfig> {
        Some(StorageConfig {
            endpoint: "http://localhost:9000".into(),
            bucket: "dicom".into(),
            access_key: "key".into(),
            secret_key: "secret".into(),
            region: "us-east-1".into(),
        })
    }

    #[cfg(not(feature = "s3"))]
    fn default_test_storage_config() -> Option<StorageConfig> {
        None
    }

    #[cfg(all(feature = "filesystem", not(feature = "s3")))]
    fn default_test_filesystem_config() -> Option<FilesystemStorageConfig> {
        Some(FilesystemStorageConfig {
            root: "./data/blobs".into(),
        })
    }

    #[cfg(any(not(feature = "filesystem"), feature = "s3"))]
    fn default_test_filesystem_config() -> Option<FilesystemStorageConfig> {
        None
    }

    fn make_config(enabled: &[&str], configs: HashMap<String, serde_json::Value>) -> AppConfig {
        AppConfig {
            server: ServerConfig::default(),
            nodes: Vec::new(),
            database: Some(DatabaseConfig {
                url: default_test_database_url().into(),
                max_connections: 20,
                run_migrations: true,
            }),
            storage: default_test_storage_config(),
            filesystem_storage: default_test_filesystem_config(),
            logging: LoggingConfig {
                level: "info".into(),
                format: LogFormat::Json,
            },
            plugins: PluginsConfig {
                enabled: enabled.iter().map(|id| (*id).to_string()).collect(),
                configs,
            },
        }
    }

    #[test]
    fn auto_enables_audit_for_basic_auth() {
        let cfg = make_config(&[BASIC_AUTH_PLUGIN_ID], HashMap::new());
        let backend_selection = select_backend_plugins(&cfg).expect("backend selection");
        let (enabled, audit_auto_enabled) = effective_enabled_plugins(&cfg, backend_selection);

        assert!(audit_auto_enabled);
        assert!(enabled.contains(&AUDIT_LOGGER_PLUGIN_ID.to_string()));
    }

    #[test]
    fn audit_opt_out_disables_secure_auto_enable() {
        let mut configs = HashMap::new();
        configs.insert(
            AUDIT_LOGGER_PLUGIN_ID.into(),
            serde_json::json!({
                "auto_enable_in_secure_deployments": false,
            }),
        );

        let cfg = make_config(&[BASIC_AUTH_PLUGIN_ID], configs);
        let backend_selection = select_backend_plugins(&cfg).expect("backend selection");
        let (enabled, audit_auto_enabled) = effective_enabled_plugins(&cfg, backend_selection);

        assert!(!audit_auto_enabled);
        assert!(!enabled.contains(&AUDIT_LOGGER_PLUGIN_ID.to_string()));
    }

    #[cfg(all(feature = "sqlite", feature = "filesystem"))]
    #[test]
    fn selects_sqlite_and_filesystem_backends_from_config() {
        let mut cfg = make_config(&[], HashMap::new());
        cfg.database = Some(DatabaseConfig {
            url: "sqlite://./data/pacsnode.db".into(),
            max_connections: 20,
            run_migrations: true,
        });
        cfg.storage = None;
        cfg.filesystem_storage = Some(FilesystemStorageConfig {
            root: "./data/blobs".into(),
        });

        let selection = select_backend_plugins(&cfg).expect("backend selection");

        assert_eq!(
            selection.metadata_plugin_id,
            SQLITE_METADATA_STORE_PLUGIN_ID
        );
        assert_eq!(selection.blob_plugin_id, FS_BLOB_STORE_PLUGIN_ID);
    }

    #[cfg(all(feature = "postgres", feature = "s3"))]
    #[test]
    fn selects_postgres_and_s3_backends_from_config() {
        let cfg = make_config(&[], HashMap::new());

        let selection = select_backend_plugins(&cfg).expect("backend selection");

        assert_eq!(selection.metadata_plugin_id, PG_METADATA_STORE_PLUGIN_ID);
        assert_eq!(selection.blob_plugin_id, S3_BLOB_STORE_PLUGIN_ID);
    }

    #[test]
    fn rejects_ambiguous_blob_backend_config() {
        let mut cfg = make_config(&[], HashMap::new());
        cfg.storage = Some(StorageConfig {
            endpoint: "http://localhost:9000".into(),
            bucket: "dicom".into(),
            access_key: "key".into(),
            secret_key: "secret".into(),
            region: "us-east-1".into(),
        });
        cfg.filesystem_storage = Some(FilesystemStorageConfig {
            root: "./data/blobs".into(),
        });

        let error = select_backend_plugins(&cfg).expect_err("ambiguous blob config should fail");
        assert!(error
            .to_string()
            .contains("configure only one blob backend"));
    }

    #[test]
    fn parse_generate_config_command_supports_output_and_force() {
        let command = parse_command(vec![
            "generate-config".into(),
            "standalone".into(),
            "--output".into(),
            "config.toml".into(),
            "--force".into(),
        ])
        .expect("command should parse");

        assert_eq!(
            command,
            Command::GenerateConfig {
                profile: GeneratedConfigProfile::Standalone,
                output: Some(PathBuf::from("config.toml")),
                force: true,
            }
        );
    }

    #[test]
    fn parse_create_admin_command_supports_optional_fields() {
        let command = parse_command(vec![
            "create-admin".into(),
            "--username".into(),
            "bootstrap-admin".into(),
            "--display-name".into(),
            "Bootstrap Admin".into(),
            "--email".into(),
            "admin@example.test".into(),
        ])
        .expect("command should parse");

        assert_eq!(
            command,
            Command::CreateAdmin {
                username: "bootstrap-admin".into(),
                display_name: Some("Bootstrap Admin".into()),
                email: Some("admin@example.test".into()),
            }
        );
    }

    #[test]
    fn generated_bootstrap_password_satisfies_policy() {
        let policy = PasswordPolicy {
            min_length: 24,
            require_uppercase: true,
            require_digit: true,
            require_special: true,
            max_failed_attempts: 5,
            lockout_duration_secs: 900,
            max_age_days: None,
        };

        let password = generate_bootstrap_password(&policy);

        assert!(password.len() >= 24);
        assert!(password.chars().any(|ch| ch.is_ascii_lowercase()));
        assert!(password.chars().any(|ch| ch.is_ascii_uppercase()));
        assert!(password.chars().any(|ch| ch.is_ascii_digit()));
        assert!(password.chars().any(|ch| !ch.is_ascii_alphanumeric()));
    }

    #[test]
    fn bootstrap_username_validation_rejects_whitespace() {
        let error = validate_bootstrap_username("bad user").expect_err("username should fail");
        assert!(error.to_string().contains("bootstrap username"));
    }

    #[test]
    fn validate_configured_nodes_rejects_duplicate_ae_titles() {
        let nodes = vec![
            DicomNode {
                ae_title: "MODALITY1".into(),
                host: "192.168.1.10".into(),
                port: 104,
                description: None,
                tls_enabled: false,
            },
            DicomNode {
                ae_title: "MODALITY1".into(),
                host: "192.168.1.11".into(),
                port: 105,
                description: None,
                tls_enabled: false,
            },
        ];

        let error = validate_configured_nodes(&nodes).expect_err("duplicates should fail");
        assert!(error
            .to_string()
            .contains("duplicate configured DICOM node AE title"));
    }

    #[test]
    fn validate_configured_nodes_rejects_blank_ae_title() {
        let nodes = vec![DicomNode {
            ae_title: "   ".into(),
            host: "192.168.1.10".into(),
            port: 104,
            description: None,
            tls_enabled: false,
        }];

        let error = validate_configured_nodes(&nodes).expect_err("blank AE title should fail");
        assert!(error
            .to_string()
            .contains("configured DICOM node AE title must not be empty"));
    }
}
