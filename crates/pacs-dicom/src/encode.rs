use dicom_toolkit_data::{DataSet, DicomWriter};
use pacs_core::PacsResult;

/// Encodes `dataset` to raw bytes using the given Transfer Syntax UID.
///
/// The resulting bytes represent a raw DICOM dataset (without a File Meta
/// Information header).  Use [`DicomWriter::write_file`] via the
/// [`dicom_toolkit_data`] crate directly if you need a Part 10 file.
///
/// # Errors
///
/// Returns [`pacs_core::PacsError::DicomParse`] if the toolkit writer fails
/// (e.g. the transfer syntax is unsupported or the dataset is malformed).
pub fn encode_dataset(dataset: &DataSet, ts_uid: &str) -> PacsResult<Vec<u8>> {
    let mut buf = Vec::new();
    let mut writer = DicomWriter::new(std::io::Cursor::new(&mut buf));
    writer
        .write_dataset(dataset, ts_uid)
        .map_err(|e| pacs_core::PacsError::DicomParse(e.to_string()))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_toolkit_data::DataSet;
    use dicom_toolkit_dict::{tags, Vr};

    /// Explicit Little Endian Uncompressed transfer syntax UID.
    const ELE_TS: &str = "1.2.840.10008.1.2.1";

    #[test]
    fn test_encode_dataset_produces_nonempty_bytes() {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_ID, Vr::LO, "TEST001");
        ds.set_string(tags::MODALITY, Vr::CS, "CT");
        let result = encode_dataset(&ds, ELE_TS);
        assert!(result.is_ok(), "encode failed: {:?}", result.err());
        assert!(!result.unwrap().is_empty());
    }

    #[test]
    fn test_encode_empty_dataset_succeeds() {
        let ds = DataSet::new();
        let result = encode_dataset(&ds, ELE_TS);
        assert!(result.is_ok());
    }

    #[test]
    fn test_encode_dataset_can_be_read_back() {
        use dicom_toolkit_data::DicomReader;

        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_ID, Vr::LO, "ROUNDTRIP");
        let encoded = encode_dataset(&ds, ELE_TS).unwrap();

        let mut reader = DicomReader::new(std::io::Cursor::new(encoded.as_slice()));
        let roundtripped = reader.read_dataset(ELE_TS).unwrap();
        assert_eq!(
            roundtripped.get_string(tags::PATIENT_ID),
            Some("ROUNDTRIP")
        );
    }
}
