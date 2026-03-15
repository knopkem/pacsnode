use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::json::DicomJson;
use super::study::StudyUid;

/// A DICOM Series Instance UID (0020,000E).
///
/// # Examples
///
/// ```
/// use pacs_core::SeriesUid;
///
/// let uid = SeriesUid::from("1.2.840.10008.5.1.4.1.1.2.1");
/// assert_eq!(uid.as_ref(), "1.2.840.10008.5.1.4.1.1.2.1");
/// assert_eq!(uid.to_string(), "1.2.840.10008.5.1.4.1.1.2.1");
/// ```
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SeriesUid(String);

impl fmt::Debug for SeriesUid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SeriesUid({})", self.0)
    }
}

impl fmt::Display for SeriesUid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for SeriesUid {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for SeriesUid {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SeriesUid {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// A DICOM series belonging to a study.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Series {
    /// Series Instance UID (0020,000E).
    pub series_uid: SeriesUid,
    /// Parent study's Study Instance UID (0020,000D).
    pub study_uid: StudyUid,
    /// Modality (0008,0060).
    pub modality: Option<String>,
    /// Series Number (0020,0011).
    pub series_number: Option<i32>,
    /// Series Description (0008,103E).
    pub description: Option<String>,
    /// Body Part Examined (0018,0015).
    pub body_part: Option<String>,
    /// Number of instances in this series.
    pub num_instances: i32,
    /// Full DICOM JSON tag set (PS3.18).
    pub metadata: DicomJson,
    /// Timestamp when the series was first stored.
    pub created_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("1.2.840.10008.5.1.4.1.1.2.1")]
    #[case("2.16.840.1.113883.6.96.1")]
    #[case("1.2.3.4.5.6")]
    fn test_series_uid_from_str(#[case] uid_str: &str) {
        let uid = SeriesUid::from(uid_str);
        assert_eq!(uid.as_ref(), uid_str);
        assert_eq!(uid.to_string(), uid_str);
    }

    #[test]
    fn test_series_uid_from_owned_string() {
        let s = String::from("1.2.3");
        let uid = SeriesUid::from(s.clone());
        assert_eq!(uid.as_ref(), s.as_str());
    }

    #[test]
    fn test_series_uid_equality() {
        let a = SeriesUid::from("1.2.3");
        let b = SeriesUid::from("1.2.3");
        let c = SeriesUid::from("9.9.9");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_series_uid_clone() {
        let a = SeriesUid::from("1.2.3");
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn test_series_uid_hash_dedup() {
        use std::collections::HashSet;
        let uids: HashSet<SeriesUid> = [
            SeriesUid::from("1.2.3"),
            SeriesUid::from("1.2.3"),
            SeriesUid::from("4.5.6"),
        ]
        .into_iter()
        .collect();
        assert_eq!(uids.len(), 2);
    }

    #[test]
    fn test_series_uid_serde_roundtrip() {
        let uid = SeriesUid::from("1.2.3.4.5");
        let json = serde_json::to_string(&uid).unwrap();
        let back: SeriesUid = serde_json::from_str(&json).unwrap();
        assert_eq!(uid, back);
    }

    #[test]
    fn test_series_uid_debug_format() {
        let uid = SeriesUid::from("1.2.3");
        assert_eq!(format!("{uid:?}"), "SeriesUid(1.2.3)");
    }

    #[test]
    fn test_series_uid_display_format() {
        let uid = SeriesUid::from("1.2.3");
        assert_eq!(format!("{uid}"), "1.2.3");
    }
}
