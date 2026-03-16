# dicom-toolkit-rs requirements for closing remaining DICOMweb gaps

This document captures the remaining work behind the still-yellow DICOMweb rows in
`DOCS/feature-matrix.md` and identifies which items should be implemented in
`pacsnode` itself versus which items would benefit from, or require, changes in
`dicom-toolkit-rs`.


---

## Current status

The following are already implemented in pacsnode:

- WADO-RS frame retrieval via `/frames/{n[,m...]}` returning native frame bytes
- WADO-RS rendered PNG previews for study/series/instance/frame routes
- Instance bulk-data endpoint plus `BulkDataURI` injection for Pixel Data
- WADO-RS retrieve `Accept`-header negotiation for `application/dicom` transfer-syntax requests
- WADO-URI `transferSyntax=...` handling for `application/dicom` retrieval
- Retrieve-time transcoding for native syntaxes, Deflated Explicit VR Little Endian,
  RLE Lossless, JPEG Baseline/Extended, JPEG-LS Lossless, JPEG 2000 Lossless,
  and JPEG 2000
- Legacy WADO-URI retrieval for `application/dicom` and rendered PNG via
  `contentType=image/png`

The remaining yellow items are yellow because they are **functional but not yet
complete or standards-comfortable**:

1. **WADO-RS — Rendered**
   - PNG-only
   - no `Accept`-header negotiation
   - no explicit rendered options such as resize/crop/window parameters

2. **WADO-RS — Bulk Data**
   - current metadata wiring advertises `BulkDataURI` only for Pixel Data
   - current endpoint is effectively top-level-tag focused
   - byte-exact bulk-data export for every binary VR is not guaranteed

3. **WADO-URI**
   - supports only `application/dicom` and `image/png`
   - does not yet implement the broader legacy query parameter surface
   - does not perform richer rendered negotiation or media-type selection

---

## What pacsnode still needs to do

These items can be implemented in the PACS layer even without toolkit changes.

### 1. WADO-RS rendered HTTP semantics

- Parse and honor the `Accept` header on rendered endpoints
- Return `406 Not Acceptable` / `415 Unsupported Media Type` where appropriate,
  instead of generic parse-style failures
- Decide a stable negotiation policy:
  - `image/png`
  - `image/jpeg` once available
  - `multipart/related` for multi-frame rendered responses
- Define a representative-image strategy for study/series rendered routes instead
  of always taking the first retrievable instance

### 2. WADO-RS bulk-data URI coverage

- Advertise `BulkDataURI` for all eligible large/binary attributes, not just
  Pixel Data
- Add endpoint coverage for nested attribute paths, not only top-level tags
- Add `Content-Location` metadata for multipart bulk-data responses where useful
- Add stronger tests for:
  - multi-fragment encapsulated Pixel Data
  - non-PixelData bulk elements
  - nested sequence/item attribute paths

### 3. WADO-URI query-parameter coverage

- Support more of the classic WADO-URI parameter surface:
  - `windowCenter`
  - `windowWidth`
  - `rows`
  - `columns`
  - `region`
  - `annotation`
  - `frameNumber`
- Validate and reject unsupported combinations with explicit HTTP status codes
- Extend smoke tests to cover WADO-URI rendered variants, not only raw DICOM

---

## dicom-toolkit-rs changes that are needed or strongly recommended

The sections below are the actual toolkit requirements.

---

## Requirement 1 — JPEG export API for rendered output

### Why

`dicom-toolkit-image` currently exposes PNG export only:

- `dicom-toolkit-image/src/export.rs:1-56`
- only `export_frame_png(...)` and `frame_to_png_bytes(...)` exist

That means pacsnode can only provide PNG rendered output today, which is why
rendered DICOMweb support remains yellow.

### Requirement

Add in-memory and file-based JPEG export helpers to `dicom-toolkit-image`.

### Proposed API

```rust
pub fn frame_to_jpeg_bytes(
    image: &DicomImage,
    frame: u32,
    quality: u8,
) -> DcmResult<Vec<u8>>;

pub fn export_frame_jpeg(
    image: &DicomImage,
    frame: u32,
    quality: u8,
    path: impl AsRef<Path>,
) -> DcmResult<()>;
```

### Acceptance criteria

- Supports grayscale and RGB rendered output
- Quality parameter is validated and documented
- Output is deterministic for the same input/options
- Unit tests cover:
  - grayscale JPEG
  - RGB JPEG
  - invalid quality values
  - out-of-range frame handling

### Priority

**High**

---

## Requirement 2 — DICOM JSON serializer mode with BulkDataURI support

### Why

