# pacsnode — Detailed API Surface & Feature Survey

## Executive Summary

**pacsnode** is a modern PACS implementation in Rust offering:
- **DICOMweb**: QIDO-RS, WADO-RS, WADO-URI, STOW-RS ✅ (95% complete)
- **DIMSE**: C-ECHO, C-STORE, C-FIND, C-MOVE, C-GET (SCP + SCU) ✅ (88% complete)
- **REST API**: Study/Series/Instance CRUD, audit logs, node registry ✅ (65% complete)
- **Transfer Syntaxes**: 11 major syntaxes supported with server-side transcoding ✅
- **Backend**: PostgreSQL + S3 (default), or SQLite + filesystem (standalone)
- **Not for clinical use** — not validated for diagnostic/therapeutic purposes

**Key Architectural Advantage**: Hybrid JSONB schema in PostgreSQL for complete metadata + indexed relational columns for QIDO performance.

---

## 1. CRATE STRUCTURE & PURPOSE

| Crate | Purpose | Key Dependencies |
|-------|---------|-----------------|
| **pacs-core** | Domain types (Study, Series, Instance), traits (MetadataStore, BlobStore), error types | serde, chrono, uuid |
| **pacs-dicom** | DICOM parsing bridge to dicom-toolkit-rs; tag extraction, JSON conversion, rendering | dicom-toolkit-{core,codec,image}, png, jpeg-encoder |
| **pacs-api** | Axum HTTP server for DICOMweb (STOW/QIDO/WADO-RS) + REST endpoints | axum, multer, tower-http |
| **pacs-dimse** | DIMSE SCP server + SCU client for C-ECHO, C-STORE, C-FIND, C-MOVE, C-GET | dicom-toolkit-net, tokio |
| **pacs-store** | PostgreSQL MetadataStore impl using sqlx with JSONB + relational hybrid schema | sqlx, chrono |
| **pacs-sqlite-store** | SQLite MetadataStore impl for standalone deployments | sqlx |
| **pacs-storage** | S3/RustFS BlobStore impl for pixel data | object_store, http |
| **pacs-fs-storage** | Filesystem BlobStore impl for standalone deployments | tokio |
| **pacs-server** | Binary entry point; config loading, startup wiring, graceful shutdown | all of above + config, tracing |
| **pacs-plugin** | Plugin trait registry, event bus (InstanceStored, AssociationEstablished, etc.) | axum, inventory, serde_json |
| **pacs-auth-plugin** | Basic JWT auth: login endpoint, bearer token validation, local credential support | jsonwebtoken, argon2 |
| **pacs-audit-plugin** | Event-based audit logging (append-only trail of store/query/delete/association events) | minimal |
| **pacs-admin-plugin** | Admin dashboard (HTML template-based with Askama) + admin API endpoints | askama, askama_axum |
| **pacs-metrics-plugin** | Prometheus `/metrics` endpoint with HTTP latency + PACS event counters | tokio |
| **pacs-viewer-plugin** | Static OHIF viewer hosting; serves pre-built SPA, falls back to index.html for SPA nav | zip, tower-http |

---

## 2. HTTP API ROUTES (pacs-api)

### 2.1 DICOMweb Endpoints

#### QIDO-RS (Query)
```
GET /wado/studies                                    → Search studies
GET /wado/studies/{study_uid}/series                → Search series in study
GET /wado/studies/{study_uid}/series/{series_uid}/instances → Search instances
```
- **Supported filters**: PatientID, PatientName, StudyDate (range), Modality, SeriesUID, SOPInstanceUID, Accession, SeriesNumber, InstanceNumber
- **Pagination**: `limit` and `offset` query parameters
- **Matching**: ILIKE-based fuzzy matching (not full-text search)
- **Response format**: application/dicom+json

#### STOW-RS (Store)
```
POST /wado/studies                                   → Store instances
```
- **Content-Type**: multipart/related; type=application/dicom with boundary
- **Response**: PS3.18 compliant JSON response with stored UIDs
- **Implementation**: Multipart DICOM upload, stores blob + metadata
- **Event**: Emits `InstanceStored` event for plugins (includes source, user_id if authenticated)

