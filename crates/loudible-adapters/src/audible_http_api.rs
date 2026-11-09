use std::sync::Arc;

use async_trait::async_trait;
use chrono::TimeDelta;
use loudible_core::{
    AudibleApi, AudibleSession, ContentLicense, Date, DeviceIdentity, ExporterError, LibraryPage,
    Marketplace, PendingLogin, WallClock,
};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use url::Url;

use crate::audible_login_factory::app;
use crate::content_metadata::{content_metadata_request, correct_chapters};
use crate::json::{self, Json};
use crate::library_response::{check_status, library_request, parse_library_page};
use crate::license_response::{license_request, parse_license};
use crate::{
    AudibleLoginFactory, HttpRequest, HttpResponse, HttpTransport, RandomSource, RequestSigner,
};

/// `AudibleApi` over HTTPS, emulating the Audible for iPhone app.
pub struct AudibleHttpApi {
    transport: Arc<dyn HttpTransport>,
    clock: Arc<dyn WallClock>,
    login_factory: AudibleLoginFactory,
}

impl AudibleHttpApi {
    pub fn new(
        transport: Arc<dyn HttpTransport>,
        random: Arc<dyn RandomSource>,
        clock: Arc<dyn WallClock>,
    ) -> Self {
        Self {
            transport,
            login_factory: AudibleLoginFactory::new(random, clock.clone()),
            clock,
        }
    }

    /// Signs a request to the Audible API with the session's device key.
    fn signed(
        &self,
        mut request: HttpRequest,
        session: &AudibleSession,
    ) -> Result<HttpRequest, ExporterError> {
        // A key we cannot use is as good as no credentials.
        RequestSigner::new(&session.adp_token, &session.device_private_key)
            .and_then(|signer| signer.sign(&mut request, self.clock.now()))
            .map_err(|_| ExporterError::AuthenticationRequired)?;
        Ok(request)
    }
}

// Signing covers method, path, query, date and body but not the host, so the session's
// device key signs requests to any store.
#[async_trait]
impl AudibleApi for AudibleHttpApi {
    fn make_login(&self, marketplace: Marketplace) -> Result<PendingLogin, ExporterError> {
        Ok(self.login_factory.make_login(marketplace))
    }

    async fn register(
        &self,
        login: &PendingLogin,
        authorization_code: &str,
    ) -> Result<AudibleSession, ExporterError> {
        let request = device_registration::request(login, authorization_code);
        let response = self.transport.send(request).await?;
        device_registration::session(&response, login, self.clock.now())
    }

    async fn refresh(&self, session: &AudibleSession) -> Result<AudibleSession, ExporterError> {
        let response = self.transport.send(token_refresh::request(session)).await?;
        token_refresh::session(&response, session, self.clock.now())
    }

    async fn library_page(
        &self,
        number: usize,
        page_size: usize,
        marketplace: Marketplace,
        session: &AudibleSession,
    ) -> Result<LibraryPage, ExporterError> {
        let request = self.signed(library_request(number, page_size, marketplace), session)?;
        let response = self.transport.send(request).await?;
        parse_library_page(&response, marketplace)
    }

    async fn license(
        &self,
        asin: &str,
        marketplace: Marketplace,
        session: &AudibleSession,
    ) -> Result<ContentLicense, ExporterError> {
        let request = self.signed(license_request(asin, marketplace), session)?;
        let response = self.transport.send(request).await?;
        let parsed = parse_license(&response, asin, session)?;
        let Some(reference) = parsed.reference else {
            return Ok(parsed.license);
        };
        let request = content_metadata_request(asin, marketplace, &reference);
        match self.transport.send(request).await {
            Err(ExporterError::Cancelled) => Err(ExporterError::Cancelled),
            Ok(metadata) if check_status(&metadata).is_ok() => {
                Ok(correct_chapters(parsed.license, &reference, &metadata.body))
            }
            // Chapter correction is optional; a granted license still permits downloading the audio.
            _ => Ok(parsed.license),
        }
    }
}

fn apply_amazon_auth_headers(
    request: &mut HttpRequest,
    marketplace: Marketplace,
    content_type: &str,
) {
    request.set_header("Content-Type", content_type);
    request.set_header("Accept", "application/json");
    request.set_header("Accept-Charset", "utf-8");
    request.set_header("x-amzn-identity-auth-domain", marketplace.amazon_api_host());
    request.set_header("User-Agent", app::API_USER_AGENT);
}

fn auth_url(marketplace: Marketplace, path: &str) -> Url {
    Url::parse(&format!("https://{}{path}", marketplace.amazon_api_host()))
        .expect("a valid host and path")
}

fn expiry(now: Date, seconds: i64) -> Date {
    now + TimeDelta::seconds(seconds)
}

pub mod device_registration {
    use super::*;

