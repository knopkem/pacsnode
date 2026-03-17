//! pacsnode filesystem-backed [`BlobStore`][pacs_core::BlobStore] implementation.
//!
//! This crate stores raw DICOM files on the local filesystem so pacsnode can run
//! as a single executable without an external S3-compatible object store.
//!
//! # Example
//!
//! ```no_run
//! use pacs_fs_storage::{FilesystemStorageConfig, FsBlobStore};
//!
//! let config = FilesystemStorageConfig {
//!     root: "./data".into(),
//! };
//! let _store = FsBlobStore::new(&config).expect("failed to build filesystem blob store");
//! ```

pub mod config;
pub mod error;
pub mod fs;
pub mod plugin;

pub use config::FilesystemStorageConfig;
pub use error::FsStorageError;
pub use fs::FsBlobStore;
pub use plugin::{FsBlobStorePlugin, FS_BLOB_STORE_PLUGIN_ID};