#### WADO-RS (Retrieve)
```
GET /wado/studies/{study_uid}                       → Retrieve all instances in study
GET /wado/studies/{study_uid}/series/{series_uid}  → Retrieve all instances in series
GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid} → Retrieve single instance
GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/frames/{frame_list} → Retrieve specific frames
```
- **Response format**: multipart/related; type=application/dicom
- **Accept header negotiation**: Honors `Accept` header for transfer-syntax preference
- **Server-side transcoding**: Into any supported target syntax on retrieve
- **Frame retrieval**: Returns native frame bytes for multi-frame images (form: `/frames/1,3,5`)

#### WADO-RS Rendered (PNG/JPEG)
```
GET /wado/studies/{study_uid}/rendered
GET /wado/studies/{study_uid}/series/{series_uid}/rendered
GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/rendered
GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/frames/{frame_list}/rendered
```
- **Rendered media types**: PNG, JPEG (quality configurable, default 90)
- **Rendered parameters**: windowCenter, windowWidth, rows, columns, region, annotation
- **Response**: multipart/related; type=image/png or image/jpeg

#### WADO-RS Metadata
```
GET /wado/studies/{study_uid}/metadata
GET /wado/studies/{study_uid}/series/{series_uid}/metadata
GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/metadata
```
- **Response format**: application/dicom+json
- **Includes**: Full DICOM tag set serialized to JSON
- **BulkDataURI**: Nested attributes with binary values include BulkDataURI references

#### WADO-RS Bulk Data
```
GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/bulkdata/{*tag_path}
```
- **Usage**: Retrieve nested binary attributes (e.g., `/bulkdata/00281010/0` for first overlay)
- **Content-Location**: Multipart response with proper MIME types for each binary chunk

#### WADO-URI (Legacy)
```
GET /wado?requestType=WADO&studyUID=...&seriesUID=...&objectUID=...
```
- **Query params**: contentType, transferSyntax, frameNumber, windowCenter, windowWidth, rows, columns, region, annotation
- **Response**: application/dicom or rendered PNG/JPEG
- **Rendered support**: Full parameter negotiation for window/level, ROI, annotations
- **Transfer-syntax rules**: Enforces WADO-URI TS negotiation rules (no TS param for rendered)

### 2.2 REST API Endpoints

#### Study Management
```
GET    /api/studies                                 → List all studies
GET    /api/studies/{study_uid}                     → Get study metadata
DELETE /api/studies/{study_uid}                     → Delete study (cascade deletes series/instances)
```

#### Series Management
```
GET    /api/studies/{study_uid}/series              → List series in study
GET    /api/series/{series_uid}                     → Get series metadata
DELETE /api/series/{series_uid}                     → Delete series (cascade deletes instances)
```

#### Instance Management
```
GET    /api/series/{series_uid}/instances           → List instances in series
GET    /api/instances/{instance_uid}                → Get instance metadata
DELETE /api/instances/{instance_uid}                → Delete instance
```

#### Node Management
```
GET    /api/nodes                                   → List registered DICOM nodes
POST   /api/nodes                                   → Register new DICOM node
DELETE /api/nodes/{ae_title}                        → Remove DICOM node
```
- **Node fields**: AE title, hostname, port, description

#### Audit Logging
```
GET    /api/audit/logs                              → Search audit log entries
GET    /api/audit/logs/{id}                         → Get specific audit log entry
```
- **Events logged**: Store, Query, Delete, StudyComplete, AssociationEstablished
- **Append-only trail**: Immutable log for compliance/forensics

### 2.3 System & Health Endpoints
```
GET    /health                                      → {"status": "ok"}
GET    /statistics                                  → {"studies": N, "series": N, "instances": N, "disk_usage_bytes": B}
GET    /system                                      → {"ae_title": "...", "http_port": 8042, "dimse_port": 4242, "version": "...", "registered_nodes": [...]}
```

### 2.4 Plugin-Contributed Routes

