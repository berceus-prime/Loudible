//! Text measuring, wrapping and drawing through Skia's paragraph layout. On macOS this is the
//! system font (San Francisco), the same one AppKit uses.

use std::{cell::RefCell, collections::HashMap};

use parley::fontique::{Blob, FontInfoOverride};
use parley::setting::Tag;
use parley::{
    FontContext, FontData, FontFamily, FontFamilyName, FontVariation, FontVariations, FontWeight,
    GenericFamily, Layout, LayoutContext, LineHeight, PositionedLayoutItem, StyleProperty,
};
use skia_safe::textlayout::Paragraph;
use skia_safe::{
    Canvas, Color, Data, Font, FontArguments, FontMgr, FontStyle, FourByteTag, Paint, Point, Rect,
    Typeface,
    font::Edging,
    font_arguments::{VariationPosition, variation_position::Coordinate},
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

/// The family editable text is laid out in: the font Skia picks for `FAMILIES`, registered with
/// Parley under this name so both draw with the same file.
const EDIT_FAMILY: &str = "skia-ui";

/// The app's own font first, then the system's for scripts and emoji it lacks.
const EDIT_FAMILIES: &[FontFamilyName<'static>] = &[
    FontFamilyName::named(EDIT_FAMILY),
    FontFamilyName::Generic(GenericFamily::SystemUi),
];

/// Measured both ways to find how to make edited text sit and run like drawn text.
const SPACING_SAMPLE: &str = "The quick brown fox jumps over the lazy dog 0123456789";

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
    editing: RefCell<Editing>,
}

impl Default for TextSystem {
    fn default() -> Self {
        let mut fonts = FontCollection::new();
        fonts.set_default_font_manager(FontMgr::new(), None);
        Self {
            fonts,
            cache: RefCell::new(HashMap::new()),
            editing: RefCell::new(Editing::new()),
        }
    }
}

#[derive(Clone, Copy)]
struct Adjustment {
    spacing: f32,
    shift: f32,
}

/// Parley's contexts for editable text, and the Skia typefaces its glyph runs draw with.
struct Editing {
    font_cx: FontContext,
    layout_cx: LayoutContext<()>,
    /// By font blob and collection index, then the `wght` and `opsz` of a variable instance.
    typefaces: HashMap<(u64, u32, u32, u32), Typeface>,
    /// How editable text is adjusted to match drawn text, by font size and weight.
    adjustments: HashMap<(u32, Weight), Adjustment>,
}

impl Editing {
    fn new() -> Self {
        let mut editing = Self {
            font_cx: FontContext::new(),
            layout_cx: LayoutContext::new(),
            typefaces: HashMap::new(),
            adjustments: HashMap::new(),
        };
        let manager = FontMgr::new();
        let typeface = FAMILIES
            .iter()
            .find_map(|family| manager.match_family_style(family, FontStyle::normal()));
        if let Some(typeface) = typeface
            && let Some((bytes, index)) = typeface.to_font_bytes()
        {
            let blob = Blob::from(bytes);
            let id = blob.id();
            editing.font_cx.collection.register_fonts(
                blob,
                Some(FontInfoOverride {
                    family_name: Some(EDIT_FAMILY),
                    ..Default::default()
                }),
            );
            // Drawing with the system's own typeface keeps its rendering, as on Skia's lines.
            editing.typefaces.insert((id, index, 0, 0), typeface);
        }
        editing
    }

    /// The typeface for a run of `font`: its instance at `weight` when Parley varied it, and
    /// at the optical size for `size`, as `TextField` asks Parley to shape with.
    fn typeface(&mut self, font: &FontData, weight: Option<f32>, size: f32) -> Option<Typeface> {
        let (blob, index) = (font.data.id(), font.index);
        let key = (blob, index, weight.map_or(0, f32::to_bits), size.to_bits());
        if let Some(typeface) = self.typefaces.get(&key) {
            return Some(typeface.clone());
        }
        let base = match self.typefaces.get(&(blob, index, 0, 0)) {
            Some(typeface) => typeface.clone(),
            None => {
                let data = Data::new_copy(font.data.data());
                let typeface = FontMgr::new().new_from_data(data, index)?;
                self.typefaces.insert((blob, index, 0, 0), typeface.clone());
                typeface
            }
        };
        let axes = base.variation_design_parameters().unwrap_or_default();
        let coordinates: Vec<Coordinate> = [(*b"wght", weight), (*b"opsz", Some(size))]
            .into_iter()
            .filter_map(|(tag, value)| {
                let axis = FourByteTag::new(u32::from_be_bytes(tag));
                let value = value.filter(|_| axes.iter().any(|known| known.tag == axis))?;
                Some(Coordinate { axis, value })
            })
            .collect();
        let arguments = FontArguments::new().set_variation_design_position(VariationPosition {
            coordinates: &coordinates,
        });
        let typeface = match coordinates.is_empty() {
            true => base,
            false => base.clone_with_arguments(&arguments).unwrap_or(base),
        };
        self.typefaces.insert(key, typeface.clone());
        Some(typeface)
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

    /// Parley styles for editable text in `style`, matching how `draw_line` sets it.
    pub fn edit_styles(&self, style: TextStyle) -> Vec<StyleProperty<'static, ()>> {
        let mut styles = edit_styles_unadjusted(style);
        styles.push(StyleProperty::LetterSpacing(self.adjustment(style).spacing));
        styles
    }

