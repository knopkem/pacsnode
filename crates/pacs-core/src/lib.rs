//! pacsnode — core domain types, traits, and error definitions.
//!
//! ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for
//! diagnostic or therapeutic purposes.
//!
//! This crate contains no I/O, no external service calls, and no async
//! executors. It is a pure-Rust domain model that all other crates depend on.

pub mod blob;
pub mod domain;
pub mod error;
pub mod store;

pub use blob::BlobStore;
pub use domain::{
    blob_key_for, DicomJson, DicomNode, Instance, InstanceQuery, PacsStatistics, Series,
    SeriesQuery, SeriesUid, SopInstanceUid, Study, StudyQuery, StudyUid,
};
pub use error::{PacsError, PacsResult};
pub use store::MetadataStore;
