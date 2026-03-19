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

---

## Quick Start Standalone Mode

> ⚠️ **Not recommended for production or clinical environments.**
> Standalone mode is a simplified single-binary deployment that replaces PostgreSQL with SQLite and S3 with a local filesystem. It is intended for **development, evaluation, and lightweight single-machine use only.**

### Why you probably want the default backend (see Quick Start Production) instead

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
# One-binary build with both runtime profiles available
cargo build --release
```

### Generate a ready-to-use config

```bash
# Standalone profile (SQLite + filesystem, viewer enabled)
./target/release/pacsnode generate-config standalone --output config.toml
```

If you omit `--output`, pacsnode prints the generated `config.toml` to stdout.

The generated config enables the bundled OHIF viewer, and the default binary
extracts that viewer into `./web/viewer/` automatically on first start.

### Running in standalone mode

```bash
./target/release/pacsnode
```

Open the admin dashboard at **http://localhost:8042/admin** and the OHIF viewer at **http://localhost:8042/viewer**.

---

## Quick Start Production (Docker Compose)

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

**Step 3 — Set a JWT secret**

Open `config.toml` (in the repo root) and replace the placeholder `jwt_secret` with a real secret before the first run:

```bash
# Generate a strong secret
openssl rand -hex 32
```

Then paste the output as `jwt_secret` in `config.toml`:

```toml
[plugins.basic-auth]
jwt_secret = "<your-generated-secret>"
```

> ⚠️ Never use the default `CHANGE_ME_…` value in an internet-facing or clinical deployment.

**Step 4 — Build and start the stack**

```bash
docker compose up -d
```

This starts four services in dependency order:
1. **PostgreSQL 16** — waits until healthy
2. **MinIO** — waits until healthy
3. **minio-init** — creates the `dicom` bucket, then exits
4. **pacsnode** — starts only after the bucket exists and postgres is ready

The first run compiles the Rust binary inside Docker; this takes a few minutes. Subsequent starts use the image cache and are instant.

**Step 5 — Create the first admin user**

```bash
docker compose exec pacsnode ./pacsnode create-admin \
  --username admin \
  --email admin@example.test
```

The command prints a one-time password. Save it — you'll need it to log in.

**Step 6 — Verify**

```bash
curl http://localhost:8042/health
# {"status":"ok"}
```

Open the admin dashboard at **http://localhost:8042/admin** and the OHIF viewer at **http://localhost:8042/viewer**. Log in with the credentials you just created.

**Services at a glance:**

| Service | Port | URL | Description |
|---------|------|-----|-------------|
| pacsnode REST/DICOMweb | `8042` | `http://localhost:8042` | STOW-RS, QIDO-RS, WADO-RS, REST API |
| Admin dashboard | `8042` | `http://localhost:8042/admin` | User, node, and audit management |
| OHIF viewer | `8042` | `http://localhost:8042/viewer` | Web DICOM viewer |
| pacsnode DIMSE | `4242` | — | C-STORE, C-FIND, C-MOVE, C-GET, C-ECHO |
| MinIO S3 API | `9000` | — | Pixel data object storage |
| MinIO web console | `9001` | `http://localhost:9001` | Browse stored DICOM files (login: see `.env`) |
| PostgreSQL | `5432` | — | Metadata database |

**Tear down:**

```bash
docker compose down          # stop, keep data volumes
docker compose down -v       # stop and delete all data
```

---

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
./target/release/pacsnode generate-config standalone --output config.toml
# Edit config.toml if needed, then run.
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
whitelisting (via admin UI) is enabled, pacsnode only accepts inbound DIMSE
associations from calling AE titles that already exist in this list. The same
registry is also used for outbound DIMSE destinations such as C-MOVE / C-STORE
SCU operations. Nodes are stored in the metadata backend's `dicom_nodes` table and
**persist across restarts**.

> **Important:** If whitelisting is enabled and a modality or remote PACS AE
> title is not present in `/api/nodes`, pacsnode rejects the DIMSE association.

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/nodes` | List all registered remote nodes |
| `POST` | `/api/nodes` | Register or update a remote node (upsert by AE title) |
| `DELETE` | `/api/nodes/{ae_title}` | Remove a remote node |

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
