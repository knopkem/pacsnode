use std::fmt;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::json::DicomJson;

/// A DICOM Study Instance UID (0020,000D).
///
/// # Examples
///
/// ```
/// use pacs_core::StudyUid;
///
/// let uid = StudyUid::from("1.2.840.10008.5.1.4.1.1.2");
/// assert_eq!(uid.as_ref(), "1.2.840.10008.5.1.4.1.1.2");
/// assert_eq!(uid.to_string(), "1.2.840.10008.5.1.4.1.1.2");
/// ```
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StudyUid(String);

impl fmt::Debug for StudyUid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StudyUid({})", self.0)
    }
}

impl fmt::Display for StudyUid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for StudyUid {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for StudyUid {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for StudyUid {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// A DICOM study, containing one or more series.
///
/// **Important:** The `patient_name` field contains PHI (Protected Health
/// Information) and **must never be logged**.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Study {
    /// Study Instance UID (0020,000D).
    pub study_uid: StudyUid,
    /// Patient ID (0010,0020).
    pub patient_id: Option<String>,
    /// Patient name (0010,0010). **PHI — do not log.**
    pub patient_name: Option<String>,
    /// Study date (0008,0020).
    pub study_date: Option<NaiveDate>,
    /// Study time (0008,0030).
    pub study_time: Option<String>,
    /// Accession number (0008,0050).
    pub accession_number: Option<String>,
    /// Modalities present in the study (0008,0061).
    pub modalities: Vec<String>,
    /// Referring physician name (0008,0090).
    pub referring_physician: Option<String>,
    /// Study description (0008,1030).
    pub description: Option<String>,
    /// Number of series in the study.
    pub num_series: i32,
    /// Number of instances in the study.
    pub num_instances: i32,
    /// Full DICOM JSON tag set (PS3.18).
    pub metadata: DicomJson,
    /// Timestamp when the study was first stored.
    pub created_at: Option<DateTime<Utc>>,
    /// Timestamp when the study was last updated.
    pub updated_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("1.2.840.10008.5.1.4.1.1.2")]
    #[case("2.16.840.1.113883.6.96")]
    #[case("1.2.3.4.5.6.7.8.9")]
    fn test_study_uid_from_str(#[case] uid_str: &str) {
        let uid = StudyUid::from(uid_str);
        assert_eq!(uid.as_ref(), uid_str);
        assert_eq!(uid.to_string(), uid_str);
    }

    #[test]
    fn test_study_uid_from_owned_string() {
        let s = String::from("1.2.3");
        let uid = StudyUid::from(s.clone());
        assert_eq!(uid.as_ref(), s.as_str());
    }

    #[test]
    fn test_study_uid_equality() {
        let a = StudyUid::from("1.2.3");
        let b = StudyUid::from("1.2.3");
        let c = StudyUid::from("9.9.9");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_study_uid_clone() {
        let a = StudyUid::from("1.2.3");
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn test_study_uid_hash_dedup() {
        use std::collections::HashSet;
        let uids: HashSet<StudyUid> = [
            StudyUid::from("1.2.3"),
            StudyUid::from("1.2.3"),
            StudyUid::from("4.5.6"),
        ]
        .into_iter()
        .collect();
        assert_eq!(uids.len(), 2);
    }

    #[test]
    fn test_study_uid_serde_roundtrip() {
        let uid = StudyUid::from("1.2.3.4.5");
        let json = serde_json::to_string(&uid).unwrap();
        let back: StudyUid = serde_json::from_str(&json).unwrap();
        assert_eq!(uid, back);
    }

    #[test]
    fn test_study_uid_debug_format() {
        let uid = StudyUid::from("1.2.3");
        assert_eq!(format!("{uid:?}"), "StudyUid(1.2.3)");
    }

    #[test]
    fn test_study_uid_display_format() {
        let uid = StudyUid::from("1.2.3");
        assert_eq!(format!("{uid}"), "1.2.3");
    }
}
