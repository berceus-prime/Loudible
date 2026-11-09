//! Pure Rust state and rules; no Skia.

pub mod app_error;
pub mod app_model;
pub mod book;
pub mod live_app;
pub mod mock_app;
pub mod platform;
pub mod preview_data;

pub use app_error::AppError;
pub use app_model::{AppModel, ModelEffect};
pub use book::*;
pub use live_app::LiveApp;
pub use mock_app::{MockApp, MockOptions, PendingLogin, Toast};
pub use preview_data::Account;
