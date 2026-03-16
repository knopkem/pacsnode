use std::{io::Cursor, sync::LazyLock};

use bytes::Bytes;
use dicom_toolkit_codec::{
    jp2k::Jp2kCodec, jpeg::JpegParams, jpeg_ls::JpegLsCodec, rle_encode_frame,
};
use dicom_toolkit_data::{DicomReader, Element, FileFormat, PixelData, Value};
use dicom_toolkit_dict::{tags, ts::transfer_syntaxes, Tag, Vr};
use dicom_toolkit_image::{frame_to_png_bytes, DicomImage};

use crate::error::{BulkDataValue, DicomError};

static SUPPORTED_RETRIEVE_TRANSFER_SYNTAXES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut syntaxes = vec![
        transfer_syntaxes::IMPLICIT_VR_LITTLE_ENDIAN.uid,
        transfer_syntaxes::EXPLICIT_VR_LITTLE_ENDIAN.uid,
        transfer_syntaxes::EXPLICIT_VR_BIG_ENDIAN.uid,
        transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid,
    ];

    for &syntax in dicom_toolkit_codec::supported_transfer_syntaxes() {
        if !syntaxes.contains(&syntax) {
            syntaxes.push(syntax);
        }
    }

    syntaxes
});

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
    decoded_frames_for_processing(&file, frame_numbers)?
        .into_iter()
        .map(|frame_bytes| {
            let dataset = dataset_for_single_frame(&file, frame_bytes)?;
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

/// Returns the transfer syntax UIDs pacsnode can retrieve directly or transcode into.
///
/// This includes the native DICOM transfer syntaxes, Deflated Explicit VR Little
/// Endian, and the compressed syntaxes that `dicom-toolkit-rs` can decode.
///
/// # Example
///
/// ```rust
/// use dicom_toolkit_dict::ts::transfer_syntaxes;
/// use pacs_dicom::supported_retrieve_transfer_syntaxes;
///
/// assert!(supported_retrieve_transfer_syntaxes()
///     .contains(&transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid));
/// ```
pub fn supported_retrieve_transfer_syntaxes() -> &'static [&'static str] {
    SUPPORTED_RETRIEVE_TRANSFER_SYNTAXES.as_slice()
}

/// Returns `true` when pacsnode can serve the requested transfer syntax on retrieve.
///
/// # Example
///
/// ```rust
/// use pacs_dicom::supports_retrieve_transfer_syntax;
///
/// assert!(supports_retrieve_transfer_syntax("1.2.840.10008.1.2.1"));
/// assert!(!supports_retrieve_transfer_syntax("1.2.3.4.5"));
/// ```
pub fn supports_retrieve_transfer_syntax(ts_uid: &str) -> bool {
    supported_retrieve_transfer_syntaxes().contains(&ts_uid)
}

/// Transcodes a DICOM Part 10 file to the requested transfer syntax for retrieval.
///
/// If the source file already uses `target_ts_uid`, the original bytes are returned.
/// For compressed syntaxes, pacsnode currently supports output to RLE, JPEG
/// Baseline/Extended, JPEG-LS Lossless, JPEG 2000 Lossless, and JPEG 2000.
///
/// # Example
///
/// ```no_run
/// use bytes::Bytes;
/// use dicom_toolkit_dict::ts::transfer_syntaxes;
/// use pacs_dicom::transcode_part10;
///
/// # fn example(data: Bytes) -> Result<(), pacs_dicom::DicomError> {
/// let _ = transcode_part10(data, transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid)?;
/// # Ok(())
/// # }
/// ```
pub fn transcode_part10(data: Bytes, target_ts_uid: &str) -> Result<Bytes, DicomError> {
    if !supports_retrieve_transfer_syntax(target_ts_uid) {
        return Err(DicomError::Unsupported {
            message: format!("transfer syntax {target_ts_uid} is not supported for retrieve"),
        });
    }

    let file = read_file(data.clone())?;
    if file.meta.transfer_syntax_uid == target_ts_uid {
        return Ok(data);
    }

    let mut transcoded = file.clone();
    transcoded.meta.transfer_syntax_uid = target_ts_uid.to_owned();

    if file.dataset.get(tags::PIXEL_DATA).is_none() {
        if is_compressed_output_transfer_syntax(target_ts_uid) {
            return Err(DicomError::Unsupported {
                message: format!(
                    "cannot transcode to compressed transfer syntax {target_ts_uid} without PixelData"
                ),
            });
        }
        return write_file(transcoded);
    }

    let bits_allocated = required_u16(
        &file.dataset,
        tags::BITS_ALLOCATED,
        "BitsAllocated (0028,0100)",
    )?;
    let pixel_vr = pixel_data_vr(bits_allocated, target_ts_uid);
    let pixel_value = if is_native_transfer_syntax(target_ts_uid) {
        Value::PixelData(PixelData::Native {
            bytes: native_pixel_bytes_for_target(&file, target_ts_uid)?,
        })
    } else {
        Value::PixelData(encode_frames_for_transfer_syntax(&file, target_ts_uid)?)
    };
    transcoded
        .dataset
        .insert(Element::new(tags::PIXEL_DATA, pixel_vr, pixel_value));

    write_file(transcoded)
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
        PixelData::Native { bytes } => raw_native_frames(file, bytes, &frame_indices),
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

fn raw_native_frames(
    file: &FileFormat,
    bytes: &[u8],
    frame_indices: &[u32],
) -> Result<Vec<Bytes>, DicomError> {
    let frame_size = frame_size_bytes(file)?;
    frame_indices
        .iter()
        .copied()
        .map(|frame_index| slice_frame(bytes, frame_size, frame_index, total_frames(file)))
        .collect()
}

fn decoded_frames_for_processing(
    file: &FileFormat,
    frame_numbers: &[u32],
) -> Result<Vec<Bytes>, DicomError> {
    let frame_indices = validate_frame_numbers(total_frames(file), frame_numbers)?;
    let pixel_data = pixel_data(&file.dataset)?;

    match pixel_data {
        PixelData::Native { bytes } => {
            let frame_size = frame_size_bytes(file)?;
            let bits_allocated = required_u16(
                &file.dataset,
                tags::BITS_ALLOCATED,
                "BitsAllocated (0028,0100)",
            )?;

            frame_indices
                .into_iter()
                .map(|frame_index| {
                    let frame = slice_frame(bytes, frame_size, frame_index, total_frames(file))?;
                    normalize_native_frame(
                        frame,
                        file.meta.transfer_syntax_uid.as_str(),
                        bits_allocated,
                    )
                })
                .collect()
        }
        PixelData::Encapsulated { .. } => extract_frames_from_file(file, frame_numbers),
    }
}

fn frame_size_bytes(file: &FileFormat) -> Result<usize, DicomError> {
    let rows = usize::from(required_u16(&file.dataset, tags::ROWS, "Rows (0028,0010)")?);
    let cols = usize::from(required_u16(
        &file.dataset,
        tags::COLUMNS,
        "Columns (0028,0011)",
    )?);
    let bits_allocated = usize::from(required_u16(
        &file.dataset,
        tags::BITS_ALLOCATED,
        "BitsAllocated (0028,0100)",
    )?);
    let samples = usize::from(file.dataset.get_u16(tags::SAMPLES_PER_PIXEL).unwrap_or(1));
    let pixels = rows
        .checked_mul(cols)
        .and_then(|value| value.checked_mul(samples))
        .ok_or_else(|| DicomError::Unsupported {
            message: "frame dimensions overflow byte-size calculation".into(),
        })?;
    let total_bits = pixels
        .checked_mul(bits_allocated)
        .ok_or_else(|| DicomError::Unsupported {
            message: "frame bit-size overflow".into(),
        })?;
    Ok(total_bits.div_ceil(8))
}

fn slice_frame(
    bytes: &[u8],
    frame_size: usize,
    frame_index: u32,
    total_frames: u32,
) -> Result<Bytes, DicomError> {
    let start = usize::try_from(frame_index)
        .ok()
        .and_then(|index| index.checked_mul(frame_size))
        .ok_or_else(|| DicomError::Unsupported {
            message: "frame offset overflow".into(),
        })?;
    let end = start
        .checked_add(frame_size)
        .ok_or_else(|| DicomError::Unsupported {
            message: "frame end overflow".into(),
        })?;
    if end > bytes.len() {
        return Err(DicomError::InvalidFrame {
            requested: frame_index + 1,
            available: total_frames,
        });
    }
    Ok(Bytes::copy_from_slice(&bytes[start..end]))
}

fn normalize_native_frame(
    frame: Bytes,
    source_ts_uid: &str,
    bits_allocated: u16,
) -> Result<Bytes, DicomError> {
    if !is_big_endian_transfer_syntax(source_ts_uid) {
        return Ok(frame);
    }

    let mut normalized = frame.to_vec();
    swap_pixel_endianness(&mut normalized, bits_allocated)?;
    Ok(Bytes::from(normalized))
}

fn native_pixel_bytes_for_target(
    file: &FileFormat,
    target_ts_uid: &str,
) -> Result<Vec<u8>, DicomError> {
    let frame_numbers: Vec<u32> = (1..=total_frames(file)).collect();
    let mut native = decoded_frames_for_processing(file, &frame_numbers)?
        .into_iter()
        .flat_map(|frame| frame.into_iter())
        .collect::<Vec<_>>();

    if is_big_endian_transfer_syntax(target_ts_uid) {
        let bits_allocated = required_u16(
            &file.dataset,
            tags::BITS_ALLOCATED,
            "BitsAllocated (0028,0100)",
        )?;
        swap_pixel_endianness(&mut native, bits_allocated)?;
    }

    Ok(native)
}

fn encode_frames_for_transfer_syntax(
    file: &FileFormat,
    target_ts_uid: &str,
) -> Result<PixelData, DicomError> {
    if !is_compressed_output_transfer_syntax(target_ts_uid) {
        return Err(DicomError::Unsupported {
            message: format!("transfer syntax {target_ts_uid} is not supported for encoding"),
        });
    }

    let rows = required_u16(&file.dataset, tags::ROWS, "Rows (0028,0010)")?;
    let cols = required_u16(&file.dataset, tags::COLUMNS, "Columns (0028,0011)")?;
    let bits_allocated = required_u16(
        &file.dataset,
        tags::BITS_ALLOCATED,
        "BitsAllocated (0028,0100)",
    )?;
    let bits_stored = file
        .dataset
        .get_u16(tags::BITS_STORED)
        .unwrap_or(bits_allocated);
    let samples = file.dataset.get_u16(tags::SAMPLES_PER_PIXEL).unwrap_or(1);
    let bits_allocated_u8 = u8::try_from(bits_allocated).map_err(|_| DicomError::Unsupported {
        message: format!("BitsAllocated {bits_allocated} exceeds codec limits"),
    })?;
    let bits_stored_u8 = u8::try_from(bits_stored).map_err(|_| DicomError::Unsupported {
        message: format!("BitsStored {bits_stored} exceeds codec limits"),
    })?;
    let samples_u8 = u8::try_from(samples).map_err(|_| DicomError::Unsupported {
        message: format!("SamplesPerPixel {samples} exceeds codec limits"),
    })?;
    let frame_numbers: Vec<u32> = (1..=total_frames(file)).collect();
    let frames = decoded_frames_for_processing(file, &frame_numbers)?;

    let mut offset_table = Vec::with_capacity(frames.len());
    let mut fragments = Vec::with_capacity(frames.len());
    let mut offset = 0u32;
    for frame in frames {
        let encoded = encode_frame_for_transfer_syntax(
            target_ts_uid,
            frame.as_ref(),
            rows,
            cols,
            samples_u8,
            bits_allocated_u8,
            bits_stored_u8,
        )?;
        let encoded_len = u32::try_from(encoded.len()).map_err(|_| DicomError::Unsupported {
            message: "encoded frame exceeds DICOM offset-table limits".into(),
        })?;
        offset_table.push(offset);
        offset = offset
            .checked_add(encoded_len)
            .ok_or_else(|| DicomError::Unsupported {
                message: "offset table overflow for encapsulated PixelData".into(),
            })?;
        fragments.push(encoded);
    }

    Ok(PixelData::Encapsulated {
        offset_table,
        fragments,
    })
}

fn encode_frame_for_transfer_syntax(
    target_ts_uid: &str,
    frame: &[u8],
    rows: u16,
    cols: u16,
    samples_per_pixel: u8,
    bits_allocated: u8,
    bits_stored: u8,
) -> Result<Vec<u8>, DicomError> {
    match target_ts_uid {
        uid if uid == transfer_syntaxes::RLE_LOSSLESS.uid => {
            rle_encode_frame(frame, rows, cols, samples_per_pixel, bits_allocated)
                .map_err(|e| DicomError::Toolkit(e.to_string()))
        }
        uid if uid == transfer_syntaxes::JPEG_BASELINE.uid
            || uid == transfer_syntaxes::JPEG_EXTENDED.uid =>
        {
            dicom_toolkit_codec::jpeg::encode_jpeg(
                frame,
                cols,
                rows,
                samples_per_pixel,
                &JpegParams::default(),
            )
            .map_err(|e| DicomError::Toolkit(e.to_string()))
        }
        uid if uid == transfer_syntaxes::JPEG_LS_LOSSLESS.uid => JpegLsCodec::encode_frame(
            frame,
            u32::from(cols),
            u32::from(rows),
            bits_stored,
            samples_per_pixel,
            0,
        )
        .map_err(|e| DicomError::Toolkit(e.to_string())),
        uid if uid == transfer_syntaxes::JPEG_2000_LOSSLESS.uid => Jp2kCodec::encode_frame(
            frame,
            u32::from(cols),
            u32::from(rows),
            bits_stored,
            samples_per_pixel,
            true,
        )
        .map_err(|e| DicomError::Toolkit(e.to_string())),
        uid if uid == transfer_syntaxes::JPEG_2000.uid => Jp2kCodec::encode_frame(
            frame,
            u32::from(cols),
            u32::from(rows),
            bits_stored,
            samples_per_pixel,
            false,
        )
        .map_err(|e| DicomError::Toolkit(e.to_string())),
        other => Err(DicomError::Unsupported {
            message: format!("transfer syntax {other} is not supported for encoding"),
        }),
    }
}

fn pixel_data_vr(bits_allocated: u16, target_ts_uid: &str) -> Vr {
    if is_compressed_output_transfer_syntax(target_ts_uid) || bits_allocated <= 8 {
        Vr::OB
    } else {
        Vr::OW
    }
}

fn is_native_transfer_syntax(ts_uid: &str) -> bool {
    matches!(
        ts_uid,
        uid if uid == transfer_syntaxes::IMPLICIT_VR_LITTLE_ENDIAN.uid
            || uid == transfer_syntaxes::EXPLICIT_VR_LITTLE_ENDIAN.uid
            || uid == transfer_syntaxes::EXPLICIT_VR_BIG_ENDIAN.uid
            || uid == transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid
    )
}

fn is_big_endian_transfer_syntax(ts_uid: &str) -> bool {
    ts_uid == transfer_syntaxes::EXPLICIT_VR_BIG_ENDIAN.uid
}

fn is_compressed_output_transfer_syntax(ts_uid: &str) -> bool {
    matches!(
        ts_uid,
        uid if uid == transfer_syntaxes::RLE_LOSSLESS.uid
            || uid == transfer_syntaxes::JPEG_BASELINE.uid
            || uid == transfer_syntaxes::JPEG_EXTENDED.uid
            || uid == transfer_syntaxes::JPEG_LS_LOSSLESS.uid
            || uid == transfer_syntaxes::JPEG_2000_LOSSLESS.uid
            || uid == transfer_syntaxes::JPEG_2000.uid
    )
}

fn swap_pixel_endianness(bytes: &mut [u8], bits_allocated: u16) -> Result<(), DicomError> {
    if bits_allocated <= 8 {
        return Ok(());
    }
    if !bits_allocated.is_multiple_of(8) {
        return Err(DicomError::Unsupported {
            message: format!(
                "cannot byte-swap native PixelData with BitsAllocated={bits_allocated}"
            ),
        });
    }

    let sample_bytes = usize::from(bits_allocated / 8);
    if !bytes.len().is_multiple_of(sample_bytes) {
        return Err(DicomError::Unsupported {
            message: "native PixelData byte length does not align with sample width".into(),
        });
    }

    for chunk in bytes.chunks_exact_mut(sample_bytes) {
        chunk.reverse();
    }

    Ok(())
}

fn write_file(file: FileFormat) -> Result<Bytes, DicomError> {
    let mut buf = Vec::new();
    dicom_toolkit_data::DicomWriter::new(Cursor::new(&mut buf))
        .write_file(&file)
        .map_err(|e| DicomError::Toolkit(e.to_string()))?;
    Ok(Bytes::from(buf))
}

fn dataset_for_single_frame(
    file: &FileFormat,
    frame_bytes: Bytes,
) -> Result<dicom_toolkit_data::DataSet, DicomError> {
    let mut dataset = file.dataset.clone();
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
    use dicom_toolkit_data::{
        DataSet, DicomReader, DicomWriter, Element, FileFormat, PixelData, Value,
    };
    use dicom_toolkit_dict::{tags, ts::transfer_syntaxes, Vr};

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

    fn make_singleframe_16bit_dicom() -> Bytes {
        let mut ds = DataSet::new();
        ds.set_string(tags::STUDY_INSTANCE_UID, Vr::UI, "1.2.3.16");
        ds.set_string(tags::SERIES_INSTANCE_UID, Vr::UI, "1.2.3.16.4");
        ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, "1.2.3.16.4.5");
        ds.set_string(tags::SOP_CLASS_UID, Vr::UI, "1.2.840.10008.5.1.4.1.1.2");
        ds.set_u16(tags::ROWS, 1);
        ds.set_u16(tags::COLUMNS, 2);
        ds.set_u16(tags::SAMPLES_PER_PIXEL, 1);
        ds.set_u16(tags::BITS_ALLOCATED, 16);
        ds.set_u16(tags::BITS_STORED, 16);
        ds.set_u16(tags::HIGH_BIT, 15);
        ds.set_u16(tags::PIXEL_REPRESENTATION, 0);
        ds.set_string(tags::PHOTOMETRIC_INTERPRETATION, Vr::CS, "MONOCHROME2");
        ds.insert(Element::new(
            tags::PIXEL_DATA,
            Vr::OW,
            Value::PixelData(PixelData::Native {
                bytes: vec![0x01, 0x00, 0x02, 0x00],
            }),
        ));

        let ff = FileFormat::from_dataset("1.2.840.10008.5.1.4.1.1.2", "1.2.3.16.4.5", ds);
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

    #[test]
    fn supported_retrieve_transfer_syntaxes_include_codec_uids() {
        let syntaxes = supported_retrieve_transfer_syntaxes();
        assert!(syntaxes.contains(&transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid));
        assert!(syntaxes.contains(&transfer_syntaxes::JPEG_BASELINE.uid));
        assert!(syntaxes.contains(&transfer_syntaxes::JPEG_LOSSLESS.uid));
        assert!(syntaxes.contains(&transfer_syntaxes::JPEG_2000_LOSSLESS.uid));
        assert!(syntaxes.contains(&transfer_syntaxes::RLE_LOSSLESS.uid));
    }

    #[test]
    fn transcode_part10_to_deflated_updates_file_meta() {
        let transcoded = transcode_part10(
            make_multiframe_dicom(),
            transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid,
        )
        .unwrap();
        let file = DicomReader::new(Cursor::new(transcoded.as_ref()))
            .read_file()
            .unwrap();
        assert_eq!(
            file.meta.transfer_syntax_uid,
            transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid
        );
        let frames = extract_frames(transcoded, &[1, 2]).unwrap();
        assert_eq!(frames[0], Bytes::from_static(&[0x11, 0x22]));
        assert_eq!(frames[1], Bytes::from_static(&[0x33, 0x44]));
    }

    #[test]
    fn transcode_part10_to_rle_roundtrips_frames() {
        let transcoded =
            transcode_part10(make_multiframe_dicom(), transfer_syntaxes::RLE_LOSSLESS.uid).unwrap();
        let file = DicomReader::new(Cursor::new(transcoded.as_ref()))
            .read_file()
            .unwrap();
        assert_eq!(
            file.meta.transfer_syntax_uid,
            transfer_syntaxes::RLE_LOSSLESS.uid
        );
        let frames = extract_frames(transcoded, &[1, 2]).unwrap();
        assert_eq!(frames[0], Bytes::from_static(&[0x11, 0x22]));
        assert_eq!(frames[1], Bytes::from_static(&[0x33, 0x44]));
    }

    #[test]
    fn transcode_part10_to_jpeg_baseline_decodes_frames() {
        let transcoded = transcode_part10(
            make_multiframe_dicom(),
            transfer_syntaxes::JPEG_BASELINE.uid,
        )
        .unwrap();
        let file = DicomReader::new(Cursor::new(transcoded.as_ref()))
            .read_file()
            .unwrap();
        assert_eq!(
            file.meta.transfer_syntax_uid,
            transfer_syntaxes::JPEG_BASELINE.uid
        );
        let frames = extract_frames(transcoded.clone(), &[1, 2]).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].len(), 2);
        assert_eq!(frames[1].len(), 2);
        let pngs = render_frames_png(transcoded, &[1]).unwrap();
        assert!(pngs[0].starts_with(&[0x89, 0x50, 0x4E, 0x47]));
    }

    #[test]
    fn transcode_part10_to_jpeg_2000_lossless_roundtrips_frames() {
        let transcoded = transcode_part10(
            make_multiframe_dicom(),
            transfer_syntaxes::JPEG_2000_LOSSLESS.uid,
        )
        .unwrap();
        let file = DicomReader::new(Cursor::new(transcoded.as_ref()))
            .read_file()
            .unwrap();
        assert_eq!(
            file.meta.transfer_syntax_uid,
            transfer_syntaxes::JPEG_2000_LOSSLESS.uid
        );
        let frames = extract_frames(transcoded, &[1, 2]).unwrap();
        assert_eq!(frames[0], Bytes::from_static(&[0x11, 0x22]));
        assert_eq!(frames[1], Bytes::from_static(&[0x33, 0x44]));
    }

    #[test]
    fn transcode_part10_rejects_jpeg_lossless_output() {
        let error = transcode_part10(
            make_multiframe_dicom(),
            transfer_syntaxes::JPEG_LOSSLESS.uid,
        )
        .unwrap_err();
        assert!(matches!(error, DicomError::Unsupported { .. }));
    }

    #[test]
    fn transcode_part10_to_big_endian_swaps_native_bytes() {
        let transcoded = transcode_part10(
            make_singleframe_16bit_dicom(),
            transfer_syntaxes::EXPLICIT_VR_BIG_ENDIAN.uid,
        )
        .unwrap();
        let file = DicomReader::new(Cursor::new(transcoded.as_ref()))
            .read_file()
            .unwrap();
        assert_eq!(
            file.meta.transfer_syntax_uid,
            transfer_syntaxes::EXPLICIT_VR_BIG_ENDIAN.uid
        );
        let frames = extract_frames(transcoded.clone(), &[1]).unwrap();
        assert_eq!(frames[0], Bytes::from_static(&[0x00, 0x01, 0x00, 0x02]));
        let pngs = render_frames_png(transcoded, &[1]).unwrap();
        assert!(pngs[0].starts_with(&[0x89, 0x50, 0x4E, 0x47]));
    }
}
