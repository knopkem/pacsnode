use pacs_core::PacsError;

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
        let e = DicomError::MissingTag { tag: "SOPInstanceUID" };
        let pacs_err: PacsError = e.into();
        assert!(pacs_err.to_string().contains(original));
    }
}
