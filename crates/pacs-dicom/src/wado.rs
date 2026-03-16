use std::io::Cursor;

use bytes::Bytes;
use dicom_toolkit_data::{DicomReader, Element, FileFormat, PixelData, Value};
use dicom_toolkit_dict::{tags, Tag, Vr};
use dicom_toolkit_image::{frame_to_png_bytes, DicomImage};

use crate::error::{BulkDataValue, DicomError};

/// Extracts one or more DICOM frames as raw native pixel bytes.
///
/// Frame numbers are **1-based**, matching DICOMweb WADO-RS semantics.
///
/// # Example
///
/// ```no_run
/// use bytes::Bytes;
/// use pacs_dicom::extract_frames;
///
/// # fn example(instance_bytes: Bytes) -> Result<(), pacs_dicom::DicomError> {
/// let frames = extract_frames(instance_bytes, &[1])?;
/// assert_eq!(frames.len(), 1);
/// # Ok(())
/// # }
/// ```
pub fn extract_frames(data: Bytes, frame_numbers: &[u32]) -> Result<Vec<Bytes>, DicomError> {
    let file = read_file(data)?;
    extract_frames_from_file(&file, frame_numbers)
}

/// Renders one or more DICOM frames as PNG images.
///
/// Frame numbers are **1-based**, matching DICOMweb WADO-RS semantics.
///
/// # Example
///
/// ```no_run
/// use bytes::Bytes;
/// use pacs_dicom::render_frames_png;
///
/// # fn example(instance_bytes: Bytes) -> Result<(), pacs_dicom::DicomError> {
/// let pngs = render_frames_png(instance_bytes, &[1])?;
/// assert!(!pngs[0].is_empty());
/// # Ok(())
/// # }
/// ```
pub fn render_frames_png(data: Bytes, frame_numbers: &[u32]) -> Result<Vec<Bytes>, DicomError> {
    let file = read_file(data)?;
    validate_frame_numbers(total_frames(&file), frame_numbers)?
        .into_iter()
        .map(|frame_index| {
            let dataset = dataset_for_single_frame(&file, frame_index)?;
            let image = DicomImage::from_dataset(&dataset)
                .map_err(|e| DicomError::Toolkit(e.to_string()))?;
            frame_to_png_bytes(&image, 0)
                .map(Bytes::from)
                .map_err(|e| DicomError::Toolkit(e.to_string()))
        })
        .collect()
}

/// Extracts raw bulk data for a single top-level DICOM tag.
///
/// For native pixel data and other binary tags, this returns a single payload.
/// For encapsulated pixel data, this returns one payload per fragment.
///
/// # Example
///
/// ```no_run
/// use bytes::Bytes;
/// use dicom_toolkit_dict::tags;
/// use pacs_dicom::{extract_bulk_data, BulkDataValue};
///
/// # fn example(instance_bytes: Bytes) -> Result<(), pacs_dicom::DicomError> {
/// match extract_bulk_data(instance_bytes, tags::PIXEL_DATA)? {
///     BulkDataValue::Single(bytes) => assert!(!bytes.is_empty()),
///     BulkDataValue::Multipart(parts) => assert!(!parts.is_empty()),
/// }
/// # Ok(())
/// # }
/// ```
pub fn extract_bulk_data(data: Bytes, tag: Tag) -> Result<BulkDataValue, DicomError> {
    let file = read_file(data)?;
    let element = file.dataset.get(tag).ok_or(DicomError::MissingTag {
        tag: "BulkDataElement",
    })?;

    match &element.value {
        Value::U8(bytes) => Ok(BulkDataValue::Single(Bytes::copy_from_slice(bytes))),
        Value::U16(values) => Ok(BulkDataValue::Single(Bytes::from(encode_u16(values)))),
        Value::I16(values) => Ok(BulkDataValue::Single(Bytes::from(encode_i16(values)))),
        Value::U32(values) => Ok(BulkDataValue::Single(Bytes::from(encode_u32(values)))),
        Value::I32(values) => Ok(BulkDataValue::Single(Bytes::from(encode_i32(values)))),
        Value::U64(values) => Ok(BulkDataValue::Single(Bytes::from(encode_u64(values)))),
        Value::I64(values) => Ok(BulkDataValue::Single(Bytes::from(encode_i64(values)))),
        Value::F32(values) => Ok(BulkDataValue::Single(Bytes::from(encode_f32(values)))),
        Value::F64(values) => Ok(BulkDataValue::Single(Bytes::from(encode_f64(values)))),
        Value::PixelData(PixelData::Native { bytes }) => {
            Ok(BulkDataValue::Single(Bytes::copy_from_slice(bytes)))
        }
        Value::PixelData(PixelData::Encapsulated { fragments, .. }) => {
            Ok(BulkDataValue::Multipart(
                fragments
                    .iter()
                    .cloned()
                    .map(Bytes::from)
                    .collect::<Vec<_>>(),
            ))
        }
        _ => Err(DicomError::Unsupported {
            message: format!("tag {} does not contain bulk data", tag_hex(tag)),
        }),
    }
}

