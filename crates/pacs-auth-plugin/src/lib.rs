//! pacsnode HTTP auth plugin.
//!
//! Provides local username/password login with refresh-token rotation, or
//! external OIDC-style bearer-token validation, plus user identity propagation
//! for secured HTTP routes.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration as StdDuration, Instant, SystemTime, UNIX_EPOCH},
};

use argon2::{
    password_hash::{Error as PasswordHashError, PasswordHash, PasswordVerifier},
    Argon2,
};
use askama::Template;
use async_trait::async_trait;
use axum::{
    body::{to_bytes, Body},
    extract::{Extension, Json, Query, State},
    http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode},
    middleware::{from_fn_with_state, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{
    decode, decode_header, encode, jwk::JwkSet, Algorithm, DecodingKey, EncodingKey, Header,
    Validation,
};
use pacs_core::{MetadataStore, PacsError, RefreshToken, RefreshTokenId, User, UserId, UserRole};
use pacs_plugin::{
    register_plugin, AppState, AuthenticatedUser, MiddlewarePlugin, Plugin, PluginContext,
    PluginError, PluginHealth, PluginManifest, RoutePlugin,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, RwLock};
use tracing::{error, warn};
use url::form_urlencoded;
use uuid::Uuid;

/// Compile-time plugin ID for the built-in HTTP auth plugin.
pub const BASIC_AUTH_PLUGIN_ID: &str = "basic-auth";

const ACCESS_COOKIE_NAME: &str = "pacsnode_access_token";
const REFRESH_COOKIE_NAME: &str = "pacsnode_refresh_token";

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
    #[serde(default)]
    public_key_pem: Option<String>,
    #[serde(default)]
    jwks_uri: Option<String>,
    #[serde(default = "default_jwks_refresh_secs")]
    jwks_refresh_secs: u64,
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
    key_source: OidcKeySource,
    user_id_claim: String,
    username_claim: String,
    role_claim: String,
    role_map: HashMap<String, UserRole>,
    default_role: UserRole,
    attributes_claims: Vec<String>,
}

#[derive(Clone)]
enum OidcKeySource {
    Static(Arc<DecodingKey>),
    Jwks(Arc<JwksKeyStore>),
    Discovery(Arc<DiscoveryKeyStore>),
}

struct JwksKeyStore {
    client: reqwest::Client,
    uri: reqwest::Url,
    refresh_interval: StdDuration,
    cache: RwLock<CachedJwks>,
    refresh_lock: Mutex<()>,
}

#[derive(Default)]
struct CachedJwks {
    fetched_at: Option<Instant>,
    keys_by_kid: HashMap<String, Arc<DecodingKey>>,
}

struct DiscoveryKeyStore {
    client: reqwest::Client,
    issuer: reqwest::Url,
    refresh_interval: StdDuration,
    cache: RwLock<CachedDiscovery>,
    refresh_lock: Mutex<()>,
}

#[derive(Default)]
struct CachedDiscovery {
    fetched_at: Option<Instant>,
    jwks_store: Option<Arc<JwksKeyStore>>,
}

