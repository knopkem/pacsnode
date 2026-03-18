use async_trait::async_trait;

use crate::domain::{
    AuditLogEntry, AuditLogPage, AuditLogQuery, DicomJson, DicomNode, Instance, InstanceQuery,
    NewAuditLogEntry, PacsStatistics, PasswordPolicy, RefreshToken, Series, SeriesQuery, SeriesUid,
    ServerSettings, SopInstanceUid, Study, StudyQuery, StudyUid, User, UserId, UserQuery,
};
use crate::error::PacsResult;

/// Persistent storage interface for DICOM metadata.
///
/// Implementors provide WADO-RS retrieve semantics and QIDO-RS query semantics
/// over the study → series → instance hierarchy.
///
/// The trait is object-safe and requires both `Send` and `Sync` so that
/// implementations can be shared across async task boundaries.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MetadataStore: Send + Sync {
    /// Stores (inserts or upserts) a [`Study`] in the metadata store.
    async fn store_study(&self, study: &Study) -> PacsResult<()>;

    /// Stores (inserts or upserts) a [`Series`] in the metadata store.
    async fn store_series(&self, series: &Series) -> PacsResult<()>;

    /// Stores (inserts or upserts) an [`Instance`] in the metadata store.
    async fn store_instance(&self, instance: &Instance) -> PacsResult<()>;

    /// Returns all studies matching the given query parameters.
    async fn query_studies(&self, q: &StudyQuery) -> PacsResult<Vec<Study>>;

    /// Returns all series matching the given query parameters.
    async fn query_series(&self, q: &SeriesQuery) -> PacsResult<Vec<Series>>;

    /// Returns all instances matching the given query parameters.
    async fn query_instances(&self, q: &InstanceQuery) -> PacsResult<Vec<Instance>>;

    /// Retrieves a single study by its UID.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PacsError::NotFound`] if no study with `uid` exists.
    async fn get_study(&self, uid: &StudyUid) -> PacsResult<Study>;

    /// Retrieves a single series by its UID.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PacsError::NotFound`] if no series with `uid` exists.
    async fn get_series(&self, uid: &SeriesUid) -> PacsResult<Series>;

    /// Retrieves a single instance by its UID.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PacsError::NotFound`] if no instance with `uid` exists.
    async fn get_instance(&self, uid: &SopInstanceUid) -> PacsResult<Instance>;

    /// Retrieves only the DICOM JSON metadata for an instance (no binary data).
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PacsError::NotFound`] if no instance with `uid` exists.
    async fn get_instance_metadata(&self, uid: &SopInstanceUid) -> PacsResult<DicomJson>;

    /// Deletes a study and all its dependent series and instances.
    async fn delete_study(&self, uid: &StudyUid) -> PacsResult<()>;

    /// Deletes a series and all its dependent instances.
    async fn delete_series(&self, uid: &SeriesUid) -> PacsResult<()>;

    /// Deletes a single instance.
    async fn delete_instance(&self, uid: &SopInstanceUid) -> PacsResult<()>;

    /// Returns aggregate statistics for the PACS system.
    async fn get_statistics(&self) -> PacsResult<PacsStatistics>;

    /// Stores or updates a user record.
    async fn store_user(&self, user: &User) -> PacsResult<()>;

    /// Retrieves a user by identifier.
    async fn get_user(&self, id: &UserId) -> PacsResult<User>;

    /// Retrieves a user by username.
    async fn get_user_by_username(&self, username: &str) -> PacsResult<User>;

    /// Returns users matching the given query parameters.
    async fn query_users(&self, q: &UserQuery) -> PacsResult<Vec<User>>;

    /// Deletes a user record.
    async fn delete_user(&self, id: &UserId) -> PacsResult<()>;

    /// Persists or updates a refresh token record.
    async fn store_refresh_token(&self, token: &RefreshToken) -> PacsResult<()>;

    /// Retrieves a refresh token by its hashed value.
    async fn get_refresh_token(&self, token_hash: &str) -> PacsResult<RefreshToken>;

    /// Revokes all active refresh tokens for a user.
    async fn revoke_refresh_tokens(&self, user_id: &UserId) -> PacsResult<()>;

    /// Returns the active password policy.
    async fn get_password_policy(&self) -> PacsResult<PasswordPolicy>;

    /// Stores or updates the active password policy.
    async fn upsert_password_policy(&self, policy: &PasswordPolicy) -> PacsResult<()>;

    /// Lists all registered remote DICOM nodes (AE whitelist).
    async fn list_nodes(&self) -> PacsResult<Vec<DicomNode>>;

    /// Inserts or updates a remote DICOM node (upsert keyed on AE title).
    async fn upsert_node(&self, node: &DicomNode) -> PacsResult<()>;

    /// Removes a remote DICOM node by AE title.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PacsError::NotFound`] if no node with the given AE title exists.
    async fn delete_node(&self, ae_title: &str) -> PacsResult<()>;

    /// Returns the persisted DIMSE listener settings, if they have been saved.
    async fn get_server_settings(&self) -> PacsResult<Option<ServerSettings>>;

    /// Inserts or updates the persisted DIMSE listener settings.
    async fn upsert_server_settings(&self, settings: &ServerSettings) -> PacsResult<()>;

    /// Searches the append-only audit log using the supplied filters.
    async fn search_audit_logs(&self, q: &AuditLogQuery) -> PacsResult<AuditLogPage>;

    /// Retrieves a single audit log row by its numeric identifier.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PacsError::NotFound`] if no audit row with `id` exists.
    async fn get_audit_log(&self, id: i64) -> PacsResult<AuditLogEntry>;

    /// Persists a new append-only audit log entry.
    async fn store_audit_log(&self, entry: &NewAuditLogEntry) -> PacsResult<()>;
}

