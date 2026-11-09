//! Text measuring, wrapping and drawing through Skia's paragraph layout. On macOS this is the
//! system font (San Francisco), the same one AppKit uses.

use std::{cell::RefCell, collections::HashMap};

use skia_safe::textlayout::Paragraph;
use skia_safe::{
    Canvas, Color, FontMgr, FontStyle, Point,
    font_style::{Slant, Weight as SkWeight, Width},
    textlayout::{FontCollection, ParagraphBuilder, ParagraphStyle, TextStyle as SkTextStyle},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular = 400,
    Medium = 500,
    Semibold = 600,
    Bold = 700,
}

/// A font role. Sizes and line heights are macOS's text styles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub line: f32,
    pub weight: Weight,
    pub rounded: bool,
    pub tabular: bool,
}

impl TextStyle {
    pub const fn new(size: f32, line: f32, weight: Weight) -> Self {
        Self {
            size,
            line,
            weight,
            rounded: false,
            tabular: false,
        }
    }

    pub const fn weight(mut self, weight: Weight) -> Self {
        self.weight = weight;
        self
    }

    pub const fn rounded(mut self) -> Self {
        self.rounded = true;
        self
    }

    pub const fn tabular(mut self) -> Self {
        self.tabular = true;
        self
    }
}

pub const BODY: TextStyle = TextStyle::new(13.0, 16.0, Weight::Regular);
pub const HEADLINE: TextStyle = TextStyle::new(13.0, 16.0, Weight::Bold);
pub const CALLOUT: TextStyle = TextStyle::new(12.0, 15.0, Weight::Regular);
pub const CAPTION: TextStyle = TextStyle::new(10.0, 13.0, Weight::Regular);
pub const TITLE3: TextStyle = TextStyle::new(15.0, 19.0, Weight::Regular);

#[cfg(target_os = "macos")]
const FAMILIES: &[&str] = &[".AppleSystemUIFont"];
#[cfg(target_os = "macos")]
const ROUNDED_FAMILIES: &[&str] = &[".AppleSystemUIFontRounded", ".AppleSystemUIFont"];
#[cfg(not(target_os = "macos"))]
const FAMILIES: &[&str] = &[
    "Segoe UI",
    "Cantarell",
    "Noto Sans",
    "DejaVu Sans",
    "sans-serif",
];
#[cfg(not(target_os = "macos"))]
const ROUNDED_FAMILIES: &[&str] = FAMILIES;

/// Skia sits half a point lower than AppKit in the same line box.
const BASELINE_NUDGE: f32 = 0.5;
const UNBOUNDED: f32 = 100_000.0;
const CACHE_LIMIT: usize = 4_000;

/// San Francisco's size-specific tracking, which CoreText applies and Skia does not.
fn tracking(size: f32) -> f32 {
    const TABLE: [(f32, f32); 5] = [
        (10.0, 0.12),
        (11.0, 0.06),
        (12.0, 0.0),
        (13.0, -0.08),
        (15.0, -0.23),
    ];
    if size >= 15.0 {
        return -0.23;
    }
    for pair in TABLE.windows(2) {
        let [(low, a), (high, b)] = [pair[0], pair[1]];
        if size <= high {
            return a + (b - a) * ((size - low) / (high - low)).max(0.0);
        }
    }
    0.0
}

