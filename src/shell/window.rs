//! A native window showing a `Stage`: winit for events, softbuffer for pixels, Skia to paint them.

use std::{
    num::NonZeroU32,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use skia_safe::{AlphaType, ColorType, ImageInfo, Point, Size, surfaces};
use softbuffer::{Context, Surface};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize},
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key as WinitKey, ModifiersState, NamedKey},
    window::{CursorIcon, Theme as WinitTheme, Window, WindowId},
};

use crate::shell::webview::{self, LoginWebView};
use crate::stage::{Effect, Key, Screen, Stage};
use crate::theme::Theme;

/// Points scrolled per wheel notch, for mice that report lines.
const LINE_HEIGHT: f32 = 28.0;

type WindowState<D> = (Arc<Window>, Surface<D, Arc<Window>>);

struct Host<D: winit::raw_window_handle::HasDisplayHandle, S> {
    stage: Stage<S>,
    size: (f64, f64),
    title: String,
    state: Option<WindowState<D>>,
    context: Option<Context<D>>,
    modifiers: ModifiersState,
    last_frame: Instant,
    clipboard: Option<arboard::Clipboard>,
    proxy: EventLoopProxy<HostEvent>,
    browser: Option<LoginWebView>,
    /// Keeps a browser that could not open from being retried on every frame.
    browser_failed: bool,
    /// Tells a load deadline from an earlier browser's.
    browser_generation: u64,
}

enum HostEvent {
    BrowserCallback(String),
    /// Something behind the screen changed.
    Wake,
    /// The sign-in page has had its time to load.
    BrowserLoadDeadline(u64),
}

/// Asks the window to redraw, from any thread.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// How long the sign-in page gets to finish loading before it counts as failed.
const BROWSER_LOAD_LIMIT: Duration = Duration::from_secs(30);

type Owned = winit::event_loop::OwnedDisplayHandle;

/// Runs `screen` in a window until it is closed.
pub fn run<S: Screen + 'static>(screen: S, title: &str, size: (f64, f64)) {
    run_with(|_| screen, title, size);
}

/// Runs the screen `make` builds, which can use `Wake` to ask for redraws from other threads.
pub fn run_with<S: Screen + 'static>(make: impl FnOnce(Wake) -> S, title: &str, size: (f64, f64)) {
    let event_loop = EventLoop::<HostEvent>::with_user_event()
        .build()
        .expect("create event loop");
    let wake_proxy = Mutex::new(event_loop.create_proxy());
    let wake: Wake = Arc::new(move || {
        if let Ok(proxy) = wake_proxy.lock() {
            let _ = proxy.send_event(HostEvent::Wake);
        }
    });
    let screen = make(wake);
    let context =
        Context::new(event_loop.owned_display_handle()).expect("create softbuffer context");
    let mut host = Host {
        stage: Stage::new(screen, Theme::LIGHT),
        size,
        title: title.to_string(),
        state: None,
        context: Some(context),
        modifiers: ModifiersState::empty(),
        last_frame: Instant::now(),
        clipboard: arboard::Clipboard::new().ok(),
        proxy: event_loop.create_proxy(),
        browser: None,
        browser_failed: false,
        browser_generation: 0,
    };
    event_loop.run_app(&mut host).expect("run event loop");
}

fn attributes(size: (f64, f64), title: &str) -> winit::window::WindowAttributes {
    let attributes = Window::default_attributes()
        .with_title(title)
        .with_inner_size(LogicalSize::new(size.0, size.1))
        .with_min_inner_size(LogicalSize::new(640.0, 420.0));
    #[cfg(target_os = "macos")]
    let attributes = {
        use winit::platform::macos::WindowAttributesExtMacOS as _;
        // The content runs under the title bar and draws its own controls where the Swift
        // app's toolbar puts them, which AppKit's standard buttons cannot reach.
        attributes
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            .with_fullsize_content_view(true)
            .with_titlebar_buttons_hidden(true)
    };
    attributes
}

fn theme_of(window: &Window) -> Theme {
    match window.theme() {
        Some(WinitTheme::Dark) => Theme::DARK,
        _ => Theme::LIGHT,
    }
}

impl<S: Screen> Host<Owned, S> {
    fn window(&self) -> Option<&Arc<Window>> {
        self.state.as_ref().map(|(window, _)| window)
    }

    fn request_redraw(&self) {
        if let Some(window) = self.window() {
            window.request_redraw();
        }
    }

