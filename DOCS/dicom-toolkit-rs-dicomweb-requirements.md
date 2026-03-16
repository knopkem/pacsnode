# remaining dicom-toolkit-rs follow-up after pacsnode DICOMweb completion

This document no longer tracks open pacsnode DICOMweb feature gaps.
Those pacsnode-side items are now implemented, and the toolkit APIs that were
previously needed to unblock them have landed in `dicom-toolkit-rs`.

The purpose of this file is now narrower: record only the **remaining upstream
`dicom-toolkit-rs` bugs, missing features, or cleanup opportunities** that were
found while finishing the DICOMweb work.

---

## Current status

The following items are now considered done and are intentionally removed from
this requirements list:

- `pacsnode` now supports:
  - rendered WADO-RS / WADO-URI negotiation for `image/png` and `image/jpeg`
  - rendered query parameter handling (`windowCenter`, `windowWidth`, `rows`,
    `columns`, `region`, `annotation`)
  - nested bulk-data attribute paths
  - request-time `BulkDataURI` injection for eligible binary attributes
  - retrieve transfer-syntax filtering based on actual encode capability
- `dicom-toolkit-rs` now provides the upstream helpers that were previously
  missing:
  - `frame_to_jpeg_bytes(...)`
  - `to_json_with_binary_mode(..., BinaryValueMode::BulkDataUri(...))`
  - `parse_attribute_path(...)`
  - `element_value_bytes(...)`
  - `encapsulated_frames(...)`
  - `RenderedFrameOptions` plus `render_frame_u8(...)`
  - `supported_encode_transfer_syntaxes()` and `can_encode(...)`
  - explicit DIMSE transfer-syntax policy fields on association config

Everything above is treated as complete for the purposes of this document.

---

## Remaining dicom-toolkit-rs follow-up

### Requirement 1 — toolkit-owned encapsulated Pixel Data construction

#### Why

While wiring retrieve transcoding in `pacsnode`, the PACS still had to assemble
`PixelData::Encapsulated` manually after encoding per-frame compressed payloads.
That includes building the Basic Offset Table (BOT).

This is easy for downstream consumers to get wrong. During the pacsnode work, an
initial implementation wrote BOT offsets using only fragment payload lengths. The
existing toolkit frame-splitting logic correctly rejected that output because BOT
entries are interpreted on fragment item boundaries, not just raw fragment data
lengths.

That class of bug should be prevented centrally in the toolkit rather than left
for every caller to rediscover.

#### Requirement

Add a toolkit-owned helper for constructing encapsulated Pixel Data from
compressed frame payloads, with standards-correct BOT generation handled
centrally.

#### Proposed API

```rust
pub struct EncapsulatedFrame {
    pub fragments: Vec<Vec<u8>>,
}

pub fn build_encapsulated_pixel_data(
    frames: &[EncapsulatedFrame],
) -> DcmResult<PixelData>;
```

Acceptable alternatives:

- a helper for the common one-fragment-per-frame case, for example
  `encapsulated_pixel_data_from_frames(&[Vec<u8>])`
- a codec-layer API that returns a fully-formed `PixelData::Encapsulated`
  instead of raw encoded buffers

#### Acceptance criteria

- BOT entries are computed consistently with the toolkit's own encapsulated
  frame parsing rules
- one-fragment-per-frame output is handled correctly for multi-frame objects
- multi-fragment-per-frame output is handled correctly
- malformed or off-by-one / off-by-header BOT calculations are covered by
  regression tests
- downstream consumers no longer need to hand-roll BOT offsets in PACS or CLI
  code

#### Priority

**High**

#### Why this belongs upstream instead of in pacsnode

Encapsulated Pixel Data construction is a DICOM encoding concern, not a PACS
policy concern. Keeping BOT construction in `dicom-toolkit-rs` reduces duplicate
logic across PACS, codec, CLI, and other consumers and makes the correct
behavior testable in one place.

---

## Recommended next step

1. Add the encapsulated Pixel Data builder/helper in `dicom-toolkit-rs`
2. Update pacsnode to consume that helper instead of manually assembling
   `PixelData::Encapsulated`
3. Keep this file trimmed to unresolved upstream work only; remove items once
   they land

---

## Source references

- pacsnode manual encapsulated output path:
  - `crates/pacs-dicom/src/wado.rs`
- toolkit encapsulated frame parsing and BOT alignment checks:
  - `../dcmtk-rs/crates/dicom-toolkit-data/src/value.rs`
- toolkit JSON `BulkDataURI` support that is now already available:
  - `../dcmtk-rs/crates/dicom-toolkit-data/src/json.rs`
- toolkit association transfer-syntax policy that is now already available:
  - `../dcmtk-rs/crates/dicom-toolkit-net/src/config.rs`