`dicom-toolkit-data/src/json.rs:203-227` currently always emits `InlineBinary`
for binary values, and for encapsulated Pixel Data it explicitly falls back to the
**first fragment only** with the inline comment:

> `full support would use BulkDataURI`

That is the main reason pacsnode currently patches only Pixel Data itself instead
of relying on the toolkit to emit standards-friendlier DICOM JSON.

### Requirement

Add a DICOM JSON serialization mode that can emit `BulkDataURI` instead of
`InlineBinary`, ideally driven by a caller-provided callback or policy.

### Proposed API

```rust
pub enum BinaryValueMode<'a> {
    InlineBinary,
    BulkDataUri(&'a dyn Fn(dicom_toolkit_dict::Tag) -> Option<String>),
}

pub fn to_json_with_binary_mode(
    dataset: &DataSet,
    mode: BinaryValueMode<'_>,
) -> DcmResult<String>;
```

### Acceptance criteria

- Works for Pixel Data and other binary VRs, not just Pixel Data
- Does not silently emit only the first encapsulated fragment
- Allows the caller to choose which tags should become `BulkDataURI`
- Existing `to_json(...)` behavior remains backward compatible

### Priority

**High**

---

## Requirement 3 — Nested attribute-path resolver for bulk-data access

### Why

pacsnode currently exposes bulk data for a top-level tag path such as `7FE00010`
only:

- `crates/pacs-api/src/routes/wado.rs:224-243`

To make WADO-RS bulk data truly green, bulk-data access should support nested
sequence/item paths as referenced by DICOM JSON metadata.

### Requirement

Add toolkit support for parsing and resolving attribute paths into concrete dataset
elements, including sequence traversal.

### Proposed API

```rust
pub enum AttributePathSegment {
    Tag(dicom_toolkit_dict::Tag),
    Item(usize),
}

pub fn parse_attribute_path(path: &str) -> DcmResult<Vec<AttributePathSegment>>;

pub fn resolve_attribute_path<'a>(
    dataset: &'a DataSet,
    path: &[AttributePathSegment],
) -> DcmResult<&'a Element>;
```

### Acceptance criteria

- Handles top-level tags
- Handles nested sequence items
- Returns useful errors for malformed paths and out-of-range items
- Is reusable for both DICOM JSON generation and HTTP bulk-data retrieval

### Priority

**Medium**

---

## Requirement 4 — Transfer-syntax-aware raw element byte export

### Why

For native Pixel Data, pacsnode can safely return preserved raw bytes.

For other numeric/binary value types, the PACS currently reconstructs bytes from
parsed numeric values. That is workable for basic cases, but it is not a robust
foundation for “green” bulk-data support because it may not preserve exact original
encoding semantics across all transfer syntaxes and VR combinations.

### Requirement

Add a toolkit helper that can produce canonical raw bytes for a specific element,
with transfer syntax/endian behavior handled centrally.

### Proposed API

```rust
pub fn element_value_bytes(
    element: &Element,
    transfer_syntax_uid: &str,
) -> DcmResult<Vec<u8>>;
```

Alternative acceptable design:

- preserve original raw encoded value bytes during parse and expose them on demand

### Acceptance criteria

- Correct for integer, float, and bulk binary VRs
- Honors endian rules where applicable
- Handles padded values consistently
- Can be used by DICOMweb bulk-data handlers without PACS-specific byte rebuilding

### Priority

**Medium**

---

## Requirement 5 — Encapsulated frame/fragments helper

### Why

pacsnode currently contains its own logic for mapping encapsulated Pixel Data
offset tables and fragments to per-frame payloads.

That logic works, but it belongs naturally in the toolkit because any DICOMweb,
rendering, or codec consumer will likely need the same behavior.

### Requirement

Add a reusable encapsulated-frame helper that converts offset tables + fragments
into frame-level compressed payloads.

### Proposed API

```rust
pub fn encapsulated_frames(
    pixel_data: &PixelData,
    number_of_frames: u32,
) -> DcmResult<Vec<Vec<u8>>>;
```

### Acceptance criteria

- Supports single-frame encapsulated objects
- Supports one-fragment-per-frame cases
- Supports multi-fragment-per-frame cases via the basic offset table
- Has regression tests for edge cases and malformed offset tables

### Priority

**Medium**

---

## Requirement 6 — High-level rendered-frame options helper

### Why

`dicom-toolkit-image` already has useful primitives:

- window/level support in `DicomImage`
- scaling helpers in `transform.rs`
- overlay extraction in `overlay.rs`

But pacsnode still has to compose these pieces manually if it wants full WADO-RS
/ WADO-URI rendered parameter support.

### Requirement

Add a higher-level rendered-frame helper that accepts structured options for
windowing, resize, crop, and optional annotation/overlay composition.