    /// The browser goes first: it is a child of the window.
    fn close(&mut self) {
        self.browser = None;
        self.state = None;
    }

    /// Every way of closing comes through here, so unfinished work is never just dropped.
    fn request_close(&mut self) {
        if self.stage.screen.close_requested() {
            self.close();
        } else {
            self.request_redraw();
        }
    }

    fn apply(&mut self, effect: Option<Effect>) {
        let Some(effect) = effect else { return };
        match effect {
            Effect::Copy(text) => {
                if let Some(clipboard) = &mut self.clipboard {
                    let _ = clipboard.set_text(text);
                }
            }
            Effect::OpenUrl(url) => {
                if let Err(error) = opener::open_browser(&url) {
                    self.stage.screen.browser_failed(error.to_string());
                }
            }
            Effect::Reveal(path) => {
                if let Err(error) = opener::reveal(&path) {
                    eprintln!("Could not show {}: {error}", path.display());
                }
            }
            Effect::Close => self.request_close(),
            Effect::Minimize => {
                if let Some(window) = self.window() {
                    window.set_minimized(true);
                }
            }
            Effect::Zoom => {
                if let Some(window) = self.window() {
                    window.set_maximized(!window.is_maximized());
                }
            }
            Effect::Drag => {
                if let Some(window) = self.window() {
                    let _ = window.drag_window();
                }
            }
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame);
        self.last_frame = now;
        self.stage.advance(elapsed);
        if self.stage.screen.is_ready_to_close() {
            self.close();
            return;
        }
        let Some((window, surface)) = &mut self.state else {
            return;
        };
        let physical = window.inner_size();
        let (Some(width), Some(height)) = (
            NonZeroU32::new(physical.width),
            NonZeroU32::new(physical.height),
        ) else {
            return;
        };
        surface.resize(width, height).expect("resize surface");
        let mut buffer = surface.buffer_mut().expect("get pixel buffer");
        // softbuffer's 0RGB words are BGRA bytes on every little-endian platform it runs on.
        let bytes: &mut [u8] =
            unsafe { std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast(), buffer.len() * 4) };
        let info = ImageInfo::new(
            (physical.width as i32, physical.height as i32),
            ColorType::BGRA8888,
            AlphaType::Premul,
            None,
        );
        let scale = window.scale_factor() as f32;
        let logical = Size::new(
            physical.width as f32 / scale,
            physical.height as f32 / scale,
        );
        let mut pixels = surfaces::wrap_pixels(&info, bytes, None, None).expect("wrap pixels");
        let canvas = pixels.canvas();
        canvas.scale((scale, scale));
        self.stage.draw(canvas, logical);
        drop(pixels);
        window.pre_present_notify();
        buffer.present().expect("present frame");
        self.sync_browser();
    }

    /// Opens, moves or closes the native browser to match what the screen just drew.
    fn sync_browser(&mut self) {
        let Some(request) = self.stage.screen.browser() else {
            self.browser = None;
            self.browser_failed = false;
            return;
        };
        if let Some(browser) = &mut self.browser
            && browser.shows(&request.page)
        {
            browser.update(&request);
            return;
        }
        if self.browser_failed {
            return;
        }
        self.browser = None;
        let Some(window) = self.window().cloned() else {
            return;
        };
        let proxy = self.proxy.clone();
        match LoginWebView::open(&window, &request, move |url| {
            let _ = proxy.send_event(HostEvent::BrowserCallback(url));
        }) {
            Ok(browser) => {
                self.browser = Some(browser);
                self.browser_generation += 1;
                let (proxy, generation) = (self.proxy.clone(), self.browser_generation);
                std::thread::spawn(move || {
                    std::thread::sleep(BROWSER_LOAD_LIMIT);
                    let _ = proxy.send_event(HostEvent::BrowserLoadDeadline(generation));
                });
            }
            Err(message) => {
                self.browser_failed = true;
                self.stage.screen.browser_failed(message);
                self.request_redraw();
            }
        }
    }

    fn translate_key(&mut self, key: &WinitKey, text: Option<&str>) -> Option<Key> {
        let command = if cfg!(target_os = "macos") {
            self.modifiers.super_key()
        } else {
            self.modifiers.control_key()
        };
        if let WinitKey::Named(named) = key {
            return Some(match named {
                NamedKey::Backspace => Key::Backspace,
                NamedKey::Delete => Key::Delete,
                NamedKey::ArrowLeft => Key::Left,
                NamedKey::ArrowRight => Key::Right,
                NamedKey::Home | NamedKey::ArrowUp => Key::Home,
                NamedKey::End | NamedKey::ArrowDown => Key::End,
                NamedKey::Enter => Key::Enter,
                NamedKey::Escape => Key::Escape,
                NamedKey::Space if !command => Key::Text(" ".into()),
                _ => return None,
            });
        }
        if command {
            return match key.to_text()? {
                "a" => Some(Key::SelectAll),
                "c" => Some(Key::Copy),
                "x" => Some(Key::Cut),
                "v" => self.clipboard.as_mut()?.get_text().ok().map(Key::Paste),
                "q" | "w" => {
                    self.request_close();
                    None
                }
                _ => None,
            };
        }
        text.filter(|text| !text.is_empty())
            .map(|text| Key::Text(text.to_string()))
    }
}

