//! Audible HTTP API, request signing, credentials and downloads, ported from `LoudibleAdapters`.

pub mod audible_http_api;
pub mod audible_login_factory;
pub mod content_metadata;
pub mod credentials;
pub mod http_transport;
pub mod json;
pub mod library_response;
pub mod license_response;
pub mod random;
pub mod request_signer;
pub mod reqwest_downloader;

pub use audible_http_api::AudibleHttpApi;
pub use audible_login_factory::{AudibleLoginFactory, pkce};
pub use credentials::{
    EncryptedFileCredentialStore, KeyFileSecretStore, KeyringSecretStore, SecretStore,
    TimeoutSecretStore, default_credential_store,
};
pub use http_transport::{HttpRequest, HttpResponse, HttpTransport, ReqwestTransport};
pub use random::{RandomSource, SystemRandomSource};
pub use request_signer::{RequestSigner, SigningError};
pub use reqwest_downloader::ReqwestDownloader;