#[cfg(test)]
mod tests {
    use super::{MetadataStore, MockMetadataStore};
    use crate::domain::{PacsStatistics, PasswordPolicy, ServerSettings, StudyUid, UserId};
    use crate::error::PacsError;

    #[tokio::test]
    async fn test_mock_get_statistics() {
        let mut mock = MockMetadataStore::new();
        mock.expect_get_statistics().once().returning(|| {
            Ok(PacsStatistics {
                num_studies: 42,
                num_series: 210,
                num_instances: 2100,
                disk_usage_bytes: 10_737_418_240,
            })
        });

        let stats = mock.get_statistics().await.unwrap();
        assert_eq!(stats.num_studies, 42);
        assert_eq!(stats.num_series, 210);
        assert_eq!(stats.num_instances, 2100);
    }

    #[tokio::test]
    async fn test_mock_get_study_not_found() {
        let mut mock = MockMetadataStore::new();
        mock.expect_get_study().once().returning(|uid| {
            Err(PacsError::NotFound {
                resource: "study",
                uid: uid.to_string(),
            })
        });

        let uid = StudyUid::from("1.2.3.4.5");
        let result = mock.get_study(&uid).await;
        assert!(matches!(result, Err(PacsError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_mock_delete_study_ok() {
        let mut mock = MockMetadataStore::new();
        mock.expect_delete_study().once().returning(|_| Ok(()));

        let uid = StudyUid::from("1.2.3");
        let result: crate::error::PacsResult<()> = mock.delete_study(&uid).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_mock_query_studies_empty() {
        let mut mock = MockMetadataStore::new();
        mock.expect_query_studies().once().returning(|_| Ok(vec![]));

        let q = crate::domain::StudyQuery::default();
        let results: Vec<crate::domain::Study> = mock.query_studies(&q).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_mock_list_nodes_empty() {
        let mut mock = MockMetadataStore::new();
        mock.expect_list_nodes().once().returning(|| Ok(vec![]));
        let nodes = mock.list_nodes().await.unwrap();
        assert!(nodes.is_empty());
    }

    #[tokio::test]
    async fn test_mock_upsert_node_ok() {
        let mut mock = MockMetadataStore::new();
        mock.expect_upsert_node().once().returning(|_| Ok(()));
        let node = crate::domain::DicomNode {
            ae_title: "MOD1".into(),
            host: "192.168.1.1".into(),
            port: 104,
            description: None,
            tls_enabled: false,
        };
        assert!(mock.upsert_node(&node).await.is_ok());
    }

    #[tokio::test]
    async fn test_mock_delete_node_not_found() {
        use crate::error::PacsError;
        let mut mock = MockMetadataStore::new();
        mock.expect_delete_node().once().returning(|ae| {
            Err(PacsError::NotFound {
                resource: "node",
                uid: ae.to_string(),
            })
        });
        let result = mock.delete_node("MISSING").await;
        assert!(matches!(result, Err(PacsError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_mock_get_server_settings() {
        let mut mock = MockMetadataStore::new();
        mock.expect_get_server_settings()
            .once()
            .returning(|| Ok(Some(ServerSettings::default())));

        let settings = mock.get_server_settings().await.unwrap();
        assert_eq!(settings, Some(ServerSettings::default()));
    }

    #[tokio::test]
    async fn test_mock_get_password_policy() {
        let mut mock = MockMetadataStore::new();
        mock.expect_get_password_policy()
            .once()
            .returning(|| Ok(PasswordPolicy::default()));

        let policy = mock.get_password_policy().await.unwrap();
        assert_eq!(policy, PasswordPolicy::default());
    }

    #[tokio::test]
    async fn test_mock_delete_user_not_found() {
        let mut mock = MockMetadataStore::new();
        mock.expect_delete_user().once().returning(|id| {
            Err(PacsError::NotFound {
                resource: "user",
                uid: id.to_string(),
            })
        });

        let result = mock.delete_user(&UserId::new()).await;
        assert!(matches!(result, Err(PacsError::NotFound { .. })));
    }
}
