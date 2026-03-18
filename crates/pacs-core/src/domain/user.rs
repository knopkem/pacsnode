use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable identifier for a pacsnode user.
///
/// # Example
///
/// ```
/// use pacs_core::UserId;
/// use uuid::Uuid;
///
/// let raw = Uuid::nil();
/// let user_id = UserId::from(raw);
/// assert_eq!(user_id.to_string(), raw.to_string());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserId(Uuid);

impl UserId {
    /// Creates a new random user identifier.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Returns the wrapped UUID value.
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl fmt::Debug for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UserId({})", self.0)
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.to_string())
    }
}

impl From<Uuid> for UserId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<UserId> for Uuid {
    fn from(value: UserId) -> Self {
        value.0
    }
}

impl FromStr for UserId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

/// Role assigned to a local or federated user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    /// Full administration rights.
    Admin,
    /// Clinical reader who can query and review studies.
    Radiologist,
    /// Acquisition or ingestion user with upload privileges.
    Technologist,
    /// Read-only user.
    #[default]
    Viewer,
    /// Upload-only automation or ingestion account.
    Uploader,
}

impl UserRole {
    /// Returns the stable storage representation for the role.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Radiologist => "radiologist",
            Self::Technologist => "technologist",
            Self::Viewer => "viewer",
            Self::Uploader => "uploader",
        }
    }
}

impl fmt::Display for UserRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for UserRole {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "admin" => Ok(Self::Admin),
            "radiologist" => Ok(Self::Radiologist),
            "technologist" => Ok(Self::Technologist),
            "viewer" => Ok(Self::Viewer),
            "uploader" => Ok(Self::Uploader),
            _ => Err("invalid user role"),
        }
    }
}

/// A pacsnode application user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    /// Stable identifier.
    pub id: UserId,
    /// Unique login name.
    pub username: String,
    /// Optional human-readable display name.
    pub display_name: Option<String>,
    /// Optional email address.
    pub email: Option<String>,
    /// Argon2 password hash for local authentication.
    pub password_hash: String,
    /// Effective role.
    pub role: UserRole,
    /// Arbitrary authorization attributes.
    pub attributes: serde_json::Value,
    /// Whether the account is active.
    pub is_active: bool,
    /// Failed login attempts since the last successful login.
    pub failed_login_attempts: u32,
    /// Account lock expiration if the account is currently locked.
    pub locked_until: Option<DateTime<Utc>>,
    /// Timestamp of the last password change.
    pub password_changed_at: Option<DateTime<Utc>>,
    /// Creation timestamp.
    pub created_at: Option<DateTime<Utc>>,
    /// Last update timestamp.
    pub updated_at: Option<DateTime<Utc>>,
}

/// Query parameters for listing users in the admin UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserQuery {
    /// Free-text match against username, display name, or email.
    pub search: Option<String>,
    /// Filter by role.
    pub role: Option<UserRole>,
    /// Filter by active state.
    pub is_active: Option<bool>,
    /// Maximum number of results to return.
    pub limit: Option<u32>,
    /// Number of rows to skip.
    pub offset: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_id_round_trips_display_and_parse() {
        let id = UserId::new();
        let parsed = UserId::from_str(&id.to_string()).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn user_role_display_and_parse_round_trip() {
        for role in [
            UserRole::Admin,
            UserRole::Radiologist,
            UserRole::Technologist,
            UserRole::Viewer,
            UserRole::Uploader,
        ] {
            assert_eq!(UserRole::from_str(role.as_str()).unwrap(), role);
        }
    }

    #[test]
    fn user_query_defaults_are_empty() {
        let query = UserQuery::default();
        assert!(query.search.is_none());
        assert!(query.role.is_none());
        assert!(query.is_active.is_none());
        assert!(query.limit.is_none());
        assert!(query.offset.is_none());
    }
}
