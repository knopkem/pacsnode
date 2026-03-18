# pacsnode — Feature Matrix & Gap Analysis

> Generated: 2026-03-16 | Based on source audit of `crates/` and comparison with Orthanc and
> enterprise PACS requirements.

---

## Legend

| Symbol | Meaning |
|--------|---------|
| ✅ | Fully implemented and tested |
| ⚠️ | Partially implemented or accept-only |
| ❌ | Not implemented |
| 🔮 | Planned (in roadmap / plan docs) |
| ➖ | Not applicable or out of scope |

---

## 1. DIMSE Network Services

| Feature | pacsnode | Orthanc | Notes |
|---------|:--------:|:-------:|-------|
| **C-ECHO SCP** | ✅ | ✅ | Verification SOP class |
| **C-ECHO SCU** | ✅ | ✅ | `DicomClient::echo()` |
| **C-STORE SCP** (receive) | ✅ | ✅ | Stores to S3 + PostgreSQL |
| **C-STORE SCU** (send) | ✅ | ✅ | `DicomClient::store()`, up to 128 SOP classes/assoc |
| **C-FIND SCP** (Patient) | ✅ | ✅ | Patient-level dedup, wildcard name matching |
| **C-FIND SCP** (Study) | ✅ | ✅ | Date range, modality, accession filters |
| **C-FIND SCP** (Series) | ✅ | ✅ | Hierarchical study→series resolution |
| **C-FIND SCP** (Image) | ✅ | ✅ | study→series→instance resolution |
| **C-FIND SCU** | ✅ | ✅ | `DicomClient::find()` — Study/Patient Root |
| **C-MOVE SCP** | ✅ | ✅ | Dynamic node lookup from registry |
| **C-MOVE SCU** | ✅ | ✅ | `DicomClient::move_instances()` |
| **C-GET SCP** | ✅ | ✅ | Returns matching instances to requester |
| **C-CANCEL** | ❌ | ✅ | No in-progress operation cancellation |
| **Storage Commitment** (N-EVENT-REPORT) | ❌ | ✅ (plugin) | Not implemented |
| **Modality Worklist** (MWL SCP) | ❌ | ✅ (plugin) | No worklist management |
| **Modality Performed Procedure Step** (MPPS) | ❌ | ❌ | Neither implements natively |
| **Association negotiation** | ✅ | ✅ | Configurable SCP-side transfer-syntax allow-list/preference order via `accept_all_transfer_syntaxes`, `accepted_transfer_syntaxes`, and `preferred_transfer_syntaxes` |
| **Max concurrent associations** | ✅ | ✅ | Configurable (default 64), semaphore-based |
| **DIMSE timeout** | ✅ | ✅ | Configurable (default 30s) |
| **AE title validation** | ✅ | ✅ | Optional registered-node whitelist rejects unknown calling AEs before DIMSE requests are handled |
| **TLS for DIMSE** | ❌ | ✅ | Plaintext TCP only |

---

## 2. DICOMweb Services