#### Admin Dashboard (pacs-admin-plugin)
- **Route prefix**: `/admin`
- **Endpoints**: Study list UI, node management UI, export forms, association stats
- **Template engine**: Askama (HTML templates)

#### Auth Plugin (pacs-auth-plugin)
- **Routes**:
  - `POST /auth/login` — Submit username + password → returns JWT access_token + expires_in
  - `POST /auth/refresh` — Submit refresh token → returns new access_token
  - **Public paths** (configurable): `/health`, `/metrics`, `/`, `/viewer` (default)
- **Middleware**: Validates bearer token in Authorization header
- **Token format**: JWT with claims: {sub, iss, exp, iat}
- **Password hashing**: Argon2

#### Metrics Plugin (pacs-metrics-plugin)
- **Route**: `GET /metrics` — Prometheus-format metrics
- **Counters**: HTTP request latency, store/query/delete event counts
- **No credentials** on `/metrics` endpoint by default (can be protected by auth)

#### OHIF Viewer Plugin (pacs-viewer-plugin)
- **Route prefix**: `/viewer` (configurable)
- **Features**:
  - Serves static SPA from `./web/viewer/` (default, auto-extracts embedded bundle)
  - Redirects `/` → `/viewer/`
  - SPA fallback: Missing browser routes return index.html
  - Root-level asset aliasing: Serves `/assets/...`, `/bundle...`, `/6409.bundle...` from viewer dir
  - Auto-generated `/viewer/app-config.js`: Configures OHIF to use pacsnode's own `/wado` endpoints
  - **Config params**:
    - `static_dir`: Directory for SPA assets (default: `./web/viewer`)
    - `route_prefix`: URL prefix (default: `/viewer`)
    - `redirect_root`: Redirect `/` to viewer (default: true)
    - `generate_app_config`: Auto-generate OHIF config (default: true)
    - `provision_embedded_bundle`: Auto-extract on startup (default: true)

---

## 3. DICOMweb Capabilities

### Fully Implemented (✅)
- **QIDO-RS**: Study/Series/Instance search with filters, pagination, fuzzy matching
- **WADO-RS**: Study/Series/Instance/Frame retrieval in multipart/related
- **WADO-RS Rendered**: PNG/JPEG with window/level, ROI, size negotiation
- **WADO-RS Metadata**: Full DICOM JSON with BulkDataURI for binary attributes
- **WADO-RS Bulk Data**: Nested attribute-path extraction for binary payloads
- **WADO-URI**: Legacy query-param WADO with rendered + transfer-syntax negotiation
- **STOW-RS**: Multipart DICOM ingest with PS3.18 response
- **Transcoding**: Server-side transcode on WADO retrieve to any supported TS
- **Frame Retrieval**: Per-frame extraction from multi-frame objects

### Not Implemented (❌)
- **UPS-RS** (Unified Procedure Step worklist API) — neither pacsnode nor Orthanc has native UPS-RS
- **Worklist management** — no MWL SOP class support

---

## 4. DIMSE Services (Network)

### SCP (Server / Service Class Provider)
| Service | Status | Details |
|---------|--------|---------|
| **C-ECHO** | ✅ | Verification SOP class; echo test endpoint |
| **C-STORE** | ✅ | Receive instances from modalities; stores to S3 + PostgreSQL |
| **C-FIND** | ✅ | Query by Patient/Study/Series/Image level; hierarchical resolution |
| **C-MOVE** | ✅ | Move instances to destination node; dynamic lookup from registry |
| **C-GET** | ✅ | Return instances directly to requester (no intermediary) |
| **C-CANCEL** | ❌ | No in-progress operation cancellation |
| **Storage Commitment (N-EVENT-REPORT)** | ❌ | Not implemented |
| **Modality Worklist (MWL SCP)** | ❌ | Not implemented; blocks RIS integration |

### SCU (Client / Service Class User)
| Service | Status | Details |
|---------|--------|---------|
| **C-ECHO** | ✅ | `DicomClient::echo()` |
| **C-STORE** | ✅ | `DicomClient::store()`; up to 128 SOP classes per association |
| **C-FIND** | ✅ | `DicomClient::find()`; Patient Root + Study Root |
| **C-MOVE** | ✅ | `DicomClient::move_instances()` |
| **C-GET** | ❓ | Toolkit support present but not exposed in public API |

