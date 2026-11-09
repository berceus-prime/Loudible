//! Failures shown as toasts: readable guidance, kept apart from the full diagnostic.

use super::book::JobKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialOperation {
    Load,
    Save,
    Delete,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AppError {
    Download {
        book_title: String,
        technical: String,
    },
    Export {
        book_title: String,
        technical: String,
    },
    Enqueue {
        kind: JobKind,
        book_count: usize,
        technical: String,
    },
    SignInBrowser {
        technical: String,
    },
    CredentialStorage {
        operation: CredentialOperation,
        technical: String,
    },
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
            Self::Enqueue {
                kind: JobKind::Export,
                ..
            } => "Couldn’t start exports",
            Self::SignInBrowser { .. } => "Sign-in page couldn’t load",
            Self::CredentialStorage { operation, .. } => match operation {
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
                "Check your connection, then try signing in with your web browser.".into()
            }
            Self::CredentialStorage { operation, .. } => match operation {
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
            .into(),
        }
    }

    pub fn technical(&self) -> &str {
        match self {
            Self::Download { technical, .. }
            | Self::Export { technical, .. }
            | Self::Enqueue { technical, .. }
            | Self::SignInBrowser { technical }
            | Self::CredentialStorage { technical, .. } => technical,
        }
    }
}
