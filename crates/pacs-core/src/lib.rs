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
pub mod policy;
pub mod store;

pub use blob::BlobStore;
pub use domain::{
    blob_key_for, AuditLogEntry, AuditLogPage, AuditLogQuery, AuthMode, DicomJson, DicomNode,
    Instance, InstanceQuery, NewAuditLogEntry, PacsStatistics, PasswordPolicy, RefreshToken,
    RefreshTokenId, Series, SeriesQuery, SeriesUid, ServerSettings, SopInstanceUid, Study,
    StudyQuery, StudyUid, TokenPair, User, UserId, UserQuery, UserRole,
};
pub use error::{PacsError, PacsResult};
pub use policy::{PolicyAction, PolicyEngine, PolicyResource, PolicyUser};
pub use store::MetadataStore;
