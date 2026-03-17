//! Error types for the filesystem-backed blob store.

use thiserror::Error;

/// Errors that can occur while configuring or operating the filesystem blob store.
#[derive(Debug, Error)]
pub enum FsStorageError {
    /// The configured root directory is invalid.
    #[error("invalid filesystem storage root: {0}")]
    InvalidRoot(String),

    /// The blob key cannot be mapped safely to a filesystem path.
    #[error("invalid blob key: {0}")]
    InvalidKey(String),

    /// A filesystem operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
