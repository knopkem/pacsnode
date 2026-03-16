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
| **Association negotiation** | ✅ | ✅ | Accepts all transfer syntaxes |
| **Max concurrent associations** | ✅ | ✅ | Configurable (default 64), semaphore-based |
| **DIMSE timeout** | ✅ | ✅ | Configurable (default 30s) |
| **AE title validation** | ⚠️ | ✅ | Accepts all callers; no whitelist filtering |
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
| **WADO-RS** — Frame Retrieval | ❌ | ✅ (plugin) | No `/frames/{n}` endpoint |
| **WADO-RS** — Rendered (thumbnail/preview) | ❌ | ✅ (plugin) | No server-side rendering |
| **WADO-RS** — Bulk Data | ❌ | ✅ (plugin) | No `/bulkdata` endpoint |
| **WADO-URI** (legacy) | ❌ | ✅ (plugin) | No `?requestType=WADO` support |
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
| **Plugin System** | ❌ | ✅ | Orthanc: C/Python/Java plugin SDK |
| **User Management** | ❌ 🔮 | ✅ (plugin) | Planned: local users + OIDC |
| **Audit Log API** | ❌ 🔮 | ✅ (plugin) | Schema exists, no code writes to it yet |

---

## 4. Transfer Syntax & Codec Support

| Transfer Syntax | pacsnode | Orthanc | Notes |
|----------------|:--------:|:-------:|-------|
| **Implicit VR Little Endian** (1.2.840.10008.1.2) | ⚠️ | ✅ | Accept-only, no transcode |
| **Explicit VR Little Endian** (1.2.840.10008.1.2.1) | ✅ | ✅ | Native encoding format |
| **Explicit VR Big Endian** (1.2.840.10008.1.2.2) | ⚠️ | ✅ | Accept-only |
| **Deflated Explicit VR LE** (1.2.840.10008.1.2.1.99) | ❌ | ✅ | |
| **JPEG Baseline** (1.2.840.10008.1.2.4.50) | ❌ | ✅ | |
| **JPEG Lossless** (1.2.840.10008.1.2.4.57/70) | ❌ | ✅ | |
| **JPEG 2000 Lossless** (1.2.840.10008.1.2.4.90) | ❌ | ✅ | dicom-toolkit has `Jp2kCodec` but not wired |
| **JPEG 2000 Lossy** (1.2.840.10008.1.2.4.91) | ❌ | ✅ | |
| **RLE Lossless** (1.2.840.10008.1.2.5) | ❌ | ✅ | |
| **MPEG-2/4** | ❌ | ⚠️ | Neither fully supports |
| **Server-side transcoding** | ❌ | ✅ | Orthanc transcodes on retrieve |
| **`Accept` header negotiation** | ❌ | ✅ | WADO-RS content negotiation |

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
| **Blob cleanup on DELETE** | ❌ | ✅ | REST deletes metadata but not S3 blobs |
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
| **Authentication** (any) | ❌ 🔮 | ✅ | Planned: local + OIDC |
| **RBAC / Role-based access** | ❌ 🔮 | ✅ (plugin) | Planned: 5-role model |
| **JWT token validation** | ❌ 🔮 | ❌ | Planned; `jsonwebtoken` crate in deps |
| **OIDC / OAuth2** | ❌ 🔮 | ✅ (plugin) | Planned |
| **API key auth** | ❌ 🔮 | ✅ | Planned as Phase 1 |
| **Audit logging** | ⚠️ 🔮 | ✅ (plugin) | Schema exists; no writes yet |
| **PHI redaction in logs** | ⚠️ | ✅ | Policy stated but no filter enforced |
| **Encryption at rest** | ❌ | ❌ | Neither implements natively (delegate to infra) |
| **CORS configuration** | ⚠️ | ✅ | Currently `permissive()`; needs tightening |
| **Rate limiting** | ❌ 🔮 | ❌ | Planned for login endpoint |
| **Account lockout** | ❌ 🔮 | ❌ | Planned |
| **HIPAA compliance** | ❌ | ⚠️ | Requires audit trail + access controls |
| **DICOM Conformance Statement** | ❌ | ✅ | No formal conformance document |

