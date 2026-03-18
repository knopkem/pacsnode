//! pacsnode basic-auth plugin.
//!
//! Provides a local username/password login endpoint, bearer-token refresh, and
//! HTTP middleware that validates JWT access tokens.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use argon2::{
    password_hash::{Error as PasswordHashError, PasswordHash, PasswordVerifier},
    Argon2,
};
use async_trait::async_trait;
use axum::{
    extract::{Extension, Json, State as AxumState},
    http::{header, HeaderMap, Method, Request, StatusCode},
    middleware::{from_fn_with_state, Next},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use pacs_plugin::{
    register_plugin, AppState, AuthenticatedUser, MiddlewarePlugin, Plugin, PluginContext,
    PluginError, PluginHealth, PluginManifest, RoutePlugin,
};
use serde::{Deserialize, Serialize};
use tracing::{error, warn};

/// Compile-time plugin ID for the built-in HTTP auth plugin.
pub const BASIC_AUTH_PLUGIN_ID: &str = "basic-auth";

#[derive(Default)]
pub struct BasicAuthPlugin {
    runtime: Option<Arc<AuthRuntime>>,
}

#[derive(Debug, Clone, Deserialize)]
struct AuthPluginConfig {
    username: String,
    password_hash: String,
    jwt_secret: String,
    #[serde(default = "default_login_path")]
    login_path: String,
    #[serde(default = "default_refresh_path")]
    refresh_path: String,
    #[serde(default = "default_public_paths")]
    public_paths: Vec<String>,
    #[serde(default = "default_token_ttl_secs")]
    token_ttl_secs: u64,
    issuer: Option<String>,
}

#[derive(Debug, Clone)]
struct AuthRuntime {
    username: String,
    password_hash: String,
    jwt_secret: String,
    login_path: String,
    refresh_path: String,
    public_paths: Vec<String>,
    token_ttl_secs: u64,
    issuer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuthClaims {
    sub: String,
    iss: String,
    exp: usize,
    iat: usize,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: u64,
}

#[derive(Debug, thiserror::Error)]
enum AuthRuntimeError {
    #[error("invalid password hash: {0}")]
    InvalidPasswordHash(String),
    #[error("jwt error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("system clock error: {0}")]
    Clock(#[from] std::time::SystemTimeError),
}

fn default_login_path() -> String {
    "/auth/login".into()
}

fn default_refresh_path() -> String {
    "/auth/refresh".into()
}

fn default_public_paths() -> Vec<String> {
    vec!["/health".into(), "/metrics".into()]
}

fn default_token_ttl_secs() -> u64 {
    3600
}

#[async_trait]
impl Plugin for BasicAuthPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            BASIC_AUTH_PLUGIN_ID,
            "Basic HTTP Auth",
            env!("CARGO_PKG_VERSION"),
        )
        .disabled_by_default()
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        let config: AuthPluginConfig =
            serde_json::from_value(ctx.config.clone()).map_err(|error| PluginError::Config {
                plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
                message: error.to_string(),
            })?;

        PasswordHash::new(&config.password_hash).map_err(|error| PluginError::Config {
            plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
            message: format!("invalid password_hash: {error}"),
        })?;

        let login_path =
            normalize_path(&config.login_path).map_err(|message| PluginError::Config {
                plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
                message,
            })?;
        let refresh_path =
            normalize_path(&config.refresh_path).map_err(|message| PluginError::Config {
                plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
                message,
            })?;
        let public_paths = config
            .public_paths
            .iter()
            .map(|path| normalize_path(path))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|message| PluginError::Config {
                plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
                message,
            })?;

        self.runtime = Some(Arc::new(AuthRuntime {
            username: config.username,
            password_hash: config.password_hash,
            jwt_secret: config.jwt_secret,
            login_path,
            refresh_path,
            public_paths,
            token_ttl_secs: config.token_ttl_secs,
            issuer: config
                .issuer
                .unwrap_or_else(|| ctx.server_info.ae_title.clone()),
        }));
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

    fn as_middleware_plugin(&self) -> Option<&dyn MiddlewarePlugin> {
        Some(self)
    }
}

