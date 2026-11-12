//! Draws a UI described as JSON.
//!
//! A [`Scene`] is a parsed, validated scene document — see `SCHEMA.md` for the format and
//! `scenes/` for real views in it. Rendering runs in four passes: resolve (expand `repeat`,
//! instantiate components, prune `when`, merge `variants`, read bindings), measure (intrinsic
//! sizes, bottom-up), layout (absolute rects, top-down) and paint. It gives back the hit regions,
//! native-view rects and content sizes the shell needs, in paint order.
//!
//! The crate owns no fonts, no network and no text editing: text measurement and drawing, image
//! loading and editable fields arrive through the traits in [`ports`].

pub mod document;
pub mod error;
pub mod kind;
pub mod layout;
pub mod paint;
pub mod ports;
pub mod resolve;
pub mod resolved;
pub mod scope;
pub mod text;
pub mod tokens;
#[cfg(feature = "testing")]
pub mod testing;
mod validate;

use std::time::Duration;

use skia_safe::Size;

pub use document::Document;
pub use error::{ErrorKind, SceneError, What};
pub use kind::NodeType;
pub use ports::{Fields, Images, TextEngine, TextStyle, Weight};
pub use text::TextSystem;
pub use paint::{Frame, Host, Region};
pub use resolve::ActionRef;
pub use resolved::Resolved;
pub use tokens::Appearance;

/// Which system the document is drawing for, since the window's own chrome differs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Platform {
    #[default]
    MacOs,
    Windows,
    Linux,
}

impl Platform {
    pub fn name(self) -> &'static str {
        match self {
            Self::MacOs => "macos",
            Self::Windows => "windows",
            Self::Linux => "linux",
        }
    }

    /// The system this was built for.
    pub fn host() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// What the host supplies for one frame.
pub struct Inputs<'a> {
    pub size: Size,
    pub appearance: Appearance,
    pub platform: Platform,
    /// Everything the document binds to and branches on.
    pub data: &'a serde_json::Value,
    /// The region under the pointer, and the one the button went down on.
    pub hot: Option<ActionRef>,
    pub pressed: Option<ActionRef>,
    pub focused_field: Option<&'a str>,
    /// macOS greys a background window's title bar controls and default buttons.
    pub window_active: bool,
    pub time: Duration,
}

impl<'a> Inputs<'a> {
    /// A frame with nothing hovered, nothing focused and an active window.
    pub fn new(size: Size, data: &'a serde_json::Value) -> Self {
        Self {
            size,
            appearance: Appearance::Light,
            platform: Platform::host(),
            data,
            hot: None,
            pressed: None,
            focused_field: None,
            window_active: true,
            time: Duration::ZERO,
        }
    }

    pub fn appearance(mut self, appearance: Appearance) -> Self {
        self.appearance = appearance;
        self
    }

    pub fn platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        self
    }

    pub fn hot(mut self, hot: ActionRef) -> Self {
        self.hot = Some(hot);
        self
    }

    pub fn pressed(mut self, pressed: ActionRef) -> Self {
        self.pressed = Some(pressed);
        self
    }

    pub fn focused_field(mut self, binding: &'a str) -> Self {
        self.focused_field = Some(binding);
        self
    }

    pub fn window_active(mut self, active: bool) -> Self {
        self.window_active = active;
        self
    }

    pub fn time(mut self, time: Duration) -> Self {
        self.time = time;
        self
    }
}

/// The host's implementations of what the crate does not own.
pub struct Renderer<'a> {
    pub text: &'a dyn TextEngine,
    pub images: &'a dyn Images,
    pub fields: &'a dyn Fields,
}

impl<'a> Renderer<'a> {
    pub fn new(
        text: &'a dyn TextEngine,
        images: &'a dyn Images,
        fields: &'a dyn Fields,
    ) -> Self {
        Self {
            text,
            images,
            fields,
        }
    }
}

/// A scene document that has been parsed and checked.
#[derive(Clone, Debug)]
pub struct Scene {
    document: Document,
}

impl Scene {
    /// Parses and validates a document. Errors carry a JSON pointer to the place at fault.
    pub fn parse(json: &str) -> Result<Self, SceneError> {
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|error| SceneError::new("", ErrorKind::NotJson(error.to_string())))?;
        Self::from_value(value)
    }

    pub fn from_value(value: serde_json::Value) -> Result<Self, SceneError> {
        validate::check_structure(&value)?;
        // The structural pass has already checked every key, so what is left for serde is types.
        let document: Document = serde_json::from_value(value)
            .map_err(|error| SceneError::new("", ErrorKind::NotJson(error.to_string())))?;
        validate::check_references(&document)?;
        Ok(Self { document })
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// The tree for one frame, with everything read and merged. `None` when the root's own
    /// `when` excludes it.
    pub fn resolve(&self, renderer: &Renderer<'_>, inputs: &Inputs<'_>) -> Option<Resolved> {
        let palette = tokens::Palette::new(&self.document.tokens, inputs.appearance);
        let resolver = resolve::Resolver {
            document: &self.document,
            palette,
            text: renderer.text,
            hot: inputs.hot.as_ref(),
            pressed: inputs.pressed.as_ref(),
            focused_field: inputs.focused_field,
            time: inputs.time,
        };
        let mut environment = scope::Locals::root();
        environment.bind_owned("platform", inputs.platform.name().into());
        environment.bind_owned("window_active", inputs.window_active.into());
        environment.bind_owned("dark", inputs.appearance.is_dark().into());
        resolver.root(inputs.data, &environment)
    }

    /// Resolves, lays out and paints one frame, giving back the regions, native-view rects and
    /// content sizes the shell needs.
    pub fn render(
        &self,
        renderer: &Renderer<'_>,
        canvas: &skia_safe::Canvas,
        inputs: &Inputs<'_>,
    ) -> Frame {
        let Some(tree) = self.resolve(renderer, inputs) else {
            return Frame::default();
        };
        let layout = layout::Layout {
            text: renderer.text,
            window: inputs.size,
        };
        let laid = layout.run(&tree, skia_safe::Rect::from_size(inputs.size));
        let mut painter = paint::Painter::new(canvas, renderer, inputs.time);
        painter.paint(&laid);
        painter.finish()
    }
}
