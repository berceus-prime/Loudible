use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::NoPadding};
use base64::Engine;
use base64::alphabet::STANDARD as STANDARD_ALPHABET;
use base64::engine::{GeneralPurpose, GeneralPurposeConfig};
use loudible_core::{
    AudibleSession, Chapter, ContentLicense, ExporterError, Marketplace, hex_encoding,
};
use sha2::{Digest, Sha256};
use url::Url;

use crate::content_metadata::ContentReference;
use crate::json::{self, Json};
use crate::library_response::check_status;
use crate::{HttpRequest, HttpResponse};

pub fn license_request(asin: &str, marketplace: Marketplace) -> HttpRequest {
    let body = serde_json::json!({
        "consumption_type": "Download",
        "supported_media_features": {
            "drm_types": ["Adrm", "Mpeg"],
            "codecs": ["mp4a.40.2"],
            "chapter_titles_type": "Tree",
            "previews": false,
            "catalog_samples": false,
        },
        "spatial": false,
        "quality": "High",
        "tenant_id": "Audible",
        "response_groups": "last_position_heard,pdf_url,content_reference,chapter_info",
    });
    let url = Url::parse(&format!(
        "https://{}/1.0/content/{asin}/licenserequest",
        marketplace.audible_api_host()
    ))
    .expect("a valid host and path");
    let mut request = HttpRequest::post(url, body.to_string().into_bytes());
    request.set_header("Content-Type", "application/json");
    request.set_header("Accept", "application/json");
    request
}

pub struct ParsedLicense {
    pub license: ContentLicense,
    pub reference: Option<ContentReference>,
}

pub fn parse_license(
    response: &HttpResponse,
    asin: &str,
    session: &AudibleSession,
) -> Result<ParsedLicense, ExporterError> {
    let value = json::parse(&response.body);
    let json = Json::new(&value);
    let license = json.get("content_license");
    let denied = |reason| ExporterError::LicenseDenied {
        asin: asin.to_string(),
        reason,
    };
    if matches!(response.status, 403 | 404) {
        // Audible reports ownership problems as plain HTTP errors with a message.
        let fallback = json
            .get("message")
            .string()
            .unwrap_or_else(|| format!("HTTP {}", response.status));
        let details = if license.is_null() { json } else { license };
        return Err(denied(denial_reason(details, fallback)));
    }
    check_status(response)?;

    if license.is_null() {
        return Err(ExporterError::MalformedResponse(format!(
            "License response for {asin} has no content_license."
        )));
    }
    let status = license.get("status_code").string();
    if status.as_deref().map(str::to_lowercase).as_deref() != Some("granted") {
        let fallback = status.unwrap_or_else(|| "unknown status".to_string());
        return Err(denied(denial_reason(license, fallback)));
    }

    let drm_type = license.get("drm_type").string();
    if drm_type.as_deref().map(str::to_lowercase).as_deref() != Some("adrm") {
        return Err(ExporterError::UnsupportedDrm {
            asin: asin.to_string(),
            drm_type: drm_type.unwrap_or_else(|| "none".to_string()),
        });
    }
    let Some(encrypted_voucher) = license.get("license_response").string() else {
        return Err(ExporterError::MissingDecryptionKey {
            asin: asin.to_string(),
        });
    };
    let voucher = voucher::decrypt(&encrypted_voucher, asin, session)?;
    let decryption =
        ContentLicense::decryption(asin, drm_type.as_deref(), voucher.key, voucher.iv)?;

    let metadata = license.get("content_metadata");
    let download_url = metadata
        .get("content_url")
        .get("offline_url")
        .string()
        .and_then(|text| Url::parse(&text).ok())
        .filter(|url| url.scheme() == "https")
        .ok_or_else(|| {
            ExporterError::MalformedResponse(format!("License for {asin} has no download URL."))
        })?;
    let chapter_info = metadata.get("chapter_info");
    let reference = metadata.get("content_reference");
    Ok(ParsedLicense {
        license: ContentLicense {
            asin: asin.to_string(),
            decryption,
            download_url,
            chapters: flatten(&chapter_info.get("chapters").array().unwrap_or_default()),
            runtime_milliseconds: chapter_info.get("runtime_length_ms").int(),
            codec: reference.get("codec").string(),
            content_format: reference.get("content_format").string(),
        },
        reference: ContentReference::new(reference),
    })
}

