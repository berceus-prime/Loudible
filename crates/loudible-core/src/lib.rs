//! Models, auth lifecycle, library rules and the export queue, ported from `LoudibleCore`.

pub mod audible_session;
pub mod audiobook;
pub mod authentication_coordinator;
pub mod book_status;
pub mod codec;
pub mod content_license;
pub mod dependencies;
pub mod download_manifest;
pub mod export_file_naming;
pub mod export_job;
pub mod export_queue;
pub mod exporter_error;
pub mod hex_encoding;
pub mod library;
pub mod local_downloads;
pub mod marketplace;
pub mod pending_login;
pub mod text;

pub use audible_session::*;
pub use audiobook::*;
pub use authentication_coordinator::{AuthState, AuthenticationCoordinator};
pub use book_status::*;
pub use content_license::*;
pub use dependencies::*;
pub use download_manifest::*;
pub use export_job::*;
pub use export_queue::{ExportQueue, Snapshot, StartError};
pub use exporter_error::*;
pub use library::*;
pub use local_downloads::LocalDownloads;
pub use marketplace::Marketplace;
pub use pending_login::*;

/// A moment in time, in UTC.
pub type Date = chrono::DateTime<chrono::Utc>;
