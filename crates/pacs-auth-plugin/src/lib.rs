//! pacsnode local multi-user auth plugin.
//!
//! Provides username/password login, refresh-token rotation, bearer-token
//! middleware, and user identity propagation for secured HTTP routes.

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
    extract::{Extension, Json, State},
    http::{header, HeaderMap, Method, Request, StatusCode},
    middleware::{from_fn_with_state, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use pacs_core::{MetadataStore, PacsError, RefreshToken, RefreshTokenId, User, UserId};
use pacs_plugin::{
    register_plugin, AppState, AuthenticatedUser, MiddlewarePlugin, Plugin, PluginContext,
    PluginError, PluginHealth, PluginManifest, RoutePlugin,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{error, warn};
use uuid::Uuid;

/// Compile-time plugin ID for the built-in HTTP auth plugin.
pub const BASIC_AUTH_PLUGIN_ID: &str = "basic-auth";

#[derive(Default)]
pub struct BasicAuthPlugin {
    runtime: Option<Arc<AuthRuntime>>,
}

#[derive(Debug, Clone, Deserialize)]
struct AuthPluginConfig {
    jwt_secret: String,
    #[serde(default = "default_login_path")]
    login_path: String,
    #[serde(default = "default_refresh_path")]
    refresh_path: String,
    #[serde(default = "default_logout_path")]
    logout_path: String,
    #[serde(default = "default_me_path")]
    me_path: String,
    #[serde(default = "default_public_paths")]
    public_paths: Vec<String>,
    #[serde(default = "default_access_token_ttl_secs", alias = "token_ttl_secs")]
    access_token_ttl_secs: u64,
    #[serde(default = "default_refresh_token_ttl_secs")]
    refresh_token_ttl_secs: u64,
    issuer: Option<String>,
}

#[derive(Clone)]
struct AuthRuntime {
    store: Arc<dyn MetadataStore>,
    jwt_secret: String,
    login_path: String,
    refresh_path: String,
    logout_path: String,
    me_path: String,
    public_paths: Vec<String>,
    access_token_ttl_secs: u64,
    refresh_token_ttl_secs: u64,
    issuer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuthClaims {
    sub: String,
    username: String,
    role: String,
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
    refresh_token: String,
    token_type: &'static str,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Debug, Serialize)]
struct MeResponse {
    user_id: String,
    username: String,
    display_name: Option<String>,
    email: Option<String>,
    role: String,
    attributes: serde_json::Value,
    is_active: bool,
}

#[derive(Debug, thiserror::Error)]
enum AuthRuntimeError {
    #[error("invalid password hash: {0}")]
    InvalidPasswordHash(String),
    #[error("jwt error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("system clock error: {0}")]
    Clock(#[from] std::time::SystemTimeError),
    #[error("store error: {0}")]
    Store(#[from] PacsError),
    #[error("invalid user id: {0}")]
    InvalidUserId(String),
}

fn default_login_path() -> String {
    "/auth/login".into()
}

fn default_refresh_path() -> String {
    "/auth/refresh".into()
}

fn default_logout_path() -> String {
    "/auth/logout".into()
}

fn default_me_path() -> String {
    "/auth/me".into()
}

fn default_public_paths() -> Vec<String> {
    vec!["/health".into(), "/metrics".into()]
}

fn default_access_token_ttl_secs() -> u64 {
    900
}

fn default_refresh_token_ttl_secs() -> u64 {
    604800
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
        let store = ctx
            .metadata_store
            .clone()
            .ok_or_else(|| PluginError::Config {
                plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
                message: "basic-auth requires an active MetadataStore plugin".into(),
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
        let logout_path =
            normalize_path(&config.logout_path).map_err(|message| PluginError::Config {
                plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
                message,
            })?;
        let me_path = normalize_path(&config.me_path).map_err(|message| PluginError::Config {
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
            store,
            jwt_secret: config.jwt_secret,
            login_path,
            refresh_path,
            logout_path,
            me_path,
            public_paths,
            access_token_ttl_secs: config.access_token_ttl_secs,
            refresh_token_ttl_secs: config.refresh_token_ttl_secs,
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
        let logout_path = runtime.logout_path.clone();
        let me_path = runtime.me_path.clone();

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
                    move |payload| refresh_handler(Arc::clone(&runtime), payload)
                }),
            )
            .route(
                &logout_path,
                post({
                    let runtime = Arc::clone(&runtime);
                    move |Extension(user)| logout_handler(Arc::clone(&runtime), user)
                }),
            )
            .route(&me_path, get(me_handler))
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
    async fn authenticate_user(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Option<User>, AuthRuntimeError> {
        let mut user = match self.store.get_user_by_username(username).await {
            Ok(user) => user,
            Err(PacsError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(AuthRuntimeError::Store(error)),
        };

        if !user.is_active || user_is_locked(&user) {
            return Ok(None);
        }

        let parsed_hash = PasswordHash::new(&user.password_hash)
            .map_err(|error| AuthRuntimeError::InvalidPasswordHash(error.to_string()))?;
        match Argon2::default().verify_password(password.as_bytes(), &parsed_hash) {
            Ok(()) => {
                if user.failed_login_attempts > 0 || user.locked_until.is_some() {
                    user.failed_login_attempts = 0;
                    user.locked_until = None;
                    self.store.store_user(&user).await?;
                }
                Ok(Some(user))
            }
            Err(PasswordHashError::Password) => {
                self.record_failed_login(&mut user).await?;
                Ok(None)
            }
            Err(error) => Err(AuthRuntimeError::InvalidPasswordHash(error.to_string())),
        }
    }

    async fn record_failed_login(&self, user: &mut User) -> Result<(), AuthRuntimeError> {
        let policy = self.store.get_password_policy().await?;
        user.failed_login_attempts = user.failed_login_attempts.saturating_add(1);
        if user.failed_login_attempts >= policy.max_failed_attempts {
            user.locked_until =
                Some(Utc::now() + Duration::seconds(i64::from(policy.lockout_duration_secs)));
            user.failed_login_attempts = 0;
        }
        self.store.store_user(user).await?;
        Ok(())
    }

    fn issue_access_token(&self, user: &User) -> Result<String, AuthRuntimeError> {
        let issued_at = unix_now_secs()?;
        let claims = AuthClaims {
            sub: user.id.to_string(),
            username: user.username.clone(),
            role: user.role.as_str().to_string(),
            iss: self.issuer.clone(),
            iat: issued_at,
            exp: issued_at + self.access_token_ttl_secs as usize,
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

    async fn issue_token_pair(&self, user: &User) -> Result<TokenResponse, AuthRuntimeError> {
        let access_token = self.issue_access_token(user)?;
        let refresh_token = raw_refresh_token();
        let refresh = RefreshToken {
            id: RefreshTokenId::new(),
            user_id: user.id,
            token_hash: hash_refresh_token(&refresh_token),
            expires_at: Utc::now() + Duration::seconds(self.refresh_token_ttl_secs as i64),
            created_at: Utc::now(),
            revoked_at: None,
        };
        self.store.store_refresh_token(&refresh).await?;

        Ok(TokenResponse {
            access_token,
            refresh_token,
            token_type: "Bearer",
            expires_in: self.access_token_ttl_secs,
        })
    }

    async fn refresh_tokens(
        &self,
        raw_refresh_token_value: &str,
    ) -> Result<Option<TokenResponse>, AuthRuntimeError> {
        let token_hash = hash_refresh_token(raw_refresh_token_value);
        let mut refresh_token = match self.store.get_refresh_token(&token_hash).await {
            Ok(token) => token,
            Err(PacsError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(AuthRuntimeError::Store(error)),
        };

        if refresh_token.revoked_at.is_some() || refresh_token.expires_at <= Utc::now() {
            return Ok(None);
        }

        let user = self.store.get_user(&refresh_token.user_id).await?;
        if !user.is_active || user_is_locked(&user) {
            return Ok(None);
        }

        refresh_token.revoked_at = Some(Utc::now());
        self.store.store_refresh_token(&refresh_token).await?;

        self.issue_token_pair(&user).await.map(Some)
    }

    async fn current_user_from_token(
        &self,
        token: &str,
    ) -> Result<AuthenticatedUser, AuthRuntimeError> {
        let claims = self.validate_token(token)?;
        let user_id = claims
            .sub
            .parse::<UserId>()
            .map_err(|error| AuthRuntimeError::InvalidUserId(error.to_string()))?;
        let user = self.store.get_user(&user_id).await?;
        if !user.is_active || user_is_locked(&user) {
            return Err(AuthRuntimeError::Store(PacsError::InvalidRequest(
                "inactive or locked user".into(),
            )));
        }

        Ok(to_authenticated_user(&user))
    }

    fn is_public_path(&self, path: &str) -> bool {
        self.public_paths
            .iter()
            .any(|public| path_matches(path, public))
            || path_matches(path, &self.login_path)
            || path_matches(path, &self.refresh_path)
    }
}

async fn login_handler(runtime: Arc<AuthRuntime>, Json(payload): Json<LoginRequest>) -> Response {
    match runtime
        .authenticate_user(&payload.username, &payload.password)
        .await
    {
        Ok(Some(user)) => match runtime.issue_token_pair(&user).await {
            Ok(tokens) => Json(tokens).into_response(),
            Err(error) => {
                error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Failed to issue token pair");
                internal_error_response()
            }
        },
        Ok(None) => unauthorized_response("invalid credentials"),
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Credential verification failed");
            internal_error_response()
        }
    }
}

async fn refresh_handler(
    runtime: Arc<AuthRuntime>,
    Json(payload): Json<RefreshRequest>,
) -> Response {
    match runtime.refresh_tokens(&payload.refresh_token).await {
        Ok(Some(tokens)) => Json(tokens).into_response(),
        Ok(None) => unauthorized_response("invalid refresh token"),
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Failed to refresh token pair");
            internal_error_response()
        }
    }
}

async fn logout_handler(runtime: Arc<AuthRuntime>, user: AuthenticatedUser) -> Response {
    let user_id = match user.user_id.parse::<UserId>() {
        Ok(user_id) => user_id,
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Failed to parse authenticated user id");
            return internal_error_response();
        }
    };

    match runtime.store.revoke_refresh_tokens(&user_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Failed to revoke refresh tokens");
            internal_error_response()
        }
    }
}

async fn me_handler(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    let user_id = match user.user_id.parse::<UserId>() {
        Ok(user_id) => user_id,
        Err(error) => {
            error!(error = %error, "Failed to parse authenticated user id");
            return internal_error_response();
        }
    };

    match state.store.get_user(&user_id).await {
        Ok(current_user) => Json(MeResponse {
            user_id: current_user.id.to_string(),
            username: current_user.username,
            display_name: current_user.display_name,
            email: current_user.email,
            role: current_user.role.to_string(),
            attributes: current_user.attributes,
            is_active: current_user.is_active,
        })
        .into_response(),
        Err(error) => {
            error!(error = %error, "Failed to load authenticated user profile");
            internal_error_response()
        }
    }
}

async fn auth_middleware(
    State(runtime): State<Arc<AuthRuntime>>,
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

    match runtime.current_user_from_token(token).await {
        Ok(user) => {
            request.extensions_mut().insert(user);
            next.run(request).await
        }
        Err(AuthRuntimeError::Jwt(_)) => unauthorized_response("invalid or expired bearer token"),
        Err(AuthRuntimeError::Store(PacsError::NotFound { .. })) => {
            unauthorized_response("unknown user")
        }
        Err(AuthRuntimeError::Store(PacsError::InvalidRequest(_))) => {
            unauthorized_response("inactive or locked user")
        }
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Token validation failed");
            internal_error_response()
        }
    }
}

fn to_authenticated_user(user: &User) -> AuthenticatedUser {
    AuthenticatedUser::new(
        user.id.to_string(),
        user.username.clone(),
        user.role.as_str(),
        user.attributes.clone(),
    )
}

fn user_is_locked(user: &User) -> bool {
    user.locked_until
        .is_some_and(|locked_until| locked_until > Utc::now())
}

fn raw_refresh_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn hash_refresh_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
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
    use chrono::{TimeZone, Utc};
    use pacs_core::{
        AuditLogEntry, AuditLogPage, AuditLogQuery, BlobStore, DicomJson, DicomNode, Instance,
        InstanceQuery, MetadataStore, PacsError, PacsResult, PacsStatistics, PasswordPolicy,
        RefreshToken, Series, SeriesQuery, SeriesUid, ServerSettings, SopInstanceUid, Study,
        StudyQuery, StudyUid, User, UserId, UserQuery, UserRole,
    };
    use std::{collections::HashMap, sync::Mutex};
    use tower::ServiceExt;
    use uuid::Uuid;

    struct TestMetadataStore {
        user: Mutex<User>,
        refresh_tokens: Mutex<HashMap<String, RefreshToken>>,
        password_policy: Mutex<PasswordPolicy>,
    }

    impl Default for TestMetadataStore {
        fn default() -> Self {
            Self {
                user: Mutex::new(User {
                    id: UserId::from(Uuid::from_u128(1)),
                    username: "admin".into(),
                    display_name: Some("Admin User".into()),
                    email: Some("admin@example.test".into()),
                    password_hash: test_password_hash(),
                    role: UserRole::Admin,
                    attributes: serde_json::json!({"department": "radiology"}),
                    is_active: true,
                    failed_login_attempts: 0,
                    locked_until: None,
                    password_changed_at: Some(Utc.with_ymd_and_hms(2026, 3, 18, 12, 0, 0).unwrap()),
                    created_at: Some(Utc.with_ymd_and_hms(2026, 3, 18, 12, 0, 0).unwrap()),
                    updated_at: Some(Utc.with_ymd_and_hms(2026, 3, 18, 12, 0, 0).unwrap()),
                }),
                refresh_tokens: Mutex::new(HashMap::new()),
                password_policy: Mutex::new(PasswordPolicy::default()),
            }
        }
    }

    #[async_trait]
    impl MetadataStore for TestMetadataStore {
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
        async fn store_user(&self, user: &User) -> PacsResult<()> {
            *self.user.lock().unwrap() = user.clone();
            Ok(())
        }
        async fn get_user(&self, id: &UserId) -> PacsResult<User> {
            let user = self.user.lock().unwrap().clone();
            if user.id == *id {
                Ok(user)
            } else {
                Err(PacsError::NotFound {
                    resource: "user",
                    uid: id.to_string(),
                })
            }
        }
        async fn get_user_by_username(&self, username: &str) -> PacsResult<User> {
            let user = self.user.lock().unwrap().clone();
            if user.username == username {
                Ok(user)
            } else {
                Err(PacsError::NotFound {
                    resource: "user",
                    uid: username.to_string(),
                })
            }
        }
        async fn query_users(&self, _q: &UserQuery) -> PacsResult<Vec<User>> {
            Ok(vec![self.user.lock().unwrap().clone()])
        }
        async fn delete_user(&self, _id: &UserId) -> PacsResult<()> {
            Ok(())
        }
        async fn store_refresh_token(&self, token: &RefreshToken) -> PacsResult<()> {
            self.refresh_tokens
                .lock()
                .unwrap()
                .insert(token.token_hash.clone(), token.clone());
            Ok(())
        }
        async fn get_refresh_token(&self, token_hash: &str) -> PacsResult<RefreshToken> {
            self.refresh_tokens
                .lock()
                .unwrap()
                .get(token_hash)
                .cloned()
                .ok_or_else(|| PacsError::NotFound {
                    resource: "refresh_token",
                    uid: token_hash.to_string(),
                })
        }
        async fn revoke_refresh_tokens(&self, user_id: &UserId) -> PacsResult<()> {
            for token in self.refresh_tokens.lock().unwrap().values_mut() {
                if token.user_id == *user_id {
                    token.revoked_at = Some(Utc::now());
                }
            }
            Ok(())
        }
        async fn get_password_policy(&self) -> PacsResult<PasswordPolicy> {
            Ok(self.password_policy.lock().unwrap().clone())
        }
        async fn upsert_password_policy(&self, policy: &PasswordPolicy) -> PacsResult<()> {
            *self.password_policy.lock().unwrap() = policy.clone();
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
                "jwt_secret": "super-secret-signing-key",
                "access_token_ttl_secs": 300,
                "refresh_token_ttl_secs": 600
            }),
            metadata_store: Some(Arc::new(TestMetadataStore::default())),
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
            store: Arc::new(TestMetadataStore::default()),
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
        assert!(json["refresh_token"].as_str().is_some());

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

    #[tokio::test]
    async fn refresh_rotates_token_pair() {
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
        let login_json: serde_json::Value = serde_json::from_slice(
            &to_bytes(login_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();

        let refresh_body = serde_json::to_vec(&serde_json::json!({
            "refresh_token": login_json["refresh_token"]
        }))
        .unwrap();
        let refresh_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/refresh")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(refresh_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(refresh_response.status(), StatusCode::OK);
    }
}
