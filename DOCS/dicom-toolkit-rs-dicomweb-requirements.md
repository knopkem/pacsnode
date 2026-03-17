# dicom-toolkit-rs follow-up status

All previously tracked pacsnode DICOMweb and DIMSE follow-up items have landed
upstream as of `dicom-toolkit-rs` commit `521429e`.

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
- PDV message control header byte bit assignment corrected to match the DICOM
  standard (PS3.8 §9.3.1 Table 9-23): bit 0 = command/data, bit 1 = last/not-last
  (was previously swapped in both `Pdv::is_last`/`is_command` and send path)
- SCP-side `recv_pdata` now buffers all PDVs from multi-PDV P-DATA-TF PDUs via
  `pdv_queue: VecDeque<Pdv>` instead of dropping every PDV after the first
- SCP-accepted associations now use the requestor's advertised `max_pdu_length`
  for outbound fragmentation instead of the SCP's own configured limit
- SCP service handlers (`store`, `find`, `get`, `move`) now check
  `CommandDataSetType` before receiving dataset PDVs and use
  `recv_optional_dimse_data()` for graceful empty-dataset handling
- Implicit VR lookup (`vr_for_tag`) moved to `dicom-toolkit-dict` with expanded
  coverage including DIMSE query tags (`QUERY_RETRIEVE_LEVEL`,
  `MODALITIES_IN_STUDY`, `ISSUER_OF_PATIENT_ID`,
  `NUMBER_OF_STUDY_RELATED_SERIES/INSTANCES`, etc.)

---

## Current remaining upstream work

No blocking upstream items at this time.

### Minor / low-priority

- `dicom-toolkit-data` Part 10 parsing (`reader.rs`) does not recover payloads
  that start with group `0002` file meta information in Explicit VR LE but omit
  the 128-byte preamble and `DICM` marker. When the preamble is absent, the
  reader falls back to raw Implicit VR LE from offset 0, which misinterprets
  Explicit VR LE meta headers. pacsnode works around this in
  `decode_store_dataset()` by synthesizing the preamble before retrying, but a
  toolkit-side heuristic (detect group 0002 at offset 0 → try Explicit VR LE
  meta parse first) would eliminate that workaround. In practice this is rarely
  triggered now that SCP dataset receive is working correctly.

---

## Source references

- toolkit upstream status checked against `dicom-toolkit-rs` commit
  `521429e121201478996712a5f447fd9451be1ff7`
- pacsnode now consumes the toolkit encapsulated Pixel Data builder from:
  - `crates/pacs-dicom/src/wado.rs`
- toolkit helper definitions live in:
  - `../dcmtk-rs/crates/dicom-toolkit-data/src/value.rs`
- classic JPEG Lossless encode capability now lives in:
  - `../dcmtk-rs/crates/dicom-toolkit-codec/src/registry.rs`
  - `../dcmtk-rs/crates/dicom-toolkit-codec/src/jpeg/lossless_encoder.rs`
- C-FIND negotiated-transfer-syntax encoding fix now lives in:
  - `../dcmtk-rs/crates/dicom-toolkit-net/src/services/find.rs`
- pacsnode DIMSE interoperability layer:
  - `crates/pacs-dimse/src/server/server_association.rs`
  - `crates/pacs-dimse/src/server/mod.rs`
