//! What a screen asks the window to show in the embedded sign-in browser.

use loudible_core::{LoginCallback, PendingLogin};
use skia_safe::Rect;
use url::Url;

/// A cookie the browser must carry before loading the page.
#[derive(Clone, Debug, PartialEq)]
pub struct BrowserCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub is_secure: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BrowserPage {
    pub url: String,
    pub user_agent: String,
    pub cookies: Vec<BrowserCookie>,
    pub callback_host: String,
    pub callback_path: String,
}

impl BrowserPage {
    /// The page a pending sign-in opens, with the device's user agent and cookies.
    pub fn for_login(login: &PendingLogin) -> Self {
        Self {
            url: login.login_url.to_string(),
            user_agent: login.user_agent.clone().unwrap_or_default(),
            cookies: login
                .cookies
                .iter()
                .map(|cookie| BrowserCookie {
                    name: cookie.name.clone(),
                    value: cookie.value.clone(),
                    domain: cookie.domain.clone(),
                    is_secure: true,
                })
                .collect(),
            callback_host: login.marketplace.amazon_host(),
            callback_path: LoginCallback::PATH.to_string(),
        }
    }

    /// True for the sign-in redirect, whether or not it carries a code.
    pub fn is_callback(&self, url: &str) -> bool {
        Url::parse(url).is_ok_and(|url| {
            url.host_str() == Some(&self.callback_host) && url.path() == self.callback_path
        })
    }
}

/// A browser to show this frame, in the window's logical coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct BrowserRequest {
    pub page: BrowserPage,
    pub rect: Rect,
    /// False while something painted by the app must show there: a native view covers Skia.
    pub is_visible: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> BrowserPage {
        BrowserPage {
            url: "https://www.amazon.co.uk/ap/signin".into(),
            user_agent: String::new(),
            cookies: vec![],
            callback_host: "www.amazon.co.uk".into(),
            callback_path: "/ap/maplanding".into(),
        }
    }

    #[test]
    fn recognises_only_the_sign_in_redirect() {
        let page = page();
        assert!(page.is_callback(
            "https://WWW.amazon.co.uk/ap/maplanding?openid.oa2.authorization_code=a|b"
        ));
        assert!(page.is_callback("https://www.amazon.co.uk/ap/maplanding"));
        assert!(!page.is_callback("https://www.amazon.com/ap/maplanding?x=1"));
        assert!(!page.is_callback("https://www.amazon.co.uk/ap/signin"));
        assert!(!page.is_callback("about:blank"));
        assert!(!page.is_callback("not a url"));
    }
}