/// Parses a top-level bulk-data tag path like `7FE00010` into a DICOM [`Tag`].
///
/// # Example
///
/// ```rust
/// use pacs_dicom::parse_bulk_data_tag_path;
///
/// let tag = parse_bulk_data_tag_path("7FE00010").unwrap();
/// assert_eq!(format!("{:04X}{:04X}", tag.group, tag.element), "7FE00010");
/// ```
pub fn parse_bulk_data_tag_path(value: &str) -> Result<Tag, DicomError> {
    if value.len() != 8 || !value.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(DicomError::InvalidTagPath {
            value: value.to_owned(),
        });
    }

    let group = u16::from_str_radix(&value[..4], 16).map_err(|_| DicomError::InvalidTagPath {
        value: value.to_owned(),
    })?;
    let element = u16::from_str_radix(&value[4..], 16).map_err(|_| DicomError::InvalidTagPath {
        value: value.to_owned(),
    })?;

    Ok(Tag::new(group, element))
}

fn read_file(data: Bytes) -> Result<FileFormat, DicomError> {
    let mut reader = DicomReader::new(Cursor::new(data.as_ref()));
    reader
        .read_file()
        .map_err(|e| DicomError::Toolkit(e.to_string()))
}

fn extract_frames_from_file(
    file: &FileFormat,
    frame_numbers: &[u32],
) -> Result<Vec<Bytes>, DicomError> {
    let frame_indices = validate_frame_numbers(total_frames(file), frame_numbers)?;
    let pixel_data = pixel_data(&file.dataset)?;

    match pixel_data {
        PixelData::Native { .. } => {
            let image = DicomImage::from_dataset(&file.dataset)
                .map_err(|e| DicomError::Toolkit(e.to_string()))?;
            frame_indices
                .into_iter()
                .map(|frame_index| {
                    image
                        .frame_bytes(frame_index)
                        .map(Bytes::copy_from_slice)
                        .map_err(|e| DicomError::Toolkit(e.to_string()))
                })
                .collect()
        }
        PixelData::Encapsulated { .. } => {
            let rows = required_u16(&file.dataset, tags::ROWS, "Rows (0028,0010)")?;
            let cols = required_u16(&file.dataset, tags::COLUMNS, "Columns (0028,0011)")?;
            let bits_allocated = required_u16(
                &file.dataset,
                tags::BITS_ALLOCATED,
                "BitsAllocated (0028,0100)",
            )?;
            let samples = file.dataset.get_u16(tags::SAMPLES_PER_PIXEL).unwrap_or(1);
            let compressed_frames = encapsulated_frames(pixel_data, total_frames(file))?;

            frame_indices
                .into_iter()
                .map(|frame_index| {
                    let compressed = compressed_frames.get(frame_index as usize).ok_or(
                        DicomError::InvalidFrame {
                            requested: frame_index + 1,
                            available: total_frames(file),
                        },
                    )?;
                    dicom_toolkit_codec::decode_pixel_data(
                        &file.meta.transfer_syntax_uid,
                        compressed,
                        rows,
                        cols,
                        bits_allocated,
                        samples,
                    )
                    .map(Bytes::from)
                    .map_err(|e| DicomError::Toolkit(e.to_string()))
                })
                .collect()
        }
    }
}

