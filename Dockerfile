# ── Builder stage ─────────────────────────────────────────────────────────────
FROM rust:1.88-slim-bookworm AS builder

# Install build deps (OpenSSL for reqwest/rustls, pkg-config, git for cargo git deps)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependency compilation: copy manifests first, then source
COPY Cargo.toml Cargo.lock ./
COPY crates/pacs-core/Cargo.toml           crates/pacs-core/Cargo.toml
COPY crates/pacs-dicom/Cargo.toml          crates/pacs-dicom/Cargo.toml
COPY crates/pacs-store/Cargo.toml          crates/pacs-store/Cargo.toml
COPY crates/pacs-storage/Cargo.toml        crates/pacs-storage/Cargo.toml
COPY crates/pacs-fs-storage/Cargo.toml     crates/pacs-fs-storage/Cargo.toml
COPY crates/pacs-sqlite-store/Cargo.toml   crates/pacs-sqlite-store/Cargo.toml
COPY crates/pacs-dimse/Cargo.toml          crates/pacs-dimse/Cargo.toml
COPY crates/pacs-api/Cargo.toml            crates/pacs-api/Cargo.toml
COPY crates/pacs-plugin/Cargo.toml         crates/pacs-plugin/Cargo.toml
COPY crates/pacs-auth-plugin/Cargo.toml    crates/pacs-auth-plugin/Cargo.toml
COPY crates/pacs-audit-plugin/Cargo.toml   crates/pacs-audit-plugin/Cargo.toml
COPY crates/pacs-admin-plugin/Cargo.toml   crates/pacs-admin-plugin/Cargo.toml
COPY crates/pacs-metrics-plugin/Cargo.toml crates/pacs-metrics-plugin/Cargo.toml
COPY crates/pacs-viewer-plugin/Cargo.toml  crates/pacs-viewer-plugin/Cargo.toml
COPY crates/pacs-server/Cargo.toml         crates/pacs-server/Cargo.toml

# Stub out src so Cargo resolves the dependency graph without full source
RUN for d in pacs-core pacs-dicom pacs-store pacs-storage pacs-fs-storage pacs-sqlite-store \
             pacs-dimse pacs-api pacs-plugin pacs-auth-plugin pacs-audit-plugin \
             pacs-admin-plugin pacs-metrics-plugin pacs-viewer-plugin; do \
      mkdir -p crates/$d/src && echo "pub fn _stub() {}" > crates/$d/src/lib.rs; \
    done && \
    mkdir -p crates/pacs-server/src && \
    echo "fn main() {}" > crates/pacs-server/src/main.rs

# Fetch deps (layer-cached if manifests unchanged)
# dicom-toolkit-rs is fetched automatically via git dependency
RUN cargo fetch

# Copy full source
COPY crates/ crates/
COPY web/ web/
COPY migrations/ migrations/

# Build release binary
RUN cargo build --release --bin pacsnode

# ── Runtime stage ──────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# ca-certificates needed for TLS to S3 and external DICOM peers
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -r -s /bin/false -u 1001 pacsnode

WORKDIR /app

COPY --from=builder /build/target/release/pacsnode ./pacsnode
COPY --from=builder /build/migrations ./migrations
COPY config.toml ./config.toml

RUN chown -R pacsnode:pacsnode /app
USER pacsnode

EXPOSE 8042 4242

ENTRYPOINT ["./pacsnode"]