### Proposed API

```rust
pub struct RenderedFrameOptions {
    pub frame: u32,
    pub window_center: Option<f64>,
    pub window_width: Option<f64>,
    pub rows: Option<u32>,
    pub columns: Option<u32>,
    pub region: Option<RenderedRegion>,
    pub burn_in_overlays: bool,
}

pub fn render_frame_u8(
    image: &DicomImage,
    options: &RenderedFrameOptions,
) -> DcmResult<Vec<u8>>;
```

### Acceptance criteria

- Window overrides work independently of dataset defaults
- Resize output is deterministic
- Region cropping is well-defined and documented
- Overlay/annotation behavior is explicit

### Priority

**Nice to have**, but it would simplify standards-compliant rendered endpoints
significantly.

---

## Requirement 7 — DIMSE association transfer-syntax policy API

### Why

pacsnode can now transcode on WADO retrieve, but for DIMSE association negotiation
it still depends on `dicom-toolkit-net::AssociationConfig`, which currently only
offers a boolean `accept_all_transfer_syntaxes`.

That means pacsnode cannot express:

- an explicit allow-list of transfer syntaxes
- an SCP preference order for offered syntaxes
- policy such as “accept JPEG 2000 but reject JPEG Lossless output until validated”

### Requirement

Extend `AssociationConfig` so an SCP can advertise and prefer an explicit transfer
syntax set rather than only “accept everything” or the hard-coded LE fallback.

### Proposed API

```rust
pub struct AssociationConfig {
    pub accept_all_transfer_syntaxes: bool,
    pub accepted_transfer_syntaxes: Vec<String>,
    pub preferred_transfer_syntaxes: Vec<String>,
    // ...
}
```

### Acceptance criteria

- SCP can reject unsupported transfer syntaxes during presentation-context negotiation
- SCP can prefer an explicit transfer syntax order
- Existing behavior remains available through `accept_all_transfer_syntaxes = true`

### Priority

**High**

---

## Requirement 8 — Separate decode vs encode capability reporting

### Why

`dicom-toolkit-codec::supported_transfer_syntaxes()` is decode-oriented, while the
currently convenient encode path does not expose the same set of transfer syntaxes
for output. In practice, pacsnode can decode JPEG Lossless objects, but it cannot
yet emit JPEG Lossless Part 10 output on retrieve.

Without separate decode/encode capability reporting, PACS code has to duplicate
toolkit-internal knowledge to avoid over-advertising transcoding support.

### Requirement

Expose distinct decode-supported and encode-supported transfer syntax sets.

### Proposed API

```rust
pub fn supported_decode_transfer_syntaxes() -> &'static [&'static str];

pub fn supported_encode_transfer_syntaxes() -> &'static [&'static str];

pub fn can_encode(ts_uid: &str) -> bool;
```

### Acceptance criteria

- The API makes it impossible to confuse “can decode existing object” with
  “can emit new object in that syntax”
- JPEG Lossless is not reported as encodable unless a real encoder exists
- pacsnode can use the same toolkit API for both retrieve negotiation and
  transcoding capability checks

### Priority

**Medium**

---

## Recommended implementation order

1. **JPEG export API**
2. **DICOM JSON `BulkDataURI` serialization mode**
3. **DIMSE association transfer-syntax policy API**
4. **Transfer-syntax-aware raw element byte export**
5. **Separate decode vs encode capability reporting**
6. **Encapsulated frame helper**
7. **Nested attribute-path resolver**
8. **High-level rendered options helper**

---

## Immediate next pacsnode follow-up after toolkit work lands

Once the relevant toolkit capabilities exist, pacsnode should:

1. Add `Accept` negotiation to rendered WADO-RS/WADO-URI routes
2. Advertise `BulkDataURI` for all eligible binary attributes
3. Support nested bulk-data attribute paths
4. Add rendered query parameter handling (`window`, `rows`, `columns`, `region`, etc.)
5. Add smoke/integration coverage for JPEG rendered output and richer bulk-data cases

---

## Source references

- pacsnode WADO-URI media handling:
  - `crates/pacs-api/src/routes/wado.rs:22-62`
- pacsnode bulk-data route and Pixel Data URI patch:
  - `crates/pacs-api/src/routes/wado.rs:224-285`
- toolkit PNG-only export surface:
  - `~/.cargo/git/checkouts/dicom-toolkit-rs-*/crates/dicom-toolkit-image/src/export.rs:1-56`
- toolkit DICOM JSON inline-binary behavior:
  - `~/.cargo/git/checkouts/dicom-toolkit-rs-*/crates/dicom-toolkit-data/src/json.rs:203-227`
