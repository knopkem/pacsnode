use bytes::Bytes;
use dicom_toolkit_data::DicomReader;
use dicom_toolkit_dict::tags;
use pacs_core::{
    blob_key_for, Instance, PacsError, PacsResult, Series, SeriesUid, SopInstanceUid, Study,
    StudyUid,
};

use crate::tags::{
    dataset_to_dicom_json, date_display_string, optional_i32, optional_i32_from_u16,
    optional_string, parse_dicom_date, required_string, BODY_PART_EXAMINED, MODALITIES_IN_STUDY,
};

/// The result of parsing a single DICOM Part 10 file from raw bytes.
///
/// Each field corresponds to a domain object or ancillary datum extracted
/// from the file.  `num_series` and `num_instances` counters are initialised
/// to `0`; callers should increment them when persisting to the metadata store.
#[derive(Debug, Clone)]
pub struct ParsedDicom {
    /// The Study-level domain object (`num_series = 0`, `num_instances = 0`).
    pub study: Study,
    /// The Series-level domain object (`num_instances = 0`).
    pub series: Series,
    /// The Instance-level domain object.
    pub instance: Instance,
    /// The original encoded bytes, ready to be written to the blob store.
    pub encoded_bytes: Bytes,
    /// Transfer Syntax UID taken from the File Meta Information (0002,0010).
    pub transfer_syntax_uid: String,
}

