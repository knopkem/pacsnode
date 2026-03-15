//! pacsnode — DICOM parsing bridge (dicom-toolkit-rs adapter).
//!
//! ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for
//! diagnostic or therapeutic purposes.
//!
//! This crate adapts the [`dicom_toolkit_data`] / [`dicom_toolkit_dict`]
//! ecosystem to the pacsnode domain types defined in `pacs-core`.
//!
//! # Quick start
//!
//! ```no_run
//! use bytes::Bytes;
//! use pacs_dicom::ParsedDicom;
//!
//! # async fn example() -> pacs_core::PacsResult<()> {
//! let raw: Bytes = Bytes::new(); // bytes from network / disk
//! let parsed = ParsedDicom::from_bytes(raw)?;
//! println!("study UID: {}", parsed.study.study_uid);
//! # Ok(())
//! # }
//! ```

pub mod encode;
pub mod error;
pub mod parser;
pub mod stow;
pub mod tags;

pub use error::DicomError;
pub use parser::ParsedDicom;
pub use stow::parse_stow_multipart;
