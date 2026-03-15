//! DIMSE-specific error type and conversions.

use dicom_toolkit_core::error::DcmError;
use pacs_core::PacsError;
use thiserror::Error;

/// All errors that can be returned by the `pacs-dimse` crate.
#[derive(Debug, Error)]
pub enum DimseError {
    /// A low-level DICOM protocol or network error.
    #[error("DICOM protocol error: {0}")]
    Dcm(#[from] DcmError),

    /// An error from the pacsnode metadata or blob storage layer.
    #[error("PACS error: {0}")]
    Pacs(#[from] PacsError),

    /// An I/O error during network operations.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// No accepted presentation context exists for the requested SOP class.
    #[error("no presentation context accepted for SOP class {0}")]
    NoPresentationContext(String),

    /// The server could not bind the configured TCP port.
    #[error("failed to bind port {port}: {source}")]
    Bind {
        /// The port that could not be bound.
        port: u16,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_presentation_context_message() {
        let err = DimseError::NoPresentationContext("1.2.3.4.5".into());
        assert!(err.to_string().contains("1.2.3.4.5"));
    }

    #[test]
    fn bind_error_message() {
        let io_err = std::io::Error::new(std::io::ErrorKind::AddrInUse, "address in use");
        let err = DimseError::Bind { port: 4242, source: io_err };
        assert!(err.to_string().contains("4242"));
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "broken pipe");
        let err = DimseError::from(io_err);
        assert!(matches!(err, DimseError::Io(_)));
    }

    #[test]
    fn from_pacs_error() {
        let pacs_err = PacsError::Internal("test".into());
        let err = DimseError::from(pacs_err);
        assert!(matches!(err, DimseError::Pacs(_)));
    }
}
