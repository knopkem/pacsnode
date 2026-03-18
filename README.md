# pacsnode

A modern, high-performance **Picture Archiving and Communication System (PACS)** built entirely in Rust. pacsnode provides full DICOMweb and DIMSE protocol support, backed by PostgreSQL + S3 by default, with a standalone SQLite + filesystem mode for single-machine deployments.

> ⚠️ **NOT FOR CLINICAL USE** — This software is not a certified medical device. It has not been validated for diagnostic or therapeutic purposes.

## Features

- **DICOMweb** — STOW-RS, QIDO-RS, WADO-RS (PS3.18 compliant)
- **DIMSE** — C-STORE, C-FIND, C-MOVE, C-GET, C-ECHO SCP + SCU
- **REST API** — Study/Series/Instance CRUD, remote node management, statistics
- **Security Plugins** — Optional local multi-user auth, refresh-token rotation, route-level authorization, audit logging, and admin dashboard
- **Federated Auth** — External OIDC bearer-token validation via issuer discovery, JWKS, or static RSA public keys
- **Backend Choice** — PostgreSQL + S3 by default (recommended), or SQLite + local filesystem in [standalone mode](#standalone-mode) for simple single-machine use
- **Plugin Architecture** — Compile-time optional plugins for auth, audit, admin, metrics, and viewer hosting
- **Async** — Built on Tokio with fully async I/O throughout
- **Single Binary** — Zero runtime dependencies beyond your selected metadata/blob backends

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
| `pacs-plugin` | Shared plugin traits, middleware/route extension points, plugin registry |
| `pacs-dicom` | Bridge to dicom-toolkit-rs — DICOM parsing, tag extraction, JSON conversion |
| `pacs-store` | PostgreSQL `MetadataStore` implementation |
| `pacs-sqlite-store` | SQLite `MetadataStore` implementation for standalone deployments |
| `pacs-storage` | S3 `BlobStore` implementation |
| `pacs-fs-storage` | Filesystem `BlobStore` implementation for standalone deployments |
| `pacs-dimse` | DICOM SCP server + SCU client (C-STORE, C-FIND, C-MOVE, C-GET, C-ECHO) |
| `pacs-api` | Axum HTTP server — DICOMweb (STOW/QIDO/WADO-RS) + REST endpoints |
| `pacs-auth-plugin` | Optional local auth and federated bearer-token validation plugin |
| `pacs-audit-plugin` | Optional append-only audit trail plugin |
| `pacs-admin-plugin` | Optional admin dashboard for system settings, users, nodes, and audit review |
| `pacs-metrics-plugin` | Optional Prometheus metrics endpoint and counters |
| `pacs-viewer-plugin` | Optional OHIF viewer hosting plugin |
| `pacs-server` | Binary entry point — config loading, startup wiring, graceful shutdown |

## Documentation

- [Feature matrix and gap analysis](DOCS/feature-matrix.md)
- [Authentication tutorial](DOCS/auth-tutorial.md)
- [OHIF integration requirements](DOCS/ohif-server-requirements.md)
- [Plugin architecture notes](DOCS/plugin-system.md)

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

## Standalone Mode

> ⚠️ **Not recommended for production or clinical environments.**
> Standalone mode is a simplified single-binary deployment that replaces PostgreSQL with SQLite and S3 with a local filesystem. It is intended for **development, evaluation, and lightweight single-machine use only.**

### Why you probably want the default backend instead

| | Default (PostgreSQL + S3) | Standalone (SQLite + filesystem) |
|---|---|---|
| **Recommended for** | Production, multi-node, clinical | Dev/eval, single machine, quick trials |
| **Concurrency** | Full multi-process, highly concurrent | Single-writer SQLite; fine for low load |
| **Scalability** | Horizontal — multiple pacsnode instances | Single machine only |
| **Backup / restore** | Standard PostgreSQL + S3 tooling | File copy of DB + blob directory |
| **QIDO performance** | GIN-indexed JSONB; fast at scale | Full-scan fallback for complex queries |
| **Operational maturity** | Mature tooling ecosystem | Minimal; SQLite WAL mode only |

### When standalone is acceptable

- Local development and integration testing without Docker.
- Evaluation or demos on a single machine.
- Very small deployments (< ~10k studies) where operational simplicity matters more than performance.

### Building the standalone binary

```bash
# Slim standalone-only build — SQLite + filesystem, no PostgreSQL or S3 required
cargo build --release --no-default-features --features standalone
```

> The default build (`cargo build --release`) now includes **both** backend pairs in one binary. At runtime, pacsnode selects SQLite vs PostgreSQL from `database.url`, and filesystem vs S3 from whether you configure `[filesystem_storage]` or `[storage]`.

### Generate a ready-to-use config

```bash
# One-binary build with both runtime profiles available
cargo build --release

# Standalone profile (SQLite + filesystem, viewer enabled)
./target/release/pacsnode generate-config standalone --output config.toml

# Production profile (PostgreSQL + S3 placeholders, viewer enabled)
./target/release/pacsnode generate-config production --output config.toml
```

If you omit `--output`, pacsnode prints the generated `config.toml` to stdout.

The generated config enables the bundled OHIF viewer, and the default binary
extracts that viewer into `./web/viewer/` automatically on first start.

### Running in standalone mode

```bash
# Configure via environment variables
export PACS_DATABASE__URL="sqlite://./data/pacsnode.db"
export PACS_FILESYSTEM_STORAGE__ROOT="./data/blobs"
./target/release/pacsnode
```

Or via `config.toml` — place it either **next to the binary** or in your working directory:

```toml
[server]
http_port = 8042      # change if 8042 is already in use

[database]
url = "sqlite://./data/pacsnode.db"

[filesystem_storage]
root = "./data/blobs"
```

> **Tip:** If you copy the binary to a deployment folder, drop `config.toml` in the same folder. pacsnode will find it automatically regardless of which directory you run it from.

Standalone mode runs embedded SQLite migrations on first start — no `sqlx-cli` or manual schema setup needed. The `./data` tree, SQLite database file, blob directory, and default `./web/viewer` directory are created automatically when needed.


## Building from Source

### Prerequisites

- **Rust 1.88+** (see `rust-toolchain.toml`)
- **Production profile:** PostgreSQL 14+ plus S3-compatible storage (MinIO, RustFS, or AWS S3)
- **Standalone profile:** no external database or object store required
- **sqlx-cli** (optional, for managing migrations manually):
  ```bash
  cargo install sqlx-cli --no-default-features --features postgres
  ```

### Build

```bash
# Clone the repository
git clone https://github.com/your-org/pacsnode.git
cd pacsnode

# Default build (single binary with both backend pairs)
cargo build --release

# Optional slim standalone-only build
cargo build --release --no-default-features --features standalone

# Optional slim production-only build
cargo build --release --no-default-features --features production

# The binary is at target/release/pacsnode
```

### Run

```bash
# Option 1: Generate a config file with the desired runtime profile
./target/release/pacsnode generate-config standalone --output config.toml
# or: ./target/release/pacsnode generate-config production --output config.toml
# Edit config.toml if needed, then run. On first start, pacsnode will create the
# default data directories it needs and extract its embedded OHIF bundle into
# ./web/viewer automatically.
./target/release/pacsnode

# Option 2: Use environment variables only (production profile)
export PACS_DATABASE__URL="postgres://pacsnode:secret@localhost:5432/pacsnode"
export PACS_STORAGE__ENDPOINT="http://localhost:9000"
export PACS_STORAGE__BUCKET="dicom"
export PACS_STORAGE__ACCESS_KEY="minioadmin"
export PACS_STORAGE__SECRET_KEY="minioadmin"
./target/release/pacsnode

# Option 3: Standalone mode via environment variables
export PACS_DATABASE__URL="sqlite://./data/pacsnode.db"
export PACS_FILESYSTEM_STORAGE__ROOT="./data/blobs"
./target/release/pacsnode
```

---

## Configuration

pacsnode uses a three-layer configuration system:

1. **`config.toml` next to the executable** — picked up automatically when the binary is run from its own directory (optional)
2. **`config.toml` in the working directory** — overrides the executable-adjacent file when both exist (optional)
3. **Environment variables** — `PACS_` prefix with `__` separator (overrides both TOML sources)

See [`config.toml.example`](config.toml.example) for a fully-commented reference.

> **Profile column:** `both` = applies to both runtime profiles · `production` = PostgreSQL + S3-compatible storage · `standalone` = SQLite + filesystem

### Configuration Reference

#### Server — `both`

| Setting | Env Var | Default | Description |
|---------|---------|---------|-------------|
| `server.http_port` | `PACS_SERVER__HTTP_PORT` | `8042` | HTTP port for REST + DICOMweb |
| `server.dicom_port` | `PACS_SERVER__DICOM_PORT` | `4242` | DIMSE protocol port |
| `server.ae_title` | `PACS_SERVER__AE_TITLE` | `PACSNODE` | DICOM Application Entity title |
| `server.ae_whitelist_enabled` | `PACS_SERVER__AE_WHITELIST_ENABLED` | `false` | Require inbound DIMSE callers to exist in the registered node list |
| `server.accept_all_transfer_syntaxes` | `PACS_SERVER__ACCEPT_ALL_TRANSFER_SYNTAXES` | `true` | Accept any DIMSE transfer syntax offered by the peer |
| `server.accepted_transfer_syntaxes` | `PACS_SERVER__ACCEPTED_TRANSFER_SYNTAXES` | `[]` | Optional DIMSE transfer-syntax allow-list used when accept-all is disabled |
| `server.preferred_transfer_syntaxes` | `PACS_SERVER__PREFERRED_TRANSFER_SYNTAXES` | `[]` | Preferred DIMSE transfer-syntax order during presentation-context selection |
| `server.max_associations` | `PACS_SERVER__MAX_ASSOCIATIONS` | `64` | Max concurrent DIMSE connections |
| `server.dimse_timeout_secs` | `PACS_SERVER__DIMSE_TIMEOUT_SECS` | `30` | DIMSE operation timeout (seconds) |

#### Database — `both`

| Setting | Env Var | Default | Description |
|---------|---------|---------|-------------|
| `database.url` | `PACS_DATABASE__URL` | *(required)* | `postgres://...` selects PostgreSQL metadata; `sqlite://...` selects SQLite metadata |
| `database.max_connections` | `PACS_DATABASE__MAX_CONNECTIONS` | `20` | Connection pool size |
| `database.run_migrations` | `PACS_DATABASE__RUN_MIGRATIONS` | `true` | Auto-run migrations on startup |

#### Storage — `production` only

| Setting | Env Var | Default | Description |
|---------|---------|---------|-------------|
| `storage.endpoint` | `PACS_STORAGE__ENDPOINT` | *(required)* | S3-compatible endpoint URL |
| `storage.bucket` | `PACS_STORAGE__BUCKET` | *(required)* | Bucket for DICOM pixel data |
| `storage.access_key` | `PACS_STORAGE__ACCESS_KEY` | *(required)* | S3 access key ID |
| `storage.secret_key` | `PACS_STORAGE__SECRET_KEY` | *(required)* | S3 secret access key |
| `storage.region` | `PACS_STORAGE__REGION` | `us-east-1` | S3 region |

#### Filesystem Storage — `standalone` only

| Setting | Env Var | Default | Description |
|---------|---------|---------|-------------|
| `filesystem_storage.root` | `PACS_FILESYSTEM_STORAGE__ROOT` | *(required)* | Root directory for filesystem-backed blob storage |

#### Logging — `both`

| Setting | Env Var | Default | Description |
|---------|---------|---------|-------------|
| `logging.level` | `PACS_LOGGING__LEVEL` | `info` | Log level ([tracing env_filter syntax](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)) |
| `logging.format` | `PACS_LOGGING__FORMAT` | `json` | `json` or `pretty` |

#### Optional Security & UI Plugins — `both`

Enable optional features by adding plugin IDs under `[plugins].enabled`.

```toml
[plugins]
enabled = ["basic-auth", "admin-dashboard"]
```

Common plugin IDs:

| Plugin ID | Purpose |
|-----------|---------|
| `basic-auth` | Local username/password auth or external OIDC bearer-token validation |
| `audit-logger` | Append-only audit trail; auto-enabled by default for secured deployments |
| `admin-dashboard` | Admin web UI for users, password policy, nodes, settings, and audit review |
| `prometheus-metrics` | `/metrics` endpoint with HTTP and PACS counters |
| `ohif-viewer` | Static OHIF viewer hosting |

For full setup examples, see [DOCS/auth-tutorial.md](DOCS/auth-tutorial.md).

#### Bootstrap DICOM Nodes — `both`

You can pre-seed the remote node registry directly from `config.toml` with
repeated `[[nodes]]` tables. On startup, pacsnode upserts each configured node
into the `dicom_nodes` table before the HTTP and DIMSE servers start listening.

```toml
[server]
ae_whitelist_enabled = true

[[nodes]]
ae_title = "MODALITY1"
host = "192.168.1.10"
port = 104
description = "CT Scanner - Room 3"
tls_enabled = false

[[nodes]]
ae_title = "REMOTEPACS"
host = "pacs.example.test"
port = 11112
tls_enabled = true
```

This is the simplest way to ship a ready-to-use whitelist in Docker, Helm, or a
checked-in deployment config. Runtime node management via `POST /api/nodes`
still works alongside it.

> **Important:** startup seeding is additive/upsert-only. If you remove a node
> from `config.toml`, pacsnode does not delete the existing row from
> `dicom_nodes`; remove it explicitly via `DELETE /api/nodes/{ae_title}`.

---

## Authentication & Authorization

`basic-auth` is the optional security plugin for HTTP routes. It supports two deployment modes:

- **Local auth** — pacsnode stores users in the metadata database, issues short-lived access tokens plus refresh tokens, enforces password policy and account lockout, and supports bootstrap admin creation from the CLI.
- **OIDC bearer validation** — pacsnode validates externally issued bearer tokens using issuer discovery, explicit JWKS, or a static RSA public key. Interactive login remains the responsibility of your identity provider or reverse proxy.

Current authorization coverage includes route-level enforcement across DICOMweb, REST, and admin surfaces, using the built-in roles `admin`, `radiologist`, `technologist`, `viewer`, and `uploader` plus optional claim/user attributes.

Use the auth tutorial for end-to-end examples:

- [DOCS/auth-tutorial.md](DOCS/auth-tutorial.md)

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

Nodes are the **AE whitelist** and remote-destination catalog. When
`server.ae_whitelist_enabled = true`, pacsnode only accepts inbound DIMSE
associations from calling AE titles that already exist in this list. The same
registry is also used for outbound DIMSE destinations such as C-MOVE / C-STORE
SCU operations. Nodes are stored in the metadata backend's `dicom_nodes` table and
**persist across restarts**.

**Enable AE whitelisting:**

```toml
[server]
ae_title = "PACSNODE"
ae_whitelist_enabled = true
```

Or via environment:

```bash
export PACS_SERVER__AE_WHITELIST_ENABLED=true
```

**Setup flow:**

1. Enable `server.ae_whitelist_enabled`.
2. Add each trusted modality / PACS AE title either to `config.toml` via
   `[[nodes]]` or to the runtime registry via `POST /api/nodes` before it opens
   a DIMSE association to pacsnode.
3. Verify the whitelist with `GET /api/nodes` or `GET /system`.

> **Important:** If whitelisting is enabled and a modality or remote PACS AE
> title is not present in `/api/nodes`, pacsnode rejects the DIMSE association.

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/nodes` | List all registered remote nodes |
| `POST` | `/api/nodes` | Register or update a remote node (upsert by AE title) |
| `DELETE` | `/api/nodes/{ae_title}` | Remove a remote node |

**Register a node:**

```bash
curl -s -X POST http://localhost:8042/api/nodes \
  -H "Content-Type: application/json" \
  -d '{
    "ae_title":    "MODALITY1",
    "host":        "192.168.1.10",
    "port":        104,
    "description": "CT Scanner — Room 3",
    "tls_enabled": false
  }'
