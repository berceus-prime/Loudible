//! UI-independent view models.
//!
//! The models live on one thread, inside a
//! [`tokio::task::LocalSet`], and report changes through [`Changes`]. The UI thread never touches
//! them; `app_runtime` runs them on their own thread and publishes immutable [`AppSnapshot`]s.

pub mod app_error;
pub mod app_runtime;
pub mod app_snapshot;
pub mod app_view_model;
pub mod book_status_text;
pub mod changes;
pub mod export_view_model;
pub mod library_view_model;
pub mod session_view_model;

pub use app_error::{AppError, ErrorToastQueue, Notice};
pub use app_runtime::{AppDependencies, AppHandle, Command, start_app};
pub use app_snapshot::{AppSnapshot, ExportSnapshot, LibrarySnapshot, SessionSnapshot};
pub use app_view_model::AppViewModel;
pub use book_status_text::{accessibility_text, transfer_text};
pub use changes::Changes;
pub use export_view_model::ExportViewModel;
pub use library_view_model::{LibraryViewModel, LoadState, MarketplaceCount};
pub use session_view_model::SessionViewModel;
