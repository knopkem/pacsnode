use bytes::Bytes;
use pacs_core::PacsError;

/// Raw bulk data extracted from a DICOM element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkDataValue {
    /// A single octet-stream payload.
    Single(Bytes),
    /// Multiple payload parts, typically one encapsulated fragment per part.
    Multipart(Vec<Bytes>),
}

/// Error type for DICOM parsing and processing operations within `pacs-dicom`.
#[derive(Debug, thiserror::Error)]
pub enum DicomError {
    /// An error propagated from the underlying `dicom-toolkit-rs` library.
    #[error("toolkit error: {0}")]
    Toolkit(String),

    /// A required DICOM attribute was absent from the dataset.
    #[error("missing required tag: {tag}")]
    MissingTag {
        /// Human-readable attribute name (e.g. `"StudyInstanceUID"`).
        tag: &'static str,
    },

    /// A DICOM date string could not be parsed into a calendar date.
    #[error("invalid date format: {value}")]
    InvalidDate {
        /// The raw string that failed to parse.
        value: String,
    },

    /// The `multipart/related` body was structurally invalid.
    #[error("invalid multipart content: {0}")]
    MultipartParse(String),

    /// A requested frame number was invalid for the instance.
    #[error("invalid frame number {requested}; instance exposes {available} frame(s)")]
    InvalidFrame {
        /// The requested 1-based frame number.
        requested: u32,
        /// Total available frame count.
        available: u32,
    },

    /// A bulk-data path was malformed.
    #[error("invalid bulk data tag path: {value}")]
    InvalidTagPath {
        /// The raw tag path string supplied by the caller.
        value: String,
    },

    /// The requested DICOM operation is not supported for the instance.
    #[error("unsupported DICOM operation: {message}")]
    Unsupported {
        /// Human-readable explanation of the unsupported condition.
        message: String,
    },
}

impl From<DicomError> for PacsError {
    fn from(e: DicomError) -> Self {
        PacsError::DicomParse(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_tag_display() {
        let e = DicomError::MissingTag { tag: "PatientID" };
        assert_eq!(e.to_string(), "missing required tag: PatientID");
    }

    #[test]
    fn test_invalid_date_display() {
        let e = DicomError::InvalidDate {
            value: "bad-date".to_owned(),
        };
        assert_eq!(e.to_string(), "invalid date format: bad-date");
    }

    #[test]
    fn test_toolkit_display() {
        let e = DicomError::Toolkit("unexpected EOF".to_owned());
        assert_eq!(e.to_string(), "toolkit error: unexpected EOF");
    }

    #[test]
    fn test_multipart_parse_display() {
        let e = DicomError::MultipartParse("no boundary".to_owned());
        assert_eq!(e.to_string(), "invalid multipart content: no boundary");
    }

    #[test]
    fn test_invalid_frame_display() {
        let e = DicomError::InvalidFrame {
            requested: 3,
            available: 2,
        };
        assert_eq!(
            e.to_string(),
            "invalid frame number 3; instance exposes 2 frame(s)"
        );
    }

    #[test]
    fn test_invalid_tag_path_display() {
        let e = DicomError::InvalidTagPath {
            value: "7FE0/0010".to_owned(),
        };
        assert_eq!(e.to_string(), "invalid bulk data tag path: 7FE0/0010");
    }

    #[test]
    fn test_unsupported_display() {
        let e = DicomError::Unsupported {
            message: "image/jpeg rendering is not available".to_owned(),
        };
        assert_eq!(
            e.to_string(),
            "unsupported DICOM operation: image/jpeg rendering is not available"
        );
    }

    #[test]
    fn test_into_pacs_error_is_dicom_parse_variant() {
        let e = DicomError::MissingTag {
            tag: "StudyInstanceUID",
        };
        let pacs_err: PacsError = e.into();
        assert!(matches!(pacs_err, PacsError::DicomParse(_)));
    }

    #[test]
    fn test_pacs_error_message_contains_original() {
        let original = "missing required tag: SOPInstanceUID";
        let e = DicomError::MissingTag {
            tag: "SOPInstanceUID",
        };
        let pacs_err: PacsError = e.into();
        assert!(pacs_err.to_string().contains(original));
    }
}
