//! Measuring, wrapping and drawing static text through Skia's paragraph layout, in the system
//! font.
//!
//! A copy of the static half of the app's `src/text.rs`, which stays the original until the app
//! moves onto this crate and that file goes away. This copy is deliberately the simpler one and
//! leaves out, for now:
//!
//! * San Francisco's size-specific tracking, which CoreText applies and Skia does not;
//! * the half-point nudge that puts Skia's baseline where AppKit's sits;
//! * AppKit's wrapping rule, where a last line of a single word pulls one more word down;
//! * the rounded face, so `TextStyle::rounded` draws in the plain system font.
//!
//! Text measured here will therefore not line-break or track exactly as the app's window does.

use std::cell::RefCell;
use std::collections::HashMap;

use skia_safe::{
    Canvas, Color, FontMgr, FontStyle, Point,
    font_style::{Slant, Weight as SkWeight, Width},
    textlayout::{FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextStyle as SkTextStyle},
};

use crate::document::Weight;
use crate::ports::{TextEngine, TextStyle};

#[cfg(target_os = "macos")]
const FAMILIES: &[&str] = &[".AppleSystemUIFont"];
#[cfg(not(target_os = "macos"))]
const FAMILIES: &[&str] = &[
    "Segoe UI",
    "Cantarell",
    "Noto Sans",
    "DejaVu Sans",
    "sans-serif",
];

const UNBOUNDED: f32 = 100_000.0;
const CACHE_LIMIT: usize = 4_000;

#[derive(PartialEq, Eq, Hash)]
struct Key {
    text: String,
    size: u32,
    weight: Weight,
    tabular: bool,
    color: u32,
    /// `f32::to_bits` of the width an ellipsis cuts the line to, or 0 for unbounded.
    limit: u32,
}

/// Lays out and caches single lines; a frame redraws mostly the same strings.
pub struct TextSystem {
    fonts: FontCollection,
    cache: RefCell<HashMap<Key, Paragraph>>,
}

impl Default for TextSystem {
    fn default() -> Self {
        let mut fonts = FontCollection::new();
        fonts.set_default_font_manager(FontMgr::new(), None);
        Self {
            fonts,
            cache: RefCell::new(HashMap::new()),
        }
    }
}

impl TextSystem {
    fn with_line<R>(
        &self,
        text: &str,
        style: TextStyle,
        color: Color,
        limit: Option<f32>,
        use_line: impl FnOnce(&Paragraph) -> R,
    ) -> R {
        let key = Key {
            text: text.to_string(),
            size: (style.size * 100.0) as u32,
            weight: style.weight,
            tabular: style.tabular,
            color: u32::from_be_bytes([color.a(), color.r(), color.g(), color.b()]),
            limit: limit.map_or(0, f32::to_bits),
        };
        let mut cache = self.cache.borrow_mut();
        if cache.len() > CACHE_LIMIT {
            cache.clear();
        }
        let line = cache
            .entry(key)
            .or_insert_with(|| self.build(text, style, color, limit));
        use_line(line)
    }

    /// With `limit`, the line is cut to that width with an ellipsis against the last kept letter.
    fn build(&self, text: &str, style: TextStyle, color: Color, limit: Option<f32>) -> Paragraph {
        let mut sk = SkTextStyle::new();
        sk.set_font_families(FAMILIES);
        sk.set_font_size(style.size);
        sk.set_font_style(FontStyle::new(
            SkWeight::from(style.weight as i32),
            Width::NORMAL,
            Slant::Upright,
        ));
        sk.set_color(color);
        if style.tabular {
            sk.add_font_feature("tnum", 1);
        }
        let mut paragraph = ParagraphStyle::new();
        paragraph.set_max_lines(1);
        if limit.is_some() {
            paragraph.set_ellipsis("…");
        }
        let mut builder = ParagraphBuilder::new(&paragraph, self.fonts.clone());
        builder.push_style(&sk);
        builder.add_text(text);
        let mut line = builder.build();
        line.layout(limit.unwrap_or(UNBOUNDED));
        line
    }

    /// The width one line of `text` takes.
    pub fn width(&self, text: &str, style: TextStyle) -> f32 {
        self.with_line(text, style, Color::BLACK, None, |line| {
            line.max_intrinsic_width().ceil()
        })
    }

    /// Draws one line in a box of `style.line` height whose top-left is `origin`, truncating
    /// with an ellipsis at `limit`.
    pub fn draw_line(
        &self,
        canvas: &Canvas,
        text: &str,
        style: TextStyle,
        color: Color,
        origin: Point,
        limit: Option<f32>,
    ) {
        let limit = limit.filter(|limit| !text.is_empty() && *limit < self.width(text, style));
        self.with_line(text, style, color, limit, |line| {
            let y = origin.y + (style.line - line.height()) / 2.0;
            line.paint(canvas, Point::new(origin.x, y));
        });
    }

    /// Splits `text` into lines no wider than `width`: greedy, a word at a time. With
    /// `max_lines` the final line holds all the remaining text, for `draw_line` to truncate.
    pub fn wrap(
        &self,
        text: &str,
        style: TextStyle,
        width: f32,
        max_lines: Option<usize>,
    ) -> Vec<String> {
        let mut lines: Vec<String> = vec![];
        for hard_line in text.split('\n') {
            let mut current = String::new();
            for word in hard_line
                .split_whitespace()
                .flat_map(|word| self.split_long_word(word, style, width))
            {
                if current.is_empty() {
                    current = word;
                    continue;
                }
                let candidate = format!("{current} {word}");
                if self.width(&candidate, style) <= width {
                    current = candidate;
                } else {
                    lines.push(std::mem::replace(&mut current, word));
                }
            }
            lines.push(current);
        }
        if let Some(max) = max_lines
            && lines.len() > max
        {
            let rest = lines.split_off(max.max(1) - 1).join(" ");
            lines.push(rest);
        }
        lines
    }

    /// Breaks a word wider than a line, such as a path or address, between characters, so that
    /// nothing runs past its box.
    fn split_long_word(&self, word: &str, style: TextStyle, width: f32) -> Vec<String> {
        if self.width(word, style) <= width {
            return vec![word.to_string()];
        }
        let mut parts = vec![String::new()];
        for letter in word.chars() {
            let last = parts.last_mut().expect("starts with one part");
            let candidate = format!("{last}{letter}");
            if !last.is_empty() && self.width(&candidate, style) > width {
                parts.push(letter.to_string());
            } else {
                *last = candidate;
            }
        }
        parts
    }
}

impl TextEngine for TextSystem {
    fn width(&self, text: &str, style: &TextStyle) -> f32 {
        TextSystem::width(self, text, *style)
    }

    fn wrap(
        &self,
        text: &str,
        style: &TextStyle,
        width: f32,
        max_lines: Option<usize>,
    ) -> Vec<String> {
        TextSystem::wrap(self, text, *style, width, max_lines)
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
        TextSystem::draw_line(self, canvas, text, *style, color, origin, limit);
    }
}
