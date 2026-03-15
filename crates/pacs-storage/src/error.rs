//! Error type returned when the S3 storage backend cannot be initialised.

use thiserror::Error;

/// Error returned by [`S3BlobStore::new`][crate::s3::S3BlobStore::new] when
/// the underlying `object_store` client cannot be constructed from the
/// supplied [`StorageConfig`][crate::config::StorageConfig].
#[derive(Debug, Error)]
pub enum StorageError {
    /// The `object_store` builder rejected the configuration (e.g., invalid
    /// endpoint URL or missing required fields).
    #[error("failed to initialise S3 store: {0}")]
    Build(#[from] object_store::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_error_display_mentions_s3_store() {
        let inner = object_store::Error::NotImplemented;
        let err = StorageError::Build(inner);
        let msg = err.to_string();
        assert!(
            msg.contains("S3 store"),
            "display should mention 'S3 store': {msg}"
        );
    }

    #[test]
    fn build_error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<StorageError>();
    }

    #[test]
    fn build_error_from_object_store_error() {
        let ose = object_store::Error::NotImplemented;
        let err = StorageError::from(ose);
        assert!(matches!(err, StorageError::Build(_)));
    }
}
