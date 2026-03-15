# GitHub Copilot Instructions — pacsnode

This is a production-grade, high-performance DICOM PACS system written in Rust.
All generated code must meet the standards below without exception.

---

## Code Quality

- Write **production-ready Rust** — no placeholder logic, no `todo!()` left in non-test code, no `unwrap()` or `expect()` outside of tests or `main` startup validation where a panic is acceptable.
- Prefer `?` for error propagation. Define domain-specific error types with `thiserror`. Never use `anyhow` in library crates; `anyhow` is acceptable only in binary entry points.
- All public items must have doc comments (`///`). Include at least one `# Example` block for non-trivial public APIs.
- No `clippy` warnings — code must pass `cargo clippy -- -D warnings` clean. Apply `#[allow(...)]` only when genuinely necessary and always with a comment explaining why.
- Format all code with `rustfmt` (default settings). Never submit unformatted code.
- Avoid `unsafe` unless interfacing with C FFI (e.g., OpenJPEG). Every `unsafe` block must have a `// SAFETY:` comment explaining the invariants upheld.

---

## Rust Patterns

Apply idiomatic Rust patterns consistently:

- **Newtype pattern** for domain identifiers (e.g., `StudyUid(String)`, `SeriesUid(String)`) — prevents mixing up UIDs at the type level.
- **Builder pattern** for structs with many optional fields (e.g., query builders, config structs). Implement via a dedicated `XxxBuilder` struct with a consuming `build() -> Result<Xxx>`.
- **Typestate pattern** for protocol state machines (e.g., DIMSE association lifecycle: `Association<Unassociated>` → `Association<Established>`).
- **`From`/`Into`/`TryFrom`/`TryInto`** for all conversions between domain types and external types (DICOM elements, database rows, API DTOs).
- **`Display` + `Error`** implementations on all error types.
- **`Default`** on config and option structs where zero-value defaults are meaningful.
- Prefer **`Arc<dyn Trait>`** for shared, injectable dependencies (`MetadataStore`, `BlobStore`) — enables testing with mocks.
- Use **`tokio::sync`** primitives (`RwLock`, `Mutex`, `broadcast`, `mpsc`) over `std::sync` in async code.
- Leverage **`tower` middleware** (tracing, timeout, rate-limit) for Axum routes rather than duplicating cross-cutting logic in handlers.
- Prefer **`bytes::Bytes`** for zero-copy binary data passing between components (DICOM pixel data, multipart bodies).

---

## Testing (Mandatory)

Testing is not optional. Every PR must include tests. The bar:

### Unit Tests
- Every module with non-trivial logic must have a `#[cfg(test)] mod tests { ... }` block in the same file.
- Test the happy path, error paths, and edge cases.
- Use **`rstest`** for parameterised tests and fixtures.
- Mock trait dependencies with **`mockall`** — never reach out to real databases or network in unit tests.

### Integration Tests
- Place integration tests in `tests/` at the crate root.
- Integration tests for `pacs-store` must run against a real PostgreSQL instance (use `testcontainers` to spin up a container in CI).
- Integration tests for `pacs-storage` must run against a real RustFS/MinIO instance (use `testcontainers`).
- Integration tests for `pacs-dimse` must open real DICOM associations using the `dicom-toolkit-rs` SCU tools.

### End-to-End Tests
- `tests/integration/` at workspace root contains E2E tests using real DICOM files from the test fixtures directory.
- E2E tests must cover the full pipeline: ingest via C-STORE → QIDO query → WADO retrieve → verify data integrity.
- Use real DICOM files sourced from public datasets (TCIA). Fixtures live in `tests/fixtures/`.

### Test Hygiene
- No `#[ignore]` without a tracking issue reference in the comment.
- All tests must be deterministic — no reliance on system time, random values, or port availability without explicit seeding/allocation.
- Use **`tokio::test`** for async tests. Use **`#[tokio::test(flavor = "multi_thread")]`** when testing concurrency behaviour.
- Assert error types precisely — match the variant, not just `is_err()`.

---

## Error Handling

- Library crates (`pacs-core`, `pacs-store`, `pacs-dicom`, etc.) define their own `Error` enum with `thiserror`.
- Errors must be meaningful to the caller — wrap lower-level errors with context using `#[from]` or `.map_err(|e| Error::StoreFailed { source: e, uid: uid.to_string() })`.
- Never silently swallow errors. Log with `tracing::error!` at the boundary where you decide not to propagate.

---

## Async & Concurrency

- All I/O is async. Blocking operations (file I/O, CPU-heavy codec work) must be dispatched via `tokio::task::spawn_blocking`.
- Avoid holding locks across `.await` points. Prefer scoped lock guards or restructure code to release before awaiting.
- Cancellation safety: document any `async fn` that is NOT cancellation-safe with a `# Cancellation Safety` section in its doc comment.

---

## Logging & Tracing

- Use `tracing` spans and events throughout, not `println!` or `log!`.
- Instrument all service-layer functions with `#[tracing::instrument(skip(self), err)]`.
- Include structured fields on spans: `study_uid`, `series_uid`, `instance_uid`, `ae_title` as appropriate.
- Log at the right level: `trace!` for per-frame/per-tag operations, `debug!` for per-instance, `info!` for per-study and connection lifecycle, `warn!` for recoverable issues, `error!` for failures.

---

## Database (sqlx + PostgreSQL)

- All queries use **compile-time verified** `sqlx::query!` / `sqlx::query_as!` macros — never runtime string queries.
- Transactions are used for any operation that touches multiple tables.
- Migrations are managed with `sqlx-cli`. Every schema change has a corresponding migration file in `migrations/`.
- Use `PgPool` everywhere; never hold a single connection across request boundaries.

---

## Security

- Never log PHI (Patient Health Information) — patient names, IDs, dates must not appear in log output. Use UIDs (study/series/instance) in structured fields instead.
- Secrets (DB passwords, JWT secrets, S3 keys) come from environment variables or config files, never hardcoded.
- Input validation on all API boundaries — reject malformed UIDs, oversized payloads, and unexpected content types with appropriate HTTP 4xx responses.