### DIMSE Configuration
- **Association negotiation**: Configurable SCP-side transfer-syntax accept list
  - `accept_all_transfer_syntaxes = true` (default)
  - `accepted_transfer_syntaxes = [...]` whitelist mode
  - `preferred_transfer_syntaxes = [...]` TS preference order
- **Max concurrent associations**: Semaphore-based (configurable, default 64)
- **DIMSE timeout**: Configurable (default 30 seconds)
- **AE title validation**: Optional registered-node whitelist; rejects unknown calling AEs before DIMSE processing
- **TLS support**: ❌ Plaintext TCP only (use reverse proxy or network isolation)
- **PDU handling**: Respects requestor's max_pdu_length for outbound fragmentation (not SCP's configured limit)

---

## 5. Transfer Syntax & Codec Support

| Transfer Syntax | pacsnode | Retrieve | Transcode | Notes |
|-----------------|:--------:|:--------:|:---------:|-------|
| **Implicit VR LE** (1.2.840.10008.1.2) | ✅ | ✅ | ✅ | Native target |
| **Explicit VR LE** (1.2.840.10008.1.2.1) | ✅ | ✅ | ✅ | Native format |
| **Explicit VR BE** (1.2.840.10008.1.2.2) | ✅ | ✅ | ✅ | Big-endian transcode |
| **Deflated Explicit VR LE** (1.2.840.10008.1.2.1.99) | ✅ | ✅ | ✅ | Read/write + transcode |
| **JPEG Baseline** (1.2.840.10008.1.2.4.50) | ✅ | ✅ | ✅ | Toolkit-backed decode + transcode |
| **JPEG Lossless** (1.2.840.10008.1.2.4.57/70) | ✅ | ✅ | ✅ | Both classic UIDs; encode via toolkit |
| **JPEG 2000 Lossless** (1.2.840.10008.1.2.4.90) | ✅ | ✅ | ✅ | Pure Rust; lossless transcode verified |
| **JPEG 2000 Lossy** (1.2.840.10008.1.2.4.91) | ⚠️ | ✅ | ✅ | Output path wired; lossy coverage thin |
| **RLE Lossless** (1.2.840.10008.1.2.5) | ✅ | ✅ | ✅ | PackBits; toolkit-backed transcode verified |
| **MPEG-2/4** | ❌ | ❌ | ❌ | Not supported by toolkit |

**Key points**:
- **Server-side transcoding** available for all WADO-RS/WADO-URI retrieve + DIMSE C-GET/C-MOVE
- **Accept header negotiation** honored for WADO-RS; transfers in requested syntax if supported
- **Retrieve-time transcode**: No performance cost if data already in target syntax (identity check)
- **Lossy J2K**: Path exists but interoperability testing incomplete

---

## 6. Storage & Data Architecture

### Metadata Storage (PostgreSQL)
- **Schema**: Hybrid relational + JSONB document
  - **Indexed columns** (~20 tags): PatientID, PatientName, StudyDate, Modality, StudyUID, SeriesUID, SOPInstanceUID, etc.
  - **Full JSON**: Complete tag set in JSONB `metadata` column
  - **GIN index**: Fast JSON path queries for less common tags
  - **Count triggers**: Auto-maintain series/instance counts on insert/delete

### Blob Storage (S3-Compatible)
- **Default**: RustFS (Rust-native, 2.3x faster than MinIO for small objects)
- **Alternative**: Any S3-compatible object store (MinIO, AWS S3, etc.)
- **Presigned URLs**: Direct S3 access bypasses server for large transfers
- **Hierarchical keys**: `study/{study_uid}/series/{series_uid}/instance/{instance_uid}.dcm`
- **Cleanup on DELETE**: REST deletes remove descendant S3 blobs + dedupe logic

### Standalone Mode (SQLite + Filesystem)
- **Use case**: Dev/eval, single machine, non-production
- **Limitations**: Single-writer SQLite; no GIN indexing; slower at scale
- **Build**: `cargo build --release --no-default-features --features standalone`

