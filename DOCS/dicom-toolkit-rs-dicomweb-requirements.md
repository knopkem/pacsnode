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

No unresolved `dicom-toolkit-rs` requirements are currently tracked for this
DICOMweb follow-up.

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
