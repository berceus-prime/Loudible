use chrono::TimeDelta;
use loudible_core::hex_encoding::{decode, encode};
use loudible_core::*;
use loudible_test_support::*;

#[test]
fn supports_the_ten_planned_marketplaces_with_us_first() {
    let ids: Vec<&str> = Marketplace::ALL.iter().map(|m| m.id).collect();
    assert_eq!(
        ids,
        ["us", "uk", "de", "fr", "it", "es", "ca", "au", "in", "jp"]
    );
    let unique: std::collections::HashSet<_> =
        Marketplace::ALL.iter().map(|m| m.marketplace_id).collect();
    assert_eq!(unique.len(), 10);
}

#[test]
fn derives_hosts() {
    for (marketplace, amazon, api, audible) in [
        (
            Marketplace::US,
            "www.amazon.com",
            "api.amazon.com",
            "api.audible.com",
        ),
        (
            Marketplace::UK,
            "www.amazon.co.uk",
            "api.amazon.co.uk",
            "api.audible.co.uk",
        ),
        (
            Marketplace::AUSTRALIA,
            "www.amazon.com.au",
            "api.amazon.com.au",
            "api.audible.com.au",
        ),
        (
            Marketplace::JAPAN,
            "www.amazon.co.jp",
            "api.amazon.co.jp",
            "api.audible.co.jp",
        ),
    ] {
        assert_eq!(marketplace.amazon_host(), amazon);
        assert_eq!(marketplace.amazon_api_host(), api);
        assert_eq!(marketplace.audible_api_host(), audible);
    }
}

#[test]
fn encodes_as_identifier_and_rejects_unknown() {
    let json = serde_json::to_string(&Marketplace::GERMANY).unwrap();
    assert_eq!(json, "\"de\"");
    assert_eq!(
        serde_json::from_str::<Marketplace>(&json).unwrap(),
        Marketplace::GERMANY
    );
    assert!(serde_json::from_str::<Marketplace>("\"xx\"").is_err());
}

#[test]
fn client_id_is_hex_of_serial_and_device_type() {
    let device = DeviceIdentity::new("ABC");
    assert_eq!(device.client_id(), encode(b"ABC#A2CZJZGLK2JJVM"));
    assert_eq!(device.client_id(), "414243234132435a4a5a474c4b324a4a564d");
}

#[test]
fn refresh_is_needed_within_leeway_of_expiry() {
    let session = AudibleSession {
        access_token_expires_at: seconds_after_reference(600),
        ..session()
    };
    assert!(!session.needs_refresh(reference_date()));
    assert!(session.needs_refresh(seconds_after_reference(301)));
    assert!(session.needs_refresh(seconds_after_reference(601)));
}

#[test]
fn audiobook_round_trips_its_marketplace() {
    let book = book_in("B1", "Book B1", Marketplace::JAPAN);
    let json = serde_json::to_string(&book).unwrap();
    assert_eq!(serde_json::from_str::<Audiobook>(&json).unwrap(), book);
}

#[test]
fn session_round_trips() {
    for serial in [
        "0123456789ABCDEF0123456789ABCDEF",
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMN",
    ] {
        let session = AudibleSession {
            device: DeviceIdentity::new(serial),
            ..session_in(Marketplace::FRANCE)
        };
        let json = serde_json::to_string(&session).unwrap();
        assert_eq!(
            serde_json::from_str::<AudibleSession>(&json).unwrap(),
            session
        );
    }
}

#[test]
fn pending_login_expires_after_lifetime() {
    let login = pending_login(Marketplace::US, login_id(1), reference_date());
    let lifetime = TimeDelta::seconds(PendingLogin::LIFETIME_SECONDS);
    assert!(!login.is_expired(reference_date() + lifetime));
    assert!(login.is_expired(reference_date() + lifetime + TimeDelta::seconds(1)));
}

#[test]
fn hex_round_trips() {
    let data = [0x00, 0x7f, 0x80, 0xff];
    assert_eq!(encode(&data), "007f80ff");
    assert_eq!(decode("007F80ff").unwrap(), data);
}

#[test]
fn hex_rejects_invalid() {
    for text in ["0", "zz", "0g", "abc"] {
        assert_eq!(decode(text), None);
    }
}

fn decryption(
    drm: Option<&str>,
    key: Option<Vec<u8>>,
    iv: Option<Vec<u8>>,
) -> Result<Decryption, ExporterError> {
    ContentLicense::decryption("B1", drm, key, iv)
}

#[test]
fn four_byte_key_without_iv_is_aax() {
    assert_eq!(
        decryption(Some("Adrm"), Some(vec![1, 2, 3, 4]), None).unwrap(),
        Decryption::Aax {
            activation_bytes: vec![1, 2, 3, 4]
        }
    );
}

#[test]
fn sixteen_byte_key_and_iv_is_aaxc() {
    assert_eq!(
        decryption(Some("ADRM"), Some(vec![1; 16]), Some(vec![2; 16])).unwrap(),
        Decryption::Aaxc {
            key: vec![1; 16],
            iv: vec![2; 16]
        }
    );
}

#[test]
fn other_drm_is_unsupported() {
    assert_eq!(
        decryption(Some("Widevine"), Some(vec![0; 16]), Some(vec![0; 16])),
        Err(ExporterError::UnsupportedDrm {
            asin: "B1".into(),
            drm_type: "Widevine".into()
        })
    );
    assert_eq!(
        decryption(None, None, None),
        Err(ExporterError::UnsupportedDrm {
            asin: "B1".into(),
            drm_type: "none".into()
        })
    );
}

#[test]
fn missing_key_is_reported() {
    assert_eq!(
        decryption(Some("Adrm"), None, None),
        Err(ExporterError::MissingDecryptionKey { asin: "B1".into() })
    );
}

#[test]
fn unexpected_key_shape_is_unsupported() {
    assert!(decryption(Some("Adrm"), Some(vec![0; 16]), None).is_err());
}
