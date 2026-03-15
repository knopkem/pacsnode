use bytes::Bytes;
use pacs_core::{PacsError, PacsResult};

use crate::parser::ParsedDicom;

// ── Internal multipart splitter ───────────────────────────────────────────────

/// Returns the byte offset of the first occurrence of `needle` in `haystack`,
/// or `None` when not found.
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Collects all byte offsets at which `needle` starts inside `data`.
fn all_occurrences(data: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut pos = 0;
    while let Some(offset) = find_bytes(&data[pos..], needle) {
        positions.push(pos + offset);
        pos += offset + needle.len();
    }
    positions
}

/// Splits a `multipart/related` body on the given `boundary` and returns the
/// raw bytes of each part body (i.e. everything after the blank line that
/// terminates the part headers).
fn split_multipart(data: &[u8], boundary: &str) -> Vec<Bytes> {
    let delim = format!("--{boundary}");
    let delim_bytes = delim.as_bytes();
    let end_marker = format!("--{boundary}--");
    let end_marker_bytes = end_marker.as_bytes();

    let boundary_positions = all_occurrences(data, delim_bytes);
    let mut bodies = Vec::new();

    for (idx, &bpos) in boundary_positions.iter().enumerate() {
        // The end marker `--boundary--` also starts with `--boundary`; stop here.
        if data[bpos..].starts_with(end_marker_bytes) {
            break;
        }

        let after_delim = bpos + delim_bytes.len();
        if after_delim >= data.len() {
            break;
        }

        // Skip the CRLF (or bare LF) that terminates the boundary line.
        let headers_start = if data[after_delim..].starts_with(b"\r\n") {
            after_delim + 2
        } else if data[after_delim..].starts_with(b"\n") {
            after_delim + 1
        } else {
            // Boundary line has no line ending — skip this malformed part.
            continue;
        };

        // Locate the blank line (CRLF CRLF) that ends the part headers.
        let Some(blank_offset) = find_bytes(&data[headers_start..], b"\r\n\r\n") else {
            continue;
        };
        let body_start = headers_start + blank_offset + 4;

        // Body ends at the next boundary, minus the preceding CRLF separator.
        let body_end = boundary_positions
            .get(idx + 1)
            .map(|&next| {
                if next >= 2 && data[next - 2..next] == *b"\r\n" {
                    next - 2
                } else {
                    next
                }
            })
            .unwrap_or(data.len());

        if body_start < body_end {
            bodies.push(Bytes::copy_from_slice(&data[body_start..body_end]));
        }
    }

    bodies
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Parses a `multipart/related` STOW-RS request body into individual DICOM
/// instances.
///
/// `boundary` must be the value extracted from the `Content-Type` header
/// (without the leading `--`).
///
/// Malformed parts are skipped with a `WARN`-level log entry.  An error is
/// returned only when **no** parts could be found or all parts failed to parse.
///
/// # Errors
///
/// - [`PacsError::DicomParse`] when the body contains no recognisable parts.
/// - [`PacsError::DicomParse`] when every part fails to parse.
pub async fn parse_stow_multipart(body: Bytes, boundary: &str) -> PacsResult<Vec<ParsedDicom>> {
    let raw_parts = split_multipart(body.as_ref(), boundary);

    if raw_parts.is_empty() {
        return Err(PacsError::DicomParse(format!(
            "no parts found in multipart body with boundary '{boundary}'"
        )));
    }

    let total = raw_parts.len();
    let mut results = Vec::with_capacity(total);
    let mut error_count: usize = 0;

    for part in raw_parts {
        match ParsedDicom::from_bytes(part) {
            Ok(parsed) => results.push(parsed),
            Err(e) => {
                error_count += 1;
                tracing::warn!(
                    error_count,
                    total,
                    error = %e,
                    "skipping malformed DICOM part in STOW-RS body"
                );
            }
        }
    }

    if results.is_empty() {
        return Err(PacsError::DicomParse(format!(
            "all {total} DICOM part(s) in STOW-RS body failed to parse"
        )));
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use dicom_toolkit_data::{DataSet, DicomWriter, FileFormat};
    use dicom_toolkit_dict::{tags, Vr};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_dicom(study: &str, series: &str, instance: &str) -> Vec<u8> {
        let mut ds = DataSet::new();
        ds.set_string(tags::STUDY_INSTANCE_UID, Vr::UI, study);
        ds.set_string(tags::SERIES_INSTANCE_UID, Vr::UI, series);
        ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, instance);
        ds.set_string(tags::SOP_CLASS_UID, Vr::UI, "1.2.840.10008.5.1.4.1.1.2");
        ds.set_string(tags::MODALITY, Vr::CS, "CT");
        let ff = FileFormat::from_dataset("1.2.840.10008.5.1.4.1.1.2", instance, ds);
        let mut buf = Vec::new();
        DicomWriter::new(std::io::Cursor::new(&mut buf))
            .write_file(&ff)
            .unwrap();
        buf
    }

    fn build_multipart(boundary: &str, parts: &[Vec<u8>]) -> Bytes {
        let mut body: Vec<u8> = Vec::new();
        for part in parts {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            body.extend_from_slice(b"Content-Type: application/dicom\r\n");
            body.extend_from_slice(b"\r\n");
            body.extend_from_slice(part);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        Bytes::from(body)
    }

    // ── find_bytes ────────────────────────────────────────────────────────────

    #[test]
    fn test_find_bytes_found() {
        assert_eq!(find_bytes(b"hello world", b"world"), Some(6));
    }

    #[test]
    fn test_find_bytes_not_found() {
        assert_eq!(find_bytes(b"hello world", b"xyz"), None);
    }

    #[test]
    fn test_find_bytes_empty_needle() {
        assert_eq!(find_bytes(b"abc", b""), Some(0));
    }

    // ── split_multipart ───────────────────────────────────────────────────────

    #[test]
    fn test_split_multipart_single_part() {
        let dicom = make_dicom("1.1", "1.1.1", "1.1.1.1");
        let body = build_multipart("bdry", &[dicom.clone()]);
        let parts = split_multipart(body.as_ref(), "bdry");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].as_ref(), dicom.as_slice());
    }

    #[test]
    fn test_split_multipart_two_parts() {
        let d1 = make_dicom("1.1", "1.1.1", "1.1.1.1");
        let d2 = make_dicom("2.2", "2.2.2", "2.2.2.2");
        let body = build_multipart("bdry2", &[d1.clone(), d2.clone()]);
        let parts = split_multipart(body.as_ref(), "bdry2");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].as_ref(), d1.as_slice());
        assert_eq!(parts[1].as_ref(), d2.as_slice());
    }

    #[test]
    fn test_split_multipart_empty_body() {
        let parts = split_multipart(b"", "bdry");
        assert!(parts.is_empty());
    }

    // ── parse_stow_multipart ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_parse_stow_multipart_single_part() {
        let dicom = make_dicom("1.2.3.10", "1.2.3.10.1", "1.2.3.10.1.1");
        let body = build_multipart("test_bdry", &[dicom]);
        let result = parse_stow_multipart(body, "test_bdry").await;
        assert!(result.is_ok(), "unexpected error: {:?}", result.err());
        let parsed = result.unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].study.study_uid.as_ref(), "1.2.3.10");
    }

    #[tokio::test]
    async fn test_parse_stow_multipart_two_parts() {
        let d1 = make_dicom("1.2.3.20", "1.2.3.20.1", "1.2.3.20.1.1");
        let d2 = make_dicom("1.2.3.30", "1.2.3.30.1", "1.2.3.30.1.1");
        let body = build_multipart("multi_bdry", &[d1, d2]);
        let parsed = parse_stow_multipart(body, "multi_bdry").await.unwrap();
        assert_eq!(parsed.len(), 2);
        let uids: Vec<_> = parsed.iter().map(|p| p.study.study_uid.as_ref()).collect();
        assert!(uids.contains(&"1.2.3.20"));
        assert!(uids.contains(&"1.2.3.30"));
    }

    #[tokio::test]
    async fn test_parse_stow_multipart_no_parts_returns_error() {
        let result = parse_stow_multipart(Bytes::from_static(b""), "bdry").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_parse_stow_multipart_all_malformed_returns_error() {
        let body = build_multipart("bdry", &[b"not-dicom".to_vec()]);
        let result = parse_stow_multipart(body, "bdry").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_parse_stow_multipart_one_good_one_bad() {
        let good = make_dicom("1.2.3.40", "1.2.3.40.1", "1.2.3.40.1.1");
        let body = build_multipart("mixed_bdry", &[good, b"garbage".to_vec()]);
        // One good part should succeed; bad part is skipped with a warning.
        let parsed = parse_stow_multipart(body, "mixed_bdry").await.unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].study.study_uid.as_ref(), "1.2.3.40");
    }
}
