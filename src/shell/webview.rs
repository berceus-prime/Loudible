//! The native browser laid over the Skia surface for signing in.

/// Gives the menu bar an Edit menu. AppKit delivers the editing shortcuts to a web view through
/// its key equivalents; without them Cmd+V does nothing on the sign-in page.
#[cfg(target_os = "macos")]
pub fn install_edit_menu() {
    use objc2::{MainThreadMarker, runtime::Sel, sel};
    use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
    use objc2_foundation::NSString;

    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let Some(menu_bar) = NSApplication::sharedApplication(main_thread).mainMenu() else {
        return;
    };
    let edit = NSMenu::initWithTitle(main_thread.alloc(), &NSString::from_str("Edit"));
    let items: [(&str, Sel, &str); 6] = [
        ("Undo", sel!(undo:), "z"),
        // An uppercase key equivalent means Shift is held too.
        ("Redo", sel!(redo:), "Z"),
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ];
    for (title, action, key) in items {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                main_thread.alloc(),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        };
        edit.addItem(&item);
    }
    let holder = NSMenuItem::new(main_thread);
    holder.setSubmenu(Some(&edit));
    // After the application menu, where macOS users expect it.
    menu_bar.insertItem_atIndex(&holder, 1);
    route_edit_shortcuts_past_the_menu();
}

/// The Edit menu would swallow these shortcuts, and winit's view has no `paste:` and so on: when
/// that view has focus they go straight to it, for the text fields painted with Skia.
#[cfg(target_os = "macos")]
fn route_edit_shortcuts_past_the_menu() {
    use std::ptr::{NonNull, null_mut};

    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};

    let handler = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        // AppKit passes a valid event and calls this on the main thread.
        let key_event = unsafe { event.as_ref() };
        let Some(main_thread) = MainThreadMarker::new() else {
            return event.as_ptr();
        };
        let is_edit_shortcut = key_event
            .modifierFlags()
            .contains(NSEventModifierFlags::Command)
            && key_event.charactersIgnoringModifiers().is_some_and(|key| {
                matches!(
                    key.to_string().to_lowercase().as_str(),
                    "z" | "x" | "c" | "v" | "a"
                )
            });
        if !is_edit_shortcut {
            return event.as_ptr();
        }
        let Some(window) = key_event.window(main_thread) else {
            return event.as_ptr();
        };
        let (Some(content), Some(responder)) = (window.contentView(), window.firstResponder())
        else {
            return event.as_ptr();
        };
        if !std::ptr::addr_eq(&*content, &*responder) {
            return event.as_ptr();
        }
        content.keyDown(key_event);
        null_mut()
    });
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler)
    };
    // Lives as long as the app.
    std::mem::forget(monitor);
}

