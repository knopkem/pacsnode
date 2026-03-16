# dicom-toolkit-rs follow-up status

Most of the previously tracked pacsnode DICOMweb and DIMSE follow-up items have
landed upstream as of `dicom-toolkit-rs` commit `aa732c8`.

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

---

## Current remaining upstream work

### `handle_find_rq` hardcodes Explicit VR LE for response dataset encoding

**Toolkit file:** `crates/dicom-toolkit-net/src/services/find.rs` — `encode_dataset()`

The private helper always encodes C-FIND response datasets as Explicit VR Little
Endian (`1.2.840.10008.1.2.1`), regardless of the transfer syntax that was
negotiated for the presentation context.  When a client offers Implicit VR LE
(`1.2.840.10008.1.2`) as its first or only transfer syntax the SCP will negotiate
that context but then send back Explicit VR LE bytes, causing the client to send
an A-ABORT (`source=0, reason=1`).

**pacsnode workaround** (`crates/pacs-dimse/src/server/mod.rs`): the
`AssociationConfig` now sets `preferred_transfer_syntaxes` to
`["1.2.840.10008.1.2.1"]` so the server always negotiates Explicit VR LE when
the client offers it.  Clients that offer *only* Implicit VR LE will still hit
this bug until it is fixed upstream.

**Upstream fix needed:** `encode_dataset` (or the call site inside
`handle_find_rq`) should accept the negotiated transfer syntax string as a
parameter and pass it to `DicomWriter::write_dataset` instead of the constant.
The same issue likely exists in `handle_get_rq` and `handle_move_rq` response
encoding.

---

## Source references

- toolkit upstream status checked against local `dcmtk-rs` checkout at commit
  `aa732c8a91a4a6de1d94f3da856581135e42ccda`
- pacsnode now consumes the toolkit encapsulated Pixel Data builder from:
  - `crates/pacs-dicom/src/wado.rs`
- toolkit helper definitions live in:
  - `../dcmtk-rs/crates/dicom-toolkit-data/src/value.rs`
- classic JPEG Lossless encode capability now lives in:
  - `../dcmtk-rs/crates/dicom-toolkit-codec/src/registry.rs`
  - `../dcmtk-rs/crates/dicom-toolkit-codec/src/jpeg/lossless_encoder.rs`