#[derive(PartialEq, Eq, Hash)]
struct Key {
    text: String,
    size: u32,
    weight: Weight,
    rounded: bool,
    tabular: bool,
    color: u32,
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
        use_line: impl FnOnce(&Paragraph) -> R,
    ) -> R {
        let key = Key {
            text: text.to_string(),
            size: (style.size * 100.0) as u32,
            weight: style.weight,
            rounded: style.rounded,
            tabular: style.tabular,
            color: u32::from_be_bytes([color.a(), color.r(), color.g(), color.b()]),
        };
        let mut cache = self.cache.borrow_mut();
        if cache.len() > CACHE_LIMIT {
            cache.clear();
        }
        let line = cache
            .entry(key)
            .or_insert_with(|| self.build(text, style, color));
        use_line(line)
    }

    fn build(&self, text: &str, style: TextStyle, color: Color) -> Paragraph {
        let mut sk = SkTextStyle::new();
        sk.set_font_families(if style.rounded {
            ROUNDED_FAMILIES
        } else {
            FAMILIES
        });
        sk.set_font_size(style.size);
        sk.set_font_style(FontStyle::new(
            SkWeight::from(style.weight as i32),
            Width::NORMAL,
            Slant::Upright,
        ));
        sk.set_color(color);
        sk.set_letter_spacing(tracking(style.size));
        if style.tabular {
            sk.add_font_feature("tnum", 1);
        }
        let mut paragraph = ParagraphStyle::new();
        paragraph.set_max_lines(1);
        let mut builder = ParagraphBuilder::new(&paragraph, self.fonts.clone());
        builder.push_style(&sk);
        builder.add_text(text);
        let mut line = builder.build();
        line.layout(UNBOUNDED);
        line
    }

    pub fn width(&self, text: &str, style: TextStyle) -> f32 {
        self.with_line(text, style, Color::BLACK, |l| {
            l.max_intrinsic_width().ceil()
        })
    }

    /// `text` cut to fit `limit` with an ellipsis against the last kept letter, as AppKit does.
    fn truncated(&self, text: &str, style: TextStyle, limit: f32) -> String {
        let ends: Vec<usize> = text.char_indices().map(|(index, _)| index).collect();
        let shortened = |count: usize| format!("{}…", text[..ends[count]].trim_end());
        let (mut fits, mut too_long) = (0, ends.len());
        while too_long - fits > 1 {
            let middle = (fits + too_long) / 2;
            if self.width(&shortened(middle), style) <= limit {
                fits = middle;
            } else {
                too_long = middle;
            }
        }
        shortened(fits)
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
        let fitted = limit
            .filter(|limit| !text.is_empty() && *limit < self.width(text, style))
            .map(|limit| self.truncated(text, style, limit));
        let text = fitted.as_deref().unwrap_or(text);
        self.with_line(text, style, color, |line| {
            let y = origin.y + (style.line - line.height()) / 2.0 - BASELINE_NUDGE;
            line.paint(canvas, Point::new(origin.x, y));
        });
    }

    /// Splits `text` into lines no wider than `width`, like AppKit: greedy, except a last line
    /// of a single word pulls one more word down, unless that leaves a lone word above it. With `max_lines` the final line holds all the
    /// remaining text, for `draw_line` to truncate.
    pub fn wrap(
        &self,
        text: &str,
        style: TextStyle,
        width: f32,
        max_lines: Option<usize>,
    ) -> Vec<String> {
        let mut lines: Vec<String> = vec![];
        let max = max_lines.unwrap_or(usize::MAX);
        for hard_line in text.split('\n') {
            let wrapped = self.wrap_line(hard_line, style, width);
            lines.extend(wrapped);
        }
        if lines.len() > max {
            let rest = lines.split_off(max - 1).join(" ");
            lines.push(rest);
        }
        lines
    }

    fn wrap_line(&self, text: &str, style: TextStyle, width: f32) -> Vec<String> {
        let mut lines: Vec<Vec<String>> = vec![vec![]];
        for word in text
            .split(' ')
            .flat_map(|w| self.split_long_word(w, style, width))
        {
            let current = lines.last_mut().expect("starts with one line");
            let candidate = current
                .iter()
                .map(String::as_str)
                .chain([word.as_str()])
                .collect::<Vec<_>>()
                .join(" ");
            if current.is_empty() || self.width(&candidate, style) <= width {
                current.push(word);
            } else {
                lines.push(vec![word]);
            }
        }
        if let [.., previous, last] = lines.as_mut_slice()
            && last.len() == 1
            && previous.len() > 2
        {
            let moved = format!("{} {}", previous[previous.len() - 1], last[0]);
            if self.width(&moved, style) <= width {
                let word = previous.pop().expect("has more than one word");
                last.insert(0, word);
            }
        }
        lines.into_iter().map(|words| words.join(" ")).collect()
    }

    /// Breaks a word wider than a line, such as a path or address, between characters.
    fn split_long_word(&self, word: &str, style: TextStyle, width: f32) -> Vec<String> {
        if self.width(word, style) <= width {
            return vec![word.to_string()];
        }
        let mut parts = vec![String::new()];
        for ch in word.chars() {
            let last = parts.last_mut().expect("starts with one part");
            let candidate = format!("{last}{ch}");
            if !last.is_empty() && self.width(&candidate, style) > width {
                parts.push(ch.to_string());
            } else {
                *last = candidate;
            }
        }
        parts
    }

    /// The byte offset of the character boundary nearest to `x` points into `text`.
    pub fn caret_index(&self, text: &str, style: TextStyle, x: f32) -> usize {
        let mut best = (0, x.abs());
        for (index, _) in text.char_indices().skip(1).chain([(text.len(), ' ')]) {
            let distance = (self.width(&text[..index], style) - x).abs();
            if distance < best.1 {
                best = (index, distance);
            }
        }
        best.0
    }
}
