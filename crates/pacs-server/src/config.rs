//! pacsnode server configuration.
//!
//! Configuration is loaded in three layers (later layers override earlier ones):
//!
//! 1. `config.toml` next to the **executable** (optional — useful when running the
//!    binary directly from its release/install directory)
//! 2. `config.toml` in the **current working directory** (optional — overrides the
//!    executable-adjacent file when both are present)
//! 3. **Environment variables** with the `PACS_` prefix (override both TOML sources).
//!    Use `__` (double underscore) as the nesting separator, e.g. `PACS_DATABASE__URL`.
//!
//! # Example config.toml
//!
//! ```toml
//! [server]
//! http_port  = 8042
//!
//! [database]
//! url             = "postgres://pacsnode:secret@localhost/pacsnode"
//! max_connections = 20
//!
//! [storage]
//! endpoint   = "http://localhost:9000"
//! bucket     = "dicom"
//! access_key = "minio_user"
//! secret_key = "minio_pass"
//! region     = "us-east-1"
//!
//! [logging]
//! level  = "info"
//! format = "json"
//! ```

use std::collections::HashMap;

use config::{Config, ConfigError, Environment, File};
use pacs_admin_plugin::ADMIN_DASHBOARD_PLUGIN_ID;
use pacs_core::{DicomNode, ServerSettings};
use pacs_viewer_plugin::OHIF_VIEWER_PLUGIN_ID;
use serde::{Deserialize, Serialize};

/// Top-level application configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    /// HTTP server settings.
    #[serde(default)]
    pub server: ServerConfig,
    /// Optional DICOM nodes to upsert into the registry on startup.
    #[serde(default)]
    pub nodes: Vec<DicomNode>,
    /// Metadata database settings for the active backend.
    #[serde(default)]
    pub database: Option<DatabaseConfig>,
    /// S3-compatible object storage settings.
    #[serde(default)]
    pub storage: Option<StorageConfig>,
    /// Filesystem blob storage settings for standalone deployments.
    #[serde(default)]
    pub filesystem_storage: Option<FilesystemStorageConfig>,
    /// Structured logging settings.
    #[serde(default)]
    pub logging: LoggingConfig,
    /// Optional plugin activation and configuration.
    #[serde(default)]
    pub plugins: PluginsConfig,
}

/// HTTP configuration and bootstrap DIMSE defaults.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ServerConfig {
    /// TCP port for the DICOMweb / REST HTTP API.
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    /// TCP port for the DICOM DIMSE SCP.
    #[serde(default = "default_dicom_port")]
    pub dicom_port: u16,
    /// DICOM Application Entity title for this PACS node.
    #[serde(default = "default_ae_title")]
    pub ae_title: String,
    /// Require inbound DIMSE callers to be registered in the node registry.
    #[serde(default)]
    pub ae_whitelist_enabled: bool,
    /// Whether the DIMSE SCP accepts any offered transfer syntax by default.
    #[serde(default = "default_true")]
    pub accept_all_transfer_syntaxes: bool,
    /// Explicit DIMSE SCP transfer syntax allow-list.
    #[serde(default)]
    pub accepted_transfer_syntaxes: Vec<String>,
    /// Preferred DIMSE SCP transfer syntax order, highest priority first.
    #[serde(default)]
    pub preferred_transfer_syntaxes: Vec<String>,
    /// Optional transfer syntax to use when archiving newly ingested DICOM objects.
    #[serde(default)]
    pub storage_transfer_syntax: Option<String>,
    /// Maximum number of concurrent DIMSE associations.
    #[serde(default = "default_max_associations")]
    pub max_associations: usize,
    /// DIMSE association timeout in seconds.
    #[serde(default = "default_dimse_timeout_secs")]
    pub dimse_timeout_secs: u64,
}

impl ServerConfig {
    /// Returns the DIMSE settings used to seed persisted admin-managed server
    /// settings when the metadata store does not have a saved row yet.
    pub fn bootstrap_server_settings(&self) -> ServerSettings {
        ServerSettings {
            dicom_port: self.dicom_port,
            ae_title: self.ae_title.clone(),
            ae_whitelist_enabled: self.ae_whitelist_enabled,
            accept_all_transfer_syntaxes: self.accept_all_transfer_syntaxes,
            accepted_transfer_syntaxes: self.accepted_transfer_syntaxes.clone(),
            preferred_transfer_syntaxes: self.preferred_transfer_syntaxes.clone(),
            storage_transfer_syntax: self.storage_transfer_syntax.clone(),
            max_associations: self.max_associations,
            dimse_timeout_secs: self.dimse_timeout_secs,
        }
    }
}

