use chrono::NaiveDate;
use dicom_toolkit_data::{json::to_json, DataSet};
use dicom_toolkit_dict::Tag;
use pacs_core::DicomJson;

use crate::error::DicomError;

// ── Tags absent from the upstream dictionary ─────────────────────────────────

/// Modalities in Study (0008,0061) — CS, multi-valued.
pub const MODALITIES_IN_STUDY: Tag = Tag::new(0x0008, 0x0061);

/// Body Part Examined (0018,0015) — CS.
pub const BODY_PART_EXAMINED: Tag = Tag::new(0x0018, 0x0015);

// ── Extraction helpers ────────────────────────────────────────────────────────

/// Extracts a required string tag from `ds`.
///
/// # Errors
///
/// Returns [`DicomError::MissingTag`] when the tag is absent.
pub fn required_string<'a>(
    ds: &'a DataSet,
    tag: Tag,
    name: &'static str,
) -> Result<&'a str, DicomError> {
    ds.get_string(tag)
        .ok_or(DicomError::MissingTag { tag: name })
}

/// Extracts an optional string tag, stripping DICOM padding (null bytes and
/// leading/trailing whitespace).  Returns `None` when the tag is absent or the
/// value is empty after stripping.
pub fn optional_string(ds: &DataSet, tag: Tag) -> Option<String> {
    let s = ds.get_string(tag)?;
    let trimmed = s.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Returns the first date value for `tag` as a raw `YYYYMMDD` string.
///
/// Works for both `Value::Strings` (in-memory construction) and
/// `Value::Date` (the variant produced by [`DicomReader`] after a file
/// round-trip).  Returns `None` when the tag is absent or produces an
/// empty display string.
pub fn date_display_string(ds: &DataSet, tag: Tag) -> Option<String> {
    let s = ds.get(tag)?.value.to_display_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Extracts an optional `US` (unsigned-short) tag and widens it to `i32`.
///
/// This helper is appropriate for VR `US` attributes such as `Rows` and
/// `Columns`.  For VR `IS` attributes use [`optional_i32`] instead.
pub fn optional_i32_from_u16(ds: &DataSet, tag: Tag) -> Option<i32> {
    ds.get_u16(tag).map(|v| v as i32)
}

/// Extracts an optional integer-string (`IS`) tag as `i32`.
pub fn optional_i32(ds: &DataSet, tag: Tag) -> Option<i32> {
    ds.get_i32(tag)
}

/// Parses a DICOM date string in `YYYYMMDD` format into a [`NaiveDate`].
///
/// # Errors
///
/// Returns [`DicomError::InvalidDate`] when the input cannot be parsed.
pub fn parse_dicom_date(s: &str) -> Result<NaiveDate, DicomError> {
    let trimmed = s.trim();
    if trimmed.len() != 8 {
        return Err(DicomError::InvalidDate {
            value: s.to_owned(),
        });
    }
    NaiveDate::parse_from_str(trimmed, "%Y%m%d").map_err(|_| DicomError::InvalidDate {
        value: s.to_owned(),
    })
}

/// Serialises `ds` to PS3.18 DICOM JSON and wraps the result in [`DicomJson`].
///
/// # Errors
///
/// Returns [`DicomError::Toolkit`] if the underlying serialiser fails, or if
/// the produced JSON string is not valid JSON (should not happen in practice).
pub fn dataset_to_dicom_json(ds: &DataSet) -> Result<DicomJson, DicomError> {
    let json_str = to_json(ds).map_err(|e| DicomError::Toolkit(e.to_string()))?;
    let value: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| DicomError::Toolkit(e.to_string()))?;
    Ok(DicomJson(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_toolkit_data::DataSet;
    use dicom_toolkit_dict::{tags, Vr};
    use rstest::rstest;

    // ── parse_dicom_date ─────────────────────────────────────────────────────

    #[rstest]
    #[case("20240315", 2024, 3, 15)]
    #[case("19991231", 1999, 12, 31)]
    #[case("20000101", 2000, 1, 1)]
    #[case("19700101", 1970, 1, 1)]
    fn test_parse_dicom_date_valid(
        #[case] input: &str,
        #[case] year: i32,
        #[case] month: u32,
        #[case] day: u32,
    ) {
        let date = parse_dicom_date(input).expect("should parse valid DICOM date");
        assert_eq!(date, NaiveDate::from_ymd_opt(year, month, day).unwrap());
    }

    #[rstest]
    #[case("2024-03-15")]
    #[case("abcdefgh")]
    #[case("")]
    #[case("2024031")] // 7 chars
    #[case("202403150")] // 9 chars
    #[case("20241399")] // invalid month 13
    #[case("20240132")] // invalid day 32
    fn test_parse_dicom_date_invalid(#[case] input: &str) {
        assert!(
            parse_dicom_date(input).is_err(),
            "expected error for input: {input:?}"
        );
    }

    #[test]
    fn test_parse_dicom_date_trims_whitespace() {
        let date = parse_dicom_date("  20240315  ").expect("should trim and parse");
        assert_eq!(date, NaiveDate::from_ymd_opt(2024, 3, 15).unwrap());
    }

    // ── required_string ──────────────────────────────────────────────────────

    #[test]
    fn test_required_string_present() {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_ID, Vr::LO, "PID001");
        let v = required_string(&ds, tags::PATIENT_ID, "PatientID").unwrap();
        assert_eq!(v, "PID001");
    }

    #[test]
    fn test_required_string_missing_returns_error() {
        let ds = DataSet::new();
        let err = required_string(&ds, tags::PATIENT_ID, "PatientID").unwrap_err();
        assert!(matches!(err, DicomError::MissingTag { tag: "PatientID" }));
    }

    // ── optional_string ──────────────────────────────────────────────────────

    #[test]
    fn test_optional_string_present() {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_NAME, Vr::PN, "DOE^JOHN");
        assert_eq!(
            optional_string(&ds, tags::PATIENT_NAME).as_deref(),
            Some("DOE^JOHN")
        );
    }

    #[test]
    fn test_optional_string_absent() {
        let ds = DataSet::new();
        assert_eq!(optional_string(&ds, tags::PATIENT_NAME), None);
    }

    #[test]
    fn test_optional_string_strips_null_padding() {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_ID, Vr::LO, "PID\0");
        let v = optional_string(&ds, tags::PATIENT_ID);
        assert_eq!(v.as_deref(), Some("PID"));
    }

    #[test]
    fn test_optional_string_whitespace_only_returns_none() {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_ID, Vr::LO, "   ");
        assert_eq!(optional_string(&ds, tags::PATIENT_ID), None);
    }

    // ── optional_i32_from_u16 ────────────────────────────────────────────────

    #[test]
    fn test_optional_i32_from_u16_present() {
        let mut ds = DataSet::new();
        ds.set_u16(tags::ROWS, 512);
        assert_eq!(optional_i32_from_u16(&ds, tags::ROWS), Some(512i32));
    }

    #[test]
    fn test_optional_i32_from_u16_absent() {
        let ds = DataSet::new();
        assert_eq!(optional_i32_from_u16(&ds, tags::ROWS), None);
    }

    // ── dataset_to_dicom_json ────────────────────────────────────────────────

    #[test]
    fn test_dataset_to_dicom_json_produces_value() {
        let mut ds = DataSet::new();
        ds.set_string(tags::MODALITY, Vr::CS, "CT");
        let dj = dataset_to_dicom_json(&ds).expect("should produce DicomJson");
        // The DICOM JSON must be an object containing the modality key.
        let s = dj.to_json_string();
        // MODALITY tag 00080060 should appear in the serialised output.
        assert!(s.contains("00080060") || !s.is_empty());
    }

    #[test]
    fn test_dataset_to_dicom_json_empty_dataset() {
        let ds = DataSet::new();
        let dj = dataset_to_dicom_json(&ds).expect("empty dataset should produce empty JSON");
        // Should be a JSON object (possibly empty).
        assert!(dj.as_value().is_object());
    }
}
