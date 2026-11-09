use std::cell::RefCell;

use loudible_core::{CredentialOperation, CredentialStorageFailure, JobKind};
use uuid::Uuid;

use crate::Changes;

/// Failures presented as toasts, with readable guidance separate from the full diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppError {
    Download {
        book_title: String,
        technical_error: String,
    },
    Export {
        book_title: String,
        technical_error: String,
    },
    Enqueue {
        kind: JobKind,
        book_count: usize,
        technical_error: String,
    },
    SignInBrowser {
        technical_error: String,
    },
    CredentialStorage(CredentialStorageFailure),
}

impl AppError {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Download { .. } => "Download failed",
            Self::Export { .. } => "Export failed",
            Self::Enqueue {
                kind: JobKind::Download,
                ..
            } => "Couldn’t start downloads",
            Self::Enqueue { .. } => "Couldn’t start exports",
            Self::SignInBrowser { .. } => "Sign-in page couldn’t load",
            Self::CredentialStorage(failure) => match failure.operation {
                CredentialOperation::Load => "Couldn’t restore your sign-in",
                CredentialOperation::Save => "Couldn’t save your sign-in",
                CredentialOperation::Delete => "Couldn’t remove saved credentials",
            },
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Download { book_title, .. } => {
                format!("“{book_title}” couldn’t be downloaded. Try downloading the book again.")
            }
            Self::Export { book_title, .. } => {
                format!("“{book_title}” couldn’t be exported to M4B. Try exporting the book again.")
            }
            Self::Enqueue { book_count, .. } => format!(
                "Couldn’t queue {book_count} selected {}. Check that the download folder is writable, then try again.",
                if *book_count == 1 { "book" } else { "books" }
            ),
            Self::SignInBrowser { .. } => {
                "Check your connection, then try signing in with your web browser.".to_string()
            }
            Self::CredentialStorage(failure) => match failure.operation {
                CredentialOperation::Load => {
                    "Your saved credentials couldn’t be read. Try signing in again."
                }
                CredentialOperation::Save => {
                    "You’re signed in, but your credentials couldn’t be saved. You may need to sign in next time."
                }
                CredentialOperation::Delete => {
                    "Your saved credentials couldn’t be removed. They may be restored next time you open the app."
                }
            }
            .to_string(),
        }
    }

    pub fn technical_error(&self) -> &str {
        match self {
            Self::Download {
                technical_error, ..
            }
            | Self::Export {
                technical_error, ..
            }
            | Self::Enqueue {
                technical_error, ..
            }
            | Self::SignInBrowser { technical_error } => technical_error,
            Self::CredentialStorage(failure) => &failure.technical_error,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub id: Uuid,
    pub error: AppError,
}

/// Keeps simultaneous failures in arrival order until each is explicitly dismissed.
#[derive(Default)]
pub struct ErrorToastQueue {
    notices: RefCell<Vec<Notice>>,
    changes: Changes,
}

impl ErrorToastQueue {
    pub fn new(changes: Changes) -> Self {
        Self {
            notices: RefCell::default(),
            changes,
        }
    }

    pub fn notices(&self) -> Vec<Notice> {
        self.notices.borrow().clone()
    }

    pub fn present(&self, error: AppError) {
        self.notices.borrow_mut().push(Notice {
            id: Uuid::new_v4(),
            error,
        });
        self.changes.mark();
    }

    /// Uses identity so a stale dismiss action cannot remove a newer error.
    pub fn dismiss(&self, id: Uuid) {
        self.notices.borrow_mut().retain(|notice| notice.id != id);
        self.changes.mark();
    }
}
