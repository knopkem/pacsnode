use std::{io::Cursor, sync::LazyLock};

use bytes::Bytes;
use dicom_toolkit_codec::{
    can_encode,
    jp2k::Jp2kCodec,
    jpeg::{encode_jpeg_lossless, JpegParams},
    jpeg_ls::JpegLsCodec,
    rle_encode_frame, supported_encode_transfer_syntaxes,
};
use dicom_toolkit_data::{
    element_value_bytes, encapsulated_frames, encapsulated_pixel_data_from_frames,
    parse_attribute_path, AttributePathSegment, DataSet, DicomReader, Element, FileFormat,
    PixelData, Value,
};
use dicom_toolkit_dict::{tags, ts::transfer_syntaxes, Tag, Vr};
use dicom_toolkit_image::{render_frame_u8, DicomImage, RenderedFrameOptions, RenderedRegion};
use jpeg_encoder::{ColorType as JpegColorType, Encoder as JpegEncoder};
use pacs_core::DicomJson;
use png::{BitDepth as PngBitDepth, ColorType as PngColorType, Encoder as PngEncoder};

use crate::error::{BulkDataValue, DicomError};

static SUPPORTED_RETRIEVE_TRANSFER_SYNTAXES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut syntaxes = vec![
        transfer_syntaxes::IMPLICIT_VR_LITTLE_ENDIAN.uid,
        transfer_syntaxes::EXPLICIT_VR_LITTLE_ENDIAN.uid,
        transfer_syntaxes::EXPLICIT_VR_BIG_ENDIAN.uid,
        transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid,
    ];

    for &syntax in supported_encode_transfer_syntaxes() {
        if is_supported_output_transfer_syntax(syntax) && !syntaxes.contains(&syntax) {
            syntaxes.push(syntax);
        }
    }

    syntaxes
});

/// Rendered media types supported by pacsnode's WADO rendered endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderedMediaType {
    /// Portable Network Graphics output.
    Png,
    /// JPEG output with an explicit encoder quality in the range `1..=100`.
    Jpeg { quality: u8 },
}

impl RenderedMediaType {
    /// Returns the HTTP content type associated with this rendered media type.
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg { .. } => "image/jpeg",
        }
    }
}

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

/// Extracts one or more DICOM frames in their stored representation.
///
/// Native pixel data is returned unchanged per frame. Encapsulated pixel data is
/// returned as the stored compressed frame payload without decoding.
pub fn extract_stored_frames(data: Bytes, frame_numbers: &[u32]) -> Result<Vec<Bytes>, DicomError> {
    let file = read_file(data)?;
    extract_stored_frames_from_file(&file, frame_numbers)
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
    render_frames_with_options(
        data,
        frame_numbers,
        RenderedMediaType::Png,
        &RenderedFrameOptions::default(),
    )
}

/// Renders one or more DICOM frames in the requested media type with optional
/// rendered-image transforms.
///
/// Frame numbers are **1-based**, matching DICOMweb WADO-RS semantics. The
/// `frame` field in `options` is ignored; the caller-selected `frame_numbers`
/// always determine which frame(s) are rendered.
///
/// # Example
///
/// ```no_run
/// use bytes::Bytes;
/// use pacs_dicom::{render_frames_with_options, RenderedFrameOptions, RenderedMediaType};
///
/// # fn example(instance_bytes: Bytes) -> Result<(), pacs_dicom::DicomError> {
/// let options = RenderedFrameOptions {
///     rows: Some(256),
///     ..Default::default()
/// };
/// let jpeg = render_frames_with_options(
///     instance_bytes,
///     &[1],
///     RenderedMediaType::Jpeg { quality: 90 },
///     &options,
/// )?;
/// assert!(!jpeg[0].is_empty());
/// # Ok(())
/// # }
/// ```
pub fn render_frames_with_options(
    data: Bytes,
    frame_numbers: &[u32],
    media_type: RenderedMediaType,
    options: &RenderedFrameOptions,
) -> Result<Vec<Bytes>, DicomError> {
    let file = read_file(data)?;
    decoded_frames_for_processing(&file, frame_numbers)?
        .into_iter()
        .map(|frame_bytes| {
            let dataset = dataset_for_single_frame(&file, frame_bytes)?;
            render_dataset_frame(&dataset, media_type, options)
        })
        .collect()
}

/// Extracts raw bulk data for a single top-level DICOM tag.
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
    extract_bulk_data_path(data, &tag_hex(tag))
}

