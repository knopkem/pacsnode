use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::json::DicomJson;
use super::series::SeriesUid;
use super::study::StudyUid;

/// A DICOM SOP Instance UID (0008,0018).
///
/// # Examples
///
/// ```
/// use pacs_core::SopInstanceUid;
///
/// let uid = SopInstanceUid::from("1.2.840.10008.5.1.4.1.1.2.1.1");
/// assert_eq!(uid.as_ref(), "1.2.840.10008.5.1.4.1.1.2.1.1");
/// ```
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SopInstanceUid(String);

impl fmt::Debug for SopInstanceUid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SopInstanceUid({})", self.0)
    }
}

impl fmt::Display for SopInstanceUid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for SopInstanceUid {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for SopInstanceUid {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SopInstanceUid {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// Returns the canonical blob store object key for a DICOM instance.
///
/// Format: `{study_uid}/{series_uid}/{instance_uid}`
///
/// # Examples
///
/// ```
/// use pacs_core::{StudyUid, SeriesUid, SopInstanceUid, blob_key_for};
///
/// let key = blob_key_for(
///     &StudyUid::from("1.2.3"),
///     &SeriesUid::from("4.5.6"),
///     &SopInstanceUid::from("7.8.9"),
/// );
/// assert_eq!(key, "1.2.3/4.5.6/7.8.9");
/// ```
pub fn blob_key_for(
    study_uid: &StudyUid,
    series_uid: &SeriesUid,
    instance_uid: &SopInstanceUid,
) -> String {
    format!("{study_uid}/{series_uid}/{instance_uid}")
}

/// A DICOM SOP Instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    /// SOP Instance UID (0008,0018).
    pub instance_uid: SopInstanceUid,
    /// Parent series' Series Instance UID (0020,000E).
    pub series_uid: SeriesUid,
    /// Parent study's Study Instance UID (0020,000D).
    pub study_uid: StudyUid,
    /// SOP Class UID (0008,0016).
    pub sop_class_uid: Option<String>,
    /// Instance Number (0020,0013).
    pub instance_number: Option<i32>,
    /// Transfer Syntax UID (0002,0010).
    pub transfer_syntax: Option<String>,
    /// Number of image rows (0028,0010).
    pub rows: Option<i32>,
    /// Number of image columns (0028,0011).
    pub columns: Option<i32>,
    /// RustFS object key: `{study_uid}/{series_uid}/{instance_uid}`.
    pub blob_key: String,
    /// Full DICOM JSON tag set (PS3.18).
    pub metadata: DicomJson,
    /// Timestamp when the instance was first stored.
    pub created_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("1.2.840.10008.5.1.4.1.1.2.1.1")]
    #[case("2.25.123456789012345678901234567890")]
    #[case("1.3.6.1.4.1.9590.100.1.2.100")]
    fn test_sop_instance_uid_from_str(#[case] uid_str: &str) {
        let uid = SopInstanceUid::from(uid_str);
        assert_eq!(uid.as_ref(), uid_str);
        assert_eq!(uid.to_string(), uid_str);
    }

    #[test]
    fn test_sop_instance_uid_from_owned_string() {
        let s = String::from("1.2.3.4");
        let uid = SopInstanceUid::from(s.clone());
        assert_eq!(uid.as_ref(), s.as_str());
    }

    #[test]
    fn test_sop_instance_uid_equality() {
        let a = SopInstanceUid::from("1.2.3");
        let b = SopInstanceUid::from("1.2.3");
        let c = SopInstanceUid::from("9.9.9");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_sop_instance_uid_clone() {
        let a = SopInstanceUid::from("1.2.3");
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn test_sop_instance_uid_hash_dedup() {
        use std::collections::HashSet;
        let uids: HashSet<SopInstanceUid> = [
            SopInstanceUid::from("1.2.3"),
            SopInstanceUid::from("1.2.3"),
            SopInstanceUid::from("4.5.6"),
        ]
        .into_iter()
        .collect();
        assert_eq!(uids.len(), 2);
    }

    #[test]
    fn test_sop_instance_uid_serde_roundtrip() {
        let uid = SopInstanceUid::from("1.2.3.4.5");
        let json = serde_json::to_string(&uid).unwrap();
        let back: SopInstanceUid = serde_json::from_str(&json).unwrap();
        assert_eq!(uid, back);
    }

    #[test]
    fn test_sop_instance_uid_debug_format() {
        let uid = SopInstanceUid::from("1.2.3");
        assert_eq!(format!("{uid:?}"), "SopInstanceUid(1.2.3)");
    }

    #[test]
    fn test_blob_key_for_basic() {
        let study = StudyUid::from("1.2.3");
        let series = SeriesUid::from("4.5.6");
        let instance = SopInstanceUid::from("7.8.9");
        let key = blob_key_for(&study, &series, &instance);
        assert_eq!(key, "1.2.3/4.5.6/7.8.9");
    }

    #[rstest]
    #[case("a.b.c", "d.e.f", "g.h.i", "a.b.c/d.e.f/g.h.i")]
    #[case("1.2.3", "4.5.6", "7.8.9", "1.2.3/4.5.6/7.8.9")]
    #[case(
        "1.2.840.10008.5.1.4.1.1.2",
        "1.2.840.10008.5.1.4.1.1.2.1",
        "1.2.840.10008.5.1.4.1.1.2.1.1",
        "1.2.840.10008.5.1.4.1.1.2/1.2.840.10008.5.1.4.1.1.2.1/1.2.840.10008.5.1.4.1.1.2.1.1"
    )]
    fn test_blob_key_for_format(
        #[case] study: &str,
        #[case] series: &str,
        #[case] instance: &str,
        #[case] expected: &str,
    ) {
        let key = blob_key_for(
            &StudyUid::from(study),
            &SeriesUid::from(series),
            &SopInstanceUid::from(instance),
        );
        assert_eq!(key, expected);
    }

    #[test]
    fn test_blob_key_for_two_slashes() {
        let key = blob_key_for(
            &StudyUid::from("s"),
            &SeriesUid::from("r"),
            &SopInstanceUid::from("i"),
        );
        assert_eq!(key.matches('/').count(), 2);
    }
}