| Feature | pacsnode | Orthanc | Notes |
|---------|:--------:|:-------:|-------|
| **QIDO-RS** — Search Studies | ✅ | ✅ (plugin) | PatientID, Name, Date, Accession, Modality, UID |
| **QIDO-RS** — Search Series | ✅ | ✅ (plugin) | SeriesUID, Modality, SeriesNumber |
| **QIDO-RS** — Search Instances | ✅ | ✅ (plugin) | SOPUID, SOPClass, InstanceNumber |
| **QIDO-RS** — Pagination | ✅ | ✅ (plugin) | `limit` + `offset` params |
| **QIDO-RS** — Fuzzy matching | ✅ | ✅ (plugin) | ILIKE-based (not full-text search) |
| **WADO-RS** — Retrieve Study | ✅ | ✅ (plugin) | `multipart/related; type=application/dicom` |
| **WADO-RS** — Retrieve Series | ✅ | ✅ (plugin) | |
| **WADO-RS** — Retrieve Instance | ✅ | ✅ (plugin) | |
| **WADO-RS** — Study Metadata | ✅ | ✅ (plugin) | `application/dicom+json` |
| **WADO-RS** — Series Metadata | ✅ | ✅ (plugin) | |
| **WADO-RS** — Instance Metadata | ✅ | ✅ (plugin) | |
| **WADO-RS** — Frame Retrieval | ✅ | ✅ (plugin) | `/frames/{n[,m...]}` returns native frame bytes |
| **WADO-RS** — Rendered (thumbnail/preview) | ✅ | ✅ (plugin) | PNG/JPEG rendered responses, `Accept` negotiation, rendered query parameters, and multipart rendered frame responses |
| **WADO-RS** — Bulk Data | ✅ | ✅ (plugin) | Nested attribute-path bulk-data retrieval, multipart `Content-Location`, and `BulkDataURI` coverage for eligible binary attributes |
| **WADO-URI** (legacy) | ✅ | ✅ (plugin) | Supports `application/dicom` plus rendered PNG/JPEG responses, rendered query parameters, and WADO-URI transfer-syntax rules |
| **STOW-RS** — Store Instances | ✅ | ✅ (plugin) | Multipart DICOM upload, PS3.18 response |
| **UPS-RS** (Worklist) | ❌ | ❌ | Neither implements natively |
| **Capabilities / Conformance** | ✅ | ✅ | `GET /system` returns AE, ports, nodes |

---

## 3. REST API (Non-DICOMweb)

| Feature | pacsnode | Orthanc | Notes |
|---------|:--------:|:-------:|-------|
| **List/Get Studies** | ✅ | ✅ | `GET /api/studies`, `GET /api/studies/{uid}` |
| **List/Get Series** | ✅ | ✅ | `GET /api/studies/{uid}/series`, `GET /api/series/{uid}` |
| **List/Get Instances** | ✅ | ✅ | `GET /api/series/{uid}/instances`, `GET /api/instances/{uid}` |
| **Delete Study** | ✅ | ✅ | `DELETE /api/studies/{uid}` |
| **Delete Series** | ✅ | ✅ | `DELETE /api/series/{uid}` |
| **Delete Instance** | ✅ | ✅ | `DELETE /api/instances/{uid}` |
| **Node Registry CRUD** | ✅ | ✅ | `GET/POST/DELETE /api/nodes` |
| **Health Check** | ✅ | ✅ | `GET /health` → `{"status":"ok"}` |
| **Statistics** | ✅ | ✅ | Study/series/instance counts, disk usage |
| **System Info** | ✅ | ✅ | AE title, ports, version, registered nodes |
| **Anonymization** | ❌ | ✅ | Orthanc: per-patient/study/series/instance |
| **Tag Modification** | ❌ | ✅ | Orthanc: in-place DICOM tag editing |
| **Merge Studies/Series** | ❌ | ✅ | Orthanc: combine resources |
| **Split Series** | ❌ | ✅ | Orthanc: reorganize instances |
| **DICOM-to-PNG/JPEG** | ❌ | ✅ | Orthanc: rendered image preview |
| **ZIP/Media Export** | ❌ | ✅ | Orthanc: download as ZIP or DICOMDIR |
| **Async Job Queue** | ❌ | ✅ | Orthanc: `/jobs` API for long tasks |
| **Lua/Python Scripting** | ❌ | ✅ | Orthanc: server-side automation |
| **Peer-to-Peer Sync** | ❌ | ✅ | Orthanc: replicate between Orthanc instances |
| **Plugin System** | ✅ | ✅ | Compile-time trait-based plugin system with built-in storage/DIMSE plugins and optional auth/audit/metrics plugins |
| **User Management** | ✅ | ✅ (plugin) | Local DB-backed users, bootstrap admin CLI, password policy, refresh tokens, and admin dashboard/API support are implemented; groups and external provisioning are still missing |
| **Admin Dashboard** | ✅ | ✅ (plugin) | Optional admin web UI covers users, password policy, nodes, server settings, and audit review |
| **Audit Log API** | ✅ | ✅ (plugin) | `GET /api/audit/logs` and `GET /api/audit/logs/{id}` provide filtered review/search over the append-only audit trail |

