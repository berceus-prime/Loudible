//! What the page says about one frame.
//!
//! Everything here has a default, so `render(canvas, document)` with no options at all draws the
//! document light, at 1x, over the whole canvas, with no data. Unknown keys are an error rather
//! than a shrug: a misspelt option would otherwise be a frame that quietly ignores it.

use std::time::Duration;

use loudible_scene::{ActionRef, Appearance, Inputs, Platform};
use serde::Deserialize;
use serde_json::Value;
use skia_safe::{Color, Size};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PlatformName {
    #[default]
    Macos,
    Windows,
    Linux,
}

impl From<PlatformName> for Platform {
    fn from(name: PlatformName) -> Self {
        match name {
            PlatformName::Macos => Platform::MacOs,
            PlatformName::Windows => Platform::Windows,
            PlatformName::Linux => Platform::Linux,
        }
    }
}

/// A region the pointer is over or pressing, named the way the document names it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub action: String,
    #[serde(default)]
    pub args: Value,
}

impl Action {
    fn reference(&self) -> ActionRef {
        ActionRef::with_args(self.action.clone(), self.args.clone())
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    /// The window in points. Without it, the canvas's own size divided by the scale.
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// Device pixels per point.
    pub scale: f32,
    pub dark: bool,
    pub platform: PlatformName,
    /// Everything the document binds to and branches on.
    pub data: Value,
    pub hot: Option<Action>,
    pub pressed: Option<Action>,
    pub focused_field: Option<String>,
    pub window_active: bool,
    /// How far into the animation this frame is.
    pub time_ms: f64,
    /// `#RRGGBB` or `#AARRGGBB` under the document. The default follows `dark`.
    pub background: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            scale: 1.0,
            dark: false,
            platform: PlatformName::default(),
            data: Value::Null,
            hot: None,
            pressed: None,
            focused_field: None,
            window_active: true,
            time_ms: 0.0,
            background: None,
        }
    }
}

impl Options {
    /// The window in points, falling back to the canvas's own pixels at this scale.
    pub fn size(&self, canvas_width: u32, canvas_height: u32) -> Size {
        let scale = self.scale.max(0.01);
        Size::new(
            self.width.unwrap_or(canvas_width as f32 / scale),
            self.height.unwrap_or(canvas_height as f32 / scale),
        )
    }

    /// Opaque, so the premultiplied pixels Skia writes are also what `ImageData` expects.
    pub fn background(&self) -> Result<Color, String> {
        match &self.background {
            Some(hex) => loudible_scene::tokens::parse_hex(hex)
                .ok_or_else(|| format!("{hex:?} is not a #RRGGBB or #AARRGGBB colour")),
            None if self.dark => Ok(Color::from(0xFF1E1E1E)),
            None => Ok(Color::WHITE),
        }
    }

    pub fn inputs(&self, size: Size) -> Inputs<'_> {
        let appearance = if self.dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        let mut inputs = Inputs::new(size, &self.data)
            .appearance(appearance)
            .platform(self.platform.into())
            .window_active(self.window_active)
            .time(Duration::from_secs_f64(self.time_ms.max(0.0) / 1000.0));
        if let Some(hot) = &self.hot {
            inputs = inputs.hot(hot.reference());
        }
        if let Some(pressed) = &self.pressed {
            inputs = inputs.pressed(pressed.reference());
        }
        if let Some(field) = &self.focused_field {
            inputs = inputs.focused_field(field);
        }
        inputs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Options {
        serde_json::from_str(json).expect("options")
    }

    #[test]
    fn an_empty_object_is_a_light_frame_at_1x() {
        let options = parse("{}");
        assert_eq!(options.scale, 1.0);
        assert!(!options.dark);
        assert!(options.window_active);
        assert_eq!(options.platform, PlatformName::Macos);
        assert_eq!(options.size(800, 400), Size::new(800.0, 400.0));
        assert_eq!(options.background().unwrap(), Color::WHITE);
    }

    #[test]
    fn the_canvas_size_is_read_in_points() {
        let options = parse(r#"{ "scale": 2 }"#);
        assert_eq!(options.size(800, 400), Size::new(400.0, 200.0));
        let given = parse(r#"{ "scale": 2, "width": 320, "height": 100 }"#);
        assert_eq!(given.size(800, 400), Size::new(320.0, 100.0));
    }

    #[test]
    fn dark_brings_its_own_background() {
        let options = parse(r#"{ "dark": true }"#);
        assert_eq!(options.background().unwrap(), Color::from(0xFF1E1E1E));
        let named = parse(r##"{ "background": "#FF102030" }"##);
        assert_eq!(named.background().unwrap(), Color::from(0xFF102030));
        assert!(parse(r#"{ "background": "mauve" }"#).background().is_err());
    }

    #[test]
    fn a_misspelt_option_is_an_error() {
        let error = serde_json::from_str::<Options>(r#"{ "darkk": true }"#)
            .expect_err("unknown keys are refused");
        assert!(error.to_string().contains("darkk"), "{error}");
    }

    #[test]
    fn inputs_carry_the_pointer_and_the_clock() {
        let options = parse(
            r#"{ "dark": true, "timeMs": 500, "hot": { "action": "play", "args": { "id": 7 } } }"#,
        );
        let inputs = options.inputs(Size::new(100.0, 50.0));
        assert_eq!(inputs.time, Duration::from_millis(500));
        assert!(inputs.appearance.is_dark());
        let hot = inputs.hot.expect("a hot region");
        assert_eq!(hot.action, "play");
        assert_eq!(hot.args["id"], 7);
    }
}
