use chrono::TimeDelta;
use percent_encoding::percent_decode_str;
use url::Url;
use uuid::Uuid;

use crate::{Date, DeviceIdentity, Marketplace};

/// A cookie the sign-in browser must carry before loading the login URL.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LoginCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
}

/// An in-progress sign-in. The PKCE verifier and device identity travel together so the
/// registration request always matches the login that produced the authorization code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingLogin {
    pub id: Uuid,
    pub marketplace: Marketplace,
    pub device: DeviceIdentity,
    pub code_verifier: String,
    pub login_url: Url,
    pub cookies: Vec<LoginCookie>,
    /// User agent the sign-in browser should present, so Amazon serves the mobile app flow.
    pub user_agent: Option<String>,
    pub created_at: Date,
}

impl PendingLogin {
    /// Sign-in attempts older than this many seconds are rejected as stale.
    pub const LIFETIME_SECONDS: i64 = 30 * 60;

    pub fn is_expired(&self, now: Date) -> bool {
        now - self.created_at > TimeDelta::seconds(Self::LIFETIME_SECONDS)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginCallbackError {
    MalformedUrl,
    UnexpectedHost(Option<String>),
    UnexpectedPath(String),
    MissingAuthorizationCode,
    /// No sign-in is waiting: it was cancelled, already completed, or superseded.
    NoPendingLogin,
    Expired,
}

/// Recognises and validates the Amazon sign-in redirect (`https://www.amazon.<tld>/ap/maplanding`).
pub struct LoginCallback;

impl LoginCallback {
    pub const PATH: &str = "/ap/maplanding";
    pub const AUTHORIZATION_CODE_PARAMETER: &str = "openid.oa2.authorization_code";

    /// True when the URL is the sign-in redirect for this login, whether or not it carries a code.
    /// The browser uses this to stop navigation and hand the URL over.
    pub fn is_callback(url: &str, login: &PendingLogin) -> bool {
        Url::parse(url).is_ok_and(|url| {
            url.host_str() == Some(&login.marketplace.amazon_host()) && url.path() == Self::PATH
        })
    }

    /// Extracts the authorization code after checking the scheme, host, and path.
    pub fn authorization_code(
        url: &str,
        login: &PendingLogin,
    ) -> Result<String, LoginCallbackError> {
        let url = Url::parse(url).map_err(|_| LoginCallbackError::MalformedUrl)?;
        if url.scheme() != "https" {
            return Err(LoginCallbackError::MalformedUrl);
        }
        if url.host_str() != Some(&login.marketplace.amazon_host()) {
            return Err(LoginCallbackError::UnexpectedHost(
                url.host_str().map(str::to_string),
            ));
        }
        if url.path() != Self::PATH {
            return Err(LoginCallbackError::UnexpectedPath(url.path().to_string()));
        }
        let codes: Vec<String> = url
            .query()
            .unwrap_or("")
            .split('&')
            .filter_map(|pair| {
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                (decoded(name) == Self::AUTHORIZATION_CODE_PARAMETER).then(|| decoded(value))
            })
            .collect();
        match codes.as_slice() {
            [code] if !code.is_empty() => Ok(code.clone()),
            _ => Err(LoginCallbackError::MissingAuthorizationCode),
        }
    }
}

/// Percent-decoding only: `url`'s form decoding would turn a `+` in a code into a space.
fn decoded(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}
