//! Ported from `LicenseAndDownloadTests.swift` and `LicenseDenialTests.swift`.

mod support;

use base64::{Engine, engine::general_purpose::STANDARD};
use loudible_adapters::license_response::{parse_license, voucher};
use loudible_adapters::*;
use loudible_core::*;
use loudible_test_support::*;
use ring::signature::{KeyPair, RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey};
use serde_json::{Value, json as object};
use support::*;

fn hex(text: &str) -> Vec<u8> {
    hex_encoding::decode(text).unwrap()
}

fn voucher_vector() -> Value {
    json("voucher-vector.json")
}

fn field<'a>(vector: &'a Value, name: &str) -> &'a str {
    vector[name].as_str().unwrap()
}

#[test]
fn derives_key_and_iv_from_device_customer_and_asin() {
    let vector = voucher_vector();
    let (key, iv) = voucher::derived_key_and_iv(field(&vector, "asin"), &session());
    assert_eq!(hex_encoding::encode(&key), field(&vector, "derived_key"));
    assert_eq!(hex_encoding::encode(&iv), field(&vector, "derived_iv"));
}

#[test]
fn decrypts_openssl_encrypted_voucher() {
    let vector = voucher_vector();
    let contents = voucher::decrypt(
        field(&vector, "voucher_base64"),
        field(&vector, "asin"),
        &session(),
    )
    .unwrap();
    assert_eq!(contents.key, Some(hex("0123456789abcdef0123456789abcdef")));
    assert_eq!(contents.iv, Some(hex("fedcba9876543210fedcba9876543210")));
}

#[test]
fn wrong_customer_cannot_read_voucher() {
    let vector = voucher_vector();
    let other = AudibleSession {
        customer_id: "amzn1.account.OTHER".into(),
        ..session()
    };
    assert!(
        voucher::decrypt(
            field(&vector, "voucher_base64"),
            field(&vector, "asin"),
            &other
        )
        .is_err()
    );
}

