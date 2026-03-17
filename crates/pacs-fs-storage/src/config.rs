//! Configuration for the filesystem-backed blob store.

/// Configuration required by [`crate::FsBlobStore`].
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct FilesystemStorageConfig {
    /// Root directory used for all blob storage operations.
    pub root: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_from_json() {
        let cfg: FilesystemStorageConfig =
            serde_json::from_str(r#"{"root":"./data"}"#).expect("deserialize failed");
        assert_eq!(cfg.root, "./data");
    }
}
