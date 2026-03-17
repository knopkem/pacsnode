//! PostgreSQL [`MetadataStore`](pacs_core::MetadataStore) implementation for pacsnode.
//!
//! Implements the [`pacs_core::MetadataStore`] trait backed by a `sqlx` PostgreSQL
//! connection pool.
//!
//! # Usage
//!
//! ```rust,no_run
//! use pacs_store::PgMetadataStore;
//! use sqlx::PgPool;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let pool = PgPool::connect("postgres://user:pass@localhost/pacs").await?;
//! let store = PgMetadataStore::new(pool);
//! # Ok(())
//! # }
//! ```
//!
//! # Integration Tests
//!
//! Run integration tests against a real PostgreSQL container (requires Docker):
//!
//! ```bash
//! cargo test -p pacs-store
//! ```
//!
//! Tests use [testcontainers](https://crates.io/crates/testcontainers) and spin up
//! a throwaway Postgres instance automatically — no manual setup required.

pub mod error;
pub mod plugin;
pub mod store;

pub(crate) mod queries;

pub use plugin::{PgMetadataStorePlugin, PG_METADATA_STORE_PLUGIN_ID};
pub use store::PgMetadataStore;
