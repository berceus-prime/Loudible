use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use loudible_core::{
    DeviceIdentity, LoginCallback, LoginCookie, Marketplace, PendingLogin, WallClock,
};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use url::Url;

use crate::RandomSource;

/// Constants identifying the emulated Audible iOS app.
pub mod app {
    pub const VERSION: &str = "4.60";
    pub const SOFTWARE_VERSION: &str = "46000856";
    pub const OS_VERSION: &str = "26.2";
    pub const BUNDLE_ID: &str = "com.audible.iphone";
    pub const APP_ID: &str = "MAPiOSLib/6.0/ToHideRetailLink";
    // WebKit freezes the web-view OS token at 18_7 even when registering as iOS 26.x.
    pub const WEB_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";
    pub const API_USER_AGENT: &str = "AmazonWebView/Audible/4.60/iOS/26.2/iPhone";
}

/// What a query item's name or value may leave unescaped. `+` is escaped so it is not read as a
/// space, and `'` because `url` would escape it anyway.
const QUERY_ITEM: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'!')
    .remove(b'$')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b',')
    .remove(b';')
    .remove(b':')
    .remove(b'@')
    .remove(b'/')
    .remove(b'?');

/// An `https` URL whose query is encoded as Audible's signed requests expect.
pub(crate) fn https_url(host: &str, path: &str, items: &[(&str, &str)]) -> Url {
    let query = items
        .iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                utf8_percent_encode(name, QUERY_ITEM),
                utf8_percent_encode(value, QUERY_ITEM)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    Url::parse(&format!("https://{host}{path}?{query}")).expect("a valid host and path")
}

/// Builds the Audible for iPhone sign-in: device serial, PKCE pair, login URL, and the
/// cookies Amazon expects the app's web view to carry.
#[derive(Clone)]
pub struct AudibleLoginFactory {
    random: Arc<dyn RandomSource>,
    clock: Arc<dyn WallClock>,
}

impl AudibleLoginFactory {
    pub fn new(random: Arc<dyn RandomSource>, clock: Arc<dyn WallClock>) -> Self {
        Self { random, clock }
    }

    pub fn make_login(&self, marketplace: Marketplace) -> PendingLogin {
        let device = DeviceIdentity::new(self.make_device_serial());
        let verifier = pkce::code_verifier(&self.random.bytes(32));
        PendingLogin {
            id: self.random.uuid(),
            marketplace,
            login_url: Self::login_url(marketplace, &device, &pkce::code_challenge(&verifier)),
            device,
            code_verifier: verifier,
            cookies: self.initial_cookies(marketplace),
            user_agent: Some(app::WEB_USER_AGENT.to_string()),
            created_at: self.clock.now(),
        }
    }

    /// Fresh registrations use the 40-character iPhone serial format from mkb79/Audible#910.
    fn make_device_serial(&self) -> String {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut serial = String::new();
        while serial.len() < 40 {
            // Reject the last eight byte values so modulo 62 does not bias character selection.
            for byte in self.random.bytes(40 - serial.len()) {
                if byte < 248 {
                    serial.push(ALPHABET[usize::from(byte) % ALPHABET.len()] as char);
                }
            }
        }
        serial
    }

    pub fn login_url(
        marketplace: Marketplace,
        device: &DeviceIdentity,
        code_challenge: &str,
    ) -> Url {
        const IDENTIFIER_SELECT: &str = "http://specs.openid.net/auth/2.0/identifier_select";
        let host = marketplace.amazon_host();
        let return_to = format!("https://{host}{}", LoginCallback::PATH);
        let handle = format!("amzn_audible_ios_{}", marketplace.id);
        let client_id = format!("device:{}", device.client_id());
        https_url(
            &host,
            "/ap/signin",
            &[
                ("openid.oa2.response_type", "code"),
                ("openid.oa2.code_challenge_method", "S256"),
                ("openid.oa2.code_challenge", code_challenge),
                ("openid.return_to", &return_to),
                ("openid.assoc_handle", &handle),
                ("openid.identity", IDENTIFIER_SELECT),
                ("pageId", "amzn_audible_ios"),
                ("accountStatusPolicy", "P1"),
                ("openid.claimed_id", IDENTIFIER_SELECT),
                ("openid.mode", "checkid_setup"),
                ("openid.ns.oa2", "http://www.amazon.com/ap/ext/oauth/2"),
                ("openid.oa2.client_id", &client_id),
                (
                    "openid.ns.pape",
                    "http://specs.openid.net/extensions/pape/1.0",
                ),
                ("marketPlaceId", marketplace.marketplace_id),
                ("openid.oa2.scope", "device_auth_access"),
                ("forceMobileLayout", "true"),
                ("openid.ns", "http://specs.openid.net/auth/2.0"),
                ("openid.pape.max_auth_age", "0"),
            ],
        )
    }

    fn initial_cookies(&self, marketplace: Marketplace) -> Vec<LoginCookie> {
        let domain = format!(".amazon.{}", marketplace.domain);
        let metadata = format!(
            r#"{{"device_user_dictionary":[],"device_registration_data":{{"software_version":"{}"}},"app_identifier":{{"app_version":"{}","bundle_id":"{}"}}}}"#,
            app::SOFTWARE_VERSION,
            app::VERSION,
            app::BUNDLE_ID
        );
        [
            ("frc", STANDARD_NO_PAD.encode(self.random.bytes(313))),
            ("map-md", STANDARD_NO_PAD.encode(metadata)),
            ("amzn-app-id", app::APP_ID.to_string()),
        ]
        .map(|(name, value)| LoginCookie {
            name: name.to_string(),
            value,
            domain: domain.clone(),
        })
        .to_vec()
    }
}

/// RFC 7636 proof key helpers.
pub mod pkce {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use sha2::{Digest, Sha256};

    pub fn code_verifier(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    pub fn code_challenge(verifier: &str) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
    }
}