---

## 7. Authentication & Authorization

### Current (⚠️ Basic)
- **Optional plugin**: `pacs-auth-plugin` (basic JWT auth)
- **Features**:
  - Single hardcoded local credential (username + Argon2 password hash)
  - Login endpoint: `POST /auth/login` → returns JWT access_token
  - Refresh endpoint: `POST /auth/refresh` → new access_token
  - Bearer token validation: JWT with {sub, iss, exp, iat}
  - Public path whitelist (configurable): e.g., `/health`, `/metrics`, `/`, `/viewer`
  - No multi-user support, no groups, no RBAC
- **Token TTL**: Configurable (default not specified in code, check config)
- **Password storage**: Argon2 with configurable params (default from Argon2 crate)

### Gaps (❌)
- ❌ No RBAC (5-role model planned)
- ❌ No OIDC / OAuth2 (planned as roadmap item)
- ❌ No API key authentication (planned as Phase 1)
- ❌ No rate limiting on login endpoint (planned)
- ❌ No account lockout (planned)
- ❌ No user management UI (add/edit/delete users)
- ❌ No group-based access control
- ❌ No per-resource permissions

---

## 8. Audit & Compliance

### Audit Logging (✅)
- **Plugin**: `pacs-audit-plugin` (append-only event trail)
- **Events logged**:
  - InstanceStored (source, user_id, SOP UID)
  - AssociationEstablished (AE title, peer address)
  - Query events (filter params, result count)
  - Delete events (resource UID, reason if provided)
  - StudyComplete (all instances ingested)
- **Query endpoints**: `/api/audit/logs` (search), `/api/audit/logs/{id}` (single)
- **Auto-enable**: Enabled by default when basic-auth plugin is active
- **Opt-out**: Can be explicitly disabled in config

### PHI Redaction (⚠️ Stated but not enforced)
- **Policy stated**: "No PHI in logs"
- **Implementation**: No actual filtering at logging boundary
- **Gaps**: Patient names, MRN still appear in logs (not redacted)

### Compliance (❌)
- ❌ HIPAA compliance requires audit trail + access controls (both partially present)
- ❌ No DICOM Conformance Statement
- ❌ Encryption at rest (delegate to infrastructure)
- ❌ TLS for HTTP (plaintext only; use reverse proxy)

---

## 9. Advanced DICOM Services (Not Implemented)

| Feature | Status | Notes |
|---------|--------|-------|
| **Structured Reports (SR)** | ❌ | No SR storage/retrieval |
| **Segmentations (SEG)** | ❌ | No SEG storage/retrieval |
| **Modality Worklist (MWL)** | ❌ | Blocks RIS/modality integration |
| **MPPS** | ❌ | Modality Performed Procedure Step not supported |
| **Hanging protocols** | ❌ | No viewer-level study layout rules |
| **Prior study prefetch** | ❌ | No proactive prior study loading |
| **Annotations persistence** | ❌ | No KIN (Key Image Notes) storage |
| **Tiled/Pyramid images** | ❌ | No WSI (Whole Slide Image) support |
| **UPS** | ❌ | No Unified Procedure Step API |

---

## 10. Web UI & Viewer Integration

### OHIF Viewer Hosting (✅)
- **Plugin**: `pacs-viewer-plugin`
- **Setup**: Serves pre-built OHIF distribution (or custom SPA)
- **Auto-provisioning**: Extracts embedded OHIF bundle on first startup
- **SPA support**: Falls back to index.html for browser navigation
- **Configuration**:
  - `static_dir = "./web/viewer"` (assets location)
  - `route_prefix = "/viewer"` (URL path)
  - `generate_app_config = true` (auto-generate OHIF config)
  - `provision_embedded_bundle = true` (auto-extract bundle)
- **Generated app-config.js**: Points OHIF at pacsnode's own `/wado` endpoints (qidoRoot, wadoRoot, etc.)
- **Asset rewriting**: Aliases root-level requests (`/assets/...`, `/bundle...`) to viewer directory

