use std::fmt;

use crate::LoginCallbackError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExporterError {
    /// No usable credentials: the user must sign in (again).
    AuthenticationRequired,
    LoginCallback(LoginCallbackError),
    Http {
        status: u16,
        message: Option<String>,
    },
    Network(String),
    MalformedResponse(String),
    LicenseDenied {
        asin: String,
        reason: String,
    },
    UnsupportedDrm {
        asin: String,
        drm_type: String,
    },
    MissingDecryptionKey {
        asin: String,
    },
    CredentialStorage(String),
    Download(String),
    Conversion(String),
    FileSystem(String),
    Cancelled,
    Unexpected(String),
}

impl fmt::Display for ExporterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthenticationRequired => write!(f, "Sign in to Audible to continue."),
            Self::LoginCallback(error) => match error {
                LoginCallbackError::MalformedUrl => {
                    write!(f, "The sign-in address is not a valid URL.")
                }
                LoginCallbackError::UnexpectedHost(host) => write!(
                    f,
                    "The sign-in address points to an unexpected host ({}).",
                    host.as_deref().unwrap_or("none")
                ),
                LoginCallbackError::UnexpectedPath(path) => {
                    write!(f, "The sign-in address has an unexpected path ({path}).")
                }
                LoginCallbackError::MissingAuthorizationCode => {
                    write!(
                        f,
                        "The sign-in address does not contain an authorization code."
                    )
                }
                LoginCallbackError::NoPendingLogin => {
                    write!(
                        f,
                        "This sign-in is no longer active. Start signing in again."
                    )
                }
                LoginCallbackError::Expired => {
                    write!(f, "This sign-in has expired. Start signing in again.")
                }
            },
            Self::Http { status, message } => match message {
                Some(message) => write!(f, "Audible returned HTTP {status}: {message}."),
                None => write!(f, "Audible returned HTTP {status}."),
            },
            Self::Network(detail) => write!(f, "Network error: {detail}"),
            Self::MalformedResponse(detail) => {
                write!(f, "Audible returned an unexpected response: {detail}")
            }
            Self::LicenseDenied { reason, .. } => {
                write!(f, "The download license was denied: {reason}")
            }
            Self::UnsupportedDrm { drm_type, .. } => {
                write!(f, "This title uses unsupported DRM ({drm_type}).")
            }
            Self::MissingDecryptionKey { .. } => {
                write!(f, "The download license does not contain a decryption key.")
            }
            Self::CredentialStorage(detail) => {
                write!(f, "Could not access saved credentials: {detail}")
            }
            Self::Download(detail) => write!(f, "Download failed: {detail}"),
            Self::Conversion(detail) => write!(f, "Conversion failed: {detail}"),
            Self::FileSystem(detail) => write!(f, "File error: {detail}"),
            Self::Cancelled => write!(f, "Cancelled."),
            Self::Unexpected(detail) => write!(f, "Unexpected error: {detail}"),
        }
    }
}

impl std::error::Error for ExporterError {}
