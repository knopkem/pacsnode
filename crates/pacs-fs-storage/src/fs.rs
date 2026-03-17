//! [`FsBlobStore`]: local filesystem implementation of [`pacs_core::BlobStore`].

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use bytes::Bytes;
use pacs_core::{BlobStore, PacsError, PacsResult};
use tokio::fs;
use tracing::{debug, trace};
use url::Url;

use crate::{config::FilesystemStorageConfig, error::FsStorageError};

/// Filesystem-backed blob store for standalone pacsnode deployments.
#[derive(Debug, Clone)]
pub struct FsBlobStore {
    root: PathBuf,
}

impl FsBlobStore {
    /// Creates a new filesystem-backed blob store.
    pub fn new(config: &FilesystemStorageConfig) -> Result<Self, FsStorageError> {
        let root = config.root.trim();
        if root.is_empty() {
            return Err(FsStorageError::InvalidRoot(
                "filesystem blob store root must not be empty".into(),
            ));
        }

        let path = PathBuf::from(root);
        std::fs::create_dir_all(&path)?;
        let canonical_root = path.canonicalize().unwrap_or(path);

        Ok(Self {
            root: canonical_root,
        })
    }

    fn path_for_key(&self, key: &str) -> PacsResult<PathBuf> {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return Err(PacsError::InvalidRequest(
                "blob key must not be empty".into(),
            ));
        }

        let mut path = self.root.clone();
        for segment in trimmed.split('/') {
            if segment.is_empty() || segment == "." || segment == ".." || segment.contains('\\') {
                return Err(PacsError::InvalidRequest(format!(
                    "blob key contains an unsafe path segment: {segment}"
                )));
            }
            path.push(segment);
        }

        Ok(path)
    }

    fn map_io_error(error: std::io::Error, key: &str) -> PacsError {
        if error.kind() == std::io::ErrorKind::NotFound {
            PacsError::NotFound {
                resource: "blob",
                uid: key.to_string(),
            }
        } else {
            PacsError::Blob(Box::new(error))
        }
    }

    fn file_url(path: &Path) -> PacsResult<String> {
        Url::from_file_path(path)
            .map(|url| url.to_string())
            .map_err(|_| {
                PacsError::Blob(Box::new(FsStorageError::InvalidRoot(
                    path.display().to_string(),
                )))
            })
    }
}

#[async_trait]
impl BlobStore for FsBlobStore {
    async fn put(&self, key: &str, data: Bytes) -> PacsResult<()> {
        let path = self.path_for_key(key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|error| Self::map_io_error(error, key))?;
        }

        trace!(key, root = %self.root.display(), "put filesystem blob");
        fs::write(path, data)
            .await
            .map_err(|error| Self::map_io_error(error, key))
    }

    async fn get(&self, key: &str) -> PacsResult<Bytes> {
        let path = self.path_for_key(key)?;
        trace!(key, root = %self.root.display(), "get filesystem blob");
        fs::read(path)
            .await
            .map(Bytes::from)
            .map_err(|error| Self::map_io_error(error, key))
    }

    async fn delete(&self, key: &str) -> PacsResult<()> {
        let path = self.path_for_key(key)?;
        debug!(key, root = %self.root.display(), "delete filesystem blob");
        match fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Self::map_io_error(error, key)),
        }
    }

    async fn exists(&self, key: &str) -> PacsResult<bool> {
        let path = self.path_for_key(key)?;
        trace!(key, root = %self.root.display(), "exists filesystem blob");
        match fs::metadata(path).await {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(Self::map_io_error(error, key)),
        }
    }

    async fn presigned_url(&self, key: &str, _ttl_secs: u32) -> PacsResult<String> {
        let path = self.path_for_key(key)?;
        let canonical = path
            .canonicalize()
            .map_err(|error| Self::map_io_error(error, key))?;
        Self::file_url(&canonical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (TempDir, FsBlobStore) {
        let dir = TempDir::new().expect("tempdir");
        let config = FilesystemStorageConfig {
            root: dir.path().to_string_lossy().to_string(),
        };
        let store = FsBlobStore::new(&config).expect("store");
        (dir, store)
    }

    #[tokio::test]
    async fn round_trip_put_get_exists_delete() {
        let (_dir, store) = test_store();
        let key = "1.2.3/4.5.6/7.8.9";
        let payload = Bytes::from_static(b"DICOM");

        store.put(key, payload.clone()).await.expect("put");
        assert!(store.exists(key).await.expect("exists"));
        assert_eq!(store.get(key).await.expect("get"), payload);

        store.delete(key).await.expect("delete");
        assert!(!store.exists(key).await.expect("exists after delete"));
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let (_dir, store) = test_store();
        store.delete("1.2.3/4.5.6/7.8.9").await.expect("delete");
    }

    #[tokio::test]
    async fn missing_get_returns_not_found() {
        let (_dir, store) = test_store();
        let error = store
            .get("1.2.3/4.5.6/7.8.9")
            .await
            .expect_err("missing get should fail");
        assert!(matches!(
            error,
            PacsError::NotFound {
                resource: "blob",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn presigned_url_returns_file_url() {
        let (_dir, store) = test_store();
        let key = "1.2.3/4.5.6/7.8.9";
        store
            .put(key, Bytes::from_static(b"DICOM"))
            .await
            .expect("put");

        let url = store.presigned_url(key, 60).await.expect("url");
        assert!(url.starts_with("file://"), "expected file URL, got {url}");
    }

    #[tokio::test]
    async fn rejects_path_traversal_keys() {
        let (_dir, store) = test_store();
        let error = store
            .put("../escape", Bytes::from_static(b"DICOM"))
            .await
            .expect_err("unsafe key should fail");
        assert!(matches!(error, PacsError::InvalidRequest(_)));
    }
}