#[test]
fn rejects_malformed_vouchers() {
    assert!(voucher::decrypt("***", "B1", &session()).is_err());
    assert!(voucher::decrypt(&STANDARD.encode([0; 8]), "B1", &session()).is_err());
    assert!(voucher::parse(br#"{"key":"zz"}"#, "B1").is_err());
}

#[test]
fn tolerates_padding_and_trailing_bytes() {
    let contents = voucher::parse(b"{\"key\":\"0a0b0c0d\"}\0\0\0", "B1").unwrap();
    assert_eq!(
        contents,
        voucher::Contents {
            key: Some(vec![10, 11, 12, 13]),
            iv: None
        }
    );
    let trailing = voucher::parse(br#"{"key":"0a0b0c0d"}garbage"#, "B1").unwrap();
    assert_eq!(trailing.key, Some(vec![10, 11, 12, 13]));
}

fn parse(body: Vec<u8>, status: u16, asin: &str) -> Result<ContentLicense, ExporterError> {
    parse_license(&response(status, body), asin, &session())
}

fn parse_fixture(name: &str, asin: &str) -> Result<ContentLicense, ExporterError> {
    parse(data(name), 200, asin)
}

fn denied(asin: &str, reason: &str) -> Result<ContentLicense, ExporterError> {
    Err(ExporterError::LicenseDenied {
        asin: asin.into(),
        reason: reason.into(),
    })
}

#[tokio::test]
async fn builds_signed_license_request() {
    let transport = MockTransport::fixed(200, data("license-aaxc.json"));
    let session = signing_session(Marketplace::US);
    let _ = api(transport.clone())
        .license("B000000001", Marketplace::US, &session)
        .await;
    let request = transport.requests.current()[0].clone();
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.url.as_str(),
        "https://api.audible.com/1.0/content/B000000001/licenserequest"
    );
    assert_eq!(
        json_body(&request),
        object!({
            "consumption_type": "Download", "tenant_id": "Audible", "quality": "High", "spatial": false,
            "response_groups": "last_position_heard,pdf_url,content_reference,chapter_info",
            "supported_media_features": {
                "drm_types": ["Adrm", "Mpeg"], "codecs": ["mp4a.40.2"], "chapter_titles_type": "Tree",
                "previews": false, "catalog_samples": false,
            },
        })
    );
    let header = request.header("x-adp-signature").unwrap();
    let (signature, date) = header.split_once(':').unwrap();
    let mut message =
        format!("POST\n/1.0/content/B000000001/licenserequest\n{date}\n").into_bytes();
    message.extend_from_slice(request.body.as_ref().unwrap());
    message.extend_from_slice(format!("\n{}", session.adp_token).as_bytes());
    let key = RequestSigner::make_key(&session.device_private_key).unwrap();
    UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, key.public_key().as_ref())
        .verify(&message, &STANDARD.decode(signature).unwrap())
        .unwrap();
}

#[tokio::test]
async fn requests_license_from_the_books_store() {
    let transport = MockTransport::fixed(200, data("license-aaxc.json"));
    let _ = api(transport.clone())
        .license(
            "B000000001",
            Marketplace::UK,
            &signing_session(Marketplace::US),
        )
        .await;
    let request = transport.requests.current()[0].clone();
    assert_eq!(
        request.url.as_str(),
        "https://api.audible.co.uk/1.0/content/B000000001/licenserequest"
    );
    assert!(request.header("x-adp-signature").is_some());
}

#[test]
fn parses_aaxc_license() {
    let license = parse_fixture("license-aaxc.json", "B000000001").unwrap();
    assert_eq!(
        license.decryption,
        Decryption::Aaxc {
            key: hex("0123456789abcdef0123456789abcdef"),
            iv: hex("fedcba9876543210fedcba9876543210")
        }
    );
    assert_eq!(
        license.download_url.as_str(),
        "https://cds.audible.example/download/B000000001.aaxc?token=synthetic"
    );
    assert_eq!(license.runtime_milliseconds, Some(185_000));
    assert_eq!(license.codec.as_deref(), Some("mp4a.40.2"));
    let chapter = |title: &str, start, length| Chapter {
        title: title.into(),
        start_milliseconds: start,
        length_milliseconds: length,
    };
    assert_eq!(
        license.chapters,
        [
            chapter("Opening Credits", 0, 15_000),
            chapter("Part One", 15_000, 5_000),
            chapter("Chapter 1", 20_000, 60_000),
            chapter("Chapter 2", 80_000, 60_000),
            chapter("Chapter 3", 140_000, 45_000),
        ]
    );
}

#[test]
fn parses_legacy_aax_activation_bytes() {
    assert_eq!(
        parse_fixture("license-aax.json", "B000000002")
            .unwrap()
            .decryption,
        Decryption::Aax {
            activation_bytes: vec![0x1c, 0xeb, 0x00, 0xda]
        }
    );
}

#[test]
fn missing_key_is_reported() {
    assert_eq!(
        parse_fixture("license-missing-key.json", "B000000003"),
        Err(ExporterError::MissingDecryptionKey {
            asin: "B000000003".into()
        })
    );
}

#[test]
fn denied_license_names_the_reason() {
    assert_eq!(
        parse_fixture("license-denied.json", "B000000005"),
        denied("B000000005", "Title is not owned by customer")
    );
    assert_eq!(
        parse(br#"{"message":"Forbidden title"}"#.to_vec(), 403, "B1"),
        denied("B1", "Forbidden title")
    );
}

#[test]
fn unsupported_drm_is_rejected_for_the_title() {
    assert_eq!(
        parse_fixture("license-widevine.json", "B000000004"),
        Err(ExporterError::UnsupportedDrm {
            asin: "B000000004".into(),
            drm_type: "Widevine".into()
        })
    );
}

#[test]
fn malformed_licenses_are_rejected() {
    assert!(parse(b"{}".to_vec(), 200, "B1").is_err());
    assert_eq!(
        parse(vec![], 401, "B1"),
        Err(ExporterError::AuthenticationRequired)
    );
    let mut root = json("license-aaxc.json");
    root["content_license"]["content_metadata"]["content_url"] =
        object!({"offline_url": "http://insecure.example/file"});
    assert_eq!(
        parse(root.to_string().into_bytes(), 200, "B000000001"),
        Err(ExporterError::MalformedResponse(
            "License for B000000001 has no download URL.".into()
        ))
    );
}

#[test]
fn preserves_validation_messages_and_codes() {
    let license = object!({
        "status_code": "Denied",
        "message": "License not granted.",
        "license_denial_reasons": [
            {"validationType": "Ownership", "message": "User has Purchase rights."},
            {"validationType": "Client", "rejectionReason": "RequesterEligibility",
             "message": "Client does not have access."},
            {"validationType": "Membership", "rejectionReason": "CustomerThrottled"},
            {"message": "Try again later."},
            {"message": " \n ", "validationType": " "},
            {},
        ],
    });
    let expected = "License not granted.\nOwnership: User has Purchase rights.\nClient / RequesterEligibility: Client does not have access.\nMembership / CustomerThrottled\nTry again later.";
    for status in [200, 403, 404] {
        for nested in [false, true] {
            if !nested && status == 200 {
                continue;
            }
            let body = if nested {
                object!({"content_license": license.clone()})
            } else {
                license.clone()
            };
            assert_eq!(
                parse(body.to_string().into_bytes(), status, "B1"),
                denied("B1", expected),
                "{status} {nested}"
            );
        }
    }
}

#[test]
fn empty_or_malformed_reasons_keep_the_summary() {
    for reasons in ["null", "[]", "{}", r#"[null, {}, 42, {"message":" "}]"#] {
        let body = format!(
            r#"{{"content_license":{{"status_code":"Denied","message":"Not granted.","license_denial_reasons":{reasons}}}}}"#
        );
        assert_eq!(
            parse(body.into_bytes(), 200, "B1"),
            denied("B1", "Not granted.")
        );
    }
}

#[test]
fn reasons_survive_without_summary_message() {
    let body = br#"{"content_license":{"status_code":"Denied","license_denial_reasons":[{"message":"Try later."}]}}"#;
    assert_eq!(
        parse(body.to_vec(), 200, "B1"),
        denied("B1", "Denied\nTry later.")
    );
}