---

## 8. Viewer & User Interface

| Feature | pacsnode | Orthanc | Notes |
|---------|:--------:|:-------:|-------|
| **Built-in web UI** | ❌ 🔮 | ✅ | Orthanc Explorer (basic); pacsnode plans OHIF |
| **OHIF Viewer integration** | ❌ 🔮 | ✅ (plugin) | Planned: custom mode + extension |
| **Stone Web Viewer** | ❌ | ✅ (plugin) | Orthanc-specific advanced viewer |
| **Custom study list / worklist UI** | ❌ 🔮 | ❌ | Planned: `@pacsnode/extension-worklist` |
| **Static file serving** | ❌ 🔮 | ✅ | Planned: `tower-http::ServeDir` |
| **Server-side rendering** (thumbnails) | ❌ | ✅ | Orthanc renders PNG/JPEG previews |

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
| **Prometheus metrics** | ❌ | ✅ (plugin) | No metrics export |
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
| **DIMSE Services** | 85% | 95% | C-CANCEL, Storage Commitment, MWL |
| **DICOMweb** | 75% | 90% | Frames, rendered, bulk data, WADO-URI |
| **REST API** | 50% | 90% | Anonymize, modify, merge, split, export, jobs |
| **Transfer Syntax / Codecs** | 25% | 85% | JPEG, J2K, RLE, transcoding |
| **Storage** | 80% | 85% | Blob cleanup, commitment, compression |
| **Database** | 95% | 85% | pacsnode ahead: JSONB, GIN, sqlx compile-time |
| **Security** | 10% | 60% | Auth, RBAC, TLS, audit — all planned |
| **Viewer / UI** | 0% | 70% | OHIF integration planned |
| **System / Ops** | 90% | 85% | Missing: job queue, metrics |
| **Enterprise Features** | 5% | 25% | Long-term roadmap items |

---

## Recommended Priority: What to Build Next

### 🔴 Critical (blocks clinical use)

1. **Authentication & RBAC** — no patient data should be accessible without login
2. **TLS termination** — at minimum via reverse proxy (Nginx/Caddy), ideally native
3. **Audit logging** — populate existing `audit_log` table (HIPAA requirement)
4. **Blob cleanup on DELETE** — REST deletes leave orphaned S3 objects
5. **CORS tightening** — replace `permissive()` with configured origins

### 🟡 High (important for interoperability)

6. **WADO-RS frame retrieval** — OHIF needs `/frames/{n}` for efficient viewing
7. **JPEG / JPEG 2000 codecs** — many modalities send compressed; need decode for viewing
8. **Anonymization API** — essential for research, sharing, and compliance
9. **OHIF viewer integration** — web-based viewing is table stakes
10. **Modality Worklist (MWL)** — required for integration with modalities/RIS
11. **DICOM Conformance Statement** — required for hospital procurement

### 🟢 Medium (quality of life / enterprise)

12. **ZIP/DICOMDIR export** — downloading studies for CD/USB
13. **Async job queue** — long-running ops (anonymize, export) shouldn't block
14. **Prometheus metrics** — production monitoring
15. **HL7/FHIR integration** — hospital system interop
16. **Prior study prefetch** — radiology workflow optimization
17. **Full-text search** — PostgreSQL tsvector for patient/study search
18. **Server-side thumbnails** — faster study browsing in viewer
19. **Study sharing URLs** — secure links for referring physicians

### 🔵 Low (nice to have / long-term)

20. **Storage commitment** (N-EVENT-REPORT)
21. **Multi-site federation / peer sync**
22. **AI/ML integration pipeline**
23. **Plugin / extension system**
24. **Teaching file management**
25. **Patient merge / reconciliation**

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