fn dataset_for_single_frame(
    file: &FileFormat,
    frame_index: u32,
) -> Result<dicom_toolkit_data::DataSet, DicomError> {
    let mut dataset = file.dataset.clone();
    let frame_bytes = extract_frames_from_file(file, &[frame_index + 1])?
        .into_iter()
        .next()
        .ok_or(DicomError::InvalidFrame {
            requested: frame_index + 1,
            available: total_frames(file),
        })?;
    let pixel_element = dataset
        .get(tags::PIXEL_DATA)
        .ok_or(DicomError::MissingTag { tag: "PixelData" })?
        .clone();
    dataset.insert(Element::new(
        tags::PIXEL_DATA,
        pixel_element.vr,
        Value::PixelData(PixelData::Native {
            bytes: frame_bytes.to_vec(),
        }),
    ));
    dataset.set_string(tags::NUMBER_OF_FRAMES, Vr::IS, "1");
    Ok(dataset)
}

fn pixel_data(dataset: &dicom_toolkit_data::DataSet) -> Result<&PixelData, DicomError> {
    match &dataset
        .get(tags::PIXEL_DATA)
        .ok_or(DicomError::MissingTag { tag: "PixelData" })?
        .value
    {
        Value::PixelData(pixel_data) => Ok(pixel_data),
        _ => Err(DicomError::Unsupported {
            message: "PixelData is present but does not use a pixel-data value type".into(),
        }),
    }
}

fn total_frames(file: &FileFormat) -> u32 {
    file.dataset
        .get(tags::NUMBER_OF_FRAMES)
        .map(|element| element.value.to_display_string())
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(1)
}

fn validate_frame_numbers(
    total_frames: u32,
    frame_numbers: &[u32],
) -> Result<Vec<u32>, DicomError> {
    if frame_numbers.is_empty() {
        return Err(DicomError::Unsupported {
            message: "at least one frame number is required".into(),
        });
    }

    frame_numbers
        .iter()
        .copied()
        .map(|frame_number| {
            if frame_number == 0 || frame_number > total_frames {
                Err(DicomError::InvalidFrame {
                    requested: frame_number,
                    available: total_frames,
                })
            } else {
                Ok(frame_number - 1)
            }
        })
        .collect()
}

fn encapsulated_frames(
    pixel_data: &PixelData,
    total_frames: u32,
) -> Result<Vec<Vec<u8>>, DicomError> {
    let PixelData::Encapsulated {
        offset_table,
        fragments,
    } = pixel_data
    else {
        return Err(DicomError::Unsupported {
            message: "encapsulated frame extraction requires compressed PixelData".into(),
        });
    };

    if fragments.is_empty() {
        return Err(DicomError::Unsupported {
            message: "encapsulated PixelData does not contain any fragments".into(),
        });
    }

    if total_frames == 1 {
        return Ok(vec![fragments.concat()]);
    }

    if !offset_table.is_empty() && offset_table.len() >= total_frames as usize {
        let fragment_starts = fragment_starts(fragments);
        let total_len = fragments.iter().map(Vec::len).sum::<usize>();
        let mut frames = Vec::with_capacity(total_frames as usize);

        for index in 0..total_frames as usize {
            let start = offset_table[index] as usize;
            let end = offset_table
                .get(index + 1)
                .copied()
                .map(|value| value as usize)
                .unwrap_or(total_len);

            if start >= end || end > total_len {
                return Err(DicomError::Unsupported {
                    message: format!(
                        "encapsulated offset table is invalid for frame {}",
                        index + 1
                    ),
                });
            }

            let mut frame = Vec::with_capacity(end - start);
            for (fragment, fragment_start) in fragments.iter().zip(fragment_starts.iter().copied())
            {
                let fragment_end = fragment_start + fragment.len();
                if fragment_end <= start || fragment_start >= end {
                    continue;
                }
                let slice_start = start.saturating_sub(fragment_start);
                let slice_end = (end - fragment_start).min(fragment.len());
                frame.extend_from_slice(&fragment[slice_start..slice_end]);
            }

            if frame.is_empty() {
                return Err(DicomError::Unsupported {
                    message: format!(
                        "could not resolve compressed payload for frame {}",
                        index + 1
                    ),
                });
            }

            frames.push(frame);
        }

        return Ok(frames);
    }

    if fragments.len() == total_frames as usize {
        return Ok(fragments.clone());
    }

    Err(DicomError::Unsupported {
        message: format!(
            "unable to map {} encapsulated fragment(s) to {} frame(s)",
            fragments.len(),
            total_frames
        ),
    })
}

