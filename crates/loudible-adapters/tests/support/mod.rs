//! Doubles shared by the adapter tests. All data is synthetic.
#![allow(dead_code)]

use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::BoxFuture;
use loudible_adapters::*;
use loudible_core::*;
use loudible_test_support::*;
use serde_json::Value;
use uuid::Uuid;

pub fn fixture_path(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

pub fn data(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

pub fn text(name: &str) -> String {
    String::from_utf8(data(name)).expect("UTF-8")
}

pub fn json(name: &str) -> Value {
    serde_json::from_slice(&data(name)).expect("valid JSON")
}

/// A session whose device key is the synthetic test key, so requests can really be signed.
pub fn signing_session(marketplace: Marketplace) -> AudibleSession {
    AudibleSession {
        device_private_key: text("synthetic-device-key.pem"),
        adp_token: "{enc:synthetic-adp-token}".into(),
        ..session_in(marketplace)
    }
}

pub fn response(status: u16, body: impl Into<Vec<u8>>) -> HttpResponse {
    HttpResponse {
        status,
        headers: Vec::new(),
        body: body.into(),
    }
}

type Handler = Arc<
    dyn Fn(HttpRequest) -> BoxFuture<'static, Result<HttpResponse, ExporterError>> + Send + Sync,
>;

/// Canned HTTP responses, recording every request.
pub struct MockTransport {
    handler: Handler,
    pub requests: Locked<Vec<HttpRequest>>,
}

impl MockTransport {
    pub fn new<F, Fut>(body: F) -> Arc<Self>
    where
        F: Fn(HttpRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<HttpResponse, ExporterError>> + Send + 'static,
    {
        Arc::new(Self {
            handler: Arc::new(move |request| body(request).boxed()),
            requests: Locked::default(),
        })
    }

    /// Replies to every request with the same status and body.
    pub fn fixed(status: u16, body: impl Into<Vec<u8>>) -> Arc<Self> {
        let body = body.into();
        Self::new(move |_| {
            let body = body.clone();
            async move { Ok(response(status, body)) }
        })
    }

    /// The license for POST requests and the metadata for GET requests.
    pub fn license_and_metadata(license: Vec<u8>, metadata: Vec<u8>) -> Arc<Self> {
        Self::new(move |request| {
            let body = if request.method == "POST" {
                license.clone()
            } else {
                metadata.clone()
            };
            async move { Ok(response(200, body)) }
        })
    }

    pub fn last(&self) -> HttpRequest {
        self.requests.current().last().expect("a request").clone()
    }
}

#[async_trait]
impl HttpTransport for MockTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, ExporterError> {
        self.requests.with(|all| all.push(request.clone()));
        (self.handler)(request).await
    }
}

/// Bytes counting up from `seed`, and UUIDs counting up from one.
pub struct SequentialRandom(Locked<(u8, u128)>);

impl SequentialRandom {
    pub fn new(seed: u8) -> Arc<Self> {
        Arc::new(Self(Locked::new((seed, 1))))
    }
}

impl RandomSource for SequentialRandom {
    fn bytes(&self, count: usize) -> Vec<u8> {
        self.0.with(|state| {
            (0..count)
                .map(|_| {
                    let byte = state.0;
                    state.0 = state.0.wrapping_add(1);
                    byte
                })
                .collect()
        })
    }

    fn uuid(&self) -> Uuid {
        self.0.with(|state| {
            state.1 += 1;
            Uuid::from_u128(state.1 - 1)
        })
    }
}

pub fn api(transport: Arc<MockTransport>) -> AudibleHttpApi {
    AudibleHttpApi::new(
        transport,
        SequentialRandom::new(0),
        Arc::new(TestClock::default()),
    )
}

pub fn query_of(url: &url::Url) -> std::collections::HashMap<String, String> {
    url.query_pairs().into_owned().collect()
}

pub fn json_body(request: &HttpRequest) -> Value {
    serde_json::from_slice(request.body.as_ref().expect("a body")).expect("a JSON body")
}