---

## 4. Transfer Syntax & Codec Support

| Transfer Syntax | pacsnode | Orthanc | Notes |
|----------------|:--------:|:-------:|-------|
| **Implicit VR Little Endian** (1.2.840.10008.1.2) | ✅ | ✅ | Native retrieve target and retrieve-time transcode target |
| **Explicit VR Little Endian** (1.2.840.10008.1.2.1) | ✅ | ✅ | Native encoding format and retrieve-time transcode target |
| **Explicit VR Big Endian** (1.2.840.10008.1.2.2) | ✅ | ✅ | Big-endian retrieve-time transcode and rendering path covered |
| **Deflated Explicit VR LE** (1.2.840.10008.1.2.1.99) | ✅ | ✅ | Read/write plus WADO retrieve-time transcode verified |
| **JPEG Baseline** (1.2.840.10008.1.2.4.50) | ✅ | ✅ | Toolkit-backed decode plus retrieve-time transcode/output exercised in tests |
| **JPEG Lossless** (1.2.840.10008.1.2.4.57/70) | ✅ | ✅ | Toolkit-backed decode plus retrieve-time transcode/output verified for both classic JPEG Lossless UIDs |
| **JPEG 2000 Lossless** (1.2.840.10008.1.2.4.90) | ✅ | ✅ | Toolkit-backed decode plus lossless retrieve-time transcode verified |
| **JPEG 2000 Lossy** (1.2.840.10008.1.2.4.91) | ⚠️ | ✅ | Retrieve-time output path is wired, but lossy quality/interoperability coverage is still thin |
| **RLE Lossless** (1.2.840.10008.1.2.5) | ✅ | ✅ | Toolkit-backed decode plus retrieve-time transcode verified |
| **MPEG-2/4** | ❌ | ⚠️ | Neither fully supports |
| **Server-side transcoding** | ✅ | ✅ | WADO-RS/WADO-URI retrieve plus DIMSE C-GET/C-MOVE can transcode into the supported output syntaxes, including classic JPEG Lossless |
| **`Accept` header negotiation** | ✅ | ✅ | WADO-RS retrieve honors `Accept` transfer-syntax requests for DICOM object retrieval |

---

## 5. Storage & Archiving

| Feature | pacsnode | Orthanc | Notes |
|---------|:--------:|:-------:|-------|
| **S3-compatible blob store** | ✅ | ✅ (plugin) | Native S3/MinIO/RustFS support |
| **Local filesystem storage** | ❌ | ✅ | Orthanc default; pacsnode requires S3 |
| **Presigned URLs** | ✅ | ❌ | Direct S3 access for large transfers |
| **Hierarchical blob keys** | ✅ | ➖ | `study/series/instance` path layout |
| **Storage commitment** | ❌ | ✅ (plugin) | N-EVENT-REPORT not implemented |
| **Compression at rest** | ❌ | ✅ | No blob-level compression |
| **Content deduplication** | ❌ | ❌ | Neither implements natively |
| **Multi-tier storage** (HSM) | ❌ | ✅ (plugin) | No hot/warm/cold tiering |
| **Retention policies** | ❌ | ❌ | No auto-delete or lifecycle rules |
| **Blob cleanup on DELETE** | ✅ | ✅ | REST deletes now remove descendant S3 blobs and dedupe repeated blob keys before object-store cleanup |
| **Backup/restore** | ❌ | ⚠️ | Relies on PostgreSQL + S3 backup tools |

---

## 6. Database & Querying

