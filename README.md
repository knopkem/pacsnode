# pacsnode

A modern, high-performance **Picture Archiving and Communication System (PACS)** built entirely in Rust. pacsnode provides full DICOMweb and DIMSE protocol support, backed by PostgreSQL for metadata and S3-compatible object storage for pixel data.

> ⚠️ **NOT FOR CLINICAL USE** — This software is not a certified medical device. It has not been validated for diagnostic or therapeutic purposes.

## Features

- **DICOMweb** — STOW-RS, QIDO-RS, WADO-RS (PS3.18 compliant)
- **DIMSE** — C-STORE, C-FIND, C-MOVE, C-GET, C-ECHO SCP + SCU
- **REST API** — Study/Series/Instance CRUD, remote node management, statistics
- **PostgreSQL** — Hybrid relational + JSONB schema with GIN indexes for fast queries
- **S3 Storage** — Pixel data stored in any S3-compatible backend (MinIO, RustFS, AWS S3)
- **Async** — Built on Tokio with fully async I/O throughout
- **Single Binary** — Zero runtime dependencies beyond PostgreSQL and S3

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                       pacsnode (binary)                      │
│                                                              │
│  ┌──────────────┐  ┌───────────────┐  ┌──────────────────┐  │
│  │  DICOM SCP   │  │   REST API    │  │  DICOMweb API    │  │
│  │  (pacs-dimse)│  │   (pacs-api)  │  │  (pacs-api)      │  │
│  └──────┬───────┘  └──────┬────────┘  └────────┬─────────┘  │
│         │                 │                     │            │
│  ┌──────┴─────────────────┴─────────────────────┴─────────┐  │
│  │                  Service Layer (pacs-core)              │  │
│  └──────┬─────────────────┬─────────────────────┬─────────┘  │
│         │                 │                     │            │
│  ┌──────┴────────┐  ┌────┴────────────┐  ┌─────┴──────────┐ │
│  │  Blob Store   │  │  Metadata Store │  │  DICOM Bridge  │ │
│  │  (pacs-storage│  │  (pacs-store)   │  │  (pacs-dicom)  │ │
│  └───────────────┘  └─────────────────┘  └────────────────┘ │
│                                                              │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │              dicom-toolkit-rs (library)                  │ │
│  │  core │ dict │ data │ net │ image │ codec               │ │
│  └─────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
         │                    │
    ┌────┴─────┐      ┌──────┴──────┐
    │ S3 / MinIO│      │ PostgreSQL  │
    └──────────┘      └─────────────┘
```

### Workspace Crates

| Crate | Role |
|-------|------|
| `pacs-core` | Domain types (`Study`, `Series`, `Instance`), UID newtypes, `MetadataStore` + `BlobStore` traits, error types |
| `pacs-dicom` | Bridge to dicom-toolkit-rs — DICOM parsing, tag extraction, JSON conversion |
| `pacs-store` | PostgreSQL `MetadataStore` implementation (sqlx, compile-time verified queries) |
| `pacs-storage` | S3 `BlobStore` implementation (object_store crate) |
| `pacs-dimse` | DICOM SCP server + SCU client (C-STORE, C-FIND, C-MOVE, C-GET, C-ECHO) |
| `pacs-api` | Axum HTTP server — DICOMweb (STOW/QIDO/WADO-RS) + REST endpoints |
| `pacs-server` | Binary entry point — config loading, startup wiring, graceful shutdown |

---

## Quick Start (Docker Compose)

> **Prerequisites:** Docker and Docker Compose installed.

**Step 1 — Copy the environment file**

```bash
cd docker
cp .env.example .env
```

The defaults in `.env` work as-is for local testing — no editing required. For production, change the passwords before proceeding.

**Step 2 — Build and start the stack**

```bash
docker compose up -d
```

This starts four services in dependency order:
1. **PostgreSQL 16** — waits until healthy
2. **MinIO** — waits until healthy
3. **minio-init** — creates the `dicom` bucket, then exits
4. **pacsnode** — starts only after the bucket exists and postgres is ready

The first run compiles the Rust binary inside Docker; this takes a few minutes. Subsequent starts use the image cache and are instant.

**Step 3 — Verify**

```bash
curl http://localhost:8042/health
# {"status":"ok"}

curl http://localhost:8042/statistics
# {"studies":0,"series":0,"instances":0,"disk_usage_bytes":0}
```

**Services at a glance:**

| Service | Port | Description |
|---------|------|-------------|
| pacsnode REST/DICOMweb | `8042` | STOW-RS, QIDO-RS, WADO-RS, REST API |
| pacsnode DIMSE | `4242` | C-STORE, C-FIND, C-MOVE, C-GET, C-ECHO |
| MinIO S3 API | `9000` | Pixel data object storage |
| MinIO web console | `9001` | Browse stored DICOM files (login: see `.env`) |
| PostgreSQL | `5432` | Metadata database |

**Tear down:**

```bash
docker compose down          # stop, keep data volumes
docker compose down -v       # stop and delete all data
```

---

## Building from Source

### Prerequisites

- **Rust 1.88+** (see `rust-toolchain.toml`)
- **PostgreSQL 14+** — running instance with a database created
- **S3-compatible storage** — MinIO, RustFS, or AWS S3
- **sqlx-cli** (optional, for managing migrations manually):
  ```bash
  cargo install sqlx-cli --no-default-features --features postgres
  ```

### Build

```bash
# Clone the repository
git clone https://github.com/your-org/pacsnode.git
cd pacsnode

# Build in release mode
cargo build --release

# The binary is at target/release/pacsnode
```

### Run

```bash
# Option 1: Use a config file
cp config.toml.example config.toml
# Edit config.toml with your database and storage settings
./target/release/pacsnode

# Option 2: Use environment variables only
export PACS_DATABASE__URL="postgres://pacsnode:secret@localhost:5432/pacsnode"
export PACS_STORAGE__ENDPOINT="http://localhost:9000"
export PACS_STORAGE__BUCKET="dicom"
export PACS_STORAGE__ACCESS_KEY="minioadmin"
export PACS_STORAGE__SECRET_KEY="minioadmin"
./target/release/pacsnode
```

---

## Configuration

pacsnode uses a two-layer configuration system:

1. **TOML file** — `config.toml` in the working directory (optional)
2. **Environment variables** — `PACS_` prefix with `__` separator (overrides TOML)

See [`config.toml.example`](config.toml.example) for a fully-commented reference.

### Configuration Reference

#### Server

| Setting | Env Var | Default | Description |
|---------|---------|---------|-------------|
| `server.http_port` | `PACS_SERVER__HTTP_PORT` | `8042` | HTTP port for REST + DICOMweb |
| `server.dicom_port` | `PACS_SERVER__DICOM_PORT` | `4242` | DIMSE protocol port |
| `server.ae_title` | `PACS_SERVER__AE_TITLE` | `PACSNODE` | DICOM Application Entity title |
| `server.max_associations` | `PACS_SERVER__MAX_ASSOCIATIONS` | `64` | Max concurrent DIMSE connections |
| `server.dimse_timeout_secs` | `PACS_SERVER__DIMSE_TIMEOUT_SECS` | `30` | DIMSE operation timeout (seconds) |

#### Database

| Setting | Env Var | Default | Description |
|---------|---------|---------|-------------|
| `database.url` | `PACS_DATABASE__URL` | *(required)* | PostgreSQL connection URL |
| `database.max_connections` | `PACS_DATABASE__MAX_CONNECTIONS` | `20` | Connection pool size |
| `database.run_migrations` | `PACS_DATABASE__RUN_MIGRATIONS` | `true` | Auto-run migrations on startup |

#### Storage

| Setting | Env Var | Default | Description |
|---------|---------|---------|-------------|
| `storage.endpoint` | `PACS_STORAGE__ENDPOINT` | *(required)* | S3-compatible endpoint URL |
| `storage.bucket` | `PACS_STORAGE__BUCKET` | *(required)* | Bucket for DICOM pixel data |
| `storage.access_key` | `PACS_STORAGE__ACCESS_KEY` | *(required)* | S3 access key ID |
| `storage.secret_key` | `PACS_STORAGE__SECRET_KEY` | *(required)* | S3 secret access key |
| `storage.region` | `PACS_STORAGE__REGION` | `us-east-1` | S3 region |

#### Logging

| Setting | Env Var | Default | Description |
|---------|---------|---------|-------------|
| `logging.level` | `PACS_LOGGING__LEVEL` | `info` | Log level ([tracing env_filter syntax](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)) |
| `logging.format` | `PACS_LOGGING__FORMAT` | `json` | `json` or `pretty` |

---

## API Reference

### DICOMweb (PS3.18)

#### STOW-RS — Store

```
POST /wado/studies
Content-Type: multipart/related; type="application/dicom"
```

Upload DICOM instances. Each multipart part contains one DICOM file. Returns a PS3.18 store response with status per instance.

#### QIDO-RS — Query

| Endpoint | Description |
|----------|-------------|
| `GET /wado/studies` | Search studies |
| `GET /wado/studies/{study_uid}/series` | Search series within a study |
| `GET /wado/studies/{study_uid}/series/{series_uid}/instances` | Search instances within a series |

**Study query parameters:**

| Parameter | Description |
|-----------|-------------|
| `PatientID` | Exact match |
| `PatientName` | Supports `*` wildcard suffix |
| `StudyDate` | Single date or range (`YYYYMMDD-YYYYMMDD`) |
| `AccessionNumber` | Exact match |
| `StudyInstanceUID` | Exact match |
| `Modality` | Filter by modality code |
| `limit` | Max results (pagination) |
| `offset` | Skip N results (pagination) |
| `fuzzymatching` | Enable fuzzy matching on names |

#### WADO-RS — Retrieve

**Instances (multipart/related):**

| Endpoint | Description |
|----------|-------------|
| `GET /wado/studies/{study_uid}` | All instances in study |
| `GET /wado/studies/{study_uid}/series/{series_uid}` | All instances in series |
| `GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}` | Single instance |

**Metadata (JSON):**

| Endpoint | Description |
|----------|-------------|
| `GET /wado/studies/{study_uid}/metadata` | Study metadata (PS3.18 JSON) |
| `GET /wado/studies/{study_uid}/series/{series_uid}/metadata` | Series metadata |
| `GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/metadata` | Instance metadata |

### REST API

#### Studies

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/studies` | List all studies |
| `GET` | `/api/studies/{study_uid}` | Get study details |
| `DELETE` | `/api/studies/{study_uid}` | Delete study (cascades to series + instances) |