impl RoutePlugin for BasicAuthPlugin {
    fn routes(&self) -> Router<AppState> {
        let Some(runtime) = self.runtime.as_ref().map(Arc::clone) else {
            warn!(
                plugin_id = BASIC_AUTH_PLUGIN_ID,
                "Auth plugin routes requested before init"
            );
            return Router::new();
        };

        let login_path = runtime.login_path.clone();
        let refresh_path = runtime.refresh_path.clone();

        Router::new()
            .route(
                &login_path,
                post({
                    let runtime = Arc::clone(&runtime);
                    move |payload| login_handler(Arc::clone(&runtime), payload)
                }),
            )
            .route(
                &refresh_path,
                post({
                    let runtime = Arc::clone(&runtime);
                    move |Extension(user)| refresh_handler(Arc::clone(&runtime), user)
                }),
            )
    }
}

impl MiddlewarePlugin for BasicAuthPlugin {
    fn apply(&self, router: Router<AppState>) -> Router<AppState> {
        let Some(runtime) = self.runtime.as_ref().map(Arc::clone) else {
            warn!(
                plugin_id = BASIC_AUTH_PLUGIN_ID,
                "Auth middleware requested before init"
            );
            return router;
        };

        router.layer(from_fn_with_state(runtime, auth_middleware))
    }

    fn priority(&self) -> i32 {
        0
    }
}

impl AuthRuntime {
    fn verify_credentials(&self, username: &str, password: &str) -> Result<bool, AuthRuntimeError> {
        if username != self.username {
            return Ok(false);
        }

        let parsed_hash = PasswordHash::new(&self.password_hash)
            .map_err(|error| AuthRuntimeError::InvalidPasswordHash(error.to_string()))?;
        match Argon2::default().verify_password(password.as_bytes(), &parsed_hash) {
            Ok(()) => Ok(true),
            Err(PasswordHashError::Password) => Ok(false),
            Err(error) => Err(AuthRuntimeError::InvalidPasswordHash(error.to_string())),
        }
    }

    fn issue_token(&self, user_id: &str) -> Result<String, AuthRuntimeError> {
        let issued_at = unix_now_secs()?;
        let claims = AuthClaims {
            sub: user_id.to_string(),
            iss: self.issuer.clone(),
            iat: issued_at,
            exp: issued_at + self.token_ttl_secs as usize,
        };

        Ok(encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )?)
    }

    fn validate_token(&self, token: &str) -> Result<AuthClaims, AuthRuntimeError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[self.issuer.as_str()]);
        Ok(decode::<AuthClaims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &validation,
        )?
        .claims)
    }

    fn is_public_path(&self, path: &str) -> bool {
        self.public_paths
            .iter()
            .any(|public| path_matches(path, public))
            || path_matches(path, &self.login_path)
    }
}

async fn login_handler(runtime: Arc<AuthRuntime>, Json(payload): Json<LoginRequest>) -> Response {
    match runtime.verify_credentials(&payload.username, &payload.password) {
        Ok(true) => match runtime.issue_token(&payload.username) {
            Ok(token) => Json(TokenResponse {
                access_token: token,
                token_type: "Bearer",
                expires_in: runtime.token_ttl_secs,
            })
            .into_response(),
            Err(error) => {
                error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Failed to issue JWT");
                internal_error_response()
            }
        },
        Ok(false) => unauthorized_response("invalid credentials"),
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Credential verification failed");
            internal_error_response()
        }
    }
}

async fn refresh_handler(runtime: Arc<AuthRuntime>, user: AuthenticatedUser) -> Response {
    match runtime.issue_token(&user.user_id) {
        Ok(token) => Json(TokenResponse {
            access_token: token,
            token_type: "Bearer",
            expires_in: runtime.token_ttl_secs,
        })
        .into_response(),
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Failed to refresh JWT");
            internal_error_response()
        }
    }
}

async fn auth_middleware(
    AxumState(runtime): AxumState<Arc<AuthRuntime>>,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if request.method() == Method::OPTIONS || runtime.is_public_path(request.uri().path()) {
        return next.run(request).await;
    }

    let token = match bearer_token(request.headers()) {
        Ok(token) => token,
        Err(message) => return unauthorized_response(message),
    };

    match runtime.validate_token(token) {
        Ok(claims) => {
            request
                .extensions_mut()
                .insert(AuthenticatedUser::new(claims.sub));
            next.run(request).await
        }
        Err(AuthRuntimeError::Jwt(_)) => unauthorized_response("invalid or expired bearer token"),
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Token validation failed");
            internal_error_response()
        }
    }
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, &'static str> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Err("missing bearer token");
    };
    let Ok(raw) = value.to_str() else {
        return Err("invalid authorization header");
    };
    let mut parts = raw.split_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (Some(scheme), Some(token), None) if scheme.eq_ignore_ascii_case("bearer") => Ok(token),
        _ => Err("invalid authorization header"),
    }
}