| Feature | pacsnode | Orthanc | Notes |
|---------|:--------:|:-------:|-------|
| **PostgreSQL** | ✅ | ✅ (plugin) | Native; compile-time verified queries (sqlx) |
| **SQLite** | ❌ | ✅ | Orthanc default for simple deployments |
| **MySQL/MariaDB** | ❌ | ✅ (plugin) | |
| **Compile-time query verification** | ✅ | ❌ | `sqlx::query!` macros — unique to pacsnode |
| **Migration management** | ✅ | ✅ | sqlx-cli migrations |
| **JSONB metadata storage** | ✅ | ❌ | Full DICOM JSON in PostgreSQL JSONB |
| **GIN indexes on metadata** | ✅ | ❌ | Fast JSON path queries |
| **Full-text search** (tsvector) | ❌ | ❌ | Neither implements PostgreSQL FTS |
| **Date range queries** | ✅ | ✅ | Study date from/to |
| **Modality filtering** | ✅ | ✅ | Study and series level |
| **Fuzzy name matching** | ✅ | ✅ | ILIKE-based prefix/suffix |
| **Count triggers** | ✅ | ❌ | Auto-maintain series/instance counts |

---

## 7. Security & Compliance

| Feature | pacsnode | Orthanc | Notes |
|---------|:--------:|:-------:|-------|
| **HTTP TLS/HTTPS** | ❌ 🔮 | ✅ | Plaintext only; use reverse proxy |
| **DIMSE TLS** | ❌ | ✅ | Plaintext TCP only |
| **Authentication** (any) | ✅ | ✅ | Optional `basic-auth` plugin provides local multi-user login, refresh-token rotation, and external bearer-token validation for secured routes |
| **RBAC / Role-based access** | ⚠️ | ✅ (plugin) | Five-role model and route-level authorization are implemented; policy scope is still expanding |
| **JWT token validation** | ✅ | ❌ | `basic-auth` plugin issues and validates JWT bearer tokens |
| **OIDC / OAuth2** | ⚠️ | ✅ (plugin) | External bearer-token validation works with issuer discovery, JWKS, or static RSA keys; interactive browser login flow remains external |
| **API key auth** | ❌ 🔮 | ✅ | Planned as Phase 1 |
| **Audit logging** | ✅ | ✅ (plugin) | `audit-logger` persists store/query/delete/study-complete/association events to `audit_log` and auto-enables for secured `basic-auth` deployments unless explicitly opted out |
| **PHI redaction in logs** | ⚠️ | ✅ | Policy stated but no filter enforced |
| **Encryption at rest** | ❌ | ❌ | Neither implements natively (delegate to infra) |
| **CORS configuration** | ⚠️ | ✅ | Currently `permissive()`; needs tightening |
| **Rate limiting** | ❌ 🔮 | ❌ | Planned for login endpoint |
| **Account lockout** | ✅ | ❌ | Enforced by the persisted password policy for local accounts |
| **HIPAA compliance** | ❌ | ⚠️ | Requires audit trail + access controls |
| **DICOM Conformance Statement** | ❌ | ✅ | No formal conformance document |

---

## 8. Viewer & User Interface

| Feature | pacsnode | Orthanc | Notes |
|---------|:--------:|:-------:|-------|
| **Built-in web UI** | ⚠️ | ✅ | Optional admin dashboard ships with pacsnode, and `ohif-viewer` can host OHIF assets, but there is no bundled diagnostic worklist/viewer shell yet |
| **OHIF Viewer integration** | ✅ | ✅ (plugin) | Optional `ohif-viewer` plugin serves a pre-built OHIF distribution with SPA fallback and optional root redirect |
| **Stone Web Viewer** | ❌ | ✅ (plugin) | Orthanc-specific advanced viewer |
| **Custom study list / worklist UI** | ❌ 🔮 | ❌ | Planned: `@pacsnode/extension-worklist` |
| **Static file serving** | ✅ | ✅ | `ohif-viewer` plugin validates and serves static SPA assets from a configured directory |
| **Server-side rendering** (thumbnails) | ⚠️ | ✅ | DICOMweb rendered PNG/JPEG endpoints exist; no integrated study list/worklist thumbnail flow yet |

---

