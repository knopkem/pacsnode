//! pacsnode — DIMSE server (SCP) and client (SCU).
//!
//! ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for
//! diagnostic or therapeutic purposes.
//!
//! This crate provides:
//! * [`server::DicomServer`] — an async SCP that listens for incoming DICOM
//!   associations and routes each DIMSE command to the storage back-end.
//! * [`client::DicomClient`] — an async SCU for C-ECHO, C-STORE, C-FIND, and
//!   C-MOVE operations.

pub mod client;
pub mod config;
pub mod error;
pub mod plugin;
pub mod server;

pub use client::DicomClient;
pub use config::DimseConfig;
pub use error::DimseError;
pub use plugin::{PACS_QUERY_SCP_PLUGIN_ID, PACS_STORE_SCP_PLUGIN_ID};
pub use server::{build_dicom_server, DicomNode, DicomServer};