impl ParsedDicom {
    /// Parses a complete DICOM Part 10 file from raw bytes.
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - `data` is not a valid DICOM Part 10 file.
    /// - Any of the three mandatory UIDs (`StudyInstanceUID`,
    ///   `SeriesInstanceUID`, `SOPInstanceUID`) are absent.
    /// - The DICOM JSON serialiser fails (should not happen in practice).
    pub fn from_bytes(data: Bytes) -> PacsResult<Self> {
        // 1. Parse the DICOM Part 10 file.
        let mut reader = DicomReader::new(std::io::Cursor::new(data.as_ref()));
        let file_format = reader
            .read_file()
            .map_err(|e| PacsError::DicomParse(e.to_string()))?;

        let ds = &file_format.dataset;
        let transfer_syntax_uid = file_format.meta.transfer_syntax_uid.clone();

        // 2. Mandatory UIDs.
        let study_uid = StudyUid::from(
            required_string(ds, tags::STUDY_INSTANCE_UID, "StudyInstanceUID")
                .map_err(PacsError::from)?,
        );
        let series_uid = SeriesUid::from(
            required_string(ds, tags::SERIES_INSTANCE_UID, "SeriesInstanceUID")
                .map_err(PacsError::from)?,
        );
        let instance_uid = SopInstanceUid::from(
            required_string(ds, tags::SOP_INSTANCE_UID, "SOPInstanceUID")
                .map_err(PacsError::from)?,
        );

        // 3. DICOM JSON metadata (shared across all three domain objects).
        let metadata = dataset_to_dicom_json(ds).map_err(PacsError::from)?;

        // 4. Study-level attributes.
        let patient_id = optional_string(ds, tags::PATIENT_ID);
        let patient_name = optional_string(ds, tags::PATIENT_NAME);
        let study_date =
            date_display_string(ds, tags::STUDY_DATE).and_then(|s| parse_dicom_date(&s).ok());
        let study_time = optional_string(ds, tags::STUDY_TIME);
        let accession_number = optional_string(ds, tags::ACCESSION_NUMBER);
        let referring_physician = optional_string(ds, tags::REFERRING_PHYSICIAN_NAME);
        let study_description = optional_string(ds, tags::STUDY_DESCRIPTION);

        // Modalities: prefer MODALITIES_IN_STUDY, fall back to MODALITY.
        let modalities = ds
            .get_strings(MODALITIES_IN_STUDY)
            .map(|s| s.to_vec())
            .or_else(|| optional_string(ds, tags::MODALITY).map(|m| vec![m]))
            .unwrap_or_default();

        // 5. Series-level attributes.
        let modality = optional_string(ds, tags::MODALITY);
        let series_number = optional_i32(ds, tags::SERIES_NUMBER);
        let series_description = optional_string(ds, tags::SERIES_DESCRIPTION);
        let body_part = optional_string(ds, BODY_PART_EXAMINED);

        // 6. Instance-level attributes.
        let sop_class_uid = optional_string(ds, tags::SOP_CLASS_UID);
        let instance_number = optional_i32(ds, tags::INSTANCE_NUMBER);
        let rows = optional_i32_from_u16(ds, tags::ROWS);
        let columns = optional_i32_from_u16(ds, tags::COLUMNS);

        // 7. Blob key.
        let blob_key = blob_key_for(&study_uid, &series_uid, &instance_uid);

        // 8. Assemble domain objects.
        let study = Study {
            study_uid: study_uid.clone(),
            patient_id,
            patient_name,
            study_date,
            study_time,
            accession_number,
            modalities,
            referring_physician,
            description: study_description,
            num_series: 0,
            num_instances: 0,
            metadata: metadata.clone(),
            created_at: None,
            updated_at: None,
        };

        let series = Series {
            series_uid: series_uid.clone(),
            study_uid: study_uid.clone(),
            modality,
            series_number,
            description: series_description,
            body_part,
            num_instances: 0,
            metadata: metadata.clone(),
            created_at: None,
        };

        let instance = Instance {
            instance_uid,
            series_uid,
            study_uid,
            sop_class_uid,
            instance_number,
            transfer_syntax: Some(transfer_syntax_uid.clone()),
            rows,
            columns,
            blob_key,
            metadata,
            created_at: None,
        };

        Ok(Self {
            study,
            series,
            instance,
            encoded_bytes: data,
            transfer_syntax_uid,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use dicom_toolkit_data::{DataSet, DicomWriter, FileFormat};
    use dicom_toolkit_dict::{tags, Vr};

    /// Builds a minimal valid DICOM Part 10 file in memory.
    fn make_test_dicom() -> Bytes {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_NAME, Vr::PN, "Test^Patient");
        ds.set_string(tags::PATIENT_ID, Vr::LO, "PID001");
        ds.set_string(tags::STUDY_DATE, Vr::DA, "20240315");
        ds.set_string(tags::STUDY_INSTANCE_UID, Vr::UI, "1.2.3.4.5");
        ds.set_string(tags::SERIES_INSTANCE_UID, Vr::UI, "1.2.3.4.5.1");
        ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, "1.2.3.4.5.1.1");
        ds.set_string(tags::SOP_CLASS_UID, Vr::UI, "1.2.840.10008.5.1.4.1.1.2");
        ds.set_string(tags::MODALITY, Vr::CS, "CT");
        ds.set_u16(tags::ROWS, 512);
        ds.set_u16(tags::COLUMNS, 512);
        let ff = FileFormat::from_dataset("1.2.840.10008.5.1.4.1.1.2", "1.2.3.4.5.1.1", ds);
        let mut buf = Vec::new();
        DicomWriter::new(std::io::Cursor::new(&mut buf))
            .write_file(&ff)
            .unwrap();
        Bytes::from(buf)
    }

    #[test]
    fn test_from_bytes_roundtrip_uids() {
        let parsed = ParsedDicom::from_bytes(make_test_dicom()).unwrap();
        assert_eq!(parsed.study.study_uid.as_ref(), "1.2.3.4.5");
        assert_eq!(parsed.series.series_uid.as_ref(), "1.2.3.4.5.1");
        assert_eq!(parsed.instance.instance_uid.as_ref(), "1.2.3.4.5.1.1");
    }

    #[test]
    fn test_from_bytes_patient_fields() {
        let parsed = ParsedDicom::from_bytes(make_test_dicom()).unwrap();
        assert_eq!(parsed.study.patient_id.as_deref(), Some("PID001"));
        assert_eq!(parsed.study.patient_name.as_deref(), Some("Test^Patient"));
    }

    #[test]
    fn test_from_bytes_study_date_parsed() {
        use chrono::NaiveDate;
        let parsed = ParsedDicom::from_bytes(make_test_dicom()).unwrap();
        assert_eq!(
            parsed.study.study_date,
            NaiveDate::from_ymd_opt(2024, 3, 15)
        );
    }

    #[test]
    fn test_from_bytes_modality_and_image_dims() {
        let parsed = ParsedDicom::from_bytes(make_test_dicom()).unwrap();
        assert_eq!(parsed.series.modality.as_deref(), Some("CT"));
        assert_eq!(parsed.instance.rows, Some(512));
        assert_eq!(parsed.instance.columns, Some(512));
    }

    #[test]
    fn test_from_bytes_sop_class_uid() {
        let parsed = ParsedDicom::from_bytes(make_test_dicom()).unwrap();
        assert_eq!(
            parsed.instance.sop_class_uid.as_deref(),
            Some("1.2.840.10008.5.1.4.1.1.2")
        );
    }

    #[test]
    fn test_from_bytes_blob_key_format() {
        let parsed = ParsedDicom::from_bytes(make_test_dicom()).unwrap();
        assert_eq!(
            parsed.instance.blob_key,
            "1.2.3.4.5/1.2.3.4.5.1/1.2.3.4.5.1.1"
        );
    }

    #[test]
    fn test_from_bytes_counters_are_zero() {
        let parsed = ParsedDicom::from_bytes(make_test_dicom()).unwrap();
        assert_eq!(parsed.study.num_series, 0);
        assert_eq!(parsed.study.num_instances, 0);
        assert_eq!(parsed.series.num_instances, 0);
    }

    #[test]
    fn test_from_bytes_transfer_syntax_populated() {
        let parsed = ParsedDicom::from_bytes(make_test_dicom()).unwrap();
        assert!(!parsed.transfer_syntax_uid.is_empty());
        assert_eq!(
            parsed.instance.transfer_syntax.as_deref(),
            Some(parsed.transfer_syntax_uid.as_str())
        );
    }

    #[test]
    fn test_from_bytes_metadata_is_object() {
        let parsed = ParsedDicom::from_bytes(make_test_dicom()).unwrap();
        assert!(parsed.study.metadata.as_value().is_object());
    }

    #[test]
    fn test_from_bytes_encoded_bytes_preserved() {
        let original = make_test_dicom();
        let parsed = ParsedDicom::from_bytes(original.clone()).unwrap();
        assert_eq!(parsed.encoded_bytes, original);
    }

    #[test]
    fn test_from_bytes_invalid_data_returns_error() {
        let bad = Bytes::from_static(b"this is not a DICOM file at all");
        assert!(ParsedDicom::from_bytes(bad).is_err());
    }

    #[test]
    fn test_from_bytes_missing_study_uid_returns_error() {
        let mut ds = DataSet::new();
        // Intentionally omit STUDY_INSTANCE_UID.
        ds.set_string(tags::SERIES_INSTANCE_UID, Vr::UI, "1.2.3");
        ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, "1.2.3.1");
        ds.set_string(tags::SOP_CLASS_UID, Vr::UI, "1.2.840.10008.5.1.4.1.1.2");
        let ff = FileFormat::from_dataset("1.2.840.10008.5.1.4.1.1.2", "1.2.3.1", ds);
        let mut buf = Vec::new();
        DicomWriter::new(std::io::Cursor::new(&mut buf))
            .write_file(&ff)
            .unwrap();
        let err = ParsedDicom::from_bytes(Bytes::from(buf));
        assert!(err.is_err());
    }
}
