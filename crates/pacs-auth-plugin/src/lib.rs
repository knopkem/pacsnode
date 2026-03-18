//! pacsnode HTTP auth plugin.
//!
//! Provides local username/password login with refresh-token rotation, or
//! external OIDC-style bearer-token validation, plus user identity propagation
//! for secured HTTP routes.

use std::{
    collections::HashMap,
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
use pacs_core::{MetadataStore, PacsError, RefreshToken, RefreshTokenId, User, UserId, UserRole};
use pacs_plugin::{
    register_plugin, AppState, AuthenticatedUser, MiddlewarePlugin, Plugin, PluginContext,
    PluginError, PluginHealth, PluginManifest, RoutePlugin,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::{error, warn};
use uuid::Uuid;

/// Compile-time plugin ID for the built-in HTTP auth plugin.
pub const BASIC_AUTH_PLUGIN_ID: &str = "basic-auth";

#[derive(Default)]
pub struct BasicAuthPlugin {
    runtime: Option<Arc<AuthRuntime>>,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum AuthPluginMode {
    #[default]
    Local,
    Oidc,
}

#[derive(Debug, Clone, Deserialize)]
struct AuthPluginConfig {
    #[serde(default)]
    mode: AuthPluginMode,
    #[serde(default)]
    jwt_secret: Option<String>,
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
    #[serde(default)]
    oidc: Option<OidcConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct OidcConfig {
    issuer: String,
    audience: String,
    public_key_pem: String,
    #[serde(default = "default_user_id_claim")]
    user_id_claim: String,
    #[serde(default = "default_username_claim")]
    username_claim: String,
    #[serde(default = "default_role_claim")]
    role_claim: String,
    #[serde(default)]
    role_map: HashMap<String, UserRole>,
    #[serde(default)]
    default_role: Option<UserRole>,
    #[serde(default)]
    attributes_claims: Vec<String>,
}

#[derive(Clone)]
struct LocalTokenConfig {
    jwt_secret: String,
    issuer: String,
}

#[derive(Clone)]
struct OidcTokenConfig {
    issuer: String,
    audience: String,
    public_key: Arc<DecodingKey>,
    user_id_claim: String,
    username_claim: String,
    role_claim: String,
    role_map: HashMap<String, UserRole>,
    default_role: UserRole,
    attributes_claims: Vec<String>,
}

#[derive(Clone)]
enum TokenMode {
    Local(LocalTokenConfig),
    Oidc(OidcTokenConfig),
}

#[derive(Clone)]
struct AuthRuntime {
    store: Arc<dyn MetadataStore>,
    token_mode: TokenMode,
    login_path: String,
    refresh_path: String,
    logout_path: String,
    me_path: String,
    public_paths: Vec<String>,
    access_token_ttl_secs: u64,
    refresh_token_ttl_secs: u64,
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
    #[error("invalid oidc claims: {0}")]
    InvalidOidcClaims(String),
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

fn default_user_id_claim() -> String {
    "sub".into()
}

fn default_username_claim() -> String {
    "preferred_username".into()
}

fn default_role_claim() -> String {
    "roles".into()
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

        let token_mode = match config.mode {
            AuthPluginMode::Local => {
                let jwt_secret = config
                    .jwt_secret
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| PluginError::Config {
                        plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
                        message: "basic-auth local mode requires jwt_secret".into(),
                    })?;

                TokenMode::Local(LocalTokenConfig {
                    jwt_secret,
                    issuer: config
                        .issuer
                        .unwrap_or_else(|| ctx.server_info.ae_title.clone()),
                })
            }
            AuthPluginMode::Oidc => {
                let oidc = config.oidc.ok_or_else(|| PluginError::Config {
                    plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
                    message: "basic-auth oidc mode requires an oidc configuration block".into(),
                })?;
                let public_key = DecodingKey::from_rsa_pem(oidc.public_key_pem.as_bytes())
                    .map_err(|error| PluginError::Config {
                        plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
                        message: format!("invalid oidc public_key_pem: {error}"),
                    })?;

                TokenMode::Oidc(OidcTokenConfig {
                    issuer: oidc.issuer,
                    audience: oidc.audience,
                    public_key: Arc::new(public_key),
                    user_id_claim: oidc.user_id_claim,
                    username_claim: oidc.username_claim,
                    role_claim: oidc.role_claim,
                    role_map: oidc.role_map,
                    default_role: oidc.default_role.unwrap_or(UserRole::Viewer),
                    attributes_claims: oidc.attributes_claims,
                })
            }
        };

        self.runtime = Some(Arc::new(AuthRuntime {
            store,
            token_mode,
            login_path,
            refresh_path,
            logout_path,
            me_path,
            public_paths,
            access_token_ttl_secs: config.access_token_ttl_secs,
            refresh_token_ttl_secs: config.refresh_token_ttl_secs,
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

        match &runtime.token_mode {
            TokenMode::Local(_) => Router::new()
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
                .route(&me_path, get(me_handler)),
            TokenMode::Oidc(_) => Router::new().route(&me_path, get(me_handler)),
        }
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
        let TokenMode::Local(local) = &self.token_mode else {
            return Err(AuthRuntimeError::InvalidOidcClaims(
                "cannot issue local tokens in oidc mode".into(),
            ));
        };
        let issued_at = unix_now_secs()?;
        let claims = AuthClaims {
            sub: user.id.to_string(),
            username: user.username.clone(),
            role: user.role.as_str().to_string(),
            iss: local.issuer.clone(),
            iat: issued_at,
            exp: issued_at + self.access_token_ttl_secs as usize,
        };

        Ok(encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(local.jwt_secret.as_bytes()),
        )?)
    }

    fn validate_local_token(
        &self,
        local: &LocalTokenConfig,
        token: &str,
    ) -> Result<AuthClaims, AuthRuntimeError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[local.issuer.as_str()]);
        Ok(decode::<AuthClaims>(
            token,
            &DecodingKey::from_secret(local.jwt_secret.as_bytes()),
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
        match &self.token_mode {
            TokenMode::Local(local) => {
                let claims = self.validate_local_token(local, token)?;
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
            TokenMode::Oidc(oidc) => current_oidc_user_from_token(oidc, token),
        }
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
    if let Ok(user_id) = user.user_id.parse::<UserId>() {
        match state.store.get_user(&user_id).await {
            Ok(current_user) => {
                return Json(MeResponse {
                    user_id: current_user.id.to_string(),
                    username: current_user.username,
                    display_name: current_user.display_name,
                    email: current_user.email,
                    role: current_user.role.to_string(),
                    attributes: current_user.attributes,
                    is_active: current_user.is_active,
                })
                .into_response();
            }
            Err(PacsError::NotFound { .. }) => {}
            Err(error) => {
                error!(error = %error, "Failed to load authenticated user profile");
                return internal_error_response();
            }
        }
    }

    Json(MeResponse {
        user_id: user.user_id,
        username: user.username,
        display_name: user
            .attributes
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::to_string),
        email: user
            .attributes
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_string),
        role: user.role,
        attributes: user.attributes,
        is_active: true,
    })
    .into_response()
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

fn current_oidc_user_from_token(
    oidc: &OidcTokenConfig,
    token: &str,
) -> Result<AuthenticatedUser, AuthRuntimeError> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[oidc.issuer.as_str()]);
    validation.set_audience(&[oidc.audience.as_str()]);

    let claims = decode::<Value>(token, &oidc.public_key, &validation)?.claims;
    authenticated_user_from_oidc_claims(&claims, oidc)
}

fn authenticated_user_from_oidc_claims(
    claims: &Value,
    oidc: &OidcTokenConfig,
) -> Result<AuthenticatedUser, AuthRuntimeError> {
    let user_id = extract_string_claim(claims, &oidc.user_id_claim).ok_or_else(|| {
        AuthRuntimeError::InvalidOidcClaims(format!(
            "missing oidc user id claim '{}'",
            oidc.user_id_claim
        ))
    })?;
    let username =
        extract_string_claim(claims, &oidc.username_claim).unwrap_or_else(|| user_id.clone());
    let role = extract_role_from_claims(claims, oidc).unwrap_or(oidc.default_role);
    let attributes = collect_oidc_attributes(claims, &oidc.attributes_claims);

    Ok(AuthenticatedUser::new(
        user_id,
        username,
        role.as_str(),
        attributes,
    ))
}

fn extract_claim_value<'a>(claims: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = claims;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn extract_string_claim(claims: &Value, path: &str) -> Option<String> {
    match extract_claim_value(claims, path)? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn extract_string_list_claim(claims: &Value, path: &str) -> Vec<String> {
    match extract_claim_value(claims, path) {
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(|value| match value {
                Value::String(value) => Some(value.clone()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn extract_role_from_claims(claims: &Value, oidc: &OidcTokenConfig) -> Option<UserRole> {
    extract_string_list_claim(claims, &oidc.role_claim)
        .into_iter()
        .find_map(|candidate| map_external_role(&candidate, &oidc.role_map))
}

fn map_external_role(candidate: &str, role_map: &HashMap<String, UserRole>) -> Option<UserRole> {
    role_map
        .get(candidate)
        .copied()
        .or_else(|| role_map.get(&candidate.to_ascii_lowercase()).copied())
        .or_else(|| candidate.to_ascii_lowercase().parse::<UserRole>().ok())
}

fn collect_oidc_attributes(claims: &Value, attribute_paths: &[String]) -> Value {
    let mut attributes = serde_json::Map::new();
    for path in attribute_paths {
        if let Some(value) = extract_claim_value(claims, path) {
            let key = path.rsplit('.').next().unwrap_or(path);
            attributes.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(attributes)
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
    use jsonwebtoken::{Algorithm, EncodingKey, Header};
    use pacs_core::{
        AuditLogEntry, AuditLogPage, AuditLogQuery, BlobStore, DicomJson, DicomNode, Instance,
        InstanceQuery, MetadataStore, PacsError, PacsResult, PacsStatistics, PasswordPolicy,
        RefreshToken, Series, SeriesQuery, SeriesUid, ServerSettings, SopInstanceUid, Study,
        StudyQuery, StudyUid, User, UserId, UserQuery, UserRole,
    };
    use std::{collections::HashMap, sync::Mutex};
    use tower::ServiceExt;
    use uuid::Uuid;

    const TEST_OIDC_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQC/BQm/ibuqjbK/\nmapMe8BowlU6p9gDISAi4+KJ/VCG+jcE3VRh5F3G/Q/aiPOLMEBUNVy2AxHLX40/\nbJ1cEsz/QSPnzjhtW5jl2TBYTx9SQ519B2CQ8n6PLJNWTcKyC3TfPnKNDUsvcTB7\n3m5iJM+4VlM3xX0BgtkWYFVDtxrGefW6XT2/cGsa9s45YWstMDgFa7H/pF4WSTK/\nePeuPc+QcuN3c8s+KMfDjj0f7HqTrzLgaGWf+5t1uMh//BD70iLkIt7CVUBDqiQO\nUpPQ46dQvmyT9CPKuB0bqrz1Yf7NHB9HP6RjQkVpUDa6g10nmFlssimtWjLgzG/Q\ngx7AcmBjAgMBAAECggEAUkS3tZP6zNI5PVbPpyAXNqcXsOrv2C0wm4Y9H4QHZhKm\nloRCXuTNVLHR3aNlDLnLwti2pLc+tzHgcgPz498/BeJGtgO1frfX6oo3TZlKGpJ/\nZgVC3DpsMnqWvDFCXI8dlzZcfI5Qps6ffIHIVaGYCsK3FYqLM5boqz/zCPZ35CmM\n6Akq1eZj3fmO02d4XMzfcTLmuxUgk5vVJNWchE36gE/Q+5WkkCGHw40VCgvO9Mns\nnv3/13HKVUCzmfSTFwXv6fYhZ8bnwF4LF6N/nCtgwXN2HMY9xcTdYXgLB7d8p+Pn\nA8aYZasTtHREMD8X1h6L9gXu3NPRZQNYXxe2DseqwQKBgQDojkVxaKQXKsysHh8l\nUxE9LUGs1Sqa8yVM/Y5+Pat3TRJ9u0NDBDtRbiAVDaOIvun17m7+6SIiq9Uvq4pZ\nTGQJW9OvYPJmkgYGVUXv9ZJYM9qVaqcr2F+XMunMkAGagMqRNTKVEKgcb+GbUtFD\nml+3Rrp67XvSAHfq+vlEVrfaQQKBgQDSRtEF6LmQhpVlB0eplfWQnRI55eCczAoC\njE+eJDjWKJvQy8wusO6HZdfxBDk8V+J/g4xURErCpfRCcpH8zQd3X8tHixFz/LIV\nf9mLA1H0I7/VLpXMeG2Zl5oqZSUk9viah+O6JnNY0TwjBEwM0NHROFI8ldw1bvSP\nsh/pb+4powKBgQC/gdGb59EhJuSvZIq/gN10ZJ1tx4kzWsG/2hoKyZw3PWfZ1Gk6\nefSjRS30SGwAQz+Ff9k14CR1Ks3/WKMwkGDc+BqllQ9o+h0t//D8/1yJeAIsA00x\nJRjq+UlhZMF9S0wFMiq6aKIX8OZ3s0aTBkCGPB969bB+qlYWUqEM7uCuQQKBgQDD\nbKVehIfRVgMKPdXQOlpa6F/EB2zUzJyQ+a4VHzzjbCJDzuQYkL9efrxOdspq1pLe\nR3fn6QBCHtH/31LmS/agbxsRhqHV1gf8CzI3DALij0b97amyuknB8S+KLy5ySEWL\n+LcgjhOte+gT8y5qyrf1Zg6n1+8sic4orjcSUMBbWQKBgQCv9oQoX5emyTuGhGjE\nAbRXj7TlNJzQ6oVpqFgwbnPEmBeqGzNsYDv54G/CPTngELqjOnDsCUQoi1PRMFRA\nuuPjlgnb9BHOB5nK4px94RevY2cd+XjIU9FBr5j0FLfQiM2gIPUCtxBBEipLO4Ys\n0dIwTvTzoXY1wF/ToBgX0nt3vQ==\n-----END PRIVATE KEY-----\n";
    const TEST_OIDC_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAvwUJv4m7qo2yv5mqTHvA\naMJVOqfYAyEgIuPiif1Qhvo3BN1UYeRdxv0P2ojzizBAVDVctgMRy1+NP2ydXBLM\n/0Ej5844bVuY5dkwWE8fUkOdfQdgkPJ+jyyTVk3Csgt03z5yjQ1LL3Ewe95uYiTP\nuFZTN8V9AYLZFmBVQ7caxnn1ul09v3BrGvbOOWFrLTA4BWux/6ReFkkyv3j3rj3P\nkHLjd3PLPijHw449H+x6k68y4Ghln/ubdbjIf/wQ+9Ii5CLewlVAQ6okDlKT0OOn\nUL5sk/QjyrgdG6q89WH+zRwfRz+kY0JFaVA2uoNdJ5hZbLIprVoy4Mxv0IMewHJg\nYwIDAQAB\n-----END PUBLIC KEY-----\n";

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
                "mode": "local",
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

    fn oidc_plugin_context() -> PluginContext {
        PluginContext {
            config: serde_json::json!({
                "mode": "oidc",
                "oidc": {
                    "issuer": "https://issuer.example.test/realms/pacs",
                    "audience": "pacsnode",
                    "public_key_pem": TEST_OIDC_PUBLIC_KEY_PEM,
                    "role_claim": "realm_access.roles",
                    "role_map": {
                        "pacs_admin": "admin"
                    },
                    "attributes_claims": ["department", "modality_access", "email"]
                }
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
        build_test_router_with_context(plugin_context()).await
    }

    async fn build_test_router_with_context(context: PluginContext) -> Router {
        let mut plugin = BasicAuthPlugin::default();
        plugin.init(&context).await.unwrap();
        plugin
            .apply(
                Router::new()
                    .route("/health", get(|| async { StatusCode::OK }))
                    .route("/api/protected", get(|| async { StatusCode::OK }))
                    .merge(plugin.routes()),
            )
            .with_state(app_state())
    }

    fn sign_oidc_token(claims: Value) -> String {
        encode(
            &Header::new(Algorithm::RS256),
            &claims,
            &EncodingKey::from_rsa_pem(TEST_OIDC_PRIVATE_KEY_PEM.as_bytes()).unwrap(),
        )
        .unwrap()
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
    async fn oidc_bearer_token_grants_access_and_me_returns_claim_profile() {
        let app = build_test_router_with_context(oidc_plugin_context()).await;
        let now = Utc::now().timestamp() as usize;
        let token = sign_oidc_token(serde_json::json!({
            "sub": "oidc-user-1",
            "preferred_username": "alice",
            "realm_access": { "roles": ["pacs_admin"] },
            "department": "radiology",
            "modality_access": ["CT"],
            "email": "alice@example.test",
            "iss": "https://issuer.example.test/realms/pacs",
            "aud": "pacsnode",
            "iat": now,
            "exp": now + 300,
        }));

        let protected_response = app
            .clone()
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

        let me_response = app
            .oneshot(
                Request::builder()
                    .uri("/auth/me")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(me_response.status(), StatusCode::OK);

        let body = to_bytes(me_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["user_id"], serde_json::json!("oidc-user-1"));
        assert_eq!(json["username"], serde_json::json!("alice"));
        assert_eq!(json["role"], serde_json::json!("admin"));
        assert_eq!(json["email"], serde_json::json!("alice@example.test"));
        assert_eq!(
            json["attributes"]["department"],
            serde_json::json!("radiology")
        );
        assert_eq!(
            json["attributes"]["modality_access"][0],
            serde_json::json!("CT")
        );
    }

    #[tokio::test]
    async fn oidc_mode_does_not_mount_local_login_route() {
        let app = build_test_router_with_context(oidc_plugin_context()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
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