#### Series

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/studies/{study_uid}/series` | List series in a study |
| `GET` | `/api/series/{series_uid}` | Get series details |
| `DELETE` | `/api/series/{series_uid}` | Delete series (cascades to instances) |

#### Instances

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/series/{series_uid}/instances` | List instances in a series |
| `GET` | `/api/instances/{instance_uid}` | Get instance details |
| `DELETE` | `/api/instances/{instance_uid}` | Delete instance |

#### Remote DICOM Nodes

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/nodes` | List registered remote nodes |
| `POST` | `/api/nodes` | Register a remote DICOM node |
| `DELETE` | `/api/nodes/{ae_title}` | Remove a remote node |

#### System

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/health` | Health check (`{"status":"ok"}`) |
| `GET` | `/statistics` | Study/series/instance counts and disk usage |

---

## DIMSE Services

pacsnode listens for DICOM associations on the configured DICOM port (default `4242`).

### SCP (Server — receives requests)

| Service | SOP Class | Description |
|---------|-----------|-------------|
| C-ECHO | Verification (`1.2.840.10008.1.1`) | Connection verification |
| C-STORE | Storage SOP Classes | Receive DICOM instances from modalities |
| C-FIND | Study Root Query/Retrieve | Respond to queries from other PACS |
| C-MOVE | Study Root Query/Retrieve | Forward instances to a requested destination |
| C-GET | Study Root Query/Retrieve | Return instances directly to the requester |

