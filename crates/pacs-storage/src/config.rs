//! Configuration for the S3-compatible blob storage backend.

/// Configuration required to connect to an S3-compatible object store
/// (AWS S3, MinIO, RustFS, etc.).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct StorageConfig {
    /// Full URL of the S3-compatible endpoint.
    ///
    /// Examples:
    /// - AWS:            `"https://s3.amazonaws.com"`
    /// - MinIO / RustFS: `"http://localhost:9000"`
    pub endpoint: String,

    /// The bucket to use for all blob operations.
    pub bucket: String,

    /// The S3 access key ID.
    pub access_key: String,

    /// The S3 secret access key.
    pub secret_key: String,

    /// AWS region string.
    ///
    /// MinIO and RustFS do not enforce the region value; `"us-east-1"` is a
    /// safe default for local deployments.
    pub region: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> StorageConfig {
        StorageConfig {
            endpoint: "http://localhost:9000".to_string(),
            bucket: "dicom".to_string(),
            access_key: "minioadmin".to_string(),
            secret_key: "miniosecret".to_string(),
            region: "us-east-1".to_string(),
        }
    }

    #[test]
    fn deserializes_from_json() {
        let json = r#"{
            "endpoint":   "http://localhost:9000",
            "bucket":     "dicom",
            "access_key": "minioadmin",
            "secret_key": "miniosecret",
            "region":     "us-east-1"
        }"#;
        let cfg: StorageConfig = serde_json::from_str(json).expect("deserialize failed");
        assert_eq!(cfg.endpoint, "http://localhost:9000");
        assert_eq!(cfg.bucket, "dicom");
        assert_eq!(cfg.access_key, "minioadmin");
        assert_eq!(cfg.secret_key, "miniosecret");
        assert_eq!(cfg.region, "us-east-1");
    }

    #[test]
    fn clone_produces_independent_copy() {
        let cfg = test_config();
        let mut cloned = cfg.clone();
        cloned.bucket = "other".to_string();
        assert_eq!(cfg.bucket, "dicom", "original should be unchanged");
        assert_eq!(cloned.bucket, "other");
    }

    #[test]
    fn debug_format_contains_endpoint() {
        let cfg = test_config();
        let debug = format!("{cfg:?}");
        assert!(
            debug.contains("localhost:9000"),
            "debug output should include endpoint: {debug}"
        );
    }
}
