//! pacsnode server configuration.
//!
//! Configuration is loaded from a TOML file, then overridden by environment
//! variables with the prefix `PACS_` (e.g. `PACS_DATABASE__URL`).
//!
//! # Example config.toml
//!
//! ```toml
//! [server]
//! http_port  = 8042
//! dicom_port = 4242
//! ae_title   = "PACSNODE"
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
use pacs_core::DicomNode;
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
    /// PostgreSQL database settings.
    pub database: DatabaseConfig,
    /// S3-compatible object storage settings.
    pub storage: StorageConfig,
    /// Structured logging settings.
    #[serde(default)]
    pub logging: LoggingConfig,
    /// Optional plugin activation and configuration.
    #[serde(default)]
    pub plugins: PluginsConfig,
}

/// HTTP + DIMSE listener configuration.
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
    /// Maximum number of concurrent DIMSE associations.
    #[serde(default = "default_max_associations")]
    pub max_associations: usize,
    /// DIMSE association timeout in seconds.
    #[serde(default = "default_dimse_timeout_secs")]
    pub dimse_timeout_secs: u64,
}

/// PostgreSQL connection configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    /// `postgres://user:password@host:port/dbname` connection string.
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

/// Log format and verbosity.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LoggingConfig {
    /// Log level filter (e.g. `"info"`, `"debug"`, `"info,pacs_dimse=trace"`).
    #[serde(default = "default_log_level")]
    pub level: String,
    /// Log output format: `"json"` for structured JSON, `"pretty"` for human-readable.
    #[serde(default = "default_log_format")]
    pub format: LogFormat,
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

impl AppConfig {
    /// Load configuration from `config.toml` (optional) + environment variables.
    ///
    /// Environment variables override file values. The prefix `PACS_` is
    /// stripped, and `__` separates nested keys (e.g. `PACS_DATABASE__URL`
    /// sets `database.url`).
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] if the file exists but is malformed, or if a
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
        Self::load_from("config")
    }

    /// Load configuration from the named file stem (without extension).
    ///
    /// Useful for testing with alternative config files.
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
        assert_eq!(cfg.server.max_associations, 64);
        assert!(cfg.nodes.is_empty());
        assert!(cfg.database.run_migrations);
        assert_eq!(cfg.storage.region, "us-east-1");
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
}