/// Extracts raw bulk data for an attribute path such as `7FE00010` or
/// `00082112/0/00111010`.
///
/// For native bulk data and other eligible binary VRs, this returns a single
/// payload. For encapsulated Pixel Data, this returns one compressed payload per
/// resolved frame.
///
/// # Example
///
/// ```no_run
/// use bytes::Bytes;
/// use pacs_dicom::{extract_bulk_data_path, BulkDataValue};
///
/// # fn example(instance_bytes: Bytes) -> Result<(), pacs_dicom::DicomError> {
/// let bulk = extract_bulk_data_path(instance_bytes, "7FE00010")?;
/// assert!(matches!(bulk, BulkDataValue::Single(_) | BulkDataValue::Multipart(_)));
/// # Ok(())
/// # }
/// ```
pub fn extract_bulk_data_path(data: Bytes, path: &str) -> Result<BulkDataValue, DicomError> {
    let path_segments = parse_bulk_data_path(path)?;
    let file = read_file(data)?;
    let (element, containing_dataset) =
        resolve_attribute_path_with_container(&file.dataset, &path_segments)?;
    extract_bulk_data_element(&file, containing_dataset, element)
}

/// Returns instance metadata with `BulkDataURI` entries injected for every
/// eligible binary attribute, including nested sequence paths.
///
/// The supplied `metadata` value is used as the base DICOM JSON structure, while
/// the raw Part 10 `data` is parsed to discover the precise attribute paths that
/// should expose bulk data.
pub fn metadata_with_bulk_data_uris<F>(
    metadata: &DicomJson,
    data: Bytes,
    resolve_uri: F,
) -> Result<DicomJson, DicomError>
where
    F: Fn(&str) -> String,
{
    let file = read_file(data)?;
    let mut patched = metadata.as_value().clone();
    patch_bulk_data_uris(&file.dataset, &mut patched, None, &resolve_uri)?;
    Ok(DicomJson::from(patched))
}

/// Parses a bulk-data attribute path such as `7FE00010` or
/// `00082112/0/00111010`.
///
/// # Example
///
/// ```rust
/// use pacs_dicom::parse_bulk_data_path;
///
/// let path = parse_bulk_data_path("00082112/0/00111010").unwrap();
/// assert_eq!(path.len(), 3);
/// ```
pub fn parse_bulk_data_path(value: &str) -> Result<Vec<AttributePathSegment>, DicomError> {
    parse_attribute_path(value).map_err(|_| DicomError::InvalidTagPath {
        value: value.to_owned(),
    })
}

/// Parses a top-level bulk-data tag path like `7FE00010` into a DICOM [`Tag`].
pub fn parse_bulk_data_tag_path(value: &str) -> Result<Tag, DicomError> {
    let path = parse_bulk_data_path(value)?;
    match path.as_slice() {
        [AttributePathSegment::Tag(tag)] => Ok(*tag),
        _ => Err(DicomError::InvalidTagPath {
            value: value.to_owned(),
        }),
    }
}

/// Returns the transfer syntax UIDs pacsnode can retrieve directly or transcode into.
///
/// This includes the native DICOM transfer syntaxes, Deflated Explicit VR Little
/// Endian, and the compressed syntaxes that pacsnode can actively encode for
/// retrieve responses.
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
/// Baseline/Extended, classic JPEG Lossless, JPEG-LS Lossless, JPEG 2000
/// Lossless, and JPEG 2000.
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

/// Prepares DIMSE-ready dataset bytes in the requested transfer syntax.
///
/// This helper accepts either a stored DICOM Part 10 object or a bare Explicit
/// VR Little Endian dataset and returns bytes suitable for a DIMSE C-STORE
/// sub-operation. File Meta Information is stripped from the output.
///
/// # Example
///
/// ```no_run
/// use bytes::Bytes;
/// use dicom_toolkit_dict::ts::transfer_syntaxes;
/// use pacs_dicom::prepare_dimse_dataset;
///
/// # fn example(data: Bytes) -> Result<(), pacs_dicom::DicomError> {
/// let _dataset = prepare_dimse_dataset(data, transfer_syntaxes::EXPLICIT_VR_LITTLE_ENDIAN.uid)?;
/// # Ok(())
/// # }
/// ```
pub fn prepare_dimse_dataset(data: Bytes, target_ts_uid: &str) -> Result<Bytes, DicomError> {
    if !supports_retrieve_transfer_syntax(target_ts_uid) {
        return Err(DicomError::Unsupported {
            message: format!("transfer syntax {target_ts_uid} is not supported for retrieve"),
        });
    }

    match read_file(data.clone()) {
        Ok(file) => {
            let part10 = if file.meta.transfer_syntax_uid == target_ts_uid {
                data
            } else {
                transcode_part10(data, target_ts_uid)?
            };
            let transcoded = read_file(part10)?;
            write_dataset_bytes(&transcoded.dataset, target_ts_uid)
        }
        Err(_) => dataset_bytes_to_target(data, target_ts_uid),
    }
}

