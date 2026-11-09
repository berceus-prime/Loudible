use serde::{Deserialize, Serialize};
use url::Url;

use crate::ExporterError;
use crate::codec::base64_bytes;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    pub start_milliseconds: i64,
    pub length_milliseconds: i64,
}

impl Chapter {
    pub fn end_milliseconds(&self) -> i64 {
        self.start_milliseconds + self.length_milliseconds
    }
}

/// Serializable so a download's manifest can keep it for offline export.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decryption {
    /// AAXC: per-title AES key and IV from the decrypted voucher.
    Aaxc {
        #[serde(with = "base64_bytes")]
        key: Vec<u8>,
        #[serde(with = "base64_bytes")]
        iv: Vec<u8>,
    },
    /// Legacy AAX: 4 activation bytes.
    Aax {
        #[serde(with = "base64_bytes")]
        activation_bytes: Vec<u8>,
    },
}

impl Decryption {
    /// Extension of the encrypted file this scheme decrypts.
    pub fn file_extension(&self) -> &'static str {
        match self {
            Self::Aaxc { .. } => "aaxc",
            Self::Aax { .. } => "aax",
        }
    }
}

/// A granted download license with everything needed to fetch and decrypt one title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentLicense {
    pub asin: String,
    pub decryption: Decryption,
    pub download_url: Url,
    pub chapters: Vec<Chapter>,
    pub runtime_milliseconds: Option<i64>,
    pub codec: Option<String>,
    pub content_format: Option<String>,
}

impl ContentLicense {
    /// Chooses the decryption scheme from the license DRM type and voucher key material:
    /// ADRM with a 4-byte key and no IV is AAX; ADRM with a 16-byte key and 16-byte IV is AAXC.
    /// Anything else is unsupported.
    pub fn decryption(
        asin: &str,
        drm_type: Option<&str>,
        key: Option<Vec<u8>>,
        iv: Option<Vec<u8>>,
    ) -> Result<Decryption, ExporterError> {
        if drm_type.map(str::to_lowercase).as_deref() != Some("adrm") {
            return Err(ExporterError::UnsupportedDrm {
                asin: asin.to_string(),
                drm_type: drm_type.unwrap_or("none").to_string(),
            });
        }
        let Some(key) = key.filter(|key| !key.is_empty()) else {
            return Err(ExporterError::MissingDecryptionKey {
                asin: asin.to_string(),
            });
        };
        match (key.len(), iv.as_ref().map(Vec::len)) {
            (4, None) => Ok(Decryption::Aax {
                activation_bytes: key,
            }),
            (16, Some(16)) => Ok(Decryption::Aaxc {
                key,
                iv: iv.unwrap_or_default(),
            }),
            (key_length, iv_length) => Err(ExporterError::UnsupportedDrm {
                asin: asin.to_string(),
                drm_type: format!(
                    "Adrm (key {key_length} bytes, iv {} bytes)",
                    iv_length.unwrap_or(0)
                ),
            }),
        }
    }
}
