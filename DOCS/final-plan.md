# pacsnode — Final Architecture & Development Plan

> **Version:** 2.0 • March 2026
> **Foundation:** dicom-toolkit-rs (dcmtk-to-Rust port)
> **Database:** PostgreSQL (exclusive)
> **License:** MIT

---

## 1. Project Overview

pacsnode is a modern, high-performance Picture Archiving and Communication System (PACS) built entirely in Rust. It targets radiology departments and healthcare institutions requiring on-premise deployments with full data sovereignty, HIPAA compliance, and Orthanc-class feature parity — with superior performance.

**Key architectural decisions (vs. prior plans):**
- **dicom-toolkit-rs replaces dicom-rs** — a clean-room Rust port of DCMTK providing complete DIMSE protocol support (SCU plus generic SCP server/provider APIs for C-STORE, C-FIND, C-GET, and C-MOVE), image codecs (JPEG, JPEG-LS, JPEG 2000, RLE), DICOM JSON/XML, TLS, character sets, and async networking. Far more mature DIMSE support than dicom-rs's dicom-ul crate.
- **PostgreSQL is the sole database** — no SurrealDB, no SQLite. Eliminates dual-backend complexity, simplifies the codebase, and leverages PostgreSQL's mature JSONB + GIN indexing for the hybrid relational/document schema that DICOM metadata demands. One backend, one test suite, one deployment target.
- **RustFS retained** for S3-compatible object storage of pixel data.

### 1.1 Current Implementation Status
- Implemented and validated: STOW-RS ingest, QIDO-RS study/series/instance queries, WADO-RS single-instance retrieval, REST health/statistics/system endpoints, and DIMSE C-ECHO/C-STORE/C-FIND.
- Implemented in source and wired through `pacs-dimse`: DIMSE C-GET SCP and C-MOVE SCP using the toolkit server/provider interfaces and destination lookup.
- Verification status: `cargo test --workspace --all-targets` passes locally, and the rebuilt Docker deployment passes the strengthened end-to-end smoke test.
- Remaining major roadmap items are mostly product/platform concerns rather than protocol primitives: auth/RBAC, audit logging, advanced operational workflows, job queueing, modality worklist/MPPS, plugins/hooks, and web UI integration.

**NOT FOR CLINICAL USE** — This software is not a certified medical device. It has not been validated for diagnostic or therapeutic use.

---

## 2. License Summary

All code dependencies are MIT or Apache-2.0 compatible. The project itself is MIT licensed.

| Dependency | License | MIT Compatible |
|---|---|---|
| Axum | MIT | ✅ |
| Tokio | MIT | ✅ |
| dicom-toolkit-rs | MIT OR Apache-2.0 | ✅ |
| object_store | MIT OR Apache-2.0 | ✅ |
| sqlx | MIT OR Apache-2.0 | ✅ |
| RustFS | Apache-2.0 | ✅ |

> **Note on dicom-toolkit-rs:** This is a clean-room Rust port inspired by DCMTK (BSD 3-Clause, OFFIS). It is independently licensed MIT/Apache-2.0. A NOTICE file credits DCMTK and CharLS as algorithmic references. Run `cargo deny check licenses` before each release.

---

## 3. Technology Stack

### 3.1 Core Runtime
- **Tokio** — async runtime (MIT)
- **Axum** — HTTP server framework for REST + DICOMweb APIs (MIT)
- **dicom-toolkit-rs** — DICOM parsing, object handling, codecs, DIMSE protocol, TLS, async networking (MIT/Apache-2.0)
- **object_store** — S3-compatible storage abstraction (MIT/Apache-2.0)

### 3.2 What dicom-toolkit-rs Provides (vs. dicom-rs)