    pub fn request(login: &PendingLogin, authorization_code: &str) -> HttpRequest {
        let body = serde_json::json!({
            "requested_token_type": ["bearer", "mac_dms", "website_cookies", "store_authentication_cookie"],
            "cookies": {"website_cookies": [], "domain": format!(".amazon.{}", login.marketplace.domain)},
            "registration_data": {
                "domain": "Device",
                "app_version": app::VERSION,
                "device_serial": login.device.serial,
                "device_type": login.device.device_type,
                "device_name": "%FIRST_NAME%%FIRST_NAME_POSSESSIVE_STRING%%DUPE_STRATEGY_1ST%Audible for iPhone",
                "os_version": app::OS_VERSION,
                "software_version": app::SOFTWARE_VERSION,
                "device_model": "iPhone",
                "app_name": "Audible",
            },
            "auth_data": {
                "client_id": login.device.client_id(),
                "authorization_code": authorization_code,
                "code_verifier": login.code_verifier,
                "code_algorithm": "SHA-256",
                "client_domain": "DeviceLegacy",
            },
            "requested_extensions": ["device_info", "customer_info"],
        });
        let url = auth_url(login.marketplace, "/auth/register");
        let mut request = HttpRequest::post(url, body.to_string().into_bytes());
        apply_amazon_auth_headers(&mut request, login.marketplace, "application/json");
        request
    }

    pub fn session(
        response: &HttpResponse,
        login: &PendingLogin,
        now: Date,
    ) -> Result<AudibleSession, ExporterError> {
        let value = json::parse(&response.body);
        let root = Json::new(&value).get("response");
        if !(200..300).contains(&response.status) {
            let error = root.get("error");
            return Err(ExporterError::Http {
                status: response.status,
                message: error.get("message").string().or(error.get("code").string()),
            });
        }
        let success = root.get("success");
        let bearer = success.get("tokens").get("bearer");
        let mac_dms = success.get("tokens").get("mac_dms");
        let extensions = success.get("extensions");
        let customer = extensions.get("customer_info");
        let (
            Some(access_token),
            Some(refresh_token),
            Some(expires_in),
            Some(adp_token),
            Some(device_private_key),
            Some(customer_id),
        ) = (
            bearer.get("access_token").string(),
            bearer.get("refresh_token").string(),
            bearer.get("expires_in").int(),
            mac_dms.get("adp_token").string(),
            mac_dms.get("device_private_key").string(),
            customer.get("user_id").string(),
        )
        else {
            return Err(ExporterError::MalformedResponse(
                "Device registration response is missing tokens.".into(),
            ));
        };
        let info = extensions.get("device_info");
        let device = DeviceIdentity {
            serial: info
                .get("device_serial_number")
                .string()
                .unwrap_or_else(|| login.device.serial.clone()),
            device_type: info
                .get("device_type")
                .string()
                .unwrap_or_else(|| login.device.device_type.clone()),
        };
        if device != login.device {
            return Err(ExporterError::MalformedResponse(
                "Audible registered a different device than the one that signed in.".into(),
            ));
        }
        Ok(AudibleSession {
            marketplace: login.marketplace,
            device,
            customer_id,
            customer_name: customer
                .get("given_name")
                .string()
                .or(customer.get("name").string()),
            access_token,
            access_token_expires_at: expiry(now, expires_in),
            refresh_token,
            adp_token,
            device_private_key,
        })
    }
}

pub mod token_refresh {
    use super::*;

    pub fn request(session: &AudibleSession) -> HttpRequest {
        let body = form_encode(&[
            ("app_name", "Audible"),
            ("app_version", app::VERSION),
            ("source_token", &session.refresh_token),
            ("requested_token_type", "access_token"),
            ("source_token_type", "refresh_token"),
        ]);
        let url = auth_url(session.marketplace, "/auth/token");
        let mut request = HttpRequest::post(url, body.into_bytes());
        apply_amazon_auth_headers(
            &mut request,
            session.marketplace,
            "application/x-www-form-urlencoded",
        );
        request
    }

    pub fn session(
        response: &HttpResponse,
        session: &AudibleSession,
        now: Date,
    ) -> Result<AudibleSession, ExporterError> {
        let value = json::parse(&response.body);
        let json = Json::new(&value);
        if !(200..300).contains(&response.status) {
            // Amazon reports a revoked or expired refresh token as an OAuth error.
            let code = json.get("error").string().or(json
                .get("response")
                .get("error")
                .get("code")
                .string());
            let rejected_grant = matches!(response.status, 400 | 403)
                && matches!(
                    code.as_deref(),
                    Some("invalid_grant" | "InvalidToken" | "unauthorized_client")
                );
            if response.status == 401 || rejected_grant {
                return Err(ExporterError::AuthenticationRequired);
            }
            return Err(ExporterError::Http {
                status: response.status,
                message: json.get("error_description").string().or(code),
            });
        }
        let (Some(access_token), Some(expires_in)) = (
            json.get("access_token").string(),
            json.get("expires_in").int(),
        ) else {
            return Err(ExporterError::MalformedResponse(
                "Token refresh response is missing the access token.".into(),
            ));
        };
        Ok(AudibleSession {
            access_token,
            access_token_expires_at: expiry(now, expires_in),
            ..session.clone()
        })
    }
}

const FORM_ITEM: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub fn form_encode(items: &[(&str, &str)]) -> String {
    items
        .iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                utf8_percent_encode(name, FORM_ITEM),
                utf8_percent_encode(value, FORM_ITEM)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}