```

**Or seed nodes from `config.toml`:**

```toml
[server]
ae_whitelist_enabled = true

[[nodes]]
ae_title = "MODALITY1"
host = "192.168.1.10"
port = 104
description = "CT Scanner - Room 3"
tls_enabled = false
```

Configured nodes are upserted into the same registry on startup, so they appear
in `GET /api/nodes` and `GET /system` just like nodes added through the REST
API.

**List nodes:**

```bash
curl -s http://localhost:8042/api/nodes
# [{"ae_title":"MODALITY1","host":"192.168.1.10","port":104,"description":"CT Scanner — Room 3","tls_enabled":false}]
```

**Remove a node:**

```bash
curl -s -X DELETE http://localhost:8042/api/nodes/MODALITY1
# HTTP 204 No Content
```

> **Note:** The `ae_title` field is the unique key. POSTing a node with an existing AE title updates it in place.

#### System

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/health` | Health check (`{"status":"ok"}`) |
| `GET` | `/statistics` | Study/series/instance counts and disk usage |
| `GET` | `/system` | Server identity, ports, and registered remote nodes |

**`GET /system` response:**

```json
{
  "ae_title":   "PACSNODE",
  "http_port":  8042,
  "dicom_port": 4242,
  "version":    "0.1.0",
  "nodes": [
    {
      "ae_title":    "MODALITY1",
      "host":        "192.168.1.10",
      "port":        104,
      "description": "CT Scanner",
      "tls_enabled": false
    }
  ]
}
```

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

