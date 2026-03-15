use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::instance::SopInstanceUid;
use super::series::SeriesUid;
use super::study::StudyUid;

/// Parameters for QIDO-RS study-level queries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StudyQuery {
    /// Filter by Patient ID (0010,0020).
    pub patient_id: Option<String>,
    /// Filter by Patient Name (0010,0010); supports `*` wildcard suffix.
    pub patient_name: Option<String>,
    /// Lower bound for study date range (inclusive), format `YYYYMMDD`.
    pub study_date_from: Option<NaiveDate>,
    /// Upper bound for study date range (inclusive), format `YYYYMMDD`.
    pub study_date_to: Option<NaiveDate>,
    /// Filter by Accession Number (0008,0050).
    pub accession_number: Option<String>,
    /// Filter by a specific Study Instance UID.
    pub study_uid: Option<StudyUid>,
    /// Filter by modality (e.g. `"CT"`, `"MR"`).
    pub modality: Option<String>,
    /// Maximum number of results to return.
    pub limit: Option<u32>,
    /// Number of results to skip (for pagination).
    pub offset: Option<u32>,
    /// Additional DICOM attributes to include in the response.
    pub include_fields: Vec<String>,
    /// Enable fuzzy matching for string attributes.
    pub fuzzy_matching: bool,
}

/// Parameters for QIDO-RS series-level queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesQuery {
    /// The parent Study Instance UID (required).
    pub study_uid: StudyUid,
    /// Filter by a specific Series Instance UID.
    pub series_uid: Option<SeriesUid>,
    /// Filter by modality (0008,0060).
    pub modality: Option<String>,
    /// Filter by Series Number (0020,0011).
    pub series_number: Option<i32>,
    /// Maximum number of results to return.
    pub limit: Option<u32>,
    /// Number of results to skip (for pagination).
    pub offset: Option<u32>,
}

/// Parameters for QIDO-RS instance-level queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceQuery {
    /// The parent Series Instance UID (required).
    pub series_uid: SeriesUid,
    /// Filter by a specific SOP Instance UID.
    pub instance_uid: Option<SopInstanceUid>,
    /// Filter by SOP Class UID (0008,0016).
    pub sop_class_uid: Option<String>,
    /// Filter by Instance Number (0020,0013).
    pub instance_number: Option<i32>,
    /// Maximum number of results to return.
    pub limit: Option<u32>,
    /// Number of results to skip (for pagination).
    pub offset: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_study_query_default_all_none() {
        let q = StudyQuery::default();
        assert!(q.patient_id.is_none());
        assert!(q.patient_name.is_none());
        assert!(q.study_date_from.is_none());
        assert!(q.study_date_to.is_none());
        assert!(q.accession_number.is_none());
        assert!(q.study_uid.is_none());
        assert!(q.modality.is_none());
        assert!(q.limit.is_none());
        assert!(q.offset.is_none());
        assert!(q.include_fields.is_empty());
        assert!(!q.fuzzy_matching);
    }

    #[test]
    fn test_study_query_serde_roundtrip() {
        let q = StudyQuery {
            patient_id: Some("P001".into()),
            modality: Some("CT".into()),
            fuzzy_matching: true,
            limit: Some(50),
            offset: Some(10),
            ..Default::default()
        };
        let json = serde_json::to_string(&q).unwrap();
        let back: StudyQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(back.patient_id.as_deref(), Some("P001"));
        assert_eq!(back.modality.as_deref(), Some("CT"));
        assert!(back.fuzzy_matching);
        assert_eq!(back.limit, Some(50));
        assert_eq!(back.offset, Some(10));
    }

    #[test]
    fn test_series_query_construction() {
        let q = SeriesQuery {
            study_uid: StudyUid::from("1.2.3"),
            series_uid: None,
            modality: Some("MR".into()),
            series_number: Some(1),
            limit: Some(10),
            offset: None,
        };
        assert_eq!(q.study_uid.as_ref(), "1.2.3");
        assert_eq!(q.modality.as_deref(), Some("MR"));
        assert_eq!(q.series_number, Some(1));
    }

    #[test]
    fn test_series_query_serde_roundtrip() {
        let q = SeriesQuery {
            study_uid: StudyUid::from("1.2.3"),
            series_uid: Some(SeriesUid::from("4.5.6")),
            modality: None,
            series_number: None,
            limit: None,
            offset: None,
        };
        let json = serde_json::to_string(&q).unwrap();
        let back: SeriesQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(back.study_uid.as_ref(), "1.2.3");
        assert_eq!(back.series_uid.as_ref().map(|u| u.as_ref()), Some("4.5.6"));
    }

    #[test]
    fn test_instance_query_construction() {
        let q = InstanceQuery {
            series_uid: SeriesUid::from("4.5.6"),
            instance_uid: None,
            sop_class_uid: Some("1.2.840.10008.5.1.4.1.1.2".into()),
            instance_number: Some(1),
            limit: Some(100),
            offset: Some(0),
        };
        assert_eq!(q.series_uid.as_ref(), "4.5.6");
        assert_eq!(q.instance_number, Some(1));
        assert_eq!(q.limit, Some(100));
    }

    #[test]
    fn test_instance_query_serde_roundtrip() {
        let q = InstanceQuery {
            series_uid: SeriesUid::from("4.5.6"),
            instance_uid: Some(SopInstanceUid::from("7.8.9")),
            sop_class_uid: None,
            instance_number: None,
            limit: None,
            offset: None,
        };
        let json = serde_json::to_string(&q).unwrap();
        let back: InstanceQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(back.series_uid.as_ref(), "4.5.6");
        assert_eq!(
            back.instance_uid.as_ref().map(|u| u.as_ref()),
            Some("7.8.9")
        );
    }
}
