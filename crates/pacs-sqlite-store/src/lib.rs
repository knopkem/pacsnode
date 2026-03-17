//! SQLite-backed [`MetadataStore`][pacs_core::MetadataStore] implementation for pacsnode.
//!
//! This crate provides a self-contained metadata store for development,
//! testing, and smaller standalone deployments where PostgreSQL is not desired.
//!
//! # Usage
//!
//! ```rust,no_run
//! use pacs_sqlite_store::SqliteMetadataStore;
//! use sqlx::SqlitePool;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let pool = SqlitePool::connect("sqlite://pacsnode.db").await?;
//! let _store = SqliteMetadataStore::new(pool);
//! # Ok(())
//! # }
//! ```

pub mod plugin;
pub mod store;

pub use plugin::{SqliteMetadataStorePlugin, SQLITE_METADATA_STORE_PLUGIN_ID};
pub use store::SqliteMetadataStore;
