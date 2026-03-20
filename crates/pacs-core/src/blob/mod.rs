use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;

use crate::error::PacsResult;

/// Object storage backend for raw DICOM binary data.
///
/// Keys follow the format `{study_uid}/{series_uid}/{instance_uid}` as
/// produced by [`crate::blob_key_for`].
///
/// The trait is object-safe and requires both `Send` and `Sync` so that
/// implementations can be shared across async task boundaries.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Stores `data` under `key`, overwriting any existing object.
    async fn put(&self, key: &str, data: Bytes) -> PacsResult<()>;

    /// Retrieves the object stored at `key`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PacsError::NotFound`] if no object exists at `key`.
    async fn get(&self, key: &str) -> PacsResult<Bytes>;

    /// Deletes the object stored at `key`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PacsError::NotFound`] if no object exists at `key`.
    async fn delete(&self, key: &str) -> PacsResult<()>;

    /// Returns `true` if an object exists at `key`, `false` otherwise.
    async fn exists(&self, key: &str) -> PacsResult<bool>;

    /// Returns a presigned URL valid for `ttl_secs` seconds.
    async fn presigned_url(&self, key: &str, ttl_secs: u32) -> PacsResult<String>;

    /// Returns the local filesystem root for backends that store blobs on disk.
    ///
    /// Backends without a stable local path (for example S3-compatible object
    /// storage) return `None`.
    fn local_filesystem_root(&self) -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{BlobStore, MockBlobStore};
    use crate::error::PacsError;
    use bytes::Bytes;

    #[tokio::test]
    async fn test_mock_put_and_get() {
        let data = Bytes::from_static(b"DICOM-DATA");
        let data_for_mock = data.clone();

        let mut mock = MockBlobStore::new();
        mock.expect_put().once().returning(|_, _| Ok(()));
        mock.expect_get()
            .once()
            .returning(move |_| Ok(data_for_mock.clone()));

        mock.put("1.2.3/4.5.6/7.8.9", data).await.unwrap();
        let result = mock.get("1.2.3/4.5.6/7.8.9").await.unwrap();
        assert_eq!(result.as_ref(), b"DICOM-DATA");
    }

    #[tokio::test]
    async fn test_mock_exists_false() {
        let mut mock = MockBlobStore::new();
        mock.expect_exists().once().returning(|_| Ok(false));

        let exists: bool = mock.exists("nonexistent/key").await.unwrap();
        assert!(!exists);
    }

    #[tokio::test]
    async fn test_mock_exists_true() {
        let mut mock = MockBlobStore::new();
        mock.expect_exists().once().returning(|_| Ok(true));

        let exists: bool = mock.exists("1.2.3/4.5.6/7.8.9").await.unwrap();
        assert!(exists);
    }

    #[tokio::test]
    async fn test_mock_presigned_url() {
        let mut mock = MockBlobStore::new();
        mock.expect_presigned_url()
            .once()
            .returning(|key, _ttl| Ok(format!("https://example.com/{key}")));

        let url = mock.presigned_url("1.2.3/4.5.6/7.8.9", 3600).await.unwrap();
        assert!(url.contains("1.2.3/4.5.6/7.8.9"));
        assert!(url.starts_with("https://"));
    }

    #[tokio::test]
    async fn test_mock_delete_not_found() {
        let mut mock = MockBlobStore::new();
        mock.expect_delete().once().returning(|key| {
            Err(PacsError::NotFound {
                resource: "blob",
                uid: key.to_string(),
            })
        });

        let result: crate::error::PacsResult<()> = mock.delete("missing/key").await;
        assert!(matches!(result, Err(PacsError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_mock_delete_ok() {
        let mut mock = MockBlobStore::new();
        mock.expect_delete().once().returning(|_| Ok(()));

        let result = mock.delete("1.2.3/4.5.6/7.8.9").await;
        assert!(result.is_ok());
    }
}