| Capability | dicom-rs | dicom-toolkit-rs |
|---|---|---|
| DICOM file I/O | ✅ | ✅ (4 uncompressed TS + deflate) |
| DICOM JSON (PS3.18) | Partial | ✅ Complete encode + decode |
| DICOM XML (PS3.19) | ❌ | ✅ Complete |
| Character sets | Basic | ✅ 15+ encodings, ISO 2022 |
| C-ECHO SCU/SCP | Basic | ✅ Full |
| C-STORE SCU/SCP | Basic | ✅ Full |
| C-FIND SCU | Partial | ✅ Full |
| C-FIND SCP | ❌ | ✅ Provider API + generic server |
| C-GET SCU | ❌ | ✅ Full |
| C-MOVE SCU | Partial | ✅ Full |
| C-MOVE SCP | ❌ | ✅ Provider API + generic server |
| C-GET SCP | ❌ | ✅ Provider API + generic server |
| Generic DicomServer | ❌ | ✅ Builder + routing + graceful shutdown |
| JPEG baseline codec | ❌ | ✅ Encode + Decode |
| JPEG-LS codec | ❌ | ✅ Pure Rust, lossless + near-lossless |
| RLE codec | ❌ | ✅ PackBits lossless |
| JPEG 2000 codec | ❌ | ✅ Pure Rust encode + decode |
| Image pipeline | Basic | ✅ W/L, LUT, VOI, overlays |
| TLS (rustls) | ❌ | ✅ Client + Server |
| Async networking | ❌ | ✅ tokio-based |
| CLI tools | Limited | ✅ 8 tools (dcmdump, echoscu, storescu, etc.) |
| Test suite | Moderate | ✅ 410 tests (unit + integration + E2E) |

### 3.3 Storage Layer
- **RustFS** — self-hosted S3-compatible object storage for pixel data. 2.3x faster than MinIO for small objects. Entire stack remains Rust-native. (Apache-2.0)
- **PostgreSQL + sqlx** — sole metadata store. Hybrid schema: indexed relational columns for QIDO-common tags + JSONB for full instance metadata. (MIT/Apache-2.0)

### 3.4 Why sqlx (not SeaORM)
SeaORM adds macro-heavy abstraction, runtime overhead, and significantly increases compile times. For high-volume QIDO queries, this overhead is undesirable. sqlx provides compile-time verified SQL with zero ORM abstraction cost — queries are real SQL, giving full control over query plans and index usage.

### 3.5 Why a Hybrid Schema
DICOMweb metadata endpoints must return the full tag set of a DICOM instance — potentially hundreds of tags varying by modality and vendor. A purely relational schema is impractical (sparse, unmaintainable, impossible to extend for private tags).

The hybrid approach:
- **~20 most commonly queried tags** as proper indexed columns (QIDO performance)
- **Complete tag set** as JSONB blob (metadata retrieval)
- **GIN index** on JSONB for querying less common tags

---

## 4. Architecture

### 4.1 System Diagram

```
┌──────────────────────────────────────────────────────────────┐
│                     pacsnode (binary)                      │
│                                                              │
│  ┌──────────────┐  ┌───────────────┐  ┌──────────────────┐  │
│  │  DICOM SCP   │  │   REST API    │  │  DICOMweb API    │  │
│  │  (toolkit-net)│  │   (axum)      │  │  (axum)          │  │
│  └──────┬───────┘  └──────┬────────┘  └────────┬─────────┘  │
│         │                 │                     │            │
│  ┌──────┴─────────────────┴─────────────────────┴─────────┐  │
│  │               Service Layer (pacs-core)                 │  │
│  │  ┌──────────┐  ┌───────────┐  ┌──────────────────────┐ │  │
│  │  │ Ingest   │  │  Query    │  │  Retrieve / Send     │ │  │
│  │  │ Pipeline │  │  Engine   │  │  Orchestrator        │ │  │
│  │  └────┬─────┘  └─────┬────┘  └──────────┬───────────┘ │  │
│  └───────┼───────────────┼──────────────────┼─────────────┘  │
│          │               │                  │                │
│  ┌───────┴───────────────┴──────────────────┴─────────────┐  │
│  │                Storage & Index Layer                     │  │
│  │  ┌────────────────┐          ┌──────────────────────┐  │  │
│  │  │  Blob Store    │          │  PostgreSQL           │  │  │
│  │  │  (RustFS / S3) │          │  (sqlx, hybrid JSONB) │  │  │
│  │  └────────────────┘          └──────────────────────┘  │  │
│  └─────────────────────────────────────────────────────────┘  │
│                                                              │
│  ┌─────────────────────────────────────────────────────────┐  │
│  │              dicom-toolkit-rs (library)                  │  │
│  │  core │ dict │ data │ net │ image │ codec               │  │
│  └─────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

### 4.2 Workspace Structure

```
pacsnode/
├── Cargo.toml                  # workspace root
├── LICENSE                     # MIT
├── NOTICE                      # DCMTK / CharLS attribution
├── crates/
│   ├── pacs-core/              # domain types, traits, errors
│   ├── pacs-dicom/             # dicom-toolkit-rs integration, DICOM object handling
│   ├── pacs-store/             # PostgreSQL MetadataStore impl (sqlx)
│   ├── pacs-storage/           # RustFS/S3 BlobStore (object_store)
│   ├── pacs-dimse/             # PACS-specific DIMSE providers/integration (built on toolkit-net)
│   ├── pacs-api/               # Axum REST + DICOMweb HTTP handlers
│   └── pacs-server/            # binary, config, startup wiring
├── migrations/
│   └── *.sql                   # sqlx-cli migration files
├── tests/
│   └── integration/            # end-to-end tests with real DICOM files
└── docker/
    └── docker-compose.yml      # PostgreSQL + RustFS + pacsnode
