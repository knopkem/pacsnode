//! pacsnode S3/RustFS [`BlobStore`][pacs_core::BlobStore] implementation.
//!
//! This crate implements the [`pacs_core::BlobStore`] trait backed by any
//! S3-compatible object store: AWS S3, MinIO, or RustFS.
//!
//! # Quick start
//!
//! ```no_run
//! use pacs_storage::{S3BlobStore, StorageConfig};
//!
//! let config = StorageConfig {
//!     endpoint:   "http://localhost:9000".into(),
//!     bucket:     "dicom".into(),
//!     access_key: "minioadmin".into(),
//!     secret_key: "minioadmin".into(),
//!     region:     "us-east-1".into(),
//! };
//! let _store = S3BlobStore::new(&config).expect("failed to build S3 store");
//! ```

pub mod config;
pub mod error;
pub mod s3;

pub use config::StorageConfig;
pub use error::StorageError;
pub use s3::S3BlobStore;
