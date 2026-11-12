//! Stub ports and a loader for the documents in `scenes/`.
//!
//! Behind the `testing` feature, for this crate's tests and its `render` example. The stub text
//! engine gives every character the same advance, which is what makes assertions on real
//! coordinates possible without a font.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use skia_safe::{Canvas, Color, Paint, Point, Rect};

use crate::ports::{FieldPaint, Fields, Images, TextEngine, TextStyle};
use crate::{Scene, SceneError};

/// Where the authored documents live.
pub fn scenes_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scenes")
}

/// Reads one file from `scenes/` and drops its top-level `$comment`.
pub fn read_scene(name: &str) -> Value {
    let path = scenes_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let mut value: Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
    if let Some(object) = value.as_object_mut() {
        object.remove("$comment");
    }
    value
}

/// Assembles a document from the shared vocabulary in `scenes/` and a root node.
///
/// The schema has no include mechanism, so the authored files hold one thing each: the tokens,
/// the symbols, a component, or a view. This puts them together the way the app will once it
/// generates its own document.
pub struct Builder {
    tokens: Value,
    symbols: Map<String, Value>,
    components: Map<String, Value>,
    root: Value,
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            tokens: json!({}),
            symbols: Map::new(),
            components: Map::new(),
            root: json!({ "type": "box" }),
        }
    }
}

impl Builder {
    /// Empty, for a document that declares only what a test needs.
    pub fn new() -> Self {
        Self::default()
    }

    /// The app's real tokens and symbols, plus the components under `scenes/`.
    pub fn standard() -> Self {
        let symbols = match read_scene("symbols.json") {
            Value::Object(map) => map,
            other => panic!("symbols.json is {other}"),
        };
        Self {
            tokens: read_scene("tokens.json"),
            symbols,
            components: Map::new(),
            root: json!({ "type": "box" }),
        }
        .component("account-avatar", "account-avatar.json")
        .component("book-tile", "book-tile.json")
    }

    pub fn tokens(mut self, tokens: Value) -> Self {
        self.tokens = tokens;
        self
    }

    pub fn symbol(mut self, name: &str, symbol: Value) -> Self {
        self.symbols.insert(name.to_string(), symbol);
        self
    }

    /// Declares `name` from a file in `scenes/`.
    pub fn component(mut self, name: &str, file: &str) -> Self {
        self.components
            .insert(name.to_string(), read_scene(file));
        self
    }

    pub fn component_value(mut self, name: &str, component: Value) -> Self {
        self.components.insert(name.to_string(), component);
        self
    }

    pub fn root(mut self, root: Value) -> Self {
        self.root = root;
        self
    }

    /// The root is the view in `scenes/<file>`.
    pub fn root_scene(self, file: &str) -> Self {
        let root = read_scene(file);
        self.root(root)
    }

    pub fn value(&self) -> Value {
        json!({
            "schema": crate::document::VERSION,
            "tokens": self.tokens,
            "symbols": Value::Object(self.symbols.clone()),
            "components": Value::Object(self.components.clone()),
            "root": self.root,
        })
    }

    pub fn build(&self) -> Result<Scene, SceneError> {
        Scene::from_value(self.value())
    }

    /// Panics with the error's path and message, which is what a test wants.
    pub fn parse(&self) -> Scene {
        self.build().unwrap_or_else(|error| panic!("{error}"))
    }
}

pub static TEXT: StubText = StubText;
pub static IMAGES: NoImages = NoImages;
pub static FIELDS: NoFields = NoFields;

/// A renderer over the stubs, for tests and the `render` example.
pub fn renderer() -> crate::Renderer<'static> {
    crate::Renderer::new(&TEXT, &IMAGES, &FIELDS)
}

/// Every character is half the point size wide, and wrapping is plain greedy.
#[derive(Clone, Copy, Debug, Default)]
pub struct StubText;

impl StubText {
    pub const ADVANCE: f32 = 0.5;

    fn line_width(text: &str, style: &TextStyle) -> f32 {
        text.chars().count() as f32 * style.size * Self::ADVANCE
    }
}

impl TextEngine for StubText {
    fn width(&self, text: &str, style: &TextStyle) -> f32 {
        Self::line_width(text, style).ceil()
    }

    fn wrap(
        &self,
        text: &str,
        style: &TextStyle,
        width: f32,
        max_lines: Option<usize>,
    ) -> Vec<String> {
        let mut lines: Vec<String> = vec![];
        for hard_line in text.split('\n') {
            for word in hard_line.split_whitespace() {
                match lines.last_mut() {
                    Some(last) if Self::line_width(&format!("{last} {word}"), style) <= width => {
                        last.push(' ');
                        last.push_str(word);
                    }
                    _ => lines.push(word.to_string()),
                }
            }
            if hard_line.is_empty() {
                lines.push(String::new());
            }
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        // With a limit, the last kept line holds the rest, for the caller to truncate.
        if let Some(max) = max_lines
            && lines.len() > max
        {
            let rest = lines.split_off(max.max(1) - 1).join(" ");
            lines.push(rest);
        }
        lines
    }

    fn draw_line(
        &self,
        canvas: &Canvas,
        text: &str,
        style: &TextStyle,
        color: Color,
        origin: Point,
        limit: Option<f32>,
    ) {
        let width = limit
            .map_or_else(|| Self::line_width(text, style), |limit| {
                Self::line_width(text, style).min(limit)
            });
        let height = style.size * 0.7;
        let top = origin.y + (style.line - height) / 2.0;
        let mut paint = Paint::default();
        paint.set_anti_alias(false).set_color(color);
        canvas.draw_rect(Rect::from_xywh(origin.x, top, width, height), &paint);
    }
}

/// No images ever arrive, so an `image` node always paints its fallback.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoImages;

impl Images for NoImages {
    fn image(&self, _source: &str) -> Option<skia_safe::Image> {
        None
    }
}

/// Draws a field's placeholder box and nothing else.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoFields;

impl Fields for NoFields {
    fn draw(&self, canvas: &Canvas, field: FieldPaint<'_>) {
        StubText.draw_line(
            canvas,
            field.placeholder,
            &field.style,
            field.placeholder_color,
            Point::new(field.rect.left, field.rect.top),
            Some(field.rect.width()),
        );
    }

    fn input_area(&self, _binding: &str) -> Option<Rect> {
        None
    }
}
