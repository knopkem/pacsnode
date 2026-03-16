# dicom-toolkit-rs DICOMweb follow-up status

As of `dicom-toolkit-rs` commit `493cd42`, all toolkit requirements that were
previously tracked for the pacsnode DICOMweb follow-up have landed upstream.

That means this file is no longer an active requirements list. It is now a small
status note so future work can quickly see that the known upstream blockers were
closed and that pacsnode should prefer the toolkit APIs over local workarounds.

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
  `493cd4298c29ceef39ec93021b4dcbb3683473bb`
- pacsnode now consumes the toolkit encapsulated Pixel Data builder from:
  - `crates/pacs-dicom/src/wado.rs`
- toolkit helper definitions live in:
  - `../dcmtk-rs/crates/dicom-toolkit-data/src/value.rs`