fn read_file(data: Bytes) -> Result<FileFormat, DicomError> {
    let mut reader = DicomReader::new(Cursor::new(data.as_ref()));
    reader
        .read_file()
        .map_err(|e| DicomError::Toolkit(e.to_string()))
}

fn dataset_bytes_to_target(data: Bytes, target_ts_uid: &str) -> Result<Bytes, DicomError> {
    let dataset = DicomReader::new(Cursor::new(data.as_ref()))
        .read_dataset(transfer_syntaxes::EXPLICIT_VR_LITTLE_ENDIAN.uid)
        .map_err(|e| DicomError::Toolkit(e.to_string()))?;

    if target_ts_uid == transfer_syntaxes::EXPLICIT_VR_LITTLE_ENDIAN.uid {
        return write_dataset_bytes(&dataset, target_ts_uid);
    }

    let sop_class_uid = required_string(&dataset, tags::SOP_CLASS_UID, "SOPClassUID (0008,0016)")?;
    let sop_instance_uid = required_string(
        &dataset,
        tags::SOP_INSTANCE_UID,
        "SOPInstanceUID (0008,0018)",
    )?;
    let file = FileFormat::from_dataset(&sop_class_uid, &sop_instance_uid, dataset);
    let transcoded = transcode_part10(write_file(file)?, target_ts_uid)?;
    let transcoded_file = read_file(transcoded)?;
    write_dataset_bytes(&transcoded_file.dataset, target_ts_uid)
}

fn render_dataset_frame(
    dataset: &DataSet,
    media_type: RenderedMediaType,
    options: &RenderedFrameOptions,
) -> Result<Bytes, DicomError> {
    let image =
        DicomImage::from_dataset(dataset).map_err(|e| DicomError::Toolkit(e.to_string()))?;
    let mut render_options = options.clone();
    render_options.frame = 0;

    let rendered =
        render_frame_u8(&image, &render_options).map_err(|e| DicomError::Toolkit(e.to_string()))?;
    let (rows, columns) = rendered_dimensions(&image, &render_options)?;
    encode_rendered_pixels(
        &rendered,
        rows,
        columns,
        image.output_channels(),
        media_type,
    )
}

fn extract_bulk_data_element(
    file: &FileFormat,
    containing_dataset: &DataSet,
    element: &Element,
) -> Result<BulkDataValue, DicomError> {
    match &element.value {
        Value::PixelData(PixelData::Native { bytes }) => {
            Ok(BulkDataValue::Single(Bytes::copy_from_slice(bytes)))
        }
        Value::PixelData(pixel_data @ PixelData::Encapsulated { .. }) => {
            Ok(BulkDataValue::Multipart(
                encapsulated_frames(pixel_data, total_frames_from_dataset(containing_dataset))
                    .map_err(|e| DicomError::Toolkit(e.to_string()))?
                    .into_iter()
                    .map(Bytes::from)
                    .collect(),
            ))
        }
        _ if is_bulk_data_eligible(element) => {
            element_value_bytes(element, file.meta.transfer_syntax_uid.as_str())
                .map(Bytes::from)
                .map(BulkDataValue::Single)
                .map_err(|e| DicomError::Toolkit(e.to_string()))
        }
        _ => Err(DicomError::Unsupported {
            message: format!("tag {} does not contain bulk data", tag_hex(element.tag)),
        }),
    }
}

