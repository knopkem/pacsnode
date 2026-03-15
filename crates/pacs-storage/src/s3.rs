//! [`S3BlobStore`]: S3-compatible implementation of [`pacs_core::BlobStore`].

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use object_store::{
    aws::{AmazonS3, AmazonS3Builder},
    path::Path,
    signer::Signer,
    ObjectStore, PutPayload,
};
use pacs_core::{BlobStore, PacsError, PacsResult};
use tracing::{debug, trace};

use crate::{config::StorageConfig, error::StorageError};

/// S3-compatible blob store backed by [`object_store::aws::AmazonS3`].
///
/// Works with AWS S3, MinIO, RustFS, and any S3-compatible endpoint.
pub struct S3BlobStore {
    store: Arc<AmazonS3>,
    /// Bucket name, kept for structured log fields.
    bucket: String,
}

impl std::fmt::Debug for S3BlobStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3BlobStore")
            .field("bucket", &self.bucket)
            .finish_non_exhaustive()
    }
}

impl S3BlobStore {
    /// Construct an [`S3BlobStore`] from the supplied [`StorageConfig`].
    ///
    /// `allow_http` is set to `true` so that local MinIO / RustFS instances
    /// without TLS work without extra configuration.
    ///
    /// The underlying builder validates the configuration but does **not**
    /// make any network requests; connection errors surface on the first
    /// storage operation.
    pub fn new(config: &StorageConfig) -> Result<Self, StorageError> {
        let store = AmazonS3Builder::new()
            .with_endpoint(&config.endpoint)
            .with_bucket_name(&config.bucket)
            .with_access_key_id(&config.access_key)
            .with_secret_access_key(&config.secret_key)
            .with_region(&config.region)
            .with_allow_http(true)
            .build()?;

        Ok(Self {
            store: Arc::new(store),
            bucket: config.bucket.clone(),
        })
    }

    /// Map an [`object_store::Error`] to a [`PacsError`].
    ///
    /// `NotFound` errors become [`PacsError::NotFound`] carrying the blob key
    /// as the `uid`.  All other errors are wrapped in [`PacsError::Blob`].
    fn map_store_error(err: object_store::Error, key: &str) -> PacsError {
        match err {
            object_store::Error::NotFound { .. } => PacsError::NotFound {
                resource: "blob",
                uid: key.to_string(),
            },
            other => PacsError::Blob(Box::new(other)),
        }
    }
}

#[async_trait]
impl BlobStore for S3BlobStore {
    /// Upload `data` to the store under `key`, overwriting any existing value.
    async fn put(&self, key: &str, data: Bytes) -> PacsResult<()> {
        let path = Path::from(key);
        let payload = PutPayload::from_bytes(data);
        trace!(bucket = %self.bucket, key, "put blob");
        self.store
            .put(&path, payload)
            .await
            .map(|_| ())
            .map_err(|e| Self::map_store_error(e, key))
    }

    /// Retrieve the bytes stored under `key`.
    ///
    /// # Errors
    ///
    /// Returns [`PacsError::NotFound`] if the key does not exist.
    async fn get(&self, key: &str) -> PacsResult<Bytes> {
        let path = Path::from(key);
        trace!(bucket = %self.bucket, key, "get blob");
        let result = self
            .store
            .get(&path)
            .await
            .map_err(|e| Self::map_store_error(e, key))?;
        result
            .bytes()
            .await
            .map_err(|e| Self::map_store_error(e, key))
    }

    /// Delete the object at `key`.
    ///
    /// This operation is **idempotent**: a missing key is silently ignored.
    async fn delete(&self, key: &str) -> PacsResult<()> {
        let path = Path::from(key);
        debug!(bucket = %self.bucket, key, "delete blob");
        match self.store.delete(&path).await {
            Ok(()) => Ok(()),
            Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(other) => Err(PacsError::Blob(Box::new(other))),
        }
    }