#[cfg(not(target_os = "macos"))]
pub fn install_edit_menu() {}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use native::LoginWebView;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub use unavailable::LoginWebView;

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod native {
    use std::cell::Cell;
    use std::rc::Rc;

    use skia_safe::Rect;
    use winit::window::Window;
    use wry::{
        WebView, WebViewBuilder,
        dpi::{LogicalPosition, LogicalSize},
    };

    use crate::shell::browser::{BrowserPage, BrowserRequest};

    pub struct LoginWebView {
        page: BrowserPage,
        rect: Rect,
        is_visible: bool,
        has_loaded: Rc<Cell<bool>>,
        view: WebView,
    }

    fn bounds(rect: Rect) -> wry::Rect {
        wry::Rect {
            position: LogicalPosition::new(rect.left as f64, rect.top as f64).into(),
            size: LogicalSize::new(rect.width() as f64, rect.height() as f64).into(),
        }
    }

    /// Stores the cookies without waiting: wry's `set_cookie` spins the run loop until done, and
    /// winit aborts when that dispatches an event from inside one of its callbacks.
    #[cfg(target_os = "macos")]
    fn load_after_cookies(view: &WebView, page: &BrowserPage) -> Result<(), String> {
        use std::{cell::Cell, rc::Rc};

        use block2::RcBlock;
        use objc2::{rc::Retained, runtime::AnyObject};
        use objc2_foundation::{
            NSDictionary, NSHTTPCookie, NSHTTPCookieDomain, NSHTTPCookieName, NSHTTPCookiePath,
            NSHTTPCookiePropertyKey, NSHTTPCookieSecure, NSHTTPCookieValue, NSString, NSURL,
            NSURLRequest,
        };
        use objc2_web_kit::WKWebView;
        use wry::WebViewExtMacOS as _;

        let url = NSURL::URLWithString(&NSString::from_str(&page.url))
            .ok_or_else(|| format!("Invalid sign-in address: {}", page.url))?;
        let request = NSURLRequest::requestWithURL(&url);
        let web_view: Retained<WKWebView> = Retained::into_super(view.webview());
        if page.cookies.is_empty() {
            unsafe { web_view.loadRequest(&request) };
            return Ok(());
        }

        let store = unsafe {
            web_view
                .configuration()
                .websiteDataStore()
                .httpCookieStore()
        };
        let remaining = Rc::new(Cell::new(page.cookies.len()));
        let on_stored = RcBlock::new(move || {
            remaining.set(remaining.get() - 1);
            if remaining.get() == 0 {
                unsafe { web_view.loadRequest(&request) };
            }
        });
        for cookie in &page.cookies {
            let [name, value, domain, path] = [
                cookie.name.as_str(),
                cookie.value.as_str(),
                cookie.domain.as_str(),
                "/",
            ]
            .map(NSString::from_str);
            let secure = NSString::from_str(if cookie.is_secure { "TRUE" } else { "FALSE" });
            let properties: Retained<NSDictionary<NSHTTPCookiePropertyKey, AnyObject>> = unsafe {
                NSDictionary::from_slices(
                    &[
                        NSHTTPCookieName,
                        NSHTTPCookieValue,
                        NSHTTPCookieDomain,
                        NSHTTPCookiePath,
                        NSHTTPCookieSecure,
                    ],
                    &[&*name, &*value, &*domain, &*path, &*secure],
                )
            };
            let cookie = unsafe { NSHTTPCookie::cookieWithProperties(&properties) }
                .ok_or_else(|| format!("Invalid sign-in cookie: {}", cookie.name))?;
            unsafe { store.setCookie_completionHandler(&cookie, Some(&on_stored)) };
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn load_after_cookies(view: &WebView, page: &BrowserPage) -> Result<(), String> {
        for cookie in &page.cookies {
            let cookie = wry::cookie::Cookie::build((cookie.name.as_str(), cookie.value.as_str()))
                .domain(cookie.domain.as_str())
                .path("/")
                .secure(cookie.is_secure)
                .build();
            view.set_cookie(&cookie)
                .map_err(|error| error.to_string())?;
        }
        view.load_url(&page.url).map_err(|error| error.to_string())
    }

    impl LoginWebView {
        pub const IS_AVAILABLE: bool = true;

        /// Opens a non-persistent browser, sets its cookies, then loads the page. Navigation to
        /// the sign-in redirect is stopped and handed to `on_callback`, once.
        pub fn open(
            window: &Window,
            request: &BrowserRequest,
            on_callback: impl Fn(String) + 'static,
        ) -> Result<Self, String> {
            let page = request.page.clone();
            let delivered = Cell::new(false);
            let has_loaded = Rc::new(Cell::new(false));
            let finished = has_loaded.clone();
            let view = WebViewBuilder::new()
                .with_user_agent(page.user_agent.clone())
                .with_incognito(true)
                .with_bounds(bounds(request.rect))
                .with_visible(request.is_visible)
                .with_on_page_load_handler(move |event, _| {
                    if matches!(event, wry::PageLoadEvent::Finished) {
                        finished.set(true);
                    }
                })
                .with_navigation_handler(move |url| {
                    if !page.is_callback(&url) {
                        return true;
                    }
                    if !delivered.replace(true) {
                        on_callback(url);
                    }
                    false
                })
                .build_as_child(window)
                .map_err(|error| error.to_string())?;
            load_after_cookies(&view, &request.page)?;
            Ok(Self {
                page: request.page.clone(),
                rect: request.rect,
                is_visible: request.is_visible,
                has_loaded,
                view,
            })
        }

        pub fn shows(&self, page: &BrowserPage) -> bool {
            self.page == *page
        }

        /// Whether any page has finished loading in this browser.
        pub fn has_loaded(&self) -> bool {
            self.has_loaded.get()
        }

        pub fn update(&mut self, request: &BrowserRequest) {
            if self.rect != request.rect && self.view.set_bounds(bounds(request.rect)).is_ok() {
                self.rect = request.rect;
            }
            if self.is_visible != request.is_visible
                && self.view.set_visible(request.is_visible).is_ok()
            {
                self.is_visible = request.is_visible;
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod unavailable {
    use winit::window::Window;

    use crate::shell::browser::{BrowserPage, BrowserRequest};

    pub struct LoginWebView;

    impl LoginWebView {
        pub const IS_AVAILABLE: bool = false;

        pub fn open(
            _: &Window,
            _: &BrowserRequest,
            _: impl Fn(String) + 'static,
        ) -> Result<Self, String> {
            Err("This platform has no embedded browser yet.".into())
        }

        pub fn shows(&self, _: &BrowserPage) -> bool {
            false
        }

        pub fn has_loaded(&self) -> bool {
            true
        }

        pub fn update(&mut self, _: &BrowserRequest) {}
    }
}
