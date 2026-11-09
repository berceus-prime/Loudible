use base64::{Engine, engine::general_purpose::STANDARD};
use loudible_core::Date;
use ring::rand::SystemRandom;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use url::Url;

use crate::HttpRequest;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SigningError {
    InvalidPem,
    UnsupportedKeyFormat,
    KeyCreationFailed(String),
    SigningFailed(String),
}

/// Signs Audible API requests with the device's RSA key (ADP signing):
/// `base64(RSA-PKCS1v1.5-SHA256("<METHOD>\n<path?query>\n<date>\n<body>\n<adp token>"))`.
pub struct RequestSigner {
    adp_token: String,
    key: RsaKeyPair,
}

impl RequestSigner {
    pub fn new(adp_token: &str, private_key_pem: &str) -> Result<Self, SigningError> {
        Ok(Self {
            adp_token: adp_token.to_string(),
            key: Self::make_key(private_key_pem)?,
        })
    }

    /// Adds `x-adp-token`, `x-adp-alg`, and `x-adp-signature` headers.
    pub fn sign(&self, request: &mut HttpRequest, date: Date) -> Result<(), SigningError> {
        let timestamp = Self::timestamp(date);
        let signature = self.signature(
            &request.method,
            &Self::path_and_query(&request.url),
            &timestamp,
            request.body.as_deref().unwrap_or_default(),
        )?;
        request.set_header("x-adp-token", self.adp_token.clone());
        request.set_header("x-adp-alg", "SHA256withRSA:1.0");
        request.set_header("x-adp-signature", format!("{signature}:{timestamp}"));
        Ok(())
    }

    pub fn signature(
        &self,
        method: &str,
        path_and_query: &str,
        timestamp: &str,
        body: &[u8],
    ) -> Result<String, SigningError> {
        let mut message = format!("{method}\n{path_and_query}\n{timestamp}\n").into_bytes();
        message.extend_from_slice(body);
        message.extend_from_slice(format!("\n{}", self.adp_token).as_bytes());
        let mut signature = vec![0; self.key.public().modulus_len()];
        self.key
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                &message,
                &mut signature,
            )
            .map_err(|error| SigningError::SigningFailed(error.to_string()))?;
        Ok(STANDARD.encode(signature))
    }

    pub fn path_and_query(url: &Url) -> String {
        match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_string(),
        }
    }

    /// UTC ISO-8601 timestamp with microseconds, e.g. `2027-01-15T08:00:00.000000Z`.
    pub fn timestamp(date: Date) -> String {
        date.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
    }

    pub fn make_key(pem: &str) -> Result<RsaKeyPair, SigningError> {
        let lines: Vec<&str> = pem
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        let header = lines
            .first()
            .filter(|line| line.starts_with("-----BEGIN "))
            .ok_or(SigningError::InvalidPem)?;
        let body: String = lines
            .iter()
            .filter(|line| !line.starts_with("-----"))
            .copied()
            .collect();
        let der = STANDARD
            .decode(body)
            .ok()
            .filter(|der| !der.is_empty())
            .ok_or(SigningError::InvalidPem)?;
        let key = match *header {
            "-----BEGIN RSA PRIVATE KEY-----" => RsaKeyPair::from_der(&der),
            "-----BEGIN PRIVATE KEY-----" => RsaKeyPair::from_pkcs8(&der),
            _ => return Err(SigningError::UnsupportedKeyFormat),
        };
        key.map_err(|error| SigningError::KeyCreationFailed(error.to_string()))
    }
}