fn path_matches(path: &str, configured: &str) -> bool {
    path == configured
        || path
            .strip_prefix(configured)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn normalize_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path configuration cannot be empty".into());
    }

    let normalized = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };

    if normalized.len() > 1 {
        Ok(normalized.trim_end_matches('/').to_string())
    } else {
        Ok(normalized)
    }
}

fn unix_now_secs() -> Result<usize, AuthRuntimeError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize)
}

fn unauthorized_response(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

fn internal_error_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": "internal error" })),
    )
        .into_response()
}

register_plugin!(BasicAuthPlugin::default);

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::{body::to_bytes, body::Body, http::Request, routing::get};
    use bytes::Bytes;
    use pacs_core::{
        AuditLogEntry, AuditLogPage, AuditLogQuery, BlobStore, DicomJson, DicomNode, Instance,
        InstanceQuery, MetadataStore, PacsError, PacsResult, PacsStatistics, Series, SeriesQuery,
        SeriesUid, ServerSettings, SopInstanceUid, Study, StudyQuery, StudyUid,
    };
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

        async fn store_audit_log(&self, _entry: &pacs_core::NewAuditLogEntry) -> PacsResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct NoopBlobStore;

    #[async_trait]
    impl BlobStore for NoopBlobStore {
        async fn put(&self, _key: &str, _data: Bytes) -> PacsResult<()> {
            Ok(())
        }
        async fn get(&self, key: &str) -> PacsResult<Bytes> {
            Err(PacsError::NotFound {
                resource: "blob",
                uid: key.to_string(),
            })
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

    fn test_password_hash() -> String {
        use argon2::password_hash::{PasswordHasher, SaltString};

        let salt = SaltString::from_b64("dGVzdHNhbHR0ZXN0c2FsdA").unwrap();
        Argon2::default()
            .hash_password(b"secret", &salt)
            .unwrap()
            .to_string()
    }

    fn plugin_context() -> PluginContext {
        PluginContext {
            config: serde_json::json!({
                "username": "admin",
                "password_hash": test_password_hash(),
                "jwt_secret": "super-secret-signing-key",
                "token_ttl_secs": 300
            }),
            metadata_store: Some(Arc::new(NoopMetadataStore)),
            blob_store: Some(Arc::new(NoopBlobStore)),
            server_info: pacs_plugin::ServerInfo {
                ae_title: "PACSNODE".into(),
                http_port: 8042,
                dicom_port: 4242,
                version: "test",
            },
            event_bus: Arc::new(pacs_plugin::EventBus::default()),
        }
    }

    fn app_state() -> AppState {
        AppState {
            server_info: pacs_plugin::ServerInfo {
                ae_title: "PACSNODE".into(),
                http_port: 8042,
                dicom_port: 4242,
                version: "test",
            },
            server_settings: ServerSettings::default(),
            store: Arc::new(NoopMetadataStore),
            blobs: Arc::new(NoopBlobStore),
            plugins: Arc::new(pacs_plugin::PluginRegistry::new()),
        }
    }

    async fn build_test_router() -> Router {
        let mut plugin = BasicAuthPlugin::default();
        plugin.init(&plugin_context()).await.unwrap();
        plugin
            .apply(
                Router::new()
                    .route("/health", get(|| async { StatusCode::OK }))
                    .route("/api/protected", get(|| async { StatusCode::OK }))
                    .merge(plugin.routes()),
            )
            .with_state(app_state())
    }

    #[tokio::test]
    async fn login_and_bearer_token_grant_access() {
        let app = build_test_router().await;
        let login_body = serde_json::to_vec(&serde_json::json!({
            "username": "admin",
            "password": "secret"
        }))
        .unwrap();
        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(login_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(login_response.status(), StatusCode::OK);

        let body = to_bytes(login_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token = json["access_token"].as_str().unwrap();

        let protected_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/protected")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(protected_response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn middleware_blocks_unauthenticated_protected_requests() {
        let app = build_test_router().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn health_path_remains_public() {
        let app = build_test_router().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