/// Metadata database connection configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    /// Backend-specific connection string.
    ///
    /// Examples:
    /// - PostgreSQL: `postgres://user:password@host:port/dbname`
    /// - SQLite: `sqlite://./data/pacsnode.db`
    pub url: String,
    /// Maximum number of pooled connections.
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    /// Run pending sqlx migrations on startup.
    #[serde(default = "default_true")]
    pub run_migrations: bool,
}

/// Object storage (S3/RustFS/MinIO) configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    /// S3-compatible endpoint URL (e.g. `http://localhost:9000`).
    pub endpoint: String,
    /// Bucket name for DICOM pixel data.
    pub bucket: String,
    /// S3 access key ID.
    pub access_key: String,
    /// S3 secret access key.
    pub secret_key: String,
    /// AWS/S3 region string (MinIO/RustFS ignore this).
    #[serde(default = "default_region")]
    pub region: String,
}

/// Local filesystem blob storage configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FilesystemStorageConfig {
    /// Root directory used for DICOM blob storage.
    pub root: String,
}

/// Log format and verbosity.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
    /// Log level filter (e.g. `"info"`, `"debug"`, `"info,pacs_dimse=trace"`).
    #[serde(default = "default_log_level")]
    pub level: String,
    /// Log output format: `"json"` for structured JSON, `"pretty"` for human-readable.
    #[serde(default = "default_log_format")]
    pub format: LogFormat,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

/// Log output format.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Machine-readable JSON (default, suitable for production).
    #[default]
    Json,
    /// Human-readable pretty-printed output (useful in development).
    Pretty,
}

/// Plugin system configuration.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PluginsConfig {
    /// Optional plugin IDs to activate in addition to the built-in default-enabled plugins.
    #[serde(default)]
    pub enabled: Vec<String>,
    /// Per-plugin configuration sections keyed by plugin ID.
    #[serde(default, flatten)]
    pub configs: HashMap<String, serde_json::Value>,
}

/// Supported generated configuration profiles for `pacsnode generate-config`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedConfigProfile {
    /// SQLite metadata + filesystem blob storage.
    Standalone,
    /// PostgreSQL metadata + S3-compatible blob storage.
    Production,
}

impl GeneratedConfigProfile {
    /// Parses a user-provided profile name.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "standalone" | "sqlite" => Some(Self::Standalone),
            "production" | "prod" | "postgres" | "postgresql" => Some(Self::Production),
            _ => None,
        }
    }

    /// Returns the canonical profile name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standalone => "standalone",
            Self::Production => "production",
        }
    }

    /// Renders a ready-to-use `config.toml` template for the selected profile.
    pub fn render(self) -> String {
        let backend = match self {
            Self::Standalone => {
                r#"# SQLite metadata + local filesystem blobs
[database]
url = "sqlite://./data/pacsnode.db"
max_connections = 20
run_migrations = true

[filesystem_storage]
root = "./data/blobs"
"#
            }
            Self::Production => {
                r#"# PostgreSQL metadata + S3-compatible blobs
# Edit these placeholders to point at your real PostgreSQL and S3/MinIO instances.
[database]
url = "postgres://pacsnode:secret@localhost:5432/pacsnode"
max_connections = 20
run_migrations = true

[storage]
endpoint = "http://localhost:9000"
bucket = "dicom"
access_key = "CHANGE_ME"
secret_key = "CHANGE_ME"
region = "us-east-1"
"#
            }
        };

        format!(
            r#"# Generated by `pacsnode generate-config {profile}`
# The bundled OHIF viewer is embedded in the pacsnode binary and extracted into
# `./web/viewer/` automatically on first start.

[server]
http_port = 8042

{backend}
[logging]
level = "info"
format = "json"

[plugins]
enabled = ["{admin_plugin_id}", "{viewer_plugin_id}"]

[plugins.{admin_plugin_id}]
route_prefix = "/admin"
redirect_root = false
activity_limit = 24

[plugins.{viewer_plugin_id}]
static_dir = "./web/viewer"
route_prefix = "/viewer"
redirect_root = true
index_file = "index.html"
fallback_file = "index.html"
generate_app_config = true
provision_embedded_bundle = true
"#,
            profile = self.as_str(),
            backend = backend,
            admin_plugin_id = ADMIN_DASHBOARD_PLUGIN_ID,
            viewer_plugin_id = OHIF_VIEWER_PLUGIN_ID,
        )
    }
}

