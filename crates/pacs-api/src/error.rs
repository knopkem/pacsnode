//! HTTP error mapping for the pacsnode API.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use pacs_core::PacsError;
use thiserror::Error;

/// Wrapper around [`PacsError`] that converts domain errors into HTTP responses.
///
/// Returned by every Axum handler as `Result<impl IntoResponse, ApiError>`.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct ApiError(pub PacsError);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self.0 {
            PacsError::NotFound { resource, uid } => {
                (StatusCode::NOT_FOUND, format!("{resource} {uid} not found"))
            }
            PacsError::DicomParse(msg) | PacsError::InvalidUid(msg) => {
                (StatusCode::BAD_REQUEST, msg.clone())
            }
            PacsError::NotAcceptable(msg) => (StatusCode::NOT_ACCEPTABLE, msg.clone()),
            PacsError::UnsupportedMediaType(msg) => {
                (StatusCode::UNSUPPORTED_MEDIA_TYPE, msg.clone())
            }
            PacsError::Store(_)
            | PacsError::Blob(_)
            | PacsError::Internal(_)
            | PacsError::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into()),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

impl From<PacsError> for ApiError {
    fn from(e: PacsError) -> Self {
        Self(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_found_is_404() {
        let err = ApiError(PacsError::NotFound {
            resource: "study",
            uid: "1.2.3".into(),
        });
        assert_eq!(err.into_response().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_dicom_parse_is_400() {
        let err = ApiError(PacsError::DicomParse("bad data".into()));
        assert_eq!(err.into_response().status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_invalid_uid_is_400() {
        let err = ApiError(PacsError::InvalidUid("bad uid".into()));
        assert_eq!(err.into_response().status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_internal_is_500() {
        let err = ApiError(PacsError::Internal("oops".into()));
        assert_eq!(
            err.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_config_is_500() {
        let err = ApiError(PacsError::Config("bad config".into()));
        assert_eq!(
            err.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_not_acceptable_is_406() {
        let err = ApiError(PacsError::NotAcceptable("image/jpeg".into()));
        assert_eq!(err.into_response().status(), StatusCode::NOT_ACCEPTABLE);
    }

    #[test]
    fn test_unsupported_media_type_is_415() {
        let err = ApiError(PacsError::UnsupportedMediaType("image/tiff".into()));
        assert_eq!(
            err.into_response().status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
    }

    #[test]
    fn test_from_pacs_error() {
        let api_err: ApiError = PacsError::Config("x".into()).into();
        assert_eq!(
            api_err.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