### Built-in UI (⚠️ None)
- pacsnode does **not** ship a bundled UI distribution
- OHIF integration is optional; requires separate viewer deployment
- Admin dashboard (HTML-templated) available in pacs-admin-plugin

### Custom Study List / Worklist UI (❌)
- ❌ Planned as `@pacsnode/extension-worklist` (not yet implemented)
- Currently relies on OHIF's default study list

---

## 11. Configuration & Deployment

### Configuration Sources
- **TOML file**: `config.toml` (main config)
- **Environment variables**: `PACS_*` prefix, `__` separator (e.g., `PACS_DATABASE__URL`)
- **Precedence**: Env vars override TOML

### Database Configuration
```toml
[database]
url = "postgresql://user:pass@localhost:5432/pacs"  # or SQLite path
max_connections = 20
```

### DIMSE Configuration
```toml
[dimse]
port = 4242
ae_title = "PACSNODE"
max_associations = 64
dimse_timeout_secs = 30
accept_all_transfer_syntaxes = true
# OR whitelist mode:
# accepted_transfer_syntaxes = ["1.2.840.10008.1.2.1", ...]
registered_nodes_only = false
```

### Storage Configuration
```toml
[storage]
endpoint = "http://localhost:9000"  # MinIO/S3
bucket = "dicom"
access_key = "minioadmin"
secret_key = "minioadmin"

# OR for filesystem (standalone):
[filesystem_storage]
root_dir = "./dicom_blobs"
```

### Plugins Configuration
```toml
[plugins]
enabled = ["pacs-audit-plugin", "pacs-auth-plugin", "pacs-metrics-plugin", "pacs-viewer-plugin"]

[plugins.pacs-auth-plugin]
username = "admin"
password_hash = "..."  # Argon2 hash
jwt_secret = "your-secret-key"
login_path = "/auth/login"
refresh_path = "/auth/refresh"
public_paths = ["/health", "/metrics", "/", "/viewer"]
token_ttl_secs = 3600

[plugins.pacs-viewer-plugin]
static_dir = "./web/viewer"
route_prefix = "/viewer"
redirect_root = true
generate_app_config = true
provision_embedded_bundle = true
```

---

## 12. Middleware & Cross-Cutting Concerns

### HTTP Middleware Stack
- **TraceLayer**: HTTP request/response logging (JSON output)
- **CorsLayer**: Permissive CORS (currently unsafe; needs tightening)
- **TimeoutLayer**: 30-second request timeout
- **RequestBodyLimitLayer**: 500 MiB body size limit
- **CompressionLayer**: gzip/deflate response compression

### Logging
- **Framework**: `tracing` crate (async-aware structured logging)
- **Output**: JSON or pretty-printed (configurable)
- **Per-crate levels**: Granular control (e.g., `RUST_LOG=pacs_api=debug`)

### Metrics
- **Prometheus format**: `/metrics` endpoint (if pacs-metrics-plugin enabled)
- **Counters**: HTTP latency, store/query/delete event counts
- **No**: Deep performance profiling, flamegraph support

---

## 13. Documented Gaps & TODOs

### 🔴 Critical (blocks clinical use)
1. **Authentication & RBAC** — no patient data without login
2. **TLS termination** — plaintext HTTP only; use reverse proxy
3. **CORS tightening** — currently `permissive()`; needs configured origins
4. **PHI log filtering** — enforce "no PHI in logs" at logging boundary

### 🟡 High Priority (interoperability)
5. **Classic JPEG Lossless encode** — toolkit-side support landed; pacsnode wiring needs verification
6. **Anonymization API** — essential for research/compliance
7. **Bundled UI on OHIF** — viewer plugin exists but no user-facing worklist shell
8. **Modality Worklist (MWL)** — required for modality/RIS integration
9. **DICOM Conformance Statement** — required for hospital procurement

