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

    /// A generic request parameter or payload value was invalid.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// The authenticated caller is not allowed to perform the requested action.
    #[error("forbidden: {0}")]
    Forbidden(String),

    /// A configuration value was invalid or missing.
    #[error("configuration error: {0}")]
    Config(String),

    /// The requested representation cannot be produced for the given resource.
    #[error("not acceptable: {0}")]
    NotAcceptable(String),

    /// The supplied media type or representation request is unsupported.
    #[error("unsupported media type: {0}")]
    UnsupportedMediaType(String),

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
    fn test_invalid_request_display() {
        let e = PacsError::InvalidRequest("limit must be greater than zero".into());
        assert_eq!(
            e.to_string(),
            "invalid request: limit must be greater than zero"
        );
    }

    #[test]
    fn test_forbidden_display() {
        let e = PacsError::Forbidden("viewer role cannot delete studies".into());
        assert_eq!(
            e.to_string(),
            "forbidden: viewer role cannot delete studies"
        );
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
    fn test_not_acceptable_display() {
        let e = PacsError::NotAcceptable("image/jpeg".into());
        assert_eq!(e.to_string(), "not acceptable: image/jpeg");
    }

    #[test]
    fn test_unsupported_media_type_display() {
        let e = PacsError::UnsupportedMediaType("image/tiff".into());
        assert_eq!(e.to_string(), "unsupported media type: image/tiff");
    }

    #[test]
    fn test_internal_display() {
        let e = PacsError::Internal("unreachable state".into());
        assert_eq!(e.to_string(), "internal error: unreachable state");
    }
}
