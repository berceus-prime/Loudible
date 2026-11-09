use std::error::Error;
use std::time::Duration;

use async_trait::async_trait;
use loudible_core::ExporterError;
use url::Url;

/// One HTTP request, in a form tests can inspect.
#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub method: String,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

fn find<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

impl HttpRequest {
    pub fn get(url: Url) -> Self {
        Self {
            method: "GET".into(),
            url,
            headers: Vec::new(),
            body: None,
        }
    }

    pub fn post(url: Url, body: Vec<u8>) -> Self {
        Self {
            method: "POST".into(),
            url,
            headers: Vec::new(),
            body: Some(body),
        }
    }

    pub fn set_header(&mut self, name: &str, value: impl Into<String>) {
        self.headers
            .retain(|(key, _)| !key.eq_ignore_ascii_case(name));
        self.headers.push((name.to_string(), value.into()));
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        find(&self.headers, name)
    }
}

#[derive(Clone, Debug, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        find(&self.headers, name)
    }
}

/// Sends one HTTP request. The Audible adapter only talks to the network through this,
/// so its tests can substitute canned responses.
#[async_trait]
pub trait HttpTransport: Send + Sync {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, ExporterError>;
}

/// A client with `URLSession`'s stall limit and none of its persistent state.
pub(crate) fn client_builder() -> reqwest::ClientBuilder {
    // Fails only when a provider is already installed, which is just as good.
    let _ = rustls::crypto::ring::default_provider().install_default();
    const STALL_LIMIT: Duration = Duration::from_secs(60);
    reqwest::Client::builder()
        .connect_timeout(STALL_LIMIT)
        .read_timeout(STALL_LIMIT)
        .referer(false)
}

/// The message and its causes, without the URL: download URLs carry access tokens.
pub(crate) fn describe(error: reqwest::Error) -> String {
    let error = error.without_url();
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self, ExporterError> {
        let client = client_builder()
            .user_agent(concat!("Loudible/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| ExporterError::Unexpected(describe(error)))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, ExporterError> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| ExporterError::Unexpected(error.to_string()))?;
        let mut builder = self.client.request(method, request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let network = |error| ExporterError::Network(describe(error));
        let response = builder.send().await.map_err(network)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_string())))
            .collect();
        let body = response.bytes().await.map_err(network)?.to_vec();
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}