```

### 4.3 MetadataStore Trait (pacs-core)

The entire PACS codebase talks only to this trait — never to PostgreSQL directly. This preserves the option of adding alternative backends in the future without any refactoring.

```rust
#[async_trait]
pub trait MetadataStore: Send + Sync {
    // Write
    async fn store_study(&self, study: &Study) -> Result<()>;
    async fn store_series(&self, series: &Series) -> Result<()>;
    async fn store_instance(&self, instance: &Instance) -> Result<()>;

    // Query (QIDO)
    async fn query_studies(&self, q: &StudyQuery) -> Result<Vec<Study>>;
    async fn query_series(&self, q: &SeriesQuery) -> Result<Vec<Series>>;
    async fn query_instances(&self, q: &InstanceQuery) -> Result<Vec<Instance>>;

    // Metadata retrieval
    async fn get_instance_metadata(&self, uid: &str) -> Result<DicomJson>;

    // Delete
    async fn delete_study(&self, uid: &str) -> Result<()>;
    async fn delete_series(&self, uid: &str) -> Result<()>;
    async fn delete_instance(&self, uid: &str) -> Result<()>;

    // Statistics
    async fn get_statistics(&self) -> Result<PacsStatistics>;
}
```

### 4.4 BlobStore Trait (pacs-core)

Abstracts pixel data storage. Default implementation targets RustFS via the object_store crate. S3-compatible — swapping to AWS S3 is a configuration change only.

```rust
#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, key: &str, data: Bytes) -> Result<()>;
    async fn get(&self, key: &str) -> Result<Bytes>;
    async fn delete(&self, key: &str) -> Result<()>;
    async fn exists(&self, key: &str) -> Result<bool>;
    async fn presigned_url(&self, key: &str) -> Result<String>;
}
```

### 4.5 Configuration

```toml
[server]
http_port = 8042
dicom_port = 4242
ae_title = "PACSNODE"

[database]
url = "postgres://pacs:secret@localhost/pacs"
max_connections = 20
run_migrations = true

[storage]
endpoint = "http://localhost:9000"
bucket = "dicom"
access_key = "rustfsadmin"
secret_key = "rustfsadmin"

[security]
tls_enabled = false
tls_cert = ""
tls_key = ""
auth_enabled = false
jwt_secret = ""

