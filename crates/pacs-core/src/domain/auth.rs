use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::user::UserId;

/// Stable identifier for a refresh-token record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RefreshTokenId(pub Uuid);

impl RefreshTokenId {
    /// Creates a new random refresh-token identifier.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Supported authentication modes for pacsnode deployments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    /// No HTTP authentication. Intended only for local development or testing.
    #[default]
    None,
    /// Local pacsnode-managed users authenticated with passwords and tokens.
    Local,
    /// External OpenID Connect identity provider.
    Oidc,
}

/// Persisted password-policy settings used for local users.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordPolicy {
    /// Minimum accepted password length.
    pub min_length: u32,
    /// Whether at least one uppercase letter is required.
    pub require_uppercase: bool,
    /// Whether at least one numeric digit is required.
    pub require_digit: bool,
    /// Whether at least one non-alphanumeric character is required.
    pub require_special: bool,
    /// Failed login attempts before an account is locked.
    pub max_failed_attempts: u32,
    /// Lockout duration after the failed-attempt threshold is reached.
    pub lockout_duration_secs: u32,
    /// Maximum password age in days, if enforced.
    pub max_age_days: Option<u32>,
}

impl Default for PasswordPolicy {
    fn default() -> Self {
        Self {
            min_length: 12,
            require_uppercase: true,
            require_digit: true,
            require_special: false,
            max_failed_attempts: 5,
            lockout_duration_secs: 900,
            max_age_days: None,
        }
    }
}

/// A persisted refresh-token record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshToken {
    /// Stable refresh-token record identifier.
    pub id: RefreshTokenId,
    /// User who owns the refresh token.
    pub user_id: UserId,
    /// SHA-256 or equivalent hash of the opaque refresh token value.
    pub token_hash: String,
    /// Expiration timestamp for the token.
    pub expires_at: DateTime<Utc>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Revocation timestamp, if the token has been invalidated.
    pub revoked_at: Option<DateTime<Utc>>,
}

/// A bearer access token paired with a refresh token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenPair {
    /// Signed access token presented on API requests.
    pub access_token: String,
    /// Opaque refresh token used to obtain a new access token.
    pub refresh_token: String,
    /// Access-token lifetime in seconds.
    pub expires_in: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_mode_default_is_none() {
        assert_eq!(AuthMode::default(), AuthMode::None);
    }

    #[test]
    fn password_policy_defaults_are_secure() {
        let policy = PasswordPolicy::default();
        assert_eq!(policy.min_length, 12);
        assert!(policy.require_uppercase);
        assert!(policy.require_digit);
        assert_eq!(policy.max_failed_attempts, 5);
    }

    #[test]
    fn refresh_token_id_is_random_uuid() {
        assert_ne!(RefreshTokenId::new(), RefreshTokenId::new());
    }
}
