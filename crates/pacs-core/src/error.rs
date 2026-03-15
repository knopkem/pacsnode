use thiserror::Error;

/// All errors that can occur within the pacsnode system.
#[derive(Debug, Error)]
pub enum PacsError {
    /// A requested DICOM resource was not found.
    #[error("not found: {resource} uid={uid}")]
    NotFound { resource: &'static str, uid: String },

    /// An error occurred in the metadata store.
    #[error("store error: {0}")]
    Store(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// An error occurred in the blob store.
    #[error("blob error: {0}")]
    Blob(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// Failed to parse DICOM data.
    #[error("dicom parse error: {0}")]
    DicomParse(String),

    /// A DICOM UID was invalid.
    #[error("invalid uid: {0}")]
    InvalidUid(String),

    /// A configuration value was invalid or missing.
    #[error("configuration error: {0}")]
    Config(String),

    /// An internal, unexpected error occurred.
    #[error("internal error: {0}")]
    Internal(String),
}

/// Convenience alias used throughout pacsnode.
pub type PacsResult<T> = Result<T, PacsError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_found_display() {
        let e = PacsError::NotFound {
            resource: "study",
            uid: "1.2.3".into(),
        };
        assert_eq!(e.to_string(), "not found: study uid=1.2.3");
    }

    #[test]
    fn test_invalid_uid_display() {
        let e = PacsError::InvalidUid("bad.uid".into());
        assert_eq!(e.to_string(), "invalid uid: bad.uid");
    }

    #[test]
    fn test_dicom_parse_display() {
        let e = PacsError::DicomParse("unexpected EOF".into());
        assert_eq!(e.to_string(), "dicom parse error: unexpected EOF");
    }

    #[test]
    fn test_config_display() {
        let e = PacsError::Config("missing db url".into());
        assert_eq!(e.to_string(), "configuration error: missing db url");
    }

    #[test]
    fn test_internal_display() {
        let e = PacsError::Internal("unreachable state".into());
        assert_eq!(e.to_string(), "internal error: unreachable state");
    }
}
