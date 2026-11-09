//! Ported from `AuthenticationAdapterTests.swift` and `DeviceProfileTests.swift`.

mod support;

use std::collections::HashSet;
use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use chrono::{DateTime, TimeDelta};
use loudible_adapters::audible_http_api::{device_registration, token_refresh};
use loudible_adapters::*;
use loudible_core::*;
use loudible_test_support::*;
use serde_json::{Value, json};
use support::*;
use url::Url;

fn factory(seed: u8) -> AudibleLoginFactory {
    AudibleLoginFactory::new(SequentialRandom::new(seed), Arc::new(TestClock::default()))
}

fn make_login(marketplace: Marketplace) -> PendingLogin {
    factory(0).make_login(marketplace)
}

#[test]
fn pkce_matches_rfc7636_example() {
    let octets = [
        116, 24, 223, 180, 151, 153, 224, 37, 79, 250, 96, 125, 216, 173, 187, 186, 22, 212, 37,
        77, 105, 214, 191, 240, 91, 88, 5, 88, 83, 132, 141, 121,
    ];
    let verifier = pkce::code_verifier(&octets);
    assert_eq!(verifier, "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
    assert_eq!(
        pkce::code_challenge(&verifier),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn keeps_verifier_and_device_together() {
    let login = make_login(Marketplace::US);
    assert_eq!(
        login.device.serial,
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMN"
    );
    assert_eq!(login.device.device_type, "A2CZJZGLK2JJVM");
    assert_eq!(
        login.code_verifier,
        pkce::code_verifier(&(40..72).collect::<Vec<u8>>())
    );
    assert_eq!(login.created_at, reference_date());
    let query = query_of(&login.login_url);
    assert_eq!(
        query["openid.oa2.client_id"],
        format!("device:{}", login.device.client_id())
    );
    assert_eq!(
        query["openid.oa2.code_challenge"],
        pkce::code_challenge(&login.code_verifier)
    );
    assert_eq!(query["openid.oa2.code_challenge_method"], "S256");
}

#[test]
fn builds_marketplace_specific_login_urls() {
    for marketplace in Marketplace::ALL {
        let login = make_login(marketplace);
        let domain = marketplace.domain;
        assert_eq!(login.login_url.scheme(), "https");
        assert_eq!(
            login.login_url.host_str(),
            Some(format!("www.amazon.{domain}").as_str())
        );
        assert_eq!(login.login_url.path(), "/ap/signin");
        let query = query_of(&login.login_url);
        assert_eq!(
            query["openid.assoc_handle"],
            format!("amzn_audible_ios_{}", marketplace.id)
        );
        assert_eq!(query["marketPlaceId"], marketplace.marketplace_id);
        assert_eq!(
            query["openid.return_to"],
            format!("https://www.amazon.{domain}/ap/maplanding")
        );
        assert_eq!(query["openid.oa2.response_type"], "code");
        assert_eq!(query["openid.oa2.scope"], "device_auth_access");
        assert_eq!(query["pageId"], "amzn_audible_ios");
        assert!(
            login
                .cookies
                .iter()
                .all(|cookie| cookie.domain == format!(".amazon.{domain}"))
        );
    }
}

#[test]
fn sets_initial_cookies_and_user_agent() {
    let login = make_login(Marketplace::US);
    let value = |name: &str| {
        login
            .cookies
            .iter()
            .find(|cookie| cookie.name == name)
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(login.cookies.len(), 3);
    assert_eq!(value("amzn-app-id"), "MAPiOSLib/6.0/ToHideRetailLink");
    assert!(!value("frc").ends_with('='));
    assert_eq!(STANDARD_NO_PAD.decode(value("frc")).unwrap().len(), 313);
    let metadata: Value =
        serde_json::from_slice(&STANDARD_NO_PAD.decode(value("map-md")).unwrap()).unwrap();
    assert_eq!(
        metadata["app_identifier"]["bundle_id"],
        "com.audible.iphone"
    );
    assert_eq!(metadata["app_identifier"]["app_version"], "4.60");
    assert_eq!(
        metadata["device_registration_data"]["software_version"],
        "46000856"
    );
    assert_eq!(
        login.user_agent.as_deref(),
        Some(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148"
        )
    );
}

#[test]
fn each_login_gets_a_fresh_identity() {
    let factory = factory(0);
    let (first, second) = (
        factory.make_login(Marketplace::US),
        factory.make_login(Marketplace::US),
    );
    assert_ne!(first.id, second.id);
    assert_ne!(first.device, second.device);
    assert_ne!(first.code_verifier, second.code_verifier);
}

#[test]
fn serial_rejects_biased_byte_values() {
    assert_eq!(
        factory(248).make_login(Marketplace::US).device.serial,
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMN"
    );
}

#[test]
fn system_random_serials_have_the_expected_format() {
    let factory =
        AudibleLoginFactory::new(Arc::new(SystemRandomSource), Arc::new(TestClock::default()));
    let serials: Vec<String> = (0..32)
        .map(|_| factory.make_login(Marketplace::US).device.serial)
        .collect();
    assert!(
        serials
            .iter()
            .all(|serial| serial.len() == 40 && serial.bytes().all(|b| b.is_ascii_alphanumeric()))
    );
    assert_eq!(serials.iter().collect::<HashSet<_>>().len(), serials.len());
}

#[test]
fn signer_matches_openssl_signatures() {
    let fixture = json("signing-vectors.json");
    for key_file in ["synthetic-device-key.pem", "synthetic-device-key-pkcs8.pem"] {
        let signer =
            RequestSigner::new(fixture["adp_token"].as_str().unwrap(), &text(key_file)).unwrap();
        for vector in fixture["vectors"].as_array().unwrap() {
            let [method, path, timestamp, body, expected] =
                ["method", "path", "timestamp", "body", "signature"]
                    .map(|k| vector[k].as_str().unwrap());
            assert_eq!(
                signer
                    .signature(method, path, timestamp, body.as_bytes())
                    .unwrap(),
                expected
            );
        }
    }
}

#[test]
fn signs_request_headers() {
    let fixture = json("signing-vectors.json");
    let token = fixture["adp_token"].as_str().unwrap();
    let signer = RequestSigner::new(token, &text("synthetic-device-key.pem")).unwrap();
    let url = Url::parse("https://api.audible.com/1.0/library?num_results=50&page=1").unwrap();
    let mut request = HttpRequest::get(url);
    signer
        .sign(
            &mut request,
            DateTime::from_timestamp(1_800_000_000, 0).unwrap(),
        )
        .unwrap();
    assert_eq!(request.header("x-adp-token"), Some(token));
    assert_eq!(request.header("x-adp-alg"), Some("SHA256withRSA:1.0"));
    assert_eq!(
        request.header("x-adp-signature").unwrap(),
        format!(
            "{}:2027-01-15T08:00:00.000000Z",
            fixture["vectors"][0]["signature"].as_str().unwrap()
        )
    );
}

#[test]
fn formats_microsecond_timestamps() {
    assert_eq!(
        RequestSigner::timestamp(DateTime::from_timestamp_millis(1_800_000_000_250).unwrap()),
        "2027-01-15T08:00:00.250000Z"
    );
}

#[test]
fn path_includes_encoded_query() {
    let url =
        Url::parse("https://api.audible.com/1.0/library?response_groups=a%2Cb&page=2").unwrap();
    assert_eq!(
        RequestSigner::path_and_query(&url),
        "/1.0/library?response_groups=a%2Cb&page=2"
    );
    assert_eq!(
        RequestSigner::path_and_query(&Url::parse("https://x.test/a/b").unwrap()),
        "/a/b"
    );
}

#[test]
fn rejects_invalid_keys() {
    let error = |pem| RequestSigner::new("t", pem).err().unwrap();
    assert_eq!(error("not a key"), SigningError::InvalidPem);
    assert_eq!(
        error("-----BEGIN EC PRIVATE KEY-----\nAAAA\n-----END EC PRIVATE KEY-----"),
        SigningError::UnsupportedKeyFormat
    );
    assert!(matches!(
        error("-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----"),
        SigningError::KeyCreationFailed(_)
    ));
}

fn uk_login() -> PendingLogin {
    pending_login(Marketplace::UK, login_id(0xAA), reference_date())
}

fn registered(
    status: u16,
    body: impl Into<Vec<u8>>,
    login: &PendingLogin,
) -> Result<AudibleSession, ExporterError> {
    device_registration::session(&response(status, body), login, reference_date())
}

#[test]
fn builds_registration_request() {
    let login = uk_login();
    let request = device_registration::request(&login, "code-123");
    assert_eq!(
        request.url.as_str(),
        "https://api.amazon.co.uk/auth/register"
    );
    assert_eq!(request.method, "POST");
    assert_eq!(request.header("Content-Type"), Some("application/json"));
    assert_eq!(
        request.header("x-amzn-identity-auth-domain"),
        Some("api.amazon.co.uk")
    );
    assert_eq!(
        request.header("User-Agent"),
        Some("AmazonWebView/Audible/4.60/iOS/26.2/iPhone")
    );
    let body = json_body(&request);
    let auth = &body["auth_data"];
    assert_eq!(auth["authorization_code"], "code-123");
    assert_eq!(auth["code_verifier"], login.code_verifier.as_str());
    assert_eq!(auth["client_id"], login.device.client_id().as_str());
    assert_eq!(auth["code_algorithm"], "SHA-256");
    let registration = &body["registration_data"];
    assert_eq!(registration["device_serial"], login.device.serial.as_str());
    assert_eq!(registration["device_type"], "A2CZJZGLK2JJVM");
    assert_eq!(registration["app_version"], "4.60");
    assert_eq!(registration["os_version"], "26.2");
    assert_eq!(registration["software_version"], "46000856");
    assert_eq!(
        body["requested_token_type"],
        json!([
            "bearer",
            "mac_dms",
            "website_cookies",
            "store_authentication_cookie"
        ])
    );
    assert_eq!(body["cookies"]["domain"], ".amazon.co.uk");
}

#[test]
fn parses_successful_registration() {
    let login = uk_login();
    let session = registered(200, data("register-success.json"), &login).unwrap();
    assert_eq!(session.marketplace, Marketplace::UK);
    assert_eq!(session.device, login.device);
    assert_eq!(session.customer_id, "amzn1.account.SYNTHETIC");
    assert_eq!(session.customer_name.as_deref(), Some("Test"));
    assert_eq!(session.access_token, "Atna|synthetic-access");
    assert_eq!(session.refresh_token, "Atnr|synthetic-refresh");
    assert_eq!(
        session.access_token_expires_at,
        reference_date() + TimeDelta::seconds(3600)
    );
    assert!(session.adp_token.starts_with("{enc:synthetic-adp}"));
    assert!(
        session
            .device_private_key
            .starts_with("-----BEGIN RSA PRIVATE KEY-----")
    );
}

#[test]
fn reports_registration_server_errors() {
    assert_eq!(
        registered(400, data("register-error.json"), &uk_login()),
        Err(ExporterError::Http {
            status: 400,
            message: Some("The authorization code is invalid or expired.".into())
        })
    );
}

#[test]
fn rejects_registrations_without_tokens() {
    let mut root = json("register-success.json");
    root["response"]["success"]["tokens"]
        .as_object_mut()
        .unwrap()
        .remove("mac_dms");
    let login = uk_login();
    assert!(registered(200, root.to_string(), &login).is_err());
    assert!(registered(200, "<html>", &login).is_err());
}

#[test]
fn rejects_registration_of_a_different_device() {
    let other = PendingLogin {
        device: DeviceIdentity::new("FFFF"),
        ..uk_login()
    };
    assert!(registered(200, data("register-success.json"), &other).is_err());
}

#[tokio::test]
async fn registers_through_transport() {
    let transport = MockTransport::fixed(200, data("register-success.json"));
    let session = api(transport.clone())
        .register(&uk_login(), "abc")
        .await
        .unwrap();
    assert_eq!(session.access_token, "Atna|synthetic-access");
    assert_eq!(transport.requests.current().len(), 1);
}

#[tokio::test]
async fn passes_on_transport_failures() {
    let transport = MockTransport::new(|_| async { Err(ExporterError::Network("offline".into())) });
    assert_eq!(
        api(transport).register(&uk_login(), "abc").await,
        Err(ExporterError::Network("offline".into()))
    );
}

#[test]
fn builds_form_encoded_refresh_request() {
    let session = session_in(Marketplace::GERMANY);
    let request = token_refresh::request(&session);
    assert_eq!(request.url.as_str(), "https://api.amazon.de/auth/token");
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.header("Content-Type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        request.header("User-Agent"),
        Some("AmazonWebView/Audible/4.60/iOS/26.2/iPhone")
    );
    let body = String::from_utf8(request.body.clone().unwrap()).unwrap();
    assert!(body.contains("Atnr%7Csynthetic-refresh"));
    let form = query_of(&Url::parse(&format!("https://x.test/?{body}")).unwrap());
    assert_eq!(form.len(), 5);
    assert_eq!(form["app_name"], "Audible");
    assert_eq!(form["app_version"], "4.60");
    assert_eq!(form["source_token"], "Atnr|synthetic-refresh");
    assert_eq!(form["requested_token_type"], "access_token");
    assert_eq!(form["source_token_type"], "refresh_token");
}

#[test]
fn applies_refreshed_token() {
    let session = session_in(Marketplace::GERMANY);
    let now = reference_date() + TimeDelta::seconds(7200);
    let refreshed =
        token_refresh::session(&response(200, data("refresh-success.json")), &session, now)
            .unwrap();
    assert_eq!(refreshed.access_token, "Atna|synthetic-refreshed");
    assert_eq!(
        refreshed.access_token_expires_at,
        now + TimeDelta::seconds(3600)
    );
    assert_eq!(refreshed.refresh_token, session.refresh_token);
    assert_eq!(refreshed.device, session.device);
}

#[test]
fn invalid_grant_requires_reauthentication() {
    let session = session_in(Marketplace::GERMANY);
    let refresh = |status, body: Vec<u8>| {
        token_refresh::session(&response(status, body), &session, reference_date())
    };
    assert_eq!(
        refresh(400, data("refresh-invalid-grant.json")),
        Err(ExporterError::AuthenticationRequired)
    );
    assert_eq!(
        refresh(401, vec![]),
        Err(ExporterError::AuthenticationRequired)
    );
    assert_eq!(
        refresh(503, vec![]),
        Err(ExporterError::Http {
            status: 503,
            message: None
        })
    );
    assert!(refresh(200, b"{}".to_vec()).is_err());
}

#[tokio::test]
async fn fresh_login_preserves_its_serial_through_registration() {
    let pending = factory(0).make_login(Marketplace::UK);
    let mut root = json("register-success.json");
    root["response"]["success"]["extensions"]["device_info"]["device_serial_number"] =
        pending.device.serial.clone().into();
    let transport = MockTransport::fixed(200, root.to_string());

    let session = api(transport.clone())
        .register(&pending, "synthetic-code")
        .await
        .unwrap();

    let client_id =
        hex_encoding::encode(format!("{}#A2CZJZGLK2JJVM", pending.device.serial).as_bytes());
    assert_eq!(
        query_of(&pending.login_url)["openid.oa2.client_id"],
        format!("device:{client_id}")
    );
    let body = json_body(&transport.last());
    assert_eq!(
        body["registration_data"]["device_serial"],
        pending.device.serial.as_str()
    );
    assert_eq!(body["auth_data"]["client_id"], client_id.as_str());
    assert_eq!(session.device, pending.device);
    assert_eq!(session.device.serial.len(), 40);
}