fn resolve_attribute_path_with_container<'a>(
    dataset: &'a DataSet,
    path: &[AttributePathSegment],
) -> Result<(&'a Element, &'a DataSet), DicomError> {
    if path.is_empty() {
        return Err(DicomError::Unsupported {
            message: "attribute path must not be empty".into(),
        });
    }

    let mut current = dataset;
    let mut index = 0usize;
    while index < path.len() {
        let AttributePathSegment::Tag(tag) = path[index] else {
            return Err(DicomError::Unsupported {
                message: "attribute path must start with a tag segment".into(),
            });
        };
        let element = current.get(tag).ok_or_else(|| DicomError::InvalidTagPath {
            value: tag_hex(tag),
        })?;
        if index == path.len() - 1 {
            return Ok((element, current));
        }

        let AttributePathSegment::Item(item_index) = path[index + 1] else {
            return Err(DicomError::Unsupported {
                message: format!(
                    "tag {} must be followed by an item index before descending",
                    tag_hex(tag)
                ),
            });
        };
        let items = element.items().ok_or_else(|| DicomError::Unsupported {
            message: format!(
                "tag {} is not a sequence and cannot be indexed",
                tag_hex(tag)
            ),
        })?;
        current = items
            .get(item_index)
            .ok_or_else(|| DicomError::InvalidTagPath {
                value: format!("{}/{}", tag_hex(tag), item_index),
            })?;
        index += 2;
    }

    Err(DicomError::Unsupported {
        message: "attribute path did not resolve to an element".into(),
    })
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
            let compressed_frames = encapsulated_frames(pixel_data, total_frames(file))
                .map_err(|e| DicomError::Toolkit(e.to_string()))?;

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

fn extract_stored_frames_from_file(
    file: &FileFormat,
    frame_numbers: &[u32],
) -> Result<Vec<Bytes>, DicomError> {
    let frame_indices = validate_frame_numbers(total_frames(file), frame_numbers)?;
    let pixel_data = pixel_data(&file.dataset)?;

    match pixel_data {
        PixelData::Native { bytes } => raw_native_frames(file, bytes, &frame_indices),
        PixelData::Encapsulated { .. } => {
            let compressed_frames = encapsulated_frames(pixel_data, total_frames(file))
                .map_err(|e| DicomError::Toolkit(e.to_string()))?;

            frame_indices
                .into_iter()
                .map(|frame_index| {
                    compressed_frames
                        .get(frame_index as usize)
                        .ok_or(DicomError::InvalidFrame {
                            requested: frame_index + 1,
                            available: total_frames(file),
                        })
                        .map(|frame| Bytes::copy_from_slice(frame))
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
    let encoded_frames = frames
        .into_iter()
        .map(|frame| {
            encode_frame_for_transfer_syntax(
                target_ts_uid,
                frame.as_ref(),
                rows,
                cols,
                samples_u8,
                bits_allocated_u8,
                bits_stored_u8,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    encapsulated_pixel_data_from_frames(&encoded_frames)
        .map_err(|e| DicomError::Toolkit(e.to_string()))
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
        uid if uid == transfer_syntaxes::JPEG_LOSSLESS.uid
            || uid == transfer_syntaxes::JPEG_LOSSLESS_SV1.uid =>
        {
            encode_jpeg_lossless(
                frame,
                cols,
                rows,
                samples_per_pixel,
                bits_allocated,
                bits_stored,
                1,
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
        uid if uid == transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY.uid => {
            Jp2kCodec::encode_frame_htj2k(
                frame,
                u32::from(cols),
                u32::from(rows),
                bits_stored,
                samples_per_pixel,
                true,
            )
            .map_err(|e| DicomError::Toolkit(e.to_string()))
        }
        uid if uid == transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000.uid => {
            // The toolkit registry defaults generic HTJ2K output to a lossless stream
            // because pacsnode does not currently expose a rendered/transcode quality knob.
            Jp2kCodec::encode_frame_htj2k(
                frame,
                u32::from(cols),
                u32::from(rows),
                bits_stored,
                samples_per_pixel,
                true,
            )
            .map_err(|e| DicomError::Toolkit(e.to_string()))
        }
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
    !is_native_transfer_syntax(ts_uid) && is_supported_output_transfer_syntax(ts_uid)
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

fn write_dataset_bytes(dataset: &DataSet, ts_uid: &str) -> Result<Bytes, DicomError> {
    let mut buf = Vec::new();
    dicom_toolkit_data::DicomWriter::new(Cursor::new(&mut buf))
        .write_dataset(dataset, ts_uid)
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

fn pixel_data(dataset: &DataSet) -> Result<&PixelData, DicomError> {
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
    total_frames_from_dataset(&file.dataset)
}

fn total_frames_from_dataset(dataset: &DataSet) -> u32 {
    dataset
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

fn patch_bulk_data_uris<F>(
    dataset: &DataSet,
    json: &mut serde_json::Value,
    parent_path: Option<&str>,
    resolve_uri: &F,
) -> Result<(), DicomError>
where
    F: Fn(&str) -> String,
{
    let Some(entries) = json.as_object_mut() else {
        return Err(DicomError::Unsupported {
            message: "instance metadata JSON must be an object".into(),
        });
    };

    for (tag, element) in dataset.iter() {
        let key = tag_hex(*tag);
        let current_path = match parent_path {
            Some(parent) => format!("{parent}/{key}"),
            None => key.clone(),
        };

        if let Some(items) = element.items() {
            if let Some(value_array) = entries
                .get_mut(&key)
                .and_then(|entry| entry.get_mut("Value"))
                .and_then(serde_json::Value::as_array_mut)
            {
                for (index, item) in items.iter().enumerate() {
                    if let Some(item_json) = value_array.get_mut(index) {
                        let item_path = format!("{current_path}/{index}");
                        patch_bulk_data_uris(
                            item,
                            item_json,
                            Some(item_path.as_str()),
                            resolve_uri,
                        )?;
                    }
                }
            }
            continue;
        }

        if is_bulk_data_eligible(element) {
            entries.insert(
                key,
                serde_json::json!({
                    "vr": element.vr.code(),
                    "BulkDataURI": resolve_uri(current_path.as_str()),
                }),
            );
        }
    }

    Ok(())
}

fn is_bulk_data_eligible(element: &Element) -> bool {
    matches!(element.value, Value::PixelData(_))
        || matches!(
            element.vr,
            Vr::OB | Vr::OD | Vr::OF | Vr::OL | Vr::OV | Vr::OW | Vr::UN
        )
}

fn rendered_dimensions(
    image: &DicomImage,
    options: &RenderedFrameOptions,
) -> Result<(u32, u32), DicomError> {
    let (rows, columns) = match options.region {
        Some(region) => cropped_dimensions(image.rows, image.columns, region)?,
        None => (image.rows, image.columns),
    };
    target_render_dimensions(rows, columns, options.rows, options.columns)
}

fn cropped_dimensions(
    rows: u32,
    columns: u32,
    region: RenderedRegion,
) -> Result<(u32, u32), DicomError> {
    validate_rendered_region(region)?;
    let start_row = (region.top * rows as f64).floor() as u32;
    let end_row = ((region.top + region.height) * rows as f64).ceil() as u32;
    let start_col = (region.left * columns as f64).floor() as u32;
    let end_col = ((region.left + region.width) * columns as f64).ceil() as u32;

    let cropped_rows = end_row.saturating_sub(start_row);
    let cropped_columns = end_col.saturating_sub(start_col);
    if cropped_rows == 0 || cropped_columns == 0 {
        return Err(DicomError::Unsupported {
            message: "rendered crop region resolved to an empty image".into(),
        });
    }

    Ok((cropped_rows, cropped_columns))
}

fn validate_rendered_region(region: RenderedRegion) -> Result<(), DicomError> {
    let values = [region.left, region.top, region.width, region.height];
    if values.iter().any(|value| !value.is_finite()) {
        return Err(DicomError::Unsupported {
            message: "rendered region values must be finite".into(),
        });
    }
    if region.left < 0.0
        || region.top < 0.0
        || region.width <= 0.0
        || region.height <= 0.0
        || region.left + region.width > 1.0
        || region.top + region.height > 1.0
    {
        return Err(DicomError::Unsupported {
            message: "rendered region must stay within [0.0, 1.0] and have positive width/height"
                .into(),
        });
    }
    Ok(())
}

fn target_render_dimensions(
    rows: u32,
    columns: u32,
    target_rows: Option<u32>,
    target_columns: Option<u32>,
) -> Result<(u32, u32), DicomError> {
    let original_rows = rows;
    let original_columns = columns;
    let (rows, columns) = match (target_rows, target_columns) {
        (Some(rows), Some(columns)) => (rows, columns),
        (Some(rows), None) => (
            rows,
            scale_preserving_aspect(original_columns, rows, original_rows)?,
        ),
        (None, Some(columns)) => (
            scale_preserving_aspect(original_rows, columns, original_columns)?,
            columns,
        ),
        (None, None) => (rows, columns),
    };

    if rows == 0 || columns == 0 {
        return Err(DicomError::Unsupported {
            message: "rendered output dimensions must be greater than zero".into(),
        });
    }

    Ok((rows, columns))
}

fn scale_preserving_aspect(
    numerator: u32,
    scaled_by: u32,
    divisor: u32,
) -> Result<u32, DicomError> {
    let scaled =
        (u64::from(numerator) * u64::from(scaled_by) + u64::from(divisor) / 2) / u64::from(divisor);
    u32::try_from(scaled).map_err(|_| DicomError::Unsupported {
        message: "scaled rendered dimension exceeds u32 range".into(),
    })
}

fn encode_rendered_pixels(
    pixels: &[u8],
    rows: u32,
    columns: u32,
    channels: u8,
    media_type: RenderedMediaType,
) -> Result<Bytes, DicomError> {
    match media_type {
        RenderedMediaType::Png => encode_png(pixels, rows, columns, channels),
        RenderedMediaType::Jpeg { quality } => {
            encode_jpeg(pixels, rows, columns, channels, quality)
        }
    }
}

fn encode_png(pixels: &[u8], rows: u32, columns: u32, channels: u8) -> Result<Bytes, DicomError> {
    let mut buf = Vec::new();
    let mut encoder = PngEncoder::new(&mut buf, columns, rows);
    encoder.set_color(png_color_type(channels)?);
    encoder.set_depth(PngBitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| DicomError::Toolkit(e.to_string()))?;
    writer
        .write_image_data(pixels)
        .map_err(|e| DicomError::Toolkit(e.to_string()))?;
    drop(writer);
    Ok(Bytes::from(buf))
}

fn encode_jpeg(
    pixels: &[u8],
    rows: u32,
    columns: u32,
    channels: u8,
    quality: u8,
) -> Result<Bytes, DicomError> {
    if !(1..=100).contains(&quality) {
        return Err(DicomError::Unsupported {
            message: format!("JPEG quality must be in the range 1..=100, got {quality}"),
        });
    }

    let width = u16::try_from(columns).map_err(|_| DicomError::Unsupported {
        message: format!("rendered JPEG width {columns} exceeds encoder limits"),
    })?;
    let height = u16::try_from(rows).map_err(|_| DicomError::Unsupported {
        message: format!("rendered JPEG height {rows} exceeds encoder limits"),
    })?;

    let mut buf = Vec::new();
    let encoder = JpegEncoder::new(&mut buf, quality);
    encoder
        .encode(pixels, width, height, jpeg_color_type(channels)?)
        .map_err(|e| DicomError::Toolkit(e.to_string()))?;
    Ok(Bytes::from(buf))
}

fn png_color_type(channels: u8) -> Result<PngColorType, DicomError> {
    match channels {
        1 => Ok(PngColorType::Grayscale),
        3 => Ok(PngColorType::Rgb),
        n => Err(DicomError::Unsupported {
            message: format!("unsupported output channel count {n} for PNG encoding"),
        }),
    }
}

fn jpeg_color_type(channels: u8) -> Result<JpegColorType, DicomError> {
    match channels {
        1 => Ok(JpegColorType::Luma),
        3 => Ok(JpegColorType::Rgb),
        n => Err(DicomError::Unsupported {
            message: format!("unsupported output channel count {n} for JPEG encoding"),
        }),
    }
}

fn required_u16(dataset: &DataSet, tag: Tag, name: &'static str) -> Result<u16, DicomError> {
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

fn required_string(dataset: &DataSet, tag: Tag, name: &'static str) -> Result<String, DicomError> {
    dataset
        .get_string(tag)
        .map(str::trim)
        .map(|value| value.trim_end_matches('\0'))
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or(DicomError::MissingTag { tag: name })
}

fn is_supported_output_transfer_syntax(ts_uid: &str) -> bool {
    if is_native_transfer_syntax(ts_uid) {
        return true;
    }

    can_encode(ts_uid)
        && matches!(
            ts_uid,
            uid if uid == transfer_syntaxes::RLE_LOSSLESS.uid
                || uid == transfer_syntaxes::JPEG_BASELINE.uid
                || uid == transfer_syntaxes::JPEG_EXTENDED.uid
                || uid == transfer_syntaxes::JPEG_LOSSLESS.uid
                || uid == transfer_syntaxes::JPEG_LOSSLESS_SV1.uid
                || uid == transfer_syntaxes::JPEG_LS_LOSSLESS.uid
                || uid == transfer_syntaxes::JPEG_2000_LOSSLESS.uid
                || uid == transfer_syntaxes::JPEG_2000.uid
                || uid == transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY.uid
                || uid == transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000.uid
        )
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
    use dicom_toolkit_dict::{tags, ts::transfer_syntaxes, Tag, Vr};
    use pacs_core::DicomJson;
    use serde_json::json;

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

    fn make_htj2k_multiframe_dicom() -> Bytes {
        let mut ds = DataSet::new();
        ds.set_string(tags::STUDY_INSTANCE_UID, Vr::UI, "1.2.3");
        ds.set_string(tags::SERIES_INSTANCE_UID, Vr::UI, "1.2.3.4");
        ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, "1.2.3.4.202");
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
            Value::PixelData(
                encapsulated_pixel_data_from_frames(&[
                    vec![0xFF, 0x4F, 0xAA],
                    vec![0xFF, 0x4F, 0xBB],
                ])
                .unwrap(),
            ),
        ));

        let mut ff = FileFormat::from_dataset("1.2.840.10008.5.1.4.1.1.2", "1.2.3.4.202", ds);
        ff.meta.transfer_syntax_uid = transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000.uid.to_owned();

        let mut buf = Vec::new();
        DicomWriter::new(Cursor::new(&mut buf))
            .write_file(&ff)
            .unwrap();
        Bytes::from(buf)
    }

    fn make_nested_bulkdata_dicom() -> Bytes {
        let mut ds = DataSet::new();
        ds.set_string(tags::STUDY_INSTANCE_UID, Vr::UI, "1.2.3");
        ds.set_string(tags::SERIES_INSTANCE_UID, Vr::UI, "1.2.3.4");
        ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, "1.2.3.4.5");
        ds.set_string(tags::SOP_CLASS_UID, Vr::UI, "1.2.840.10008.5.1.4.1.1.2");
        ds.insert(Element::bytes(
            Tag::new(0x0011, 0x1010),
            Vr::OB,
            vec![0xAA, 0xBB, 0xCC, 0xDD],
        ));

        let mut item = DataSet::new();
        item.insert(Element::bytes(
            Tag::new(0x0011, 0x1011),
            Vr::OB,
            vec![0xDE, 0xAD],
        ));
        ds.set_sequence(Tag::new(0x0008, 0x2112), vec![item]);

        let ff = FileFormat::from_dataset("1.2.840.10008.5.1.4.1.1.2", "1.2.3.4.5", ds);
        let mut buf = Vec::new();
        DicomWriter::new(Cursor::new(&mut buf))
            .write_file(&ff)
            .unwrap();
        Bytes::from(buf)
    }

    fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
        assert!(bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]));
        let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        (width, height)
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
    fn extract_stored_frames_returns_encapsulated_payloads_without_decoding() {
        let frames = extract_stored_frames(make_htj2k_multiframe_dicom(), &[1, 2]).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], Bytes::from_static(&[0xFF, 0x4F, 0xAA]));
        assert_eq!(frames[1], Bytes::from_static(&[0xFF, 0x4F, 0xBB]));
    }

    #[test]
    fn render_frames_png_returns_png_bytes() {
        let pngs = render_frames_png(make_multiframe_dicom(), &[1]).unwrap();
        assert_eq!(pngs.len(), 1);
        assert!(pngs[0].starts_with(&[0x89, 0x50, 0x4E, 0x47]));
    }

    #[test]
    fn render_frames_with_options_returns_jpeg_bytes() {
        let frames = render_frames_with_options(
            make_multiframe_dicom(),
            &[1],
            RenderedMediaType::Jpeg { quality: 90 },
            &RenderedFrameOptions::default(),
        )
        .unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].starts_with(&[0xFF, 0xD8, 0xFF]));
    }

    #[test]
    fn render_frames_with_options_resizes_png_preserving_aspect() {
        let frames = render_frames_with_options(
            make_multiframe_dicom(),
            &[1],
            RenderedMediaType::Png,
            &RenderedFrameOptions {
                rows: Some(4),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(png_dimensions(&frames[0]), (8, 4));
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
    fn extract_bulk_data_path_supports_nested_sequence_items() {
        let bulk_data =
            extract_bulk_data_path(make_nested_bulkdata_dicom(), "00082112/0/00111011").unwrap();
        assert_eq!(
            bulk_data,
            BulkDataValue::Single(Bytes::from_static(&[0xDE, 0xAD]))
        );
    }

    #[test]
    fn metadata_with_bulk_data_uris_patches_nested_binary_attributes() {
        let metadata = DicomJson::from(json!({
            "00111010": {
                "vr": "OB",
                "InlineBinary": "qrvM3Q=="
            },
            "00082112": {
                "vr": "SQ",
                "Value": [
                    {
                        "00111011": {
                            "vr": "OB",
                            "InlineBinary": "3q0="
                        }
                    }
                ]
            }
        }));

        let patched =
            metadata_with_bulk_data_uris(&metadata, make_nested_bulkdata_dicom(), |path| {
                format!("/bulk/{path}")
            })
            .unwrap();
        let value = patched.as_value();

        assert_eq!(value["00111010"]["BulkDataURI"], json!("/bulk/00111010"));
        assert!(value["00111010"].get("InlineBinary").is_none());
        assert_eq!(
            value["00082112"]["Value"][0]["00111011"]["BulkDataURI"],
            json!("/bulk/00082112/0/00111011")
        );
        assert!(value["00082112"]["Value"][0]["00111011"]
            .get("InlineBinary")
            .is_none());
    }

    #[test]
    fn parse_bulk_data_tag_path_accepts_hex() {
        let tag = parse_bulk_data_tag_path("7FE00010").unwrap();
        assert_eq!(tag, tags::PIXEL_DATA);
    }

    #[test]
    fn supported_retrieve_transfer_syntaxes_only_include_encodable_outputs() {
        let syntaxes = supported_retrieve_transfer_syntaxes();
        assert!(syntaxes.contains(&transfer_syntaxes::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN.uid));
        assert!(syntaxes.contains(&transfer_syntaxes::JPEG_BASELINE.uid));
        assert!(syntaxes.contains(&transfer_syntaxes::JPEG_LOSSLESS.uid));
        assert!(syntaxes.contains(&transfer_syntaxes::JPEG_LOSSLESS_SV1.uid));
        assert!(syntaxes.contains(&transfer_syntaxes::JPEG_2000_LOSSLESS.uid));
        assert!(syntaxes.contains(&transfer_syntaxes::RLE_LOSSLESS.uid));
        assert!(supports_retrieve_transfer_syntax(
            transfer_syntaxes::JPEG_BASELINE.uid
        ));
        assert!(supports_retrieve_transfer_syntax(
            transfer_syntaxes::JPEG_LOSSLESS.uid
        ));
        assert!(supports_retrieve_transfer_syntax(
            transfer_syntaxes::JPEG_LOSSLESS_SV1.uid
        ));
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
    fn transcode_part10_to_jpeg_lossless_roundtrips_frames() {
        let transcoded = transcode_part10(
            make_multiframe_dicom(),
            transfer_syntaxes::JPEG_LOSSLESS.uid,
        )
        .unwrap();
        let file = DicomReader::new(Cursor::new(transcoded.as_ref()))
            .read_file()
            .unwrap();
        assert_eq!(
            file.meta.transfer_syntax_uid,
            transfer_syntaxes::JPEG_LOSSLESS.uid
        );
        let frames = extract_frames(transcoded, &[1, 2]).unwrap();
        assert_eq!(frames[0], Bytes::from_static(&[0x11, 0x22]));
        assert_eq!(frames[1], Bytes::from_static(&[0x33, 0x44]));
    }

    #[test]
    fn prepare_dimse_dataset_transcodes_part10_to_requested_transfer_syntax() {
        let dataset_bytes =
            prepare_dimse_dataset(make_multiframe_dicom(), transfer_syntaxes::RLE_LOSSLESS.uid)
                .unwrap();
        let dataset = DicomReader::new(Cursor::new(dataset_bytes.as_ref()))
            .read_dataset(transfer_syntaxes::RLE_LOSSLESS.uid)
            .unwrap();
        let pixel_data = dataset.get(tags::PIXEL_DATA).unwrap();
        assert!(matches!(
            pixel_data.value,
            Value::PixelData(PixelData::Encapsulated { .. })
        ));
    }

    #[test]
    fn prepare_dimse_dataset_transcodes_bare_dataset_from_explicit_le() {
        let file = read_file(make_multiframe_dicom()).unwrap();
        let bare_dataset = write_dataset_bytes(
            &file.dataset,
            transfer_syntaxes::EXPLICIT_VR_LITTLE_ENDIAN.uid,
        )
        .unwrap();
        let transcoded =
            prepare_dimse_dataset(bare_dataset, transfer_syntaxes::JPEG_BASELINE.uid).unwrap();
        let dataset = DicomReader::new(Cursor::new(transcoded.as_ref()))
            .read_dataset(transfer_syntaxes::JPEG_BASELINE.uid)
            .unwrap();
        let pixel_data = dataset.get(tags::PIXEL_DATA).unwrap();
        assert!(matches!(
            pixel_data.value,
            Value::PixelData(PixelData::Encapsulated { .. })
        ));
    }

    #[test]
    fn prepare_dimse_dataset_rejects_unsupported_transfer_syntax() {
        let dataset_bytes = prepare_dimse_dataset(
            make_multiframe_dicom(),
            transfer_syntaxes::JPEG_LOSSLESS_SV1.uid,
        )
        .unwrap();
        let dataset = DicomReader::new(Cursor::new(dataset_bytes.as_ref()))
            .read_dataset(transfer_syntaxes::JPEG_LOSSLESS_SV1.uid)
            .unwrap();
        let pixel_data = dataset.get(tags::PIXEL_DATA).unwrap();
        assert!(matches!(
            pixel_data.value,
            Value::PixelData(PixelData::Encapsulated { .. })
        ));
    }

    #[test]
    fn transcode_part10_rejects_unknown_output_transfer_syntax() {
        let error = transcode_part10(make_multiframe_dicom(), "1.2.3.4.5").unwrap_err();
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