## 9. System & Operations

| Feature | pacsnode | Orthanc | Notes |
|---------|:--------:|:-------:|-------|
| **Health endpoint** | ✅ | ✅ | |
| **Statistics endpoint** | ✅ | ✅ | |
| **System info endpoint** | ✅ | ✅ | |
| **Structured logging** (JSON) | ✅ | ⚠️ | tracing with JSON/pretty output |
| **Configurable log levels** | ✅ | ✅ | Per-crate granularity |
| **TOML + env config** | ✅ | ✅ (JSON) | `PACS_` prefix, `__` separator |
| **Graceful shutdown** | ✅ | ✅ | HTTP + DIMSE coordinated |
| **DB connection pooling** | ✅ | ✅ | PgPool, configurable max |
| **DIMSE connection limiting** | ✅ | ✅ | Semaphore-based (default 64) |
| **Docker support** | ✅ | ✅ | Multi-stage build, docker-compose |
| **Database migrations** | ✅ | ✅ | sqlx-cli, auto-run on startup |
| **Async job queue** | ❌ | ✅ | Orthanc: `/jobs` API |
| **Prometheus metrics** | ✅ | ✅ (plugin) | Optional `prometheus-metrics` plugin exposes `/metrics` plus HTTP latency and PACS event counters |
| **Clustering / HA** | ❌ | ⚠️ | Neither has native clustering |
| **Hot config reload** | ❌ | ❌ | Requires restart |

---

## 10. Advanced / Enterprise PACS Features

These features are found in professional/enterprise PACS systems (Sectra, Fujifilm Synapse,
Philips, GE, Intelerad) but are typically beyond the scope of open-source PACS like Orthanc.
Listed here for completeness and long-term roadmap consideration.

| Feature | pacsnode | Orthanc | Enterprise PACS | Priority |
|---------|:--------:|:-------:|:---------------:|:--------:|
| **Hanging protocols** | ❌ | ❌ | ✅ | Medium |
| **Prior study prefetch** | ❌ | ⚠️ (Lua) | ✅ | Medium |
| **AI/ML integration pipeline** | ❌ | ⚠️ (plugin) | ✅ | Low |
| **HL7 / FHIR integration** | ❌ | ❌ | ✅ | Medium |
| **RIS integration** | ❌ | ❌ | ✅ | Medium |
| **Modality Worklist (MWL)** | ❌ | ✅ (plugin) | ✅ | High |
| **Report generation / DICOM SR** | ❌ | ⚠️ | ✅ | Medium |
| **Key Image Notes** | ❌ | ❌ | ✅ | Low |
| **Speech recognition / dictation** | ❌ | ❌ | ✅ | Low |
| **Annotations persistence** | ❌ | ❌ | ✅ | Medium |
| **Study sharing / URL links** | ❌ | ⚠️ | ✅ | Medium |
| **Multi-site / federation** | ❌ | ✅ (peers) | ✅ | Low |
| **Vendor Neutral Archive** (VNA) | ⚠️ | ⚠️ | ✅ | Low |
| **IHE profile compliance** | ❌ | ⚠️ | ✅ | Medium |
| **Disaster recovery / replication** | ❌ | ❌ | ✅ | Medium |
| **Teaching file management** | ❌ | ✅ (plugin) | ✅ | Low |
| **Anonymization / de-identification** | ❌ | ✅ | ✅ | High |
| **Patient merge / reconciliation** | ❌ | ❌ | ✅ | Medium |
| **Cross-enterprise document sharing** (XDS) | ❌ | ❌ | ✅ | Low |
| **Mobile / tablet viewer** | ❌ 🔮 | ⚠️ | ✅ | Medium |
| **Teleradiology support** | ❌ | ⚠️ | ✅ | Medium |

---

## Summary Scorecard

