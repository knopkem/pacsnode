/// Authenticated request principal inserted by HTTP auth middleware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedUser {
    /// Stable user identifier.
    pub user_id: String,
}

impl AuthenticatedUser {
    /// Creates a new authenticated user principal.
    ///
    /// # Example
    ///
    /// ```rust
    /// use pacs_plugin::AuthenticatedUser;
    ///
    /// let user = AuthenticatedUser::new("admin");
    /// assert_eq!(user.user_id, "admin");
    /// ```
    pub fn new(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
        }
    }
}