#[derive(Debug, Deserialize)]
struct OidcDiscoveryDocument {
    issuer: Option<String>,
    jwks_uri: String,
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

#[derive(Debug, Deserialize, Default)]
struct LoginPageQuery {
    redirect: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LoginFormRequest {
    username: String,
    password: String,
    redirect: Option<String>,
}

#[derive(Debug)]
enum LoginSubmission {
    Json(LoginRequest),
    Form(LoginFormRequest),
}

#[derive(Debug)]
enum RequestAuthentication {
    Authenticated(AuthenticatedUser),
    Refreshed(AuthenticatedUser, TokenResponse),
}

#[derive(Debug)]
enum AuthFailure {
    Missing(&'static str),
    Invalid(&'static str),
    Runtime(AuthRuntimeError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Template)]
#[template(path = "login.html")]
struct LoginPageTemplate {
    login_path: String,
    redirect_target: Option<String>,
    username: String,
    error_message: Option<String>,
}

#[derive(Debug, thiserror::Error)]
enum AuthRuntimeError {
    #[error("invalid password hash: {0}")]
    InvalidPasswordHash(String),
    #[error("jwt error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("invalid oidc claims: {0}")]
    InvalidOidcClaims(String),
    #[error("jwks fetch failed: {0}")]
    JwksFetch(#[from] reqwest::Error),
    #[error("invalid jwks response: {0}")]
    InvalidJwks(String),
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

fn default_jwks_refresh_secs() -> u64 {
    300
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
                let public_key_pem = oidc
                    .public_key_pem
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                let jwks_uri = oidc
                    .jwks_uri
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                let key_source = match (public_key_pem, jwks_uri) {
                    (Some(public_key_pem), None) => {
                        let public_key = DecodingKey::from_rsa_pem(public_key_pem.as_bytes())
                            .map_err(|error| PluginError::Config {
                                plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
                                message: format!("invalid oidc public_key_pem: {error}"),
                            })?;
                        OidcKeySource::Static(Arc::new(public_key))
                    }
                    (None, Some(jwks_uri)) => {
                        let uri = reqwest::Url::parse(&jwks_uri).map_err(|error| {
                            PluginError::Config {
                                plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
                                message: format!("invalid oidc jwks_uri: {error}"),
                            }
                        })?;
                        OidcKeySource::Jwks(Arc::new(JwksKeyStore::new(
                            reqwest::Client::new(),
                            uri,
                            StdDuration::from_secs(oidc.jwks_refresh_secs),
                        )))
                    }
                    (Some(_), Some(_)) => return Err(PluginError::Config {
                        plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
                        message:
                            "oidc configuration must set exactly one of public_key_pem or jwks_uri"
                                .into(),
                    }),
                    (None, None) => {
                        let issuer = reqwest::Url::parse(&oidc.issuer).map_err(|error| {
                            PluginError::Config {
                                plugin_id: BASIC_AUTH_PLUGIN_ID.into(),
                                message: format!("invalid oidc issuer url: {error}"),
                            }
                        })?;
                        OidcKeySource::Discovery(Arc::new(DiscoveryKeyStore::new(
                            reqwest::Client::new(),
                            issuer,
                            StdDuration::from_secs(oidc.jwks_refresh_secs),
                        )))
                    }
                };

                TokenMode::Oidc(OidcTokenConfig {
                    issuer: oidc.issuer,
                    audience: oidc.audience,
                    key_source,
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

impl JwksKeyStore {
    fn new(client: reqwest::Client, uri: reqwest::Url, refresh_interval: StdDuration) -> Self {
        Self {
            client,
            uri,
            refresh_interval,
            cache: RwLock::new(CachedJwks::default()),
            refresh_lock: Mutex::new(()),
        }
    }

    async fn decoding_key_for_kid(&self, kid: &str) -> Result<Arc<DecodingKey>, AuthRuntimeError> {
        let stale_key = {
            let cache = self.cache.read().await;
            if let Some(key) = cache.keys_by_kid.get(kid) {
                if cache.is_fresh(self.refresh_interval) {
                    return Ok(Arc::clone(key));
                }
                Some(Arc::clone(key))
            } else {
                None
            }
        };

        if let Err(error) = self.refresh_keys(Some(kid)).await {
            if let Some(key) = stale_key {
                warn!(jwks_uri = %self.uri, kid, error = %error, "jwks refresh failed; using stale cached key");
                return Ok(key);
            }
            return Err(error);
        }

        let cache = self.cache.read().await;
        cache
            .keys_by_kid
            .get(kid)
            .cloned()
            .or(stale_key)
            .ok_or_else(|| {
                AuthRuntimeError::InvalidOidcClaims(format!(
                    "no jwks signing key found for kid '{kid}'"
                ))
            })
    }

    async fn refresh_keys(&self, required_kid: Option<&str>) -> Result<(), AuthRuntimeError> {
        if !self.should_refresh(required_kid).await {
            return Ok(());
        }

        let _refresh = self.refresh_lock.lock().await;
        if !self.should_refresh(required_kid).await {
            return Ok(());
        }

        let response = self
            .client
            .get(self.uri.clone())
            .send()
            .await?
            .error_for_status()?;
        let jwks = response.json::<JwkSet>().await?;
        let keys_by_kid = decode_keys_from_jwks(jwks)?;

        let mut cache = self.cache.write().await;
        cache.keys_by_kid = keys_by_kid;
        cache.fetched_at = Some(Instant::now());
        Ok(())
    }

    async fn should_refresh(&self, required_kid: Option<&str>) -> bool {
        let cache = self.cache.read().await;
        if !cache.is_fresh(self.refresh_interval) {
            return true;
        }

        required_kid.is_some_and(|kid| !cache.keys_by_kid.contains_key(kid))
    }
}

impl DiscoveryKeyStore {
    fn new(client: reqwest::Client, issuer: reqwest::Url, refresh_interval: StdDuration) -> Self {
        Self {
            client,
            issuer,
            refresh_interval,
            cache: RwLock::new(CachedDiscovery::default()),
            refresh_lock: Mutex::new(()),
        }
    }

    async fn decoding_key_for_kid(&self, kid: &str) -> Result<Arc<DecodingKey>, AuthRuntimeError> {
        let stale_store = {
            let cache = self.cache.read().await;
            if cache.is_fresh(self.refresh_interval) {
                cache.jwks_store.clone()
            } else {
                None
            }
        };

        if let Some(store) = stale_store {
            return store.decoding_key_for_kid(kid).await;
        }

        if let Err(error) = self.refresh_store().await {
            let cached_store = {
                let cache = self.cache.read().await;
                cache.jwks_store.clone()
            };
            if let Some(store) = cached_store {
                warn!(issuer = %self.issuer, kid, error = %error, "oidc discovery refresh failed; using stale discovered jwks uri");
                return store.decoding_key_for_kid(kid).await;
            }
            return Err(error);
        }

        let store = {
            let cache = self.cache.read().await;
            cache.jwks_store.clone()
        }
        .ok_or_else(|| {
            AuthRuntimeError::InvalidJwks("oidc discovery did not yield a jwks uri".into())
        })?;

        store.decoding_key_for_kid(kid).await
    }

    async fn refresh_store(&self) -> Result<(), AuthRuntimeError> {
        if !self.should_refresh().await {
            return Ok(());
        }

        let _refresh = self.refresh_lock.lock().await;
        if !self.should_refresh().await {
            return Ok(());
        }

        let discovery_url = openid_configuration_url(&self.issuer)?;
        let response = self
            .client
            .get(discovery_url)
            .send()
            .await?
            .error_for_status()?;
        let document = response.json::<OidcDiscoveryDocument>().await?;
        if let Some(issuer) = document.issuer.as_deref() {
            if issuer != self.issuer.as_str() {
                return Err(AuthRuntimeError::InvalidJwks(format!(
                    "oidc discovery issuer mismatch: expected '{}', got '{issuer}'",
                    self.issuer
                )));
            }
        }

        let jwks_uri = reqwest::Url::parse(&document.jwks_uri).map_err(|error| {
            AuthRuntimeError::InvalidJwks(format!("invalid discovered jwks_uri: {error}"))
        })?;
        let jwks_store = Arc::new(JwksKeyStore::new(
            self.client.clone(),
            jwks_uri,
            self.refresh_interval,
        ));

        let mut cache = self.cache.write().await;
        cache.jwks_store = Some(jwks_store);
        cache.fetched_at = Some(Instant::now());
        Ok(())
    }

    async fn should_refresh(&self) -> bool {
        let cache = self.cache.read().await;
        !cache.is_fresh(self.refresh_interval)
    }
}

impl CachedJwks {
    fn is_fresh(&self, refresh_interval: StdDuration) -> bool {
        self.fetched_at
            .is_some_and(|fetched_at| fetched_at.elapsed() <= refresh_interval)
    }
}

impl CachedDiscovery {
    fn is_fresh(&self, refresh_interval: StdDuration) -> bool {
        self.fetched_at
            .is_some_and(|fetched_at| fetched_at.elapsed() <= refresh_interval)
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
                    get({
                        let runtime = Arc::clone(&runtime);
                        move |query| login_page_handler(Arc::clone(&runtime), query)
                    })
                    .post({
                        let runtime = Arc::clone(&runtime);
                        move |request| login_handler(Arc::clone(&runtime), request)
                    }),
                )
                .route(
                    &refresh_path,
                    post({
                        let runtime = Arc::clone(&runtime);
                        move |request| refresh_handler(Arc::clone(&runtime), request)
                    }),
                )
                .route(
                    &logout_path,
                    post({
                        let runtime = Arc::clone(&runtime);
                        move |request| logout_handler(Arc::clone(&runtime), request)
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
            TokenMode::Oidc(oidc) => current_oidc_user_from_token(oidc, token).await,
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

async fn login_page_handler(
    runtime: Arc<AuthRuntime>,
    Query(query): Query<LoginPageQuery>,
) -> Response {
    render_login_page(
        runtime.as_ref(),
        sanitize_redirect_target(query.redirect.as_deref()),
        String::new(),
        None,
        StatusCode::OK,
    )
}

async fn login_handler(runtime: Arc<AuthRuntime>, request: Request<Body>) -> Response {
    let secure_cookies = request_uses_secure_transport(request.headers());
    let submission = match parse_login_submission(request).await {
        Ok(submission) => submission,
        Err(response) => return response,
    };

    let (username, password, redirect_target, browser_response) = match submission {
        LoginSubmission::Json(payload) => (payload.username, payload.password, None, false),
        LoginSubmission::Form(payload) => (
            payload.username,
            payload.password,
            sanitize_redirect_target(payload.redirect.as_deref()),
            true,
        ),
    };

    match runtime.authenticate_user(&username, &password).await {
        Ok(Some(user)) => match runtime.issue_token_pair(&user).await {
            Ok(tokens) => {
                let mut response = if browser_response {
                    redirect_response(redirect_target.as_deref().unwrap_or("/"))
                } else {
                    Json(tokens.clone()).into_response()
                };
                append_auth_cookies(
                    response.headers_mut(),
                    runtime.as_ref(),
                    &tokens,
                    secure_cookies,
                );
                response
            }
            Err(error) => {
                error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Failed to issue token pair");
                internal_error_response()
            }
        },
        Ok(None) => {
            if browser_response {
                render_login_page(
                    runtime.as_ref(),
                    redirect_target,
                    username,
                    Some("Invalid username or password.".into()),
                    StatusCode::UNAUTHORIZED,
                )
            } else {
                unauthorized_response("invalid credentials")
            }
        }
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Credential verification failed");
            internal_error_response()
        }
    }
}

async fn refresh_handler(runtime: Arc<AuthRuntime>, request: Request<Body>) -> Response {
    let headers = request.headers().clone();
    let secure_cookies = request_uses_secure_transport(&headers);
    let body = match to_bytes(request.into_body(), usize::MAX).await {
        Ok(body) => body,
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Failed to read refresh request body");
            return internal_error_response();
        }
    };

    let refresh_token = if !body.is_empty() {
        match serde_json::from_slice::<RefreshRequest>(&body) {
            Ok(payload) => payload.refresh_token,
            Err(_) => match cookie_value(&headers, REFRESH_COOKIE_NAME) {
                Some(token) => token,
                None => return unauthorized_response("invalid refresh token"),
            },
        }
    } else {
        match cookie_value(&headers, REFRESH_COOKIE_NAME) {
            Some(token) => token,
            None => return unauthorized_response("invalid refresh token"),
        }
    };

    match runtime.refresh_tokens(&refresh_token).await {
        Ok(Some(tokens)) => {
            let mut response = Json(tokens.clone()).into_response();
            append_auth_cookies(
                response.headers_mut(),
                runtime.as_ref(),
                &tokens,
                secure_cookies,
            );
            response
        }
        Ok(None) => unauthorized_response("invalid refresh token"),
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Failed to refresh token pair");
            internal_error_response()
        }
    }
}

async fn logout_handler(runtime: Arc<AuthRuntime>, request: Request<Body>) -> Response {
    let prefers_html = prefers_html_response(request.headers());
    let secure_cookies = request_uses_secure_transport(request.headers());
    let user = match request.extensions().get::<AuthenticatedUser>().cloned() {
        Some(user) => user,
        None => {
            let mut response = if prefers_html {
                redirect_response(&runtime.login_path)
            } else {
                unauthorized_response("missing bearer token")
            };
            append_clear_auth_cookies(response.headers_mut(), secure_cookies);
            return response;
        }
    };

    let user_id = match user.user_id.parse::<UserId>() {
        Ok(user_id) => user_id,
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Failed to parse authenticated user id");
            return internal_error_response();
        }
    };

    match runtime.store.revoke_refresh_tokens(&user_id).await {
        Ok(()) => {
            let mut response = if prefers_html {
                redirect_response(&runtime.login_path)
            } else {
                StatusCode::NO_CONTENT.into_response()
            };
            append_clear_auth_cookies(response.headers_mut(), secure_cookies);
            response
        }
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
    let secure_cookies = request_uses_secure_transport(request.headers());
    if request.method() == Method::OPTIONS || runtime.is_public_path(request.uri().path()) {
        return next.run(request).await;
    }

    match authenticate_request(runtime.as_ref(), request.headers()).await {
        Ok(RequestAuthentication::Authenticated(user)) => {
            request.extensions_mut().insert(user);
            next.run(request).await
        }
        Ok(RequestAuthentication::Refreshed(user, tokens)) => {
            request.extensions_mut().insert(user);
            let mut response = next.run(request).await;
            append_auth_cookies(
                response.headers_mut(),
                runtime.as_ref(),
                &tokens,
                secure_cookies,
            );
            response
        }
        Err(AuthFailure::Missing(message)) | Err(AuthFailure::Invalid(message)) => {
            login_required_response(runtime.as_ref(), &request, message)
        }
        Err(AuthFailure::Runtime(AuthRuntimeError::Jwt(_))) => login_required_response(
            runtime.as_ref(),
            &request,
            "invalid or expired bearer token",
        ),
        Err(AuthFailure::Runtime(AuthRuntimeError::Store(PacsError::NotFound { .. }))) => {
            login_required_response(runtime.as_ref(), &request, "unknown user")
        }
        Err(AuthFailure::Runtime(AuthRuntimeError::Store(PacsError::InvalidRequest(_)))) => {
            login_required_response(runtime.as_ref(), &request, "inactive or locked user")
        }
        Err(AuthFailure::Runtime(error)) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Token validation failed");
            internal_error_response()
        }
    }
}

async fn authenticate_request(
    runtime: &AuthRuntime,
    headers: &HeaderMap,
) -> Result<RequestAuthentication, AuthFailure> {
    match &runtime.token_mode {
        TokenMode::Local(_) => {
            let access_cookie = cookie_value(headers, ACCESS_COOKIE_NAME);
            if let Some(access_token) = access_cookie.as_deref() {
                match runtime.current_user_from_token(access_token).await {
                    Ok(user) => return Ok(RequestAuthentication::Authenticated(user)),
                    Err(AuthRuntimeError::Jwt(_))
                    | Err(AuthRuntimeError::Store(PacsError::NotFound { .. }))
                    | Err(AuthRuntimeError::Store(PacsError::InvalidRequest(_))) => {}
                    Err(error) => return Err(AuthFailure::Runtime(error)),
                }
            }

            if let Some(refresh_token) = cookie_value(headers, REFRESH_COOKIE_NAME) {
                match runtime.refresh_tokens(&refresh_token).await {
                    Ok(Some(tokens)) => {
                        let user = runtime
                            .current_user_from_token(&tokens.access_token)
                            .await
                            .map_err(AuthFailure::Runtime)?;
                        return Ok(RequestAuthentication::Refreshed(user, tokens));
                    }
                    Ok(None) => {}
                    Err(error) => return Err(AuthFailure::Runtime(error)),
                }
            }

            match bearer_token(headers) {
                Ok(token) => runtime
                    .current_user_from_token(token)
                    .await
                    .map(RequestAuthentication::Authenticated)
                    .map_err(AuthFailure::Runtime),
                Err(message) => {
                    if access_cookie.is_some()
                        || cookie_value(headers, REFRESH_COOKIE_NAME).is_some()
                    {
                        Err(AuthFailure::Invalid("invalid or expired session"))
                    } else {
                        Err(AuthFailure::Missing(message))
                    }
                }
            }
        }
        TokenMode::Oidc(_) => match bearer_token(headers) {
            Ok(token) => runtime
                .current_user_from_token(token)
                .await
                .map(RequestAuthentication::Authenticated)
                .map_err(AuthFailure::Runtime),
            Err(message) => Err(AuthFailure::Missing(message)),
        },
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

async fn current_oidc_user_from_token(
    oidc: &OidcTokenConfig,
    token: &str,
) -> Result<AuthenticatedUser, AuthRuntimeError> {
    let header = decode_header(token)?;
    let algorithm = oidc_signing_algorithm(&header)?;
    let decoding_key = oidc.decoding_key_for_header(&header).await?;

    let mut validation = Validation::new(algorithm);
    validation.set_issuer(&[oidc.issuer.as_str()]);
    validation.set_audience(&[oidc.audience.as_str()]);

    let claims = decode::<Value>(token, decoding_key.as_ref(), &validation)?.claims;
    authenticated_user_from_oidc_claims(&claims, oidc)
}

impl OidcTokenConfig {
    async fn decoding_key_for_header(
        &self,
        header: &Header,
    ) -> Result<Arc<DecodingKey>, AuthRuntimeError> {
        match &self.key_source {
            OidcKeySource::Static(key) => Ok(Arc::clone(key)),
            OidcKeySource::Jwks(store) => {
                let kid = header.kid.as_deref().ok_or_else(|| {
                    AuthRuntimeError::InvalidOidcClaims(
                        "oidc jwks tokens must include a kid header".into(),
                    )
                })?;
                store.decoding_key_for_kid(kid).await
            }
            OidcKeySource::Discovery(store) => {
                let kid = header.kid.as_deref().ok_or_else(|| {
                    AuthRuntimeError::InvalidOidcClaims(
                        "oidc discovery tokens must include a kid header".into(),
                    )
                })?;
                store.decoding_key_for_kid(kid).await
            }
        }
    }
}

fn openid_configuration_url(issuer: &reqwest::Url) -> Result<reqwest::Url, AuthRuntimeError> {
    let mut url = issuer.clone();
    let issuer_path = issuer.path().trim_matches('/');
    let well_known_path = if issuer_path.is_empty() {
        "/.well-known/openid-configuration".to_string()
    } else {
        format!("/.well-known/openid-configuration/{issuer_path}")
    };
    url.set_path(&well_known_path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn oidc_signing_algorithm(header: &Header) -> Result<Algorithm, AuthRuntimeError> {
    match header.alg {
        Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 => Ok(header.alg),
        unsupported => Err(AuthRuntimeError::InvalidOidcClaims(format!(
            "unsupported oidc signing algorithm '{unsupported:?}'"
        ))),
    }
}

fn decode_keys_from_jwks(
    jwks: JwkSet,
) -> Result<HashMap<String, Arc<DecodingKey>>, AuthRuntimeError> {
    let mut keys_by_kid = HashMap::new();
    for jwk in jwks.keys {
        let Some(kid) = jwk.common.key_id.clone() else {
            continue;
        };

        if let Ok(decoding_key) = DecodingKey::from_jwk(&jwk) {
            keys_by_kid.insert(kid, Arc::new(decoding_key));
        }
    }

    if keys_by_kid.is_empty() {
        return Err(AuthRuntimeError::InvalidJwks(
            "jwks response did not contain any usable signing keys".into(),
        ));
    }

    Ok(keys_by_kid)
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

fn append_auth_cookies(
    headers: &mut HeaderMap,
    runtime: &AuthRuntime,
    tokens: &TokenResponse,
    secure: bool,
) {
    append_set_cookie(
        headers,
        build_cookie(
            ACCESS_COOKIE_NAME,
            &tokens.access_token,
            runtime.access_token_ttl_secs,
            secure,
        ),
    );
    append_set_cookie(
        headers,
        build_cookie(
            REFRESH_COOKIE_NAME,
            &tokens.refresh_token,
            runtime.refresh_token_ttl_secs,
            secure,
        ),
    );
}

fn append_clear_auth_cookies(headers: &mut HeaderMap, secure: bool) {
    append_set_cookie(headers, clear_cookie(ACCESS_COOKIE_NAME, secure));
    append_set_cookie(headers, clear_cookie(REFRESH_COOKIE_NAME, secure));
}

fn append_set_cookie(headers: &mut HeaderMap, value: String) {
    match HeaderValue::from_str(&value) {
        Ok(value) => {
            headers.append(header::SET_COOKIE, value);
        }
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Failed to encode Set-Cookie header");
        }
    }
}

fn build_cookie(name: &str, value: &str, max_age_secs: u64, secure: bool) -> String {
    let secure_suffix = if secure { "; Secure" } else { "" };
    format!("{name}={value}; Path=/; Max-Age={max_age_secs}; HttpOnly; SameSite=Lax{secure_suffix}")
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

fn render_login_page(
    runtime: &AuthRuntime,
    redirect_target: Option<String>,
    username: String,
    error_message: Option<String>,
    status: StatusCode,
) -> Response {
    let template = LoginPageTemplate {
        login_path: runtime.login_path.clone(),
        redirect_target,
        username,
        error_message,
    };
    match template.render() {
        Ok(html) => (
            status,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            html,
        )
            .into_response(),
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Failed to render login page");
            internal_error_response()
        }
    }
}

fn login_required_response(
    runtime: &AuthRuntime,
    request: &Request<Body>,
    message: &'static str,
) -> Response {
    let redirect_target = sanitize_redirect_target(
        request
            .uri()
            .path_and_query()
            .map(|value| value.as_str())
            .or_else(|| Some(request.uri().path())),
    )
    .unwrap_or_else(|| "/".into());
    let login_url = login_url_with_redirect(&runtime.login_path, &redirect_target);

    if is_htmx_request(request.headers()) {
        return (
            StatusCode::UNAUTHORIZED,
            [("HX-Redirect", login_url)],
            "authentication required",
        )
            .into_response();
    }

    if request.method() == Method::GET && prefers_html_response(request.headers()) {
        return redirect_response(&login_url);
    }

    unauthorized_response(message)
}

fn redirect_response(location: &str) -> Response {
    (
        StatusCode::SEE_OTHER,
        [(header::LOCATION, location.to_string())],
    )
        .into_response()
}

fn prefers_html_response(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/html") || value.contains("application/xhtml+xml"))
}

fn is_htmx_request(headers: &HeaderMap) -> bool {
    headers
        .get("HX-Request")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn login_url_with_redirect(login_path: &str, redirect_target: &str) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("redirect", redirect_target);
    format!("{login_path}?{}", serializer.finish())
}

fn sanitize_redirect_target(redirect_target: Option<&str>) -> Option<String> {
    let target = redirect_target?.trim();
    if target.is_empty() || !target.starts_with('/') || target.starts_with("//") {
        return None;
    }
    Some(target.to_string())
}

async fn parse_login_submission(request: Request<Body>) -> Result<LoginSubmission, Response> {
    let headers = request.headers().clone();
    let body = match to_bytes(request.into_body(), usize::MAX).await {
        Ok(body) => body,
        Err(error) => {
            error!(plugin_id = BASIC_AUTH_PLUGIN_ID, error = %error, "Failed to read login request body");
            return Err(internal_error_response());
        }
    };

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if content_type.starts_with("application/json") {
        return serde_json::from_slice::<LoginRequest>(&body)
            .map(LoginSubmission::Json)
            .map_err(|_| unauthorized_response("invalid login payload"));
    }

    if content_type.starts_with("application/x-www-form-urlencoded") {
        let fields: HashMap<String, String> =
            form_urlencoded::parse(body.as_ref()).into_owned().collect();
        let username = fields.get("username").cloned().unwrap_or_default();
        let password = fields.get("password").cloned().unwrap_or_default();
        return Ok(LoginSubmission::Form(LoginFormRequest {
            username,
            password,
            redirect: fields.get("redirect").cloned(),
        }));
    }

    Err((
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        Json(serde_json::json!({
            "error": "login requires application/json or application/x-www-form-urlencoded"
        })),
    )
        .into_response())
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
    use axum::{body::to_bytes, body::Body, extract::State, http::Request, routing::get};
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
    use tokio::{net::TcpListener, sync::RwLock as AsyncRwLock, task::JoinHandle};
    use tower::ServiceExt;
    use uuid::Uuid;

    const TEST_OIDC_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQC/BQm/ibuqjbK/\nmapMe8BowlU6p9gDISAi4+KJ/VCG+jcE3VRh5F3G/Q/aiPOLMEBUNVy2AxHLX40/\nbJ1cEsz/QSPnzjhtW5jl2TBYTx9SQ519B2CQ8n6PLJNWTcKyC3TfPnKNDUsvcTB7\n3m5iJM+4VlM3xX0BgtkWYFVDtxrGefW6XT2/cGsa9s45YWstMDgFa7H/pF4WSTK/\nePeuPc+QcuN3c8s+KMfDjj0f7HqTrzLgaGWf+5t1uMh//BD70iLkIt7CVUBDqiQO\nUpPQ46dQvmyT9CPKuB0bqrz1Yf7NHB9HP6RjQkVpUDa6g10nmFlssimtWjLgzG/Q\ngx7AcmBjAgMBAAECggEAUkS3tZP6zNI5PVbPpyAXNqcXsOrv2C0wm4Y9H4QHZhKm\nloRCXuTNVLHR3aNlDLnLwti2pLc+tzHgcgPz498/BeJGtgO1frfX6oo3TZlKGpJ/\nZgVC3DpsMnqWvDFCXI8dlzZcfI5Qps6ffIHIVaGYCsK3FYqLM5boqz/zCPZ35CmM\n6Akq1eZj3fmO02d4XMzfcTLmuxUgk5vVJNWchE36gE/Q+5WkkCGHw40VCgvO9Mns\nnv3/13HKVUCzmfSTFwXv6fYhZ8bnwF4LF6N/nCtgwXN2HMY9xcTdYXgLB7d8p+Pn\nA8aYZasTtHREMD8X1h6L9gXu3NPRZQNYXxe2DseqwQKBgQDojkVxaKQXKsysHh8l\nUxE9LUGs1Sqa8yVM/Y5+Pat3TRJ9u0NDBDtRbiAVDaOIvun17m7+6SIiq9Uvq4pZ\nTGQJW9OvYPJmkgYGVUXv9ZJYM9qVaqcr2F+XMunMkAGagMqRNTKVEKgcb+GbUtFD\nml+3Rrp67XvSAHfq+vlEVrfaQQKBgQDSRtEF6LmQhpVlB0eplfWQnRI55eCczAoC\njE+eJDjWKJvQy8wusO6HZdfxBDk8V+J/g4xURErCpfRCcpH8zQd3X8tHixFz/LIV\nf9mLA1H0I7/VLpXMeG2Zl5oqZSUk9viah+O6JnNY0TwjBEwM0NHROFI8ldw1bvSP\nsh/pb+4powKBgQC/gdGb59EhJuSvZIq/gN10ZJ1tx4kzWsG/2hoKyZw3PWfZ1Gk6\nefSjRS30SGwAQz+Ff9k14CR1Ks3/WKMwkGDc+BqllQ9o+h0t//D8/1yJeAIsA00x\nJRjq+UlhZMF9S0wFMiq6aKIX8OZ3s0aTBkCGPB969bB+qlYWUqEM7uCuQQKBgQDD\nbKVehIfRVgMKPdXQOlpa6F/EB2zUzJyQ+a4VHzzjbCJDzuQYkL9efrxOdspq1pLe\nR3fn6QBCHtH/31LmS/agbxsRhqHV1gf8CzI3DALij0b97amyuknB8S+KLy5ySEWL\n+LcgjhOte+gT8y5qyrf1Zg6n1+8sic4orjcSUMBbWQKBgQCv9oQoX5emyTuGhGjE\nAbRXj7TlNJzQ6oVpqFgwbnPEmBeqGzNsYDv54G/CPTngELqjOnDsCUQoi1PRMFRA\nuuPjlgnb9BHOB5nK4px94RevY2cd+XjIU9FBr5j0FLfQiM2gIPUCtxBBEipLO4Ys\n0dIwTvTzoXY1wF/ToBgX0nt3vQ==\n-----END PRIVATE KEY-----\n";
    const TEST_OIDC_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAvwUJv4m7qo2yv5mqTHvA\naMJVOqfYAyEgIuPiif1Qhvo3BN1UYeRdxv0P2ojzizBAVDVctgMRy1+NP2ydXBLM\n/0Ej5844bVuY5dkwWE8fUkOdfQdgkPJ+jyyTVk3Csgt03z5yjQ1LL3Ewe95uYiTP\nuFZTN8V9AYLZFmBVQ7caxnn1ul09v3BrGvbOOWFrLTA4BWux/6ReFkkyv3j3rj3P\nkHLjd3PLPijHw449H+x6k68y4Ghln/ubdbjIf/wQ+9Ii5CLewlVAQ6okDlKT0OOn\nUL5sk/QjyrgdG6q89WH+zRwfRz+kY0JFaVA2uoNdJ5hZbLIprVoy4Mxv0IMewHJg\nYwIDAQAB\n-----END PUBLIC KEY-----\n";
    const TEST_OIDC_JWK_N: &str = "vwUJv4m7qo2yv5mqTHvAaMJVOqfYAyEgIuPiif1Qhvo3BN1UYeRdxv0P2ojzizBAVDVctgMRy1-NP2ydXBLM_0Ej5844bVuY5dkwWE8fUkOdfQdgkPJ-jyyTVk3Csgt03z5yjQ1LL3Ewe95uYiTPuFZTN8V9AYLZFmBVQ7caxnn1ul09v3BrGvbOOWFrLTA4BWux_6ReFkkyv3j3rj3PkHLjd3PLPijHw449H-x6k68y4Ghln_ubdbjIf_wQ-9Ii5CLewlVAQ6okDlKT0OOnUL5sk_QjyrgdG6q89WH-zRwfRz-kY0JFaVA2uoNdJ5hZbLIprVoy4Mxv0IMewHJgYw";
    const TEST_OIDC_ROTATED_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQDQa+0vZ9khsk8t\nBQzH5ZdLvoKV7zQ9TEyxFpVFj70Gj+2w7DCD4I6fH5GiZpwZZ4TjnqtUbhH2aMpz\nVGQDKAvFv12D/u6MjMjyFPubOp6L/3KayWNCzP71ikhQTZXwc52HNrp1Xx7BRWa0\nl2PEIkArZ6b+pgubB69+29kfuz9xP6SbmGxGWt2XbYIFpbuTfSso07vcgcppl7nO\n/EofiJ4nLeHTK0Z3t4GfSNZq9viY187v5WZ+kwLpky5Xf96IyOBgdkqhyJ97MiUX\nvtRBX9PVMh47rj0w5Smiuuj+tYjP0VngjKwPAXm7dYubO+gcKvknM2jiBvyhhiVS\noSpHai1PAgMBAAECggEAPY36fXs6tgh++Mlahnkoz26DC7wbXhU4Oz7ztBkpFxSP\n+yYuh+xcwuMkdGXAqYIYzc7xQ9zEQlWdoSUl6oa7v1nuyQqUMn9r449N5gEQjUFS\n/CMJRVPc4vDFva3EYEENH7+KnxqKL0OLez+Q7/67m/YfbGrm15EUBC/y9rurF4tP\nXwVI+aWxfD8OOywx5x+trByCPQzXA+g8J/rG7L41uf6d4q3PJdsvMdmQrImxCcUK\n3+TGRukiwGioO5avv5Wuyg7eKZwcX8O0ZNR8w1OqsIINUlznxRBMu3Qj9TuhIhd8\nUebimf+wJyoLHrJe3yDmqXPj+2mT+HP5jEe6CCwAWQKBgQDrlSKJcv3YjPVKCvHs\n9JNsNXcV+DxMJdR+8X09AqSXEY/xwikSJdHdbtx4xQ0jQcDNxcdNYAnQlu645nbW\n69YEKsyezrhj6yPqKcHHNg+6m7VIAxseD4btqcg8TAZRrFV7tNxZD9v3jeDPBCw0\nPwIsLDNjSF7aeI/pYB2GL7FVBwKBgQDifCv2+7iOe143WsqzYIzmi8xaHH68RBOk\n/VHHdp2XhMP4qNl/EoGJv98TksUuuTUcs8ztNK8zMETI+Z3prAVWwQFnhJoVRkP3\nWY8RL7txOQ36F3QIGOi1dWAQLIy0DqoKr1pfpHhqP0Zrpipx8cg0sGxilviXDYm7\nn4rkNUTbeQKBgAas3iKo6Hp3XAfyEXLWZ0r8pNgxhXve4ouKSjMtXP6O19ZQ2xsR\niUXN+19MrheeqFjsTr5phz2q2S7SEPH8Er9hexTQ5LaoFgdvkXcUmBOAj/1vYRhT\n9k3LrsnOmas8x9tOf6PiaCg2k/UpuBru4h/gTMB2b4GfQuyo9Y000sCHAoGAW9zt\noDIde31Ci8VBrlwdCm3tpycjqI0cQrGU+Ah+hzSMoFEsVsRU0mCGxNOlMvxgNJIh\nLp1N6r9LRxEoId1qFPQX87rvHG3xp2QmCVyI9LWlm6jjoV0pFmDTY/wN3gKMqeTS\nDTUSulWL5KHzWWAuSmC8tYhysCIHmZhup32LvlECgYBK4VqZS7rabPHrMFlDYmkX\nIZKXwGrPQr7UGDVzhMC+TucBOCaEfxxPZNyf2NarHh3gmSDMtpqTauqsJTnwlGBO\nBm2bb1z8P17aXmv/3Am734YFIDEWJ58VzHwLc4iEvulC714VoDhruDrroFrQKJS3\nIax6DFsseM2xq34z1v3uUA==\n-----END PRIVATE KEY-----\n";
    const TEST_OIDC_ROTATED_JWK_N: &str = "0GvtL2fZIbJPLQUMx-WXS76Cle80PUxMsRaVRY-9Bo_tsOwwg-COnx-RomacGWeE456rVG4R9mjKc1RkAygLxb9dg_7ujIzI8hT7mzqei_9ymsljQsz-9YpIUE2V8HOdhza6dV8ewUVmtJdjxCJAK2em_qYLmwevftvZH7s_cT-km5hsRlrdl22CBaW7k30rKNO73IHKaZe5zvxKH4ieJy3h0ytGd7eBn0jWavb4mNfO7-VmfpMC6ZMuV3_eiMjgYHZKocifezIlF77UQV_T1TIeO649MOUporro_rWIz9FZ4IysDwF5u3WLmzvoHCr5JzNo4gb8oYYlUqEqR2otTw";

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

    fn jwks_plugin_context(jwks_uri: &str) -> PluginContext {
        PluginContext {
            config: serde_json::json!({
                "mode": "oidc",
                "oidc": {
                    "issuer": "https://issuer.example.test/realms/pacs",
                    "audience": "pacsnode",
                    "jwks_uri": jwks_uri,
                    "jwks_refresh_secs": 3600,
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

    fn discovery_plugin_context(issuer: &str) -> PluginContext {
        PluginContext {
            config: serde_json::json!({
                "mode": "oidc",
                "oidc": {
                    "issuer": issuer,
                    "audience": "pacsnode",
                    "jwks_refresh_secs": 3600,
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

    fn response_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
        headers
            .get_all(header::SET_COOKIE)
            .iter()
            .find_map(|value| {
                let raw = value.to_str().ok()?;
                let first_segment = raw.split(';').next()?;
                let (cookie_name, cookie_value) = first_segment.split_once('=')?;
                (cookie_name == name).then(|| cookie_value.to_string())
            })
    }

    fn response_set_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
        headers
            .get_all(header::SET_COOKIE)
            .iter()
            .find_map(|value| {
                let raw = value.to_str().ok()?;
                raw.starts_with(&format!("{name}="))
                    .then(|| raw.to_string())
            })
    }

    fn sign_oidc_token_with_key(private_key_pem: &str, kid: Option<&str>, claims: Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = kid.map(str::to_string);
        encode(
            &header,
            &claims,
            &EncodingKey::from_rsa_pem(private_key_pem.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn sign_oidc_token(claims: Value) -> String {
        sign_oidc_token_with_key(TEST_OIDC_PRIVATE_KEY_PEM, None, claims)
    }

    fn sign_oidc_token_with_kid(claims: Value, kid: &str) -> String {
        sign_oidc_token_with_key(TEST_OIDC_PRIVATE_KEY_PEM, Some(kid), claims)
    }

    fn sign_rotated_oidc_token_with_kid(claims: Value, kid: &str) -> String {
        sign_oidc_token_with_key(TEST_OIDC_ROTATED_PRIVATE_KEY_PEM, Some(kid), claims)
    }

    fn oidc_jwk(kid: &str, modulus: &str) -> Value {
        serde_json::json!({
            "kty": "RSA",
            "kid": kid,
            "use": "sig",
            "alg": "RS256",
            "n": modulus,
            "e": "AQAB"
        })
    }

    async fn jwks_handler(State(jwks): State<Arc<AsyncRwLock<Value>>>) -> Json<Value> {
        Json(jwks.read().await.clone())
    }

    async fn spawn_jwks_server(
        initial_jwks: Value,
    ) -> (String, Arc<AsyncRwLock<Value>>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(AsyncRwLock::new(initial_jwks));
        let app = Router::new()
            .route("/jwks", get(jwks_handler))
            .with_state(Arc::clone(&state));
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (format!("http://{address}/jwks"), state, handle)
    }

    async fn spawn_oidc_discovery_server(
        issuer_path: &str,
        initial_jwks: Value,
    ) -> (String, Arc<AsyncRwLock<Value>>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let issuer_path = issuer_path.trim_matches('/').to_string();
        let jwks_path = format!("/{issuer_path}/certs");
        let config_path = format!("/.well-known/openid-configuration/{issuer_path}");
        let issuer = format!("http://{address}/{issuer_path}");
        let jwks_uri = format!("http://{address}{jwks_path}");
        let state = Arc::new(AsyncRwLock::new(initial_jwks));
        let app = Router::new()
            .route(&jwks_path, get(jwks_handler))
            .route(
                &config_path,
                get({
                    let issuer = issuer.clone();
                    let jwks_uri = jwks_uri.clone();
                    move || {
                        let issuer = issuer.clone();
                        let jwks_uri = jwks_uri.clone();
                        async move {
                            Json(serde_json::json!({
                                "issuer": issuer,
                                "jwks_uri": jwks_uri,
                            }))
                        }
                    }
                }),
            )
            .with_state(Arc::clone(&state));
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (issuer, state, handle)
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
    async fn login_page_renders_html_form() {
        let app = build_test_router().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/auth/login?redirect=%2Fviewer%2F")
                    .header(header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<form method=\"post\" action=\"/auth/login\">"));
        assert!(html.contains("name=\"redirect\" value=\"/viewer/\""));
    }

    #[tokio::test]
    async fn form_login_sets_cookies_and_redirects() {
        let app = build_test_router().await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "username=admin&password=secret&redirect=%2Fviewer%2F",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::LOCATION], "/viewer/");
        assert!(response_cookie(response.headers(), ACCESS_COOKIE_NAME).is_some());
        assert!(response_cookie(response.headers(), REFRESH_COOKIE_NAME).is_some());
    }

    #[tokio::test]
    async fn html_navigation_redirects_to_login() {
        let app = build_test_router().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/protected")
                    .header(header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers()[header::LOCATION],
            "/auth/login?redirect=%2Fapi%2Fprotected"
        );
    }

    #[tokio::test]
    async fn cookie_session_grants_access() {
        let app = build_test_router().await;
        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("username=admin&password=secret"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let access_cookie = response_cookie(login_response.headers(), ACCESS_COOKIE_NAME).unwrap();
        let refresh_cookie =
            response_cookie(login_response.headers(), REFRESH_COOKIE_NAME).unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/protected")
                    .header(
                        header::COOKIE,
                        format!(
                            "{ACCESS_COOKIE_NAME}={access_cookie}; {REFRESH_COOKIE_NAME}={refresh_cookie}"
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn refresh_cookie_renews_browser_session() {
        let app = build_test_router().await;
        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "username": "admin",
                            "password": "secret"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let refresh_cookie =
            response_cookie(login_response.headers(), REFRESH_COOKIE_NAME).unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/protected")
                    .header(
                        header::COOKIE,
                        format!("{REFRESH_COOKIE_NAME}={refresh_cookie}"),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response_cookie(response.headers(), ACCESS_COOKIE_NAME).is_some());
    }

    #[tokio::test]
    async fn forwarded_https_marks_and_clears_secure_cookies() {
        let app = build_test_router().await;
        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .header("X-Forwarded-Proto", "https")
                    .body(Body::from(
                        "username=admin&password=secret&redirect=%2Fadmin",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        let access_cookie_header =
            response_set_cookie(login_response.headers(), ACCESS_COOKIE_NAME).unwrap();
        let refresh_cookie_header =
            response_set_cookie(login_response.headers(), REFRESH_COOKIE_NAME).unwrap();
        assert!(access_cookie_header.contains("; Secure"));
        assert!(refresh_cookie_header.contains("; Secure"));

        let access_cookie = response_cookie(login_response.headers(), ACCESS_COOKIE_NAME).unwrap();
        let refresh_cookie =
            response_cookie(login_response.headers(), REFRESH_COOKIE_NAME).unwrap();

        let logout_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/logout")
                    .header(header::ACCEPT, "text/html")
                    .header("X-Forwarded-Proto", "https")
                    .header(
                        header::COOKIE,
                        format!(
                            "{ACCESS_COOKIE_NAME}={access_cookie}; {REFRESH_COOKIE_NAME}={refresh_cookie}"
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let cleared_access =
            response_set_cookie(logout_response.headers(), ACCESS_COOKIE_NAME).unwrap();
        let cleared_refresh =
            response_set_cookie(logout_response.headers(), REFRESH_COOKIE_NAME).unwrap();
        assert!(cleared_access.contains("Max-Age=0"));
        assert!(cleared_access.contains("; Secure"));
        assert!(cleared_refresh.contains("Max-Age=0"));
        assert!(cleared_refresh.contains("; Secure"));
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
    async fn oidc_jwks_mode_fetches_key_and_grants_access() {
        let now = Utc::now().timestamp() as usize;
        let jwks = serde_json::json!({
            "keys": [oidc_jwk("kid-1", TEST_OIDC_JWK_N)]
        });
        let (jwks_uri, _state, server) = spawn_jwks_server(jwks).await;
        let app = build_test_router_with_context(jwks_plugin_context(&jwks_uri)).await;
        let token = sign_oidc_token_with_kid(
            serde_json::json!({
                "sub": "oidc-user-1",
                "preferred_username": "alice",
                "realm_access": { "roles": ["pacs_admin"] },
                "department": "radiology",
                "email": "alice@example.test",
                "iss": "https://issuer.example.test/realms/pacs",
                "aud": "pacsnode",
                "iat": now,
                "exp": now + 300,
            }),
            "kid-1",
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/protected")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        server.abort();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn oidc_jwks_mode_refreshes_when_kid_rotates() {
        let initial_jwks = serde_json::json!({
            "keys": [oidc_jwk("kid-1", TEST_OIDC_JWK_N)]
        });
        let (jwks_uri, state, server) = spawn_jwks_server(initial_jwks).await;
        let app = build_test_router_with_context(jwks_plugin_context(&jwks_uri)).await;
        let now = Utc::now().timestamp() as usize;

        let first_token = sign_oidc_token_with_kid(
            serde_json::json!({
                "sub": "oidc-user-1",
                "preferred_username": "alice",
                "realm_access": { "roles": ["pacs_admin"] },
                "iss": "https://issuer.example.test/realms/pacs",
                "aud": "pacsnode",
                "iat": now,
                "exp": now + 300,
            }),
            "kid-1",
        );

        let first_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/protected")
                    .header(header::AUTHORIZATION, format!("Bearer {first_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first_response.status(), StatusCode::OK);

        *state.write().await = serde_json::json!({
            "keys": [oidc_jwk("kid-2", TEST_OIDC_ROTATED_JWK_N)]
        });

        let second_token = sign_rotated_oidc_token_with_kid(
            serde_json::json!({
                "sub": "oidc-user-2",
                "preferred_username": "bob",
                "realm_access": { "roles": ["pacs_admin"] },
                "iss": "https://issuer.example.test/realms/pacs",
                "aud": "pacsnode",
                "iat": now,
                "exp": now + 300,
            }),
            "kid-2",
        );

        let second_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/protected")
                    .header(header::AUTHORIZATION, format!("Bearer {second_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        server.abort();
        assert_eq!(second_response.status(), StatusCode::OK);
    }

    #[test]
    fn openid_configuration_url_handles_path_issuers() {
        let issuer = reqwest::Url::parse("https://issuer.example.test/realms/pacs").unwrap();
        let discovery_url = openid_configuration_url(&issuer).unwrap();
        assert_eq!(
            discovery_url.as_str(),
            "https://issuer.example.test/.well-known/openid-configuration/realms/pacs"
        );
    }

    #[tokio::test]
    async fn oidc_discovery_mode_resolves_jwks_from_issuer() {
        let now = Utc::now().timestamp() as usize;
        let initial_jwks = serde_json::json!({
            "keys": [oidc_jwk("kid-1", TEST_OIDC_JWK_N)]
        });
        let (issuer, _state, server) =
            spawn_oidc_discovery_server("realms/pacs", initial_jwks).await;
        let app = build_test_router_with_context(discovery_plugin_context(&issuer)).await;
        let token = sign_oidc_token_with_kid(
            serde_json::json!({
                "sub": "oidc-user-1",
                "preferred_username": "alice",
                "realm_access": { "roles": ["pacs_admin"] },
                "iss": issuer,
                "aud": "pacsnode",
                "iat": now,
                "exp": now + 300,
            }),
            "kid-1",
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/protected")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        server.abort();
        assert_eq!(response.status(), StatusCode::OK);
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