| Category | pacsnode | Orthanc | Gap |
|----------|:--------:|:-------:|:---:|
| **DIMSE Services** | 88% | 95% | C-CANCEL, Storage Commitment, MWL |
| **DICOMweb** | 95% | 95% | Main remaining gap is UPS-RS/worklist surface, which Orthanc also lacks natively |
| **REST API** | 75% | 90% | Biggest gaps are anonymize/modify/merge/split/export/jobs rather than core administration |
| **Transfer Syntax / Codecs** | 80% | 85% | Main remaining gaps are classic JPEG Lossless encode support, lossy J2K hardening, and MPEG |
| **Storage** | 85% | 85% | Main remaining gaps are storage commitment, compression-at-rest, and lifecycle/retention tooling |
| **Database** | 95% | 85% | pacsnode ahead: JSONB, GIN, sqlx compile-time |
| **Security** | 65% | 60% | Main remaining gaps are policy coverage expansion, API keys, TLS, CORS hardening, and PHI log filtering |
| **Viewer / UI** | 55% | 70% | Admin UI and OHIF hosting now exist, but a bundled diagnostic worklist/viewer shell is still missing |
| **System / Ops** | 95% | 85% | Main remaining gaps are async jobs, HA/federation work, and hot reload |
| **Enterprise Features** | 5% | 25% | Long-term roadmap items |

---

## Recommended Priority: What to Build Next

### 🔴 Critical (blocks clinical use)

1. **Authorization coverage hardening** — login, roles, and OIDC bearer validation exist, but policy coverage still needs to be completed and reviewed across every workflow
2. **TLS termination** — at minimum via reverse proxy (Nginx/Caddy), ideally native
3. **CORS tightening** — replace `permissive()` with configured origins
4. **PHI log filtering** — enforce the stated “no PHI in logs” policy at the logging boundary

### 🟡 High (important for interoperability)

5. **Classic JPEG Lossless output** — DIMSE transfer-syntax policy wiring is done, but pacsnode still needs upstream classic JPEG Lossless encode support to emit 1.2.840.10008.1.2.4.57/70 on retrieve
6. **Anonymization API** — essential for research, sharing, and compliance
7. **Clinical worklist / bundled UI on top of OHIF hosting** — the viewer host exists, but a user-facing study/worklist shell is still missing
8. **Modality Worklist (MWL)** — required for integration with modalities/RIS
9. **DICOM Conformance Statement** — required for hospital procurement

### 🟢 Medium (quality of life / enterprise)

10. **ZIP/DICOMDIR export** — downloading studies for CD/USB
11. **Async job queue** — long-running ops (anonymize, export) shouldn't block
12. **Metrics dashboards / deeper instrumentation** — the `/metrics` endpoint exists, but production dashboards and broader coverage are still needed
13. **HL7/FHIR integration** — hospital system interop
14. **Prior study prefetch** — radiology workflow optimization
15. **Full-text search** — PostgreSQL tsvector for patient/study search
16. **Server-side thumbnails** — faster study browsing in viewer
17. **Study sharing URLs** — secure links for referring physicians

### 🔵 Low (nice to have / long-term)

18. **Storage commitment** (N-EVENT-REPORT)
19. **Multi-site federation / peer sync**
20. **AI/ML integration pipeline**
21. **Plugin ecosystem expansion** (anonymization, codecs, HL7, export)
22. **Teaching file management**
23. **Patient merge / reconciliation**

---

## pacsnode Strengths vs Orthanc

| Advantage | Details |
|-----------|---------|
| **Modern async Rust** | Tokio-based, zero-cost abstractions, memory safe |
| **Compile-time SQL** | `sqlx::query!` prevents SQL injection and schema drift |
| **Cloud-native storage** | S3 blob store is first-class, not a plugin |
| **JSONB metadata** | Full DICOM JSON in PostgreSQL with GIN indexes |
| **Structured logging** | `tracing` with JSON output, per-crate log levels |
| **Type-safe DIMSE** | Rust type system prevents protocol-level bugs |
| **Presigned URLs** | Direct S3 access bypasses server for large transfers |
| **Configurable via env** | Docker/K8s friendly with `PACS_` prefix convention |