    /// How far below a line box's top to draw a Parley layout of `style`, so its baseline
    /// lands where `draw_line` puts it.
    pub fn edit_baseline_shift(&self, style: TextStyle) -> f32 {
        self.adjustment(style).shift
    }

    fn adjustment(&self, style: TextStyle) -> Adjustment {
        let key = (style.size.to_bits(), style.weight);
        if let Some(adjustment) = self.editing.borrow().adjustments.get(&key) {
            return *adjustment;
        }
        let adjustment = self.measure_adjustment(style);
        self.editing
            .borrow_mut()
            .adjustments
            .insert(key, adjustment);
        adjustment
    }

    /// Skia rounds each advance and then applies `tracking`, while Parley keeps advances
    /// fractional; and the two split a line box differently. Rather than reason about either,
    /// measure the same sample both ways and make up the difference.
    fn measure_adjustment(&self, style: TextStyle) -> Adjustment {
        let (drawn_width, drawn_baseline) =
            self.with_line(SPACING_SAMPLE, style, Color::BLACK, |line| {
                let top = (style.line - line.height()) / 2.0 - BASELINE_NUDGE;
                // Skia's paragraph paints on a whole-point baseline below its top.
                let baseline = (line.alphabetic_baseline() + 0.5).floor();
                (line.max_intrinsic_width(), top + baseline)
            });
        let (edited_width, edited_baseline) = self.edit(|fonts, layouts| {
            let mut builder = layouts.ranged_builder(fonts, SPACING_SAMPLE, 1.0, true);
            for property in edit_styles_unadjusted(style) {
                builder.push_default(property);
            }
            let mut layout = builder.build(SPACING_SAMPLE);
            layout.break_all_lines(None);
            (
                layout.full_width(),
                layout.first_baseline().unwrap_or_default(),
            )
        });
        Adjustment {
            spacing: (drawn_width - edited_width) / SPACING_SAMPLE.chars().count() as f32,
            shift: drawn_baseline - edited_baseline,
        }
    }

    /// Runs `edit` with Parley's contexts, for driving a `PlainEditor`.
    pub fn edit<R>(&self, edit: impl FnOnce(&mut FontContext, &mut LayoutContext<()>) -> R) -> R {
        let editing = &mut *self.editing.borrow_mut();
        edit(&mut editing.font_cx, &mut editing.layout_cx)
    }

    /// Paints a Parley layout with its top-left at `origin`, underlining the runs that ask for
    /// it (an input method's unconfirmed text).
    pub fn draw_layout(&self, canvas: &Canvas, layout: &Layout<()>, origin: Point, color: Color) {
        let editing = &mut *self.editing.borrow_mut();
        let mut paint = Paint::default();
        paint.set_anti_alias(true).set_color(color);
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                let weight = run
                    .synthesis()
                    .variation_settings()
                    .iter()
                    .find(|(tag, _)| tag.to_be_bytes() == *b"wght")
                    .map(|(_, value)| *value);
                let Some(typeface) = editing.typeface(run.font(), weight, run.font_size()) else {
                    continue;
                };
                let mut font = Font::from_typeface(typeface, run.font_size());
                font.set_subpixel(true);
                font.set_edging(Edging::AntiAlias);
                font.set_embolden(run.synthesis().embolden());
                let (glyphs, positions): (Vec<u16>, Vec<Point>) = glyph_run
                    .positioned_glyphs()
                    .map(|glyph| (glyph.id as u16, Point::new(glyph.x, glyph.y)))
                    .unzip();
                canvas.draw_glyphs_at(&glyphs, positions.as_slice(), origin, &font, &paint);
                if let Some(underline) = &glyph_run.style().underline {
                    let metrics = run.font_metrics();
                    let offset = underline.offset.unwrap_or(metrics.underline_offset);
                    let size = underline.size.unwrap_or(metrics.underline_size).max(1.0);
                    let top = origin.y + glyph_run.baseline() - offset;
                    let left = origin.x + glyph_run.offset();
                    canvas.draw_rect(
                        Rect::from_xywh(left, top, glyph_run.advance(), size),
                        &paint,
                    );
                }
            }
        }
    }
}

fn edit_styles_unadjusted(style: TextStyle) -> Vec<StyleProperty<'static, ()>> {
    // CoreText picks San Francisco's optical size from the point size; Parley does not.
    let optical = FontVariation {
        tag: Tag::new(b"opsz"),
        value: style.size,
    };
    vec![
        StyleProperty::FontFamily(FontFamily::List(EDIT_FAMILIES.into())),
        StyleProperty::FontSize(style.size),
        StyleProperty::FontWeight(FontWeight::new(style.weight as i32 as f32)),
        StyleProperty::FontVariations(FontVariations::List(vec![optical].into())),
        StyleProperty::LineHeight(LineHeight::Absolute(style.line)),
    ]
}