impl AppConfig {
    /// Load configuration from `config.toml` + environment variables.
    ///
    /// Sources are applied in priority order (highest wins):
    ///
    /// 1. `config.toml` next to the executable — found automatically, no path
    ///    configuration required; useful when shipping the binary with a bundled
    ///    config file in the same directory.
    /// 2. `config.toml` in the **current working directory** — overrides the
    ///    executable-adjacent file when both exist.
    /// 3. **Environment variables** — `PACS_` prefix, `__` separator
    ///    (e.g. `PACS_SERVER__HTTP_PORT=9000` overrides `server.http_port`).
    ///
    /// All sources are optional; missing files are silently skipped.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] if a file exists but is malformed, or if a
    /// required field is missing after all sources are merged.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use pacs_server::config::AppConfig;
    /// let cfg = AppConfig::load().expect("failed to load config");
    /// println!("listening on :{}", cfg.server.http_port);
    /// ```
    pub fn load() -> Result<Self, ConfigError> {
        let mut builder = Config::builder();

        // Layer 1: config.toml next to the executable (lowest file priority).
        // Resolved via current_exe() so it works regardless of working directory.
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let exe_config = exe_dir.join("config");
                let exe_config = exe_config.to_string_lossy();
                builder = builder.add_source(File::with_name(exe_config.as_ref()).required(false));
            }
        }

        // Layer 2: config.toml in the current working directory (overrides exe-adjacent).
        builder = builder.add_source(File::with_name("config").required(false));

        // Layer 3: environment variables (highest priority, override both files).
        builder = builder.add_source(
            Environment::with_prefix("PACS")
                .prefix_separator("_")
                .separator("__")
                .try_parsing(true),
        );

        builder.build()?.try_deserialize()
    }

    /// Load configuration from the named file stem (without extension).
    ///
    /// Useful for testing with alternative config files.
    #[cfg(test)]
    pub fn load_from(file_stem: &str) -> Result<Self, ConfigError> {
        Config::builder()
            // Optional TOML config file
            .add_source(File::with_name(file_stem).required(false))
            // Environment variable overrides:
            //   PACS_DATABASE__URL   → database.url
            //   PACS_SERVER__HTTP_PORT → server.http_port
            // prefix_separator("_") separates the "PACS" prefix from the key;
            // separator("__") separates nested key segments.
            .add_source(
                Environment::with_prefix("PACS")
                    .prefix_separator("_")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?
            .try_deserialize()
    }
}

// ── Defaults ──────────────────────────────────────────────────────────────────