### SCU (Client — sends requests)

| Service | Description |
|---------|-------------|
| C-ECHO | Verify connectivity to a remote node |
| C-STORE | Push instances to a remote PACS |
| C-FIND | Query a remote PACS |
| C-MOVE | Request a remote PACS to send instances |

### Example: Send a DICOM file with storescu

```bash
# Using dicom-toolkit-rs CLI tools
storescu --host localhost --port 4242 --ae-title MODALITY \
         --called-ae PACSNODE path/to/image.dcm

# Or verify connectivity first
echoscu --host localhost --port 4242 --ae-title MODALITY \
        --called-ae PACSNODE
```

---

## Database

pacsnode uses PostgreSQL with a hybrid schema: indexed relational columns for fast QIDO queries, plus JSONB columns with GIN indexes for full metadata retrieval.

### Schema Overview

| Table | Purpose |
|-------|---------|
| `studies` | Study-level metadata (patient info, dates, modalities) + full JSONB |
| `series` | Series-level metadata (modality, body part) + full JSONB |
| `instances` | Instance-level metadata (SOP class, transfer syntax, blob key) + full JSONB |
| `dicom_nodes` | Registered remote DICOM Application Entities |
| `audit_log` | Append-only HIPAA audit trail |

### Migrations

Migrations are managed with [sqlx-cli](https://crates.io/crates/sqlx-cli) and live in the `migrations/` directory. By default, pacsnode runs pending migrations automatically on startup (`database.run_migrations = true`).

To manage migrations manually:

```bash
# Install sqlx-cli
cargo install sqlx-cli --no-default-features --features postgres

# Run pending migrations
sqlx migrate run --source migrations/ --database-url postgres://...

# Check migration status
sqlx migrate info --source migrations/ --database-url postgres://...
```

---

## Development

### Running Tests

```bash
# Unit tests (no external services needed)
cargo test

# Integration tests require PostgreSQL + MinIO (see docker/docker-compose.yml)
cd docker && docker compose up -d postgres minio
cargo test -- --include-ignored
```

### Linting

```bash
# Clippy (must pass with zero warnings)
cargo clippy -- -D warnings

# Format check
cargo fmt -- --check
```

### Project Layout

```
pacsnode/
├── Cargo.toml              # Workspace root
├── config.toml.example     # Configuration reference
├── Dockerfile              # Multi-stage build
├── NOTICE                  # Third-party attribution
├── crates/
│   ├── pacs-core/          # Domain types, traits, errors
│   ├── pacs-dicom/         # dicom-toolkit-rs adapter
│   ├── pacs-store/         # PostgreSQL MetadataStore
│   ├── pacs-storage/       # S3 BlobStore
│   ├── pacs-dimse/         # DIMSE SCP/SCU
│   ├── pacs-api/           # Axum HTTP handlers
│   └── pacs-server/        # Binary, config, startup
├── migrations/             # PostgreSQL migrations (sqlx)
├── docker/
│   ├── docker-compose.yml  # Full stack: PostgreSQL + MinIO + pacsnode
│   └── .env.example        # Environment variable template
└── tests/                  # End-to-end tests
```

---

## dicom-toolkit-rs

pacsnode is built on [dicom-toolkit-rs](https://github.com/knopkem/dicom-toolkit-rs), a clean-room Rust port inspired by DCMTK. It provides:

- DICOM file I/O (4 uncompressed transfer syntaxes + deflate)
- Complete DICOM JSON (PS3.18) and XML (PS3.19) support
- Full DIMSE protocol: C-ECHO, C-STORE, C-FIND, C-GET, C-MOVE (SCU)
- Image codecs: JPEG baseline, JPEG-LS, RLE
- Image pipeline: Window/Level, LUT, VOI, overlays
- TLS via rustls, async networking via Tokio
- 15+ character set encodings including ISO 2022

The dependency is currently referenced as a git dependency (branch `main`). Once published to crates.io, it will be switched to a version dependency.

---

## Security

- **PHI Protection** — Patient names, IDs, and dates are never written to log output. Only DICOM UIDs appear in structured log fields.
- **Audit Logging** — All data access is recorded in an append-only `audit_log` table (HIPAA compliance).
- **TLS** — Configurable for both HTTP (Axum) and DIMSE (dicom-toolkit-rs rustls).
- **Authentication** — JWT + Basic Auth support for the REST/DICOMweb API.
- **Secrets** — All credentials are loaded from configuration or environment variables, never hardcoded.
- **Input Validation** — Malformed UIDs, oversized payloads, and unexpected content types are rejected with appropriate HTTP 4xx responses.

---

## License

This project is licensed under the [MIT License](LICENSE).

See [NOTICE](NOTICE) for third-party attribution.