fn fragment_starts(fragments: &[Vec<u8>]) -> Vec<usize> {
    let mut offset = 0usize;
    fragments
        .iter()
        .map(|fragment| {
            let start = offset;
            offset += fragment.len();
            start
        })
        .collect()
}

fn required_u16(
    dataset: &dicom_toolkit_data::DataSet,
    tag: Tag,
    name: &'static str,
) -> Result<u16, DicomError> {
    dataset
        .get_u16(tag)
        .or_else(|| {
            dataset
                .get(tag)
                .map(|element| element.value.to_display_string())
                .and_then(|value| value.trim().parse::<u16>().ok())
        })
        .ok_or(DicomError::MissingTag { tag: name })
}

fn encode_u16(values: &[u16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn encode_i16(values: &[i16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn encode_u32(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn encode_i32(values: &[i32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn encode_u64(values: &[u64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn encode_i64(values: &[i64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn encode_f32(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn encode_f64(values: &[f64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn tag_hex(tag: Tag) -> String {
    format!("{:04X}{:04X}", tag.group, tag.element)
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use dicom_toolkit_data::{DataSet, DicomWriter, Element, FileFormat, PixelData, Value};
    use dicom_toolkit_dict::{tags, Vr};

    use super::*;

    fn make_multiframe_dicom() -> Bytes {
        let mut ds = DataSet::new();
        ds.set_string(tags::STUDY_INSTANCE_UID, Vr::UI, "1.2.3");
        ds.set_string(tags::SERIES_INSTANCE_UID, Vr::UI, "1.2.3.4");
        ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, "1.2.3.4.5");
        ds.set_string(tags::SOP_CLASS_UID, Vr::UI, "1.2.840.10008.5.1.4.1.1.2");
        ds.set_u16(tags::ROWS, 1);
        ds.set_u16(tags::COLUMNS, 2);
        ds.set_u16(tags::SAMPLES_PER_PIXEL, 1);
        ds.set_u16(tags::BITS_ALLOCATED, 8);
        ds.set_u16(tags::BITS_STORED, 8);
        ds.set_u16(tags::HIGH_BIT, 7);
        ds.set_u16(tags::PIXEL_REPRESENTATION, 0);
        ds.set_string(tags::PHOTOMETRIC_INTERPRETATION, Vr::CS, "MONOCHROME2");
        ds.set_string(tags::NUMBER_OF_FRAMES, Vr::IS, "2");
        ds.insert(Element::new(
            tags::PIXEL_DATA,
            Vr::OB,
            Value::PixelData(PixelData::Native {
                bytes: vec![0x11, 0x22, 0x33, 0x44],
            }),
        ));

        let ff = FileFormat::from_dataset("1.2.840.10008.5.1.4.1.1.2", "1.2.3.4.5", ds);
        let mut buf = Vec::new();
        DicomWriter::new(Cursor::new(&mut buf))
            .write_file(&ff)
            .unwrap();
        Bytes::from(buf)
    }

    #[test]
    fn extract_frames_returns_requested_native_frames() {
        let frames = extract_frames(make_multiframe_dicom(), &[1, 2]).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], Bytes::from_static(&[0x11, 0x22]));
        assert_eq!(frames[1], Bytes::from_static(&[0x33, 0x44]));
    }

    #[test]
    fn extract_frames_rejects_out_of_range_frame() {
        let error = extract_frames(make_multiframe_dicom(), &[3]).unwrap_err();
        assert!(matches!(
            error,
            DicomError::InvalidFrame {
                requested: 3,
                available: 2
            }
        ));
    }

    #[test]
    fn render_frames_png_returns_png_bytes() {
        let pngs = render_frames_png(make_multiframe_dicom(), &[1]).unwrap();
        assert_eq!(pngs.len(), 1);
        assert!(pngs[0].starts_with(&[0x89, 0x50, 0x4E, 0x47]));
    }

    #[test]
    fn extract_bulk_data_returns_native_pixel_payload() {
        let bulk_data = extract_bulk_data(make_multiframe_dicom(), tags::PIXEL_DATA).unwrap();
        assert_eq!(
            bulk_data,
            BulkDataValue::Single(Bytes::from_static(&[0x11, 0x22, 0x33, 0x44]))
        );
    }

    #[test]
    fn parse_bulk_data_tag_path_accepts_hex() {
        let tag = parse_bulk_data_tag_path("7FE00010").unwrap();
        assert_eq!(tag, tags::PIXEL_DATA);
    }
}
