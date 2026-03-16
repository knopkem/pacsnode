# dicom-toolkit-rs follow-up status

Most of the previously tracked pacsnode DICOMweb and DIMSE follow-up items have
landed upstream as of `dicom-toolkit-rs` commit `55eb062`.

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
- classic JPEG Lossless encode support for:
  - `1.2.840.10008.1.2.4.57`
  - `1.2.840.10008.1.2.4.70`
- C-FIND response dataset encoding now honors the negotiated presentation-context
  transfer syntax instead of hardcoding Explicit VR Little Endian

---

## Current remaining upstream work

There are no currently tracked pacsnode DICOMweb/DIMSE blockers in
`dicom-toolkit-rs`.

---

## Source references

- toolkit upstream status checked against local `dcmtk-rs` checkout at commit
  `55eb06228ce39e59b017c1c60167a57721665e42`
- pacsnode now consumes the toolkit encapsulated Pixel Data builder from:
  - `crates/pacs-dicom/src/wado.rs`
- toolkit helper definitions live in:
  - `../dcmtk-rs/crates/dicom-toolkit-data/src/value.rs`
- classic JPEG Lossless encode capability now lives in:
  - `../dcmtk-rs/crates/dicom-toolkit-codec/src/registry.rs`
  - `../dcmtk-rs/crates/dicom-toolkit-codec/src/jpeg/lossless_encoder.rs`
- C-FIND negotiated-transfer-syntax encoding fix now lives in:
  - `../dcmtk-rs/crates/dicom-toolkit-net/src/services/find.rs`