fn default_http_port() -> u16 {
    8042
}
fn default_dicom_port() -> u16 {
    4242
}
fn default_ae_title() -> String {
    "PACSNODE".to_string()
}
fn default_max_associations() -> usize {
    64
}
fn default_dimse_timeout_secs() -> u64 {
    30
}
fn default_max_connections() -> u32 {
    20
}
fn default_true() -> bool {
    true
}
fn default_region() -> String {
    "us-east-1".to_string()
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_log_format() -> LogFormat {
    LogFormat::Json
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            http_port: default_http_port(),
            dicom_port: default_dicom_port(),
            ae_title: default_ae_title(),
            ae_whitelist_enabled: false,
            accept_all_transfer_syntaxes: default_true(),
            accepted_transfer_syntaxes: Vec::new(),
            preferred_transfer_syntaxes: Vec::new(),
            storage_transfer_syntax: None,
            max_associations: default_max_associations(),
            dimse_timeout_secs: default_dimse_timeout_secs(),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn clear_transfer_syntax_policy_env() {
        std::env::remove_var("PACS_SERVER__ACCEPT_ALL_TRANSFER_SYNTAXES");
        std::env::remove_var("PACS_SERVER__ACCEPTED_TRANSFER_SYNTAXES");
        std::env::remove_var("PACS_SERVER__ACCEPTED_TRANSFER_SYNTAXES__0");
        std::env::remove_var("PACS_SERVER__ACCEPTED_TRANSFER_SYNTAXES__1");
        std::env::remove_var("PACS_SERVER__PREFERRED_TRANSFER_SYNTAXES");
        std::env::remove_var("PACS_SERVER__PREFERRED_TRANSFER_SYNTAXES__0");
        std::env::remove_var("PACS_SERVER__STORAGE_TRANSFER_SYNTAX");
    }

    /// All defaults should be sensible without any file or env vars.
    #[test]
    #[serial]
    fn defaults_are_sensible() {
        clear_transfer_syntax_policy_env();
        // Minimal env to satisfy required fields.
        std::env::set_var("PACS_DATABASE__URL", "postgres://u:p@localhost/pacs");
        std::env::set_var("PACS_STORAGE__ENDPOINT", "http://localhost:9000");
        std::env::set_var("PACS_STORAGE__BUCKET", "dicom");
        std::env::set_var("PACS_STORAGE__ACCESS_KEY", "key");
        std::env::set_var("PACS_STORAGE__SECRET_KEY", "secret");

        let cfg = AppConfig::load_from("nonexistent_config_file").expect("load failed");
        assert_eq!(cfg.server.http_port, 8042);
        assert_eq!(cfg.server.dicom_port, 4242);
        assert_eq!(cfg.server.ae_title, "PACSNODE");
        assert!(!cfg.server.ae_whitelist_enabled);
        assert!(cfg.server.accept_all_transfer_syntaxes);
        assert!(cfg.server.accepted_transfer_syntaxes.is_empty());
        assert!(cfg.server.preferred_transfer_syntaxes.is_empty());
        assert!(cfg.server.storage_transfer_syntax.is_none());
        assert_eq!(cfg.server.max_associations, 64);
        assert!(cfg.nodes.is_empty());
        assert!(
            cfg.database
                .as_ref()
                .expect("database config")
                .run_migrations
        );
        assert_eq!(
            cfg.storage.as_ref().expect("storage config").region,
            "us-east-1"
        );
        assert_eq!(cfg.logging.format, LogFormat::Json);

        // Cleanup
        std::env::remove_var("PACS_DATABASE__URL");
        std::env::remove_var("PACS_STORAGE__ENDPOINT");
        std::env::remove_var("PACS_STORAGE__BUCKET");
        std::env::remove_var("PACS_STORAGE__ACCESS_KEY");
        std::env::remove_var("PACS_STORAGE__SECRET_KEY");
        clear_transfer_syntax_policy_env();
    }

    #[test]
    #[serial]
    fn env_var_overrides_default_http_port() {
        clear_transfer_syntax_policy_env();
        std::env::set_var("PACS_DATABASE__URL", "postgres://u:p@localhost/pacs");
        std::env::set_var("PACS_STORAGE__ENDPOINT", "http://localhost:9000");
        std::env::set_var("PACS_STORAGE__BUCKET", "dicom");
        std::env::set_var("PACS_STORAGE__ACCESS_KEY", "key");
        std::env::set_var("PACS_STORAGE__SECRET_KEY", "secret");
        std::env::set_var("PACS_SERVER__HTTP_PORT", "9999");

        let cfg = AppConfig::load_from("nonexistent_config_file").expect("load failed");
        assert_eq!(cfg.server.http_port, 9999);

        std::env::remove_var("PACS_DATABASE__URL");
        std::env::remove_var("PACS_STORAGE__ENDPOINT");
        std::env::remove_var("PACS_STORAGE__BUCKET");
        std::env::remove_var("PACS_STORAGE__ACCESS_KEY");
        std::env::remove_var("PACS_STORAGE__SECRET_KEY");
        std::env::remove_var("PACS_SERVER__HTTP_PORT");
        clear_transfer_syntax_policy_env();
    }

    #[test]
    #[serial]
    fn env_var_overrides_ae_whitelist_toggle() {
        clear_transfer_syntax_policy_env();
        std::env::set_var("PACS_DATABASE__URL", "postgres://u:p@localhost/pacs");
        std::env::set_var("PACS_STORAGE__ENDPOINT", "http://localhost:9000");
        std::env::set_var("PACS_STORAGE__BUCKET", "dicom");
        std::env::set_var("PACS_STORAGE__ACCESS_KEY", "key");
        std::env::set_var("PACS_STORAGE__SECRET_KEY", "secret");
        std::env::set_var("PACS_SERVER__AE_WHITELIST_ENABLED", "true");

        let cfg = AppConfig::load_from("nonexistent_config_file").expect("load failed");
        assert!(cfg.server.ae_whitelist_enabled);

        std::env::remove_var("PACS_DATABASE__URL");
        std::env::remove_var("PACS_STORAGE__ENDPOINT");
        std::env::remove_var("PACS_STORAGE__BUCKET");
        std::env::remove_var("PACS_STORAGE__ACCESS_KEY");
        std::env::remove_var("PACS_STORAGE__SECRET_KEY");
        std::env::remove_var("PACS_SERVER__AE_WHITELIST_ENABLED");
        clear_transfer_syntax_policy_env();
    }

    #[test]
    #[serial]
    fn env_var_overrides_transfer_syntax_policy() {
        clear_transfer_syntax_policy_env();
        std::env::set_var("PACS_DATABASE__URL", "postgres://u:p@localhost/pacs");
        std::env::set_var("PACS_STORAGE__ENDPOINT", "http://localhost:9000");
        std::env::set_var("PACS_STORAGE__BUCKET", "dicom");
        std::env::set_var("PACS_STORAGE__ACCESS_KEY", "key");
        std::env::set_var("PACS_STORAGE__SECRET_KEY", "secret");
        std::env::set_var("PACS_SERVER__ACCEPT_ALL_TRANSFER_SYNTAXES", "false");

        let cfg = AppConfig::load_from("nonexistent_config_file").expect("load failed");
        assert!(!cfg.server.accept_all_transfer_syntaxes);
        assert!(cfg.server.accepted_transfer_syntaxes.is_empty());
        assert!(cfg.server.preferred_transfer_syntaxes.is_empty());

        std::env::remove_var("PACS_DATABASE__URL");
        std::env::remove_var("PACS_STORAGE__ENDPOINT");
        std::env::remove_var("PACS_STORAGE__BUCKET");
        std::env::remove_var("PACS_STORAGE__ACCESS_KEY");
        std::env::remove_var("PACS_STORAGE__SECRET_KEY");
        clear_transfer_syntax_policy_env();
    }

    #[test]
    fn transfer_syntax_policy_deserializes_from_toml() {
        let toml = r#"
            [server]
            http_port = 8042
            dicom_port = 4242
            ae_title = "PACSNODE"
            accept_all_transfer_syntaxes = false
            accepted_transfer_syntaxes = ["1.2.840.10008.1.2.1", "1.2.840.10008.1.2.4.50"]
            preferred_transfer_syntaxes = ["1.2.840.10008.1.2.4.50"]
            storage_transfer_syntax = "1.2.840.10008.1.2.4.90"
            [database]
            url = "postgres://u:p@h/db"
            [storage]
            endpoint = "http://localhost:9000"
            bucket = "dicom"
            access_key = "k"
            secret_key = "s"
        "#;
        let cfg: AppConfig = config::Config::builder()
            .add_source(config::File::from_str(toml, config::FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert!(!cfg.server.accept_all_transfer_syntaxes);
        assert_eq!(
            cfg.server.accepted_transfer_syntaxes,
            vec![
                "1.2.840.10008.1.2.1".to_string(),
                "1.2.840.10008.1.2.4.50".to_string(),
            ]
        );
        assert_eq!(
            cfg.server.preferred_transfer_syntaxes,
            vec!["1.2.840.10008.1.2.4.50".to_string()]
        );
        assert_eq!(
            cfg.server.storage_transfer_syntax.as_deref(),
            Some("1.2.840.10008.1.2.4.90")
        );
    }

    #[test]
    fn configured_nodes_deserialize_from_toml() {
        let toml = r#"
            [server]
            http_port = 8042
            dicom_port = 4242
            ae_title = "PACSNODE"
            [database]
            url = "postgres://u:p@h/db"
            [storage]
            endpoint = "http://localhost:9000"
            bucket = "dicom"
            access_key = "k"
            secret_key = "s"

            [[nodes]]
            ae_title = "MODALITY1"
            host = "192.168.1.10"
            port = 104
            description = "CT Scanner"

            [[nodes]]
            ae_title = "REMOTEPACS"
            host = "pacs.example.test"
            port = 11112
            tls_enabled = true
        "#;
        let cfg: AppConfig = config::Config::builder()
            .add_source(config::File::from_str(toml, config::FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(cfg.nodes.len(), 2);
        assert_eq!(cfg.nodes[0].ae_title, "MODALITY1");
        assert_eq!(cfg.nodes[0].host, "192.168.1.10");
        assert_eq!(cfg.nodes[0].port, 104);
        assert_eq!(cfg.nodes[0].description.as_deref(), Some("CT Scanner"));
        assert!(!cfg.nodes[0].tls_enabled);

        assert_eq!(cfg.nodes[1].ae_title, "REMOTEPACS");
        assert_eq!(cfg.nodes[1].host, "pacs.example.test");
        assert_eq!(cfg.nodes[1].port, 11112);
        assert!(cfg.nodes[1].description.is_none());
        assert!(cfg.nodes[1].tls_enabled);
    }

    #[test]
    fn log_format_deserializes_pretty() {
        let toml = r#"
            [server]
            http_port = 8042
            dicom_port = 4242
            [database]
            url = "postgres://u:p@h/db"
            [storage]
            endpoint  = "http://localhost:9000"
            bucket    = "dicom"
            access_key = "k"
            secret_key = "s"
            [logging]
            format = "pretty"
        "#;
        let cfg: AppConfig = config::Config::builder()
            .add_source(config::File::from_str(toml, config::FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(cfg.logging.format, LogFormat::Pretty);
    }

    #[test]
    fn filesystem_storage_deserializes_from_toml() {
        let toml = r#"
            [server]
            http_port = 8042
            dicom_port = 4242
            [database]
            url = "sqlite://./data/pacsnode.db"
            [filesystem_storage]
            root = "./data/blobs"
        "#;
        let cfg: AppConfig = config::Config::builder()
            .add_source(config::File::from_str(toml, config::FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(
            cfg.filesystem_storage
                .as_ref()
                .expect("filesystem storage")
                .root,
            "./data/blobs"
        );
    }

    #[test]
    fn generated_standalone_config_is_valid_and_enables_admin_and_viewer() {
        let toml = GeneratedConfigProfile::Standalone.render();
        let cfg: AppConfig = config::Config::builder()
            .add_source(config::File::from_str(&toml, config::FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(
            cfg.database.as_ref().expect("database config").url,
            "sqlite://./data/pacsnode.db"
        );
        assert_eq!(
            cfg.filesystem_storage
                .as_ref()
                .expect("filesystem storage")
                .root,
            "./data/blobs"
        );
        assert!(cfg.storage.is_none());
        assert_eq!(
            cfg.server.bootstrap_server_settings(),
            ServerSettings::default()
        );
        assert!(cfg
            .plugins
            .enabled
            .contains(&ADMIN_DASHBOARD_PLUGIN_ID.to_string()));
        assert!(cfg
            .plugins
            .enabled
            .contains(&OHIF_VIEWER_PLUGIN_ID.to_string()));
        assert!(!toml.contains("dicom_port ="));
        assert!(!toml.contains("ae_title ="));
        assert!(toml.contains("[plugins.admin-dashboard]"));
        assert!(toml.contains("[plugins.ohif-viewer]"));
        assert!(toml.contains(
            "[plugins.admin-dashboard]\nroute_prefix = \"/admin\"\nredirect_root = false"
        ));
        assert!(toml.contains("[plugins.ohif-viewer]\nstatic_dir = \"./web/viewer\"\nroute_prefix = \"/viewer\"\nredirect_root = true"));
    }

    #[test]
    fn generated_production_config_is_valid_and_enables_admin_and_viewer() {
        let toml = GeneratedConfigProfile::Production.render();
        let cfg: AppConfig = config::Config::builder()
            .add_source(config::File::from_str(&toml, config::FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(
            cfg.database.as_ref().expect("database config").url,
            "postgres://pacsnode:secret@localhost:5432/pacsnode"
        );
        assert_eq!(
            cfg.storage.as_ref().expect("storage config").bucket,
            "dicom"
        );
        assert!(cfg.filesystem_storage.is_none());
        assert_eq!(
            cfg.server.bootstrap_server_settings(),
            ServerSettings::default()
        );
        assert!(cfg
            .plugins
            .enabled
            .contains(&ADMIN_DASHBOARD_PLUGIN_ID.to_string()));
        assert!(cfg
            .plugins
            .enabled
            .contains(&OHIF_VIEWER_PLUGIN_ID.to_string()));
        assert!(!toml.contains("dicom_port ="));
        assert!(!toml.contains("ae_title ="));
        assert!(toml.contains("[plugins.admin-dashboard]"));
        assert!(toml.contains("[plugins.ohif-viewer]"));
        assert!(toml.contains(
            "[plugins.admin-dashboard]\nroute_prefix = \"/admin\"\nredirect_root = false"
        ));
        assert!(toml.contains("[plugins.ohif-viewer]\nstatic_dir = \"./web/viewer\"\nroute_prefix = \"/viewer\"\nredirect_root = true"));
    }
}