impl<S: Screen> ApplicationHandler<HostEvent> for Host<Owned, S> {
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: HostEvent) {
        match event {
            HostEvent::BrowserCallback(url) => {
                let effect = self.stage.screen.browser_callback(url);
                self.apply(effect);
                self.request_redraw();
            }
            HostEvent::Wake => self.request_redraw(),
            HostEvent::BrowserLoadDeadline(generation) => {
                // wry reports no failed navigation, so a page that never finishes is the signal.
                if generation == self.browser_generation
                    && self.browser.as_ref().is_some_and(|b| !b.has_loaded())
                {
                    self.stage
                        .screen
                        .browser_failed("The sign-in page did not finish loading.".into());
                    self.request_redraw();
                }
            }
        }
        if self.state.is_none() {
            event_loop.exit();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        webview::install_edit_menu();
        let window = Arc::new(
            event_loop
                .create_window(attributes(self.size, &self.title))
                .expect("create window"),
        );
        let context = self
            .context
            .as_ref()
            .expect("context lives as long as the host");
        let surface = Surface::new(context, window.clone()).expect("create surface");
        self.stage.theme = theme_of(&window);
        self.last_frame = Instant::now();
        self.state = Some((window, surface));
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let scale = self.window().map_or(1.0, |window| window.scale_factor());
        match event {
            WindowEvent::CloseRequested => self.request_close(),
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.request_redraw()
            }
            WindowEvent::Focused(active) => {
                self.stage.is_active = active;
                if active {
                    self.stage.screen.window_focused();
                }
                self.request_redraw();
            }
            WindowEvent::ThemeChanged(theme) => {
                self.stage.theme = if theme == WinitTheme::Dark {
                    Theme::DARK
                } else {
                    Theme::LIGHT
                };
                self.request_redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let LogicalPosition { x, y } = position.to_logical::<f32>(scale);
                let changed = self.stage.pointer_moved(Point::new(x, y));
                if let Some(window) = self.window() {
                    window.set_cursor(if self.stage.is_over_text() {
                        CursorIcon::Text
                    } else {
                        CursorIcon::Default
                    });
                }
                if changed {
                    self.request_redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if self.stage.pointer_moved(Point::new(-1.0, -1.0)) {
                    self.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let effect = match (button, state) {
                    (MouseButton::Left, ElementState::Pressed) => self.stage.pointer_down(),
                    (MouseButton::Left, ElementState::Released) => self.stage.pointer_up(),
                    (MouseButton::Right, ElementState::Pressed) => self.stage.secondary_click(),
                    _ => None,
                };
                self.apply(effect);
                self.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let delta_y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * LINE_HEIGHT,
                    MouseScrollDelta::PixelDelta(position) => position.to_logical::<f32>(scale).y,
                };
                self.stage.screen.scroll(delta_y);
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let text = event.text.as_ref().map(|text| text.to_string());
                if let Some(key) = self.translate_key(&event.logical_key, text.as_deref()) {
                    let effect = self.stage.screen.key(key);
                    self.apply(effect);
                    self.request_redraw();
                }
            }
            _ => {}
        }
        if self.state.is_none() {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let wait = [self.stage.screen.next_frame(), self.stage.tooltip_due()]
            .into_iter()
            .flatten()
            .min();
        if let Some(wait) = wait {
            let target = self.last_frame + wait.max(Duration::from_millis(1));
            if Instant::now() < target {
                event_loop.set_control_flow(ControlFlow::WaitUntil(target));
                return;
            }
            self.request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}