### �� Medium Priority (enterprise quality)
10. **ZIP/DICOMDIR export** — download studies for CD/USB
11. **Async job queue** — long-running ops shouldn't block
12. **Deeper metrics/dashboards** — `/metrics` exists, but production instrumentation needed
13. **HL7/FHIR integration** — hospital system interop
14. **Prior study prefetch** — radiology workflow optimization
15. **Full-text search** — PostgreSQL tsvector for patient/study search
16. **Server-side thumbnails** — faster study browsing
17. **Study sharing URLs** — secure links for referring physicians

### 🔵 Low Priority (nice to have)
18. **Storage commitment** (N-EVENT-REPORT)
19. **Multi-site federation / peer sync**
20. **AI/ML integration pipeline**
21. **Teaching file management**
22. **Patient merge / reconciliation**

---

## 14. Strengths vs. Orthanc

| Advantage | Details |
|-----------|---------|
| **Modern async Rust** | Tokio-based, zero-cost abstractions, memory safe |
| **Compile-time SQL** | `sqlx::query!` prevents SQL injection and schema drift |
| **Cloud-native storage** | S3 blob store is first-class; not a plugin |
| **JSONB metadata** | Full DICOM JSON in PostgreSQL with GIN indexes for performance |
| **Structured logging** | `tracing` with JSON, per-crate log levels |
| **Type-safe DIMSE** | Rust type system prevents protocol-level bugs |
| **Presigned URLs** | Direct S3 access bypasses server for large transfers |
| **Docker/K8s friendly** | `PACS_` env var convention, TOML config |

---

## 15. Summary Matrix

| Category | Coverage | Key Gaps |
|----------|:--------:|----------|
| **DIMSE Services** | 88% | C-CANCEL, Storage Commitment, MWL |
| **DICOMweb** | 95% | UPS-RS (neither pacsnode nor Orthanc) |
| **REST API** | 65% | Anonymize, modify, merge, split, export, jobs |
| **Transfer Syntax** | 80% | JPEG Lossless encode, lossy J2K hardening, MPEG |
| **Storage** | 85% | Storage commitment, compression-at-rest, retention policies |
| **Database** | 95% | pacsnode ahead: JSONB, GIN, sqlx compile-time |
| **Security** | 45% | RBAC, OIDC, TLS, CORS, PHI log filtering |
| **Viewer / UI** | 45% | OHIF hosting done; bundled assets + worklist shell missing |
| **System / Ops** | 95% | Async jobs, HA/clustering, hot reload |
| **Enterprise Features** | 5% | Long-term roadmap items |

---

## 16. Deployment Checklist for Production

- [ ] Enable `pacs-auth-plugin` with strong JWT secret
- [ ] Configure PostgreSQL (production-grade: replicas, backups, SSL)
- [ ] Configure S3 (production bucket, IAM, versioning)
- [ ] Enable `pacs-audit-plugin` for audit trail
- [ ] Tighten CORS (replace `permissive()` with allowed origins)
- [ ] Use reverse proxy (Nginx/Caddy) for TLS termination
- [ ] Configure PHI log filtering (custom policy at logging boundary)
- [ ] Implement RBAC policy (currently not available; custom plugin needed)
- [ ] Plan for DICOM Conformance Statement (manual documentation)
- [ ] Set up monitoring (Prometheus scrape `/metrics`, Grafana dashboards)
- [ ] Document admin procedures (node registration, user management, cleanup)
- [ ] Plan for backup/restore (PostgreSQL + S3 backup strategy)

---

## 17. References

- **Feature Matrix**: `/Users/macair/projects/dicom/pacsnode/DOCS/feature-matrix.md`
- **Architecture Plan**: `/Users/macair/projects/dicom/pacsnode/DOCS/final-plan.md`
- **OHIF Integration**: `/Users/macair/projects/dicom/pacsnode/OHIF_VIEWER_INTEGRATION.md`
- **Router**: `/Users/macair/projects/dicom/pacsnode/crates/pacs-api/src/router.rs`
- **DIMSE Server**: `/Users/macair/projects/dicom/pacsnode/crates/pacs-dimse/src/server/mod.rs`
- **Auth Plugin**: `/Users/macair/projects/dicom/pacsnode/crates/pacs-auth-plugin/src/lib.rs`