[logging]
level = "info"
format = "json"
```

---

## 5. PostgreSQL Schema

Hybrid schema: indexed relational columns for QIDO performance + JSONB for full metadata retrieval.

```sql
-- Studies
CREATE TABLE studies (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    study_uid       TEXT NOT NULL UNIQUE,
    patient_id      TEXT,
    patient_name    TEXT,
    study_date      DATE,
    study_time      TEXT,
    accession_number TEXT,
    modalities      TEXT[],
    referring_physician TEXT,
    description     TEXT,
    num_series      INTEGER DEFAULT 0,
    num_instances   INTEGER DEFAULT 0,
    metadata        JSONB NOT NULL,
    created_at      TIMESTAMPTZ DEFAULT NOW(),
    updated_at      TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_studies_patient_id ON studies(patient_id);
CREATE INDEX idx_studies_patient_name ON studies(patient_name);
CREATE INDEX idx_studies_study_date ON studies(study_date);
CREATE INDEX idx_studies_accession ON studies(accession_number);
CREATE INDEX idx_studies_modalities ON studies USING GIN(modalities);
CREATE INDEX idx_studies_metadata ON studies USING GIN(metadata jsonb_path_ops);
CREATE INDEX idx_studies_updated_at ON studies(updated_at);

-- Series
CREATE TABLE series (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    series_uid      TEXT NOT NULL UNIQUE,
    study_uid       TEXT NOT NULL REFERENCES studies(study_uid) ON DELETE CASCADE,
    modality        TEXT,
    series_number   INTEGER,
    description     TEXT,
    body_part       TEXT,
    num_instances   INTEGER DEFAULT 0,
    metadata        JSONB NOT NULL,
    created_at      TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_series_study ON series(study_uid);
CREATE INDEX idx_series_modality ON series(modality);
CREATE INDEX idx_series_metadata ON series USING GIN(metadata jsonb_path_ops);

-- Instances
CREATE TABLE instances (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    instance_uid    TEXT NOT NULL UNIQUE,
    series_uid      TEXT NOT NULL REFERENCES series(series_uid) ON DELETE CASCADE,
    study_uid       TEXT NOT NULL,
    sop_class_uid   TEXT,
    instance_number INTEGER,
    transfer_syntax TEXT,
    rows            INTEGER,
    columns         INTEGER,
    blob_key        TEXT NOT NULL,
    metadata        JSONB NOT NULL,
    created_at      TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_instances_series ON instances(series_uid);
CREATE INDEX idx_instances_study ON instances(study_uid);
CREATE INDEX idx_instances_sop_class ON instances(sop_class_uid);
CREATE INDEX idx_instances_metadata ON instances USING GIN(metadata jsonb_path_ops);

-- DICOM nodes (remote AE titles for C-STORE/C-FIND/C-MOVE)
CREATE TABLE dicom_nodes (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    ae_title    TEXT NOT NULL UNIQUE,
    host        TEXT NOT NULL,
    port        INTEGER NOT NULL,
    description TEXT,
    created_at  TIMESTAMPTZ DEFAULT NOW()
);

-- Audit log (append-only, HIPAA compliance)
CREATE TABLE audit_log (
    id          BIGSERIAL PRIMARY KEY,
    timestamp   TIMESTAMPTZ DEFAULT NOW(),
    user_id     TEXT,
    action      TEXT NOT NULL,
    resource    TEXT NOT NULL,
    resource_uid TEXT,
    source_ip   TEXT,
    details     JSONB
);

CREATE INDEX idx_audit_timestamp ON audit_log(timestamp);
CREATE INDEX idx_audit_user ON audit_log(user_id);
CREATE INDEX idx_audit_resource ON audit_log(resource_uid);
```

---

## 6. Toolkit Integration Status: What pacsnode Adds on Top of dicom-toolkit-rs

dicom-toolkit-rs now provides the generic async DIMSE server, SCP provider traits, DIMSE request handlers, and JPEG 2000 codec support. The `pacs-dimse` crate therefore focuses on PACS-specific storage/query integration, destination lookup wiring, and interoperability validation rather than rebuilding the protocol stack from scratch.

| Component | Status in toolkit | pacsnode action |
|---|---|---|
| C-STORE SCP | `StoreServiceProvider` + `DicomServer` available | Implement PACS ingest provider and keep validating with real senders |
| C-FIND SCP | `FindServiceProvider` + `DicomServer` available | Implement database-backed query provider and expand interoperability coverage |
| C-GET SCP | `GetServiceProvider` + C-STORE sub-op handler available | Implement storage-backed retrieve provider and add broader end-to-end validation |
| C-MOVE SCP | `MoveServiceProvider` + sub-association handler available | Implement node lookup/forwarding policy and validate against remote destinations |
| Generic DicomServer | Present (`DicomServerBuilder`, routing, graceful shutdown, max-association limiting) | Reuse toolkit server and keep pacsnode-specific wrapper thin |
| JPEG 2000 codec | Present (pure-Rust encode + decode) | Add PACS workflow fixtures, interoperability testing, and performance validation |

Most of the remaining project-specific value lies in PACS business logic, storage/query integration, operational policy, and production hardening.

---

## 7. DICOMweb Implementation

### 7.1 STOW-RS (Store)
Accept multipart DICOM uploads. For each part: parse with dicom-toolkit-rs, extract metadata into Study/Series/Instance structs, store pixel data to RustFS via BlobStore, store metadata to PostgreSQL via MetadataStore. Blob key: `{study_uid}/{series_uid}/{instance_uid}`.

### 7.2 QIDO-RS (Query)
Map DICOMweb query parameters to PostgreSQL queries (indexed columns for common tags, JSONB GIN index for rare tags). Return DICOM JSON. Support fuzzy matching on patient name, date ranges, modality filtering, `includefield`, pagination via `offset`/`limit`.

### 7.3 WADO-RS (Retrieve)
Return DICOM instances as multipart MIME responses. Pixel data fetched from RustFS via BlobStore. Support both inline pixel data and bulk data URLs (presigned RustFS URLs) for large studies.

### 7.4 REST API (Orthanc-compatible endpoints)
Patient/Study/Series/Instance CRUD, upload, download (DICOM/PNG/JPEG), search with filters, modality management, send-to-modality, system info, statistics, bulk operations (delete study, anonymize).

---

## 8. DIMSE Protocol

Built on dicom-toolkit-rs's async networking, generic `DicomServer`, and DIMSE provider interfaces. The `pacs-dimse` crate adds PACS-specific providers, storage/blob integration, destination lookup wiring, and project-level tests.

- **C-STORE SCP** — receive DICOM files pushed from modalities (highest priority)
- **C-ECHO SCP** — verification (already supported by toolkit, wire into server)
- **C-FIND SCP** — respond to queries from modalities and other PACS
- **C-MOVE SCP** — handle retrieval requests between PACS systems
- **C-GET SCP** — return instances directly to requester
- **C-STORE SCU** — push images to other PACS (forwarding rules)

---

## 9. Production Hardening

### 9.1 Security
- OAuth2 / SMART on FHIR authentication for OHIF and clinical app compatibility
- TLS via rustls (dicom-toolkit-rs already provides this for DIMSE)
- JWT + Basic Auth for REST API
- Role-based access control (admin, read-write, read-only)
- DICOM association-level auth (AE title allowlists, TLS cert verification)

### 9.2 Compliance
- HIPAA audit log — append-only `audit_log` table in PostgreSQL
- All logs include: timestamp, user identity, resource accessed, operation type, source IP
- Immutability enforced at application layer (no UPDATE/DELETE on audit_log)

### 9.3 Observability
- Prometheus metrics: query latency, storage throughput, DIMSE connection counts
- Structured tracing via the `tracing` crate
- Health check endpoint for load balancer / Kubernetes liveness probes
- `/statistics` endpoint (study/series/instance counts, disk usage)

### 9.4 Deployment
- **Single docker-compose.yml**: PostgreSQL + RustFS + pacsnode (zero external dependencies)
- **Single static binary** — no C++ runtime, no plugins, no shared libs
- Kubernetes Helm chart with configurable replicas
- sqlx-cli migration management for PostgreSQL schema versioning in CI/CD
- Environment variable overrides for 12-factor app compliance

---

## 10. Development Phases

### Current Validated Progress (2026-03-15)
- `cargo test --workspace --all-targets` passes locally.
- `pacs-store` integration tests run against a real PostgreSQL container, defaulting to `postgres:16-alpine` (override with `PACSNODE_TEST_POSTGRES_TAG` when needed).
- `scripts/smoke-test.sh` passes end to end against the rebuilt Docker deployment (`20/20` checks).
- Phase 1 capabilities validated: STOW-RS ingest, C-STORE SCP ingest, C-ECHO SCP, configuration loading, migrations, and health/statistics endpoints.
- Phase 2 capabilities validated: QIDO-RS study/series/instance queries and C-FIND SCP.
- Phase 3 capabilities validated: WADO-RS single-instance retrieval.
- QIDO-RS smoke coverage now verifies matching Study Instance UID, Series Instance UID, and SOP Instance UID values in returned DICOM JSON payloads, not only HTTP `200`.
- The previously observed empty QIDO-RS payload was traced to a stale Docker container image and resolved by rebuilding the deployment.

### Phase 1 — Skeleton & Core Pipeline
- Workspace setup with all crates
- Core domain types, traits (`MetadataStore`, `BlobStore`), error types
- PostgreSQL schema + sqlx migrations
- RustFS BlobStore implementation
- STOW-RS vertical slice: parse DICOM (dicom-toolkit-rs) → store pixel data (RustFS) → store metadata (PostgreSQL)
- C-STORE SCP: receive DICOM files from modalities → same ingest pipeline
- C-ECHO SCP wired into DIMSE server
- TOML configuration + environment variable overrides
- Basic health check endpoint
- Integration test: roundtrip a real DICOM file (TCIA dataset)

### Phase 2 — Query Surface
- QIDO-RS: full query surface with fuzzy matching, date ranges, modality filters, pagination
- C-FIND SCP: map DIMSE queries to PostgreSQL, return matching results
- REST API: list/search patients, studies, series, instances with filters
- Integration test suite with diverse DICOM files (multi-modality, multi-vendor)
- GIN index validation with query performance benchmarks

### Phase 3 — Retrieve
- WADO-RS: multipart MIME responses, inline pixel data, presigned bulk data URLs
- WADO-URI: legacy single-instance retrieval
- C-MOVE SCP: open association to destination, forward instances
- C-GET SCP: return instances directly to requester
- C-STORE SCU: push images to remote PACS
- REST API: download instance as DICOM/PNG/JPEG, send-to-modality
- Thumbnail/preview generation (dicom-toolkit-rs image pipeline)

### Phase 4 — Production Readiness
- Authentication: JWT + Basic Auth + OAuth2/SMART on FHIR
- Role-based access control
- HIPAA audit logging (append-only audit_log table)
- Connection pooling, rate limiting
- Prometheus metrics + structured tracing
- Docker image + docker-compose.yml (PostgreSQL + RustFS + pacsnode)
- Kubernetes Helm chart
- CI/CD pipeline (build, test, lint, migration check)

### Phase 5 — Advanced Features
- Anonymization (PS3.15 Basic Application Level Confidentiality Profile)
- Job queue for long-running operations (async transfers, batch anonymization)
- Plugin/hook system (on-receive, on-store, on-stable-study)
- Lua/Rhai scripting for lightweight automation
- Modality Worklist SCP + MPPS
- JPEG 2000 interoperability, fixture coverage, and performance hardening
- Web UI integration (OHIF viewer via DICOMweb)

---

## 11. Key Risks & Mitigations

| Risk | Severity | Mitigation |
|---|---|---|
| **DICOM edge cases** — real-world files from different vendors are frequently non-conformant | High | Budget significant testing time with real imaging data from multiple modalities/vendors. Use TCIA public datasets. |
| **DIMSE integration complexity** — toolkit primitives exist, but PACS-specific provider behavior and interoperability still need validation | Medium | Keep `pacs-dimse` thin over toolkit-net, and validate C-STORE/C-FIND/C-GET/C-MOVE behavior early with real peers and smoke tests. |
| **PostgreSQL JSONB performance at scale** — GIN indexes on large JSONB columns | Medium | Benchmark with 1M+ instances early. The hybrid approach (relational columns for common queries) mitigates this — JSONB GIN is only for uncommon tag queries. |
| **JPEG 2000** — codec support exists, but interoperability and performance across vendor datasets still need proof | Medium | Add real JPEG 2000 fixtures, benchmark decode/encode paths, and validate against public/vendor sample data before relying on it operationally. |
| **dicom-toolkit-rs upstream changes** — port may not be on crates.io yet | Low | Pin to git commit hash. Contribute fixes upstream. |

---

## 12. First Steps (Validation Prototype)

Before writing production code, validate the core pipeline:

1. Parse a real DICOM file (TCIA public dataset) with dicom-toolkit-rs
2. Extract key tags and build Study/Series/Instance domain objects
3. Store pixel data to a local RustFS instance via object_store
4. Store metadata to PostgreSQL via sqlx
5. Retrieve both and verify roundtrip integrity
6. Test C-STORE SCP: send a DICOM file with `storescu` CLI tool → verify it arrives and is stored

This prototype validation has now been exercised successfully against both the current source build and the rebuilt Docker deployment. It validated the full pipeline and surfaced a stale-container regression in QIDO-RS behavior early, before it could be mistaken for a source-level defect.
