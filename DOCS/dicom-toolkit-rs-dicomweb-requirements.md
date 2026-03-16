# dicom-toolkit-rs follow-up status

Most of the previously tracked pacsnode DICOMweb and DIMSE follow-up items have
landed upstream as of `dicom-toolkit-rs` commit `493cd42`.

This file stays trimmed to the unresolved upstream work only. When pacsnode
finds a toolkit-side blocker, add it here; when the upstream fix lands, remove
it again.

---

## Implemented upstream

The following toolkit capabilities are now available upstream and should be used
instead of pacsnode-specific patching where possible:

- rendered JPEG export helpers
- DICOM JSON `BulkDataURI` serialization support
- nested attribute-path parsing
- transfer-syntax-aware raw element byte export
- encapsulated frame extraction helpers
- toolkit-owned encapsulated Pixel Data construction via:
  - `build_encapsulated_pixel_data(...)`
  - `encapsulated_pixel_data_from_frames(...)`
- high-level rendered frame options helpers
- separate encode capability reporting
- explicit DIMSE transfer-syntax policy configuration

---

## Current remaining upstream work

- **Classic JPEG Lossless encode support** (`1.2.840.10008.1.2.4.57` /
  `1.2.840.10008.1.2.4.70`)
  - `pacsnode` now wires DIMSE SCP transfer-syntax policy into association
    negotiation and uses retrieve-time transcoding for DIMSE C-GET/C-MOVE.
  - The remaining blocker is upstream: `dicom-toolkit-rs` can decode classic
    JPEG Lossless, but `supported_encode_transfer_syntaxes()` / `can_encode()`
    still exclude `JPEG_LOSSLESS` and `JPEG_LOSSLESS_SV1`.
  - Until that lands upstream, pacsnode cannot truly emit classic JPEG Lossless
    on retrieve even though the rest of the retrieve/transcode path is wired.

If future pacsnode work uncovers a new toolkit bug, missing feature, or cleanup
opportunity, add it here and remove it again once the upstream change lands.

---

## Source references

- toolkit upstream status checked against local `dcmtk-rs` checkout at commit
  `493cd4298c29ceef39ec93021b4dcbb3683473bb`
- pacsnode now consumes the toolkit encapsulated Pixel Data builder from:
  - `crates/pacs-dicom/src/wado.rs`
- toolkit helper definitions live in:
  - `../dcmtk-rs/crates/dicom-toolkit-data/src/value.rs`
- classic JPEG Lossless encode capability is currently absent from:
  - `../dcmtk-rs/crates/dicom-toolkit-codec/src/registry.rs`
  - `../dcmtk-rs/crates/dicom-toolkit-codec/src/jpeg/mod.rs`
