use chrono::TimeDelta;
use serde::{Deserialize, Serialize};

use crate::{Date, Marketplace, hex_encoding};

/// The registered virtual device. Generated once per sign-in attempt and kept with the
/// pending PKCE verifier, because registration must present the same serial the login used.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub serial: String,
    pub device_type: String,
}

impl DeviceIdentity {
    /// Audible for iPhone device type.
    pub const IPHONE_DEVICE_TYPE: &str = "A2CZJZGLK2JJVM";

    pub fn new(serial: impl Into<String>) -> Self {
        Self {
            serial: serial.into(),
            device_type: Self::IPHONE_DEVICE_TYPE.to_string(),
        }
    }

    /// OAuth client id: hex of `"<serial>#<deviceType>"`.
    pub fn client_id(&self) -> String {
        hex_encoding::encode(format!("{}#{}", self.serial, self.device_type).as_bytes())
    }
}

/// A registered, signed-in Audible device and its tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudibleSession {
    pub marketplace: Marketplace,
    pub device: DeviceIdentity,
    /// Amazon customer id (`customer_info.user_id`). Part of the voucher key derivation.
    pub customer_id: String,
    pub customer_name: Option<String>,
    pub access_token: String,
    pub access_token_expires_at: Date,
    pub refresh_token: String,
    /// MAC/DMS token used to sign Audible API requests.
    pub adp_token: String,
    /// PEM-encoded RSA private key issued at registration.
    pub device_private_key: String,
}

impl AudibleSession {
    /// Margin in seconds before expiry at which the access token is refreshed.
    pub const REFRESH_LEEWAY_SECONDS: i64 = 300;

    pub fn needs_refresh(&self, now: Date) -> bool {
        now + TimeDelta::seconds(Self::REFRESH_LEEWAY_SECONDS) >= self.access_token_expires_at
    }
}

/// A credential-storage failure that does not necessarily invalidate the in-memory session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialStorageFailure {
    pub operation: CredentialOperation,
    pub technical_error: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialOperation {
    Load,
    Save,
    Delete,
}