    /// Return `true` if an object exists at `key`, `false` otherwise.
    async fn exists(&self, key: &str) -> PacsResult<bool> {
        let path = Path::from(key);
        trace!(bucket = %self.bucket, key, "exists blob");
        match self.store.head(&path).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(other) => Err(PacsError::Blob(Box::new(other))),
        }
    }

    /// Generate a pre-signed GET URL valid for `ttl_secs` seconds.
    async fn presigned_url(&self, key: &str, ttl_secs: u32) -> PacsResult<String> {
        let path = Path::from(key);
        let expires_in = Duration::from_secs(u64::from(ttl_secs));
        trace!(bucket = %self.bucket, key, ttl_secs, "presign blob");
        let url = self
            .store
            .signed_url(http::Method::GET, &path, expires_in)
            .await
            .map_err(|e| Self::map_store_error(e, key))?;
        Ok(url.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn test_config() -> StorageConfig {
        StorageConfig {
            endpoint: "http://localhost:9000".to_string(),
            bucket: "test-bucket".to_string(),
            access_key: "minioadmin".to_string(),
            secret_key: "minioadmin".to_string(),
            region: "us-east-1".to_string(),
        }
    }

    // ── construction ─────────────────────────────────────────────────────────

    #[test]
    fn new_with_valid_config_succeeds() {
        let result = S3BlobStore::new(&test_config());
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    #[test]
    fn new_stores_bucket_name() {
        let store = S3BlobStore::new(&test_config()).expect("build failed in test");
        assert_eq!(store.bucket, "test-bucket");
    }

    #[test]
    fn new_different_regions_succeed() {
        for region in ["us-east-1", "eu-west-1", "ap-southeast-1", "us-east-1"] {
            let mut cfg = test_config();
            cfg.region = region.to_string();
            assert!(
                S3BlobStore::new(&cfg).is_ok(),
                "build failed for region={region}"
            );
        }
    }

    // ── key → path ───────────────────────────────────────────────────────────

    #[rstest]
    #[case("simple", "simple")]
    #[case("a/b/c", "a/b/c")]
    #[case(
        "studies/1.2.3/series/4.5.6/instances/7.8.9",
        "studies/1.2.3/series/4.5.6/instances/7.8.9"
    )]
    #[case(
        "1.2.840.10008.5.1.4.1.1.2/1.3.12.2.1107/1.3.12.2.999",
        "1.2.840.10008.5.1.4.1.1.2/1.3.12.2.1107/1.3.12.2.999"
    )]
    fn key_to_path_preserves_string(#[case] key: &str, #[case] expected: &str) {
        let path = Path::from(key);
        assert_eq!(path.as_ref(), expected);
    }

    // ── error mapping ────────────────────────────────────────────────────────

    #[test]
    fn map_store_error_not_found_yields_pacs_not_found() {
        let key = "studies/1.2.3/blob.dcm";
        let source: Box<dyn std::error::Error + Send + Sync> =
            Box::new(std::io::Error::other("404 Not Found"));
        let err = object_store::Error::NotFound {
            path: key.to_string(),
            source,
        };
        let pacs_err = S3BlobStore::map_store_error(err, key);
        match pacs_err {
            PacsError::NotFound { resource, uid } => {
                assert_eq!(resource, "blob");
                assert_eq!(uid, key);
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[rstest]
    #[case("key-a")]
    #[case("nested/path/key")]
    #[case("special_chars-and.dots")]
    #[case("1.2.840.10008/1.3.12/9.8.7")]
    fn map_store_error_not_found_captures_key(#[case] key: &str) {
        let source: Box<dyn std::error::Error + Send + Sync> =
            Box::new(std::io::Error::other("not found"));
        let err = object_store::Error::NotFound {
            path: key.to_string(),
            source,
        };
        let pacs_err = S3BlobStore::map_store_error(err, key);
        match pacs_err {
            PacsError::NotFound { uid, resource } => {
                assert_eq!(uid, key, "uid should match key");
                assert_eq!(resource, "blob");
            }
            other => panic!("expected NotFound for key={key:?}, got {other:?}"),
        }
    }

    #[test]
    fn map_store_error_generic_yields_pacs_blob() {
        let key = "some/key";
        let source: Box<dyn std::error::Error + Send + Sync> =
            Box::new(std::io::Error::other("connection refused"));
        let err = object_store::Error::Generic {
            store: "S3",
            source,
        };
        let pacs_err = S3BlobStore::map_store_error(err, key);
        assert!(
            matches!(pacs_err, PacsError::Blob(_)),
            "expected Blob, got {pacs_err:?}"
        );
    }

    #[test]
    fn map_store_error_not_implemented_yields_pacs_blob() {
        let pacs_err = S3BlobStore::map_store_error(object_store::Error::NotImplemented, "any/key");
        assert!(
            matches!(pacs_err, PacsError::Blob(_)),
            "expected Blob, got {pacs_err:?}"
        );
    }

    #[test]
    fn s3_blob_store_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<S3BlobStore>();
    }
}