fn denial_reason(license: Json, fallback: String) -> String {
    fn text(value: Json) -> Option<String> {
        Some(value.string()?.trim().to_string()).filter(|text| !text.is_empty())
    }
    let mut lines = vec![text(license.get("message")).unwrap_or(fallback)];
    for reason in license
        .get("license_denial_reasons")
        .array()
        .unwrap_or_default()
    {
        let message = text(reason.get("message"));
        let rejection = text(reason.get("rejectionReason"));
        if message.is_none() && rejection.is_none() {
            continue;
        }
        let label = [text(reason.get("validationType")), rejection]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" / ");
        let parts: Vec<String> = [Some(label).filter(|label| !label.is_empty()), message]
            .into_iter()
            .flatten()
            .collect();
        lines.push(parts.join(": "));
    }
    lines.join("\n")
}

/// Flattens the chapter tree in playback order. A parent keeps its own (intro) segment
/// as a chapter when it has one; zero-length entries are dropped.
pub fn flatten(chapters: &[Json]) -> Vec<Chapter> {
    let mut result: Vec<Chapter> = Vec::new();
    for chapter in chapters {
        if let (Some(start), Some(length)) = (
            chapter.get("start_offset_ms").int(),
            chapter.get("length_ms").int(),
        ) && length > 0
        {
            result.push(Chapter {
                title: chapter
                    .get("title")
                    .string()
                    .unwrap_or_else(|| format!("Chapter {}", result.len() + 1)),
                start_milliseconds: start,
                length_milliseconds: length,
            });
        }
        result.extend(flatten(
            &chapter.get("chapters").array().unwrap_or_default(),
        ));
    }
    result.sort_by_key(|chapter| chapter.start_milliseconds);
    result
}

/// The license "voucher": AES-128-CBC encrypted JSON holding the content key and IV.
/// Key and IV are the halves of SHA-256(device type + device serial + customer id + ASIN).
pub mod voucher {
    use super::*;

    const BLOCK_SIZE: usize = 16;
    // Accepts what Foundation's decoder accepts, including stray bits in the final character.
    const BASE64: GeneralPurpose = GeneralPurpose::new(
        &STANDARD_ALPHABET,
        GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
    );

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Contents {
        pub key: Option<Vec<u8>>,
        pub iv: Option<Vec<u8>>,
    }

    pub fn decrypt(
        base64: &str,
        asin: &str,
        session: &AudibleSession,
    ) -> Result<Contents, ExporterError> {
        let mut ciphertext = BASE64
            .decode(base64)
            .ok()
            .filter(|bytes| bytes.len() >= BLOCK_SIZE)
            .ok_or_else(|| {
                ExporterError::MalformedResponse(format!(
                    "License voucher for {asin} is not valid base64."
                ))
            })?;
        let (key, iv) = derived_key_and_iv(asin, session);
        let whole_blocks = ciphertext.len() - ciphertext.len() % BLOCK_SIZE;
        let plaintext = cbc::Decryptor::<aes::Aes128>::new_from_slices(&key, &iv)
            .expect("16-byte key and IV")
            .decrypt_padded_mut::<NoPadding>(&mut ciphertext[..whole_blocks])
            .map_err(|_| {
                ExporterError::MalformedResponse(format!(
                    "License voucher for {asin} could not be decrypted."
                ))
            })?;
        parse(plaintext, asin)
    }

    pub fn derived_key_and_iv(asin: &str, session: &AudibleSession) -> (Vec<u8>, Vec<u8>) {
        let material = format!(
            "{}{}{}{asin}",
            session.device.device_type, session.device.serial, session.customer_id
        );
        let digest = Sha256::digest(material.as_bytes());
        (digest[..16].to_vec(), digest[16..].to_vec())
    }

    /// The plaintext is JSON padded with NULs; tolerate trailing garbage after the object.
    pub fn parse(plaintext: &[u8], asin: &str) -> Result<Contents, ExporterError> {
        let mut bytes = plaintext;
        while let [rest @ .., 0] = bytes {
            bytes = rest;
        }
        if let Some(end) = bytes.iter().rposition(|byte| *byte == b'}') {
            bytes = &bytes[..=end];
        }
        let value = json::parse(bytes);
        let json = Json::new(&value);
        if !json.is_object() {
            return Err(ExporterError::MalformedResponse(format!(
                "License voucher for {asin} is not readable."
            )));
        }
        let hex = |field: &str| -> Result<Option<Vec<u8>>, ExporterError> {
            let Some(text) = json.get(field).string().filter(|text| !text.is_empty()) else {
                return Ok(None);
            };
            hex_encoding::decode(&text).map(Some).ok_or_else(|| {
                ExporterError::MalformedResponse(format!(
                    "License voucher for {asin} has an invalid {field}."
                ))
            })
        };
        Ok(Contents {
            key: hex("key")?,
            iv: hex("iv")?,
        })
    }
}