## Metadata Storage

By default, pacsnode uses PostgreSQL with a hybrid schema: indexed relational columns for fast QIDO queries, plus JSONB columns with GIN indexes for full metadata retrieval. Standalone builds use SQLite with equivalent logical tables and trigger-maintained counters.

### Schema Overview

| Table | Purpose |
|-------|---------|
| `studies` | Study-level metadata (patient info, dates, modalities) + full JSONB |
| `series` | Series-level metadata (modality, body part) + full JSONB |
| `instances` | Instance-level metadata (SOP class, transfer syntax, blob key) + full JSONB |
| `dicom_nodes` | Registered remote DICOM Application Entities |
| `audit_log` | Append-only HIPAA audit trail |

### Migrations

PostgreSQL migrations are managed with [sqlx-cli](https://crates.io/crates/sqlx-cli) and live in the workspace `migrations/` directory. The standalone SQLite backend ships its own embedded migrations under `crates/pacs-sqlite-store/migrations/`. By default, pacsnode runs pending migrations automatically on startup (`database.run_migrations = true`).

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
# Unit tests / workspace tests that do not need Docker
cargo test --workspace --all-targets --exclude pacs-store && cargo test -p pacs-store --lib

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
│   ├── pacs-sqlite-store/  # SQLite MetadataStore
│   ├── pacs-storage/       # S3 BlobStore
│   ├── pacs-fs-storage/    # Filesystem BlobStore
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

- **PHI Protection** — Logging is designed around UID-based fields, but PHI redaction still needs hardening and should not be treated as complete.
- **Audit Logging** — The optional `audit-logger` plugin records data access and administrative events in an append-only `audit_log` table.
- **TLS** — Native HTTP and DIMSE TLS are not implemented yet; terminate TLS at a reverse proxy today.
- **Authentication** — The optional `basic-auth` plugin supports local multi-user auth and external OIDC bearer-token validation for REST, DICOMweb, and admin routes.
- **Authorization** — Built-in roles and attribute-aware policy checks protect route access, but policy coverage is still expanding.
- **Secrets** — All credentials are loaded from configuration or environment variables, never hardcoded.
- **Input Validation** — Malformed UIDs, oversized payloads, and unexpected content types are rejected with appropriate HTTP 4xx responses.

---

## License

This project is licensed under the [MIT License](LICENSE).

See [NOTICE](NOTICE) for third-party attribution.
