//! What the crate needs from its host.
//!
//! Text measurement and drawing, image loading and editable fields all exist in the app already:
//! San Francisco's tracking and AppKit's wrapping in its text system, covers fetched and decoded
//! off the UI thread, and a Parley-backed field with its own selection geometry and caret. Rather
//! than duplicate any of it, the renderer asks for it through these three traits.

use skia_safe::{Canvas, Color, Image, Point, Rect};

pub use crate::document::Weight;

/// A text style with every token and override already resolved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    /// The line box this text sits in, which is not the font's own line height.
    pub line: f32,
    pub weight: Weight,
    pub rounded: bool,
    pub tabular: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: 13.0,
            line: 16.0,
            weight: Weight::Regular,
            rounded: false,
            tabular: false,
        }
    }
}

/// Measuring and drawing static text.
pub trait TextEngine {
    /// The width one line of `text` takes, rounded up as the app's own measurement does.
    fn width(&self, text: &str, style: &TextStyle) -> f32;

    /// Splits `text` into lines no wider than `width`. `max_lines` leaves the remainder on the
    /// last line, for the caller to truncate.
    fn wrap(
        &self,
        text: &str,
        style: &TextStyle,
        width: f32,
        max_lines: Option<usize>,
    ) -> Vec<String>;

    /// Draws one line in a box `style.line` high whose top-left is `origin`, truncating with an
    /// ellipsis at `limit`.
    fn draw_line(
        &self,
        canvas: &Canvas,
        text: &str,
        style: &TextStyle,
        color: Color,
        origin: Point,
        limit: Option<f32>,
    );
}

/// Resolving an image source to pixels. `None` means "not here yet", and the node paints its
/// fallback children instead.
pub trait Images {
    fn image(&self, source: &str) -> Option<Image>;
}

/// Painting an editable field, which owns its own text, selection and caret.
pub trait Fields {
    fn draw(&self, canvas: &Canvas, field: FieldPaint<'_>);

    /// Where text is being typed, in window points, so input methods can place their panels.
    fn input_area(&self, binding: &str) -> Option<Rect>;
}

/// Everything a field needs to paint itself, once layout has placed it.
#[derive(Clone, Copy, Debug)]
pub struct FieldPaint<'a> {
    pub binding: &'a str,
    pub rect: Rect,
    pub style: TextStyle,
    pub color: Color,
    pub placeholder: &'a str,
    pub placeholder_color: Color,
    pub selection_color: Color,
    pub caret_color: Color,
    pub focused: bool,
    /// True on the half of the blink period the caret shows.
    pub caret_visible: bool,
}
