use serde_json::Value;

/// Authenticated request principal inserted by HTTP auth middleware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedUser {
    /// Stable user identifier.
    pub user_id: String,
    /// Login name associated with the authenticated user.
    pub username: String,
    /// Effective role assigned to the user.
    pub role: String,
    /// Authorization attributes associated with the user.
    pub attributes: Value,
}

impl AuthenticatedUser {
    /// Creates a new authenticated user principal.
    ///
    /// # Example
    ///
    /// ```rust
    /// use pacs_plugin::AuthenticatedUser;
    /// use serde_json::json;
    ///
    /// let user = AuthenticatedUser::new("1", "admin", "admin", json!({"department": "radiology"}));
    /// assert_eq!(user.user_id, "1");
    /// assert_eq!(user.username, "admin");
    /// ```
    pub fn new(
        user_id: impl Into<String>,
        username: impl Into<String>,
        role: impl Into<String>,
        attributes: Value,
    ) -> Self {
        Self {
            user_id: user_id.into(),
            username: username.into(),
            role: role.into(),
            attributes,
        }
    }
}
