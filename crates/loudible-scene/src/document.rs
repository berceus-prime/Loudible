//! The schema as Rust types. One `Props` struct carries every property; which of them a given
//! node type accepts is checked in `validate`, so a misplaced prop is an error with a path
//! rather than a silently ignored field.
//!
//! See `SCHEMA.md` for what each of these means.

use std::collections::BTreeMap;

use serde::Deserialize;

pub const VERSION: &str = "loudible.scene/1";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    #[serde(rename = "$comment", default)]
    pub comment: Option<String>,
    pub schema: String,
    #[serde(default)]
    pub tokens: Tokens,
    #[serde(default)]
    pub symbols: BTreeMap<String, Symbol>,
    #[serde(default)]
    pub components: BTreeMap<String, Component>,
    pub root: Node,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tokens {
    #[serde(rename = "$comment", default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub colors: BTreeMap<String, TokenColor>,
    #[serde(default)]
    pub text: BTreeMap<String, TextStyleLiteral>,
    #[serde(default)]
    pub metrics: BTreeMap<String, f32>,
}

/// One colour, or the pair an appearance picks from.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum TokenColor {
    Both(String),
    Pair {
        light: String,
        dark: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextStyleLiteral {
    pub size: f32,
    pub line: f32,
    #[serde(default)]
    pub weight: Weight,
    #[serde(default)]
    pub rounded: bool,
    #[serde(default)]
    pub tabular: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Weight {
    #[default]
    Regular = 400,
    Medium = 500,
    Semibold = 600,
    Bold = 700,
}

/// A reusable vector drawing on its own grid, scaled to the `symbol` node's box.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Symbol {
    #[serde(rename = "$comment", default)]
    pub comment: Option<String>,
    #[serde(default = "default_grid")]
    pub grid: f32,
    #[serde(default)]
    pub props: Props,
    pub children: Vec<Node>,
}

fn default_grid() -> f32 {
    24.0
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    #[serde(rename = "$comment", default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub params: Vec<Param>,
    pub body: Box<Node>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Param {
    pub name: String,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    #[serde(rename = "$comment", default)]
    pub comment: Option<String>,
    /// A node type, or the name of a component.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub when: Option<Condition>,
    #[serde(default)]
    pub repeat: Option<Repeat>,
    #[serde(default)]
    pub props: Props,
    #[serde(default)]
    pub variants: Vec<Variant>,
    #[serde(default)]
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repeat {
    #[serde(rename = "for")]
    pub binding: String,
    #[serde(rename = "in")]
    pub source: String,
    #[serde(default)]
    pub index: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub when: Condition,
    pub props: Props,
}

/// Every property in the schema. All optional: `validate` decides which are legal where, and
/// `resolve` merges a node's own props with the variants that apply.
///
/// Unknown keys are collected into `params`, because a component instance carries its arguments
/// as props. Typos are caught by the structural pass, which runs before this is deserialised and
/// knows what each node type accepts.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Props {
    // Box
    pub size: Option<SizeSpec>,
    pub place: Option<Place>,
    pub anchor: Option<Anchor>,
    pub padding: Option<Edges>,
    pub margin: Option<Edges>,
    pub inset: Option<Edges>,
    pub align_self: Option<Align>,
    pub slot: Option<Slot>,
    pub grow: Option<f32>,
    pub fill: Option<ColorSpec>,
    pub color: Option<ColorSpec>,
    pub stroke: Option<Stroke>,
    pub radius: Option<Radius>,
    pub radius_rule: Option<RadiusRule>,
    pub shadow: Option<Shadows>,
    pub material: Option<Material>,
    pub opacity: Option<NumberSpec>,
    pub blend: Option<Blend>,
    pub isolate: Option<bool>,
    pub clip: Option<bool>,
    pub hit: Option<Hit>,
    pub z: Option<f32>,

    // stack, bar, grid
    pub axis: Option<Axis>,
    pub gap: Option<f32>,
    pub align: Option<Align>,
    pub justify: Option<Justify>,
    pub wrap: Option<bool>,
    pub min_cell: Option<f32>,
    pub row_gap: Option<f32>,
    pub cell_ratio: Option<f32>,

    // scroll
    pub offset: Option<NumberSpec>,
    pub scrollbar: Option<Scrollbar>,

    // viewport
    pub content: Option<SizeSpec>,
    pub origin: Option<Origin>,
    pub background: Option<ColorSpec>,

    // text, field
    pub value: Option<StringSpec>,
    pub style: Option<TextStyleSpec>,
    pub max_lines: Option<usize>,
    pub truncate: Option<Truncate>,
    pub limit: Option<f32>,
    pub binding: Option<String>,
    pub placeholder: Option<StringSpec>,
    pub placeholder_color: Option<ColorSpec>,
    pub selection_color: Option<ColorSpec>,
    pub caret_color: Option<ColorSpec>,
    pub caret_blink_ms: Option<f32>,

    // symbol, host
    pub name: Option<StringSpec>,

    // image
    pub source: Option<StringSpec>,
    pub fit: Option<Fit>,
    pub sampling: Option<Sampling>,

    // arc
    pub start_deg: Option<f32>,
    pub sweep_deg: Option<NumberSpec>,
    pub progress: Option<NumberSpec>,
    pub min_progress: Option<f32>,
    pub track: Option<Stroke>,

    // path, line
    pub grid: Option<f32>,
    pub points: Option<Vec<Point>>,
    pub commands: Option<Vec<PathCommand>>,
    pub from: Option<Point>,
    pub to: Option<Point>,

    // spinner
    pub spokes: Option<usize>,
    pub period_ms: Option<f32>,

    /// A component instance's arguments, by parameter name.
    #[serde(flatten)]
    pub params: BTreeMap<String, serde_json::Value>,
}

/// Copies every set field of `$source` over `$target`, which is what applying a variant means.
macro_rules! overlay {
    ($target:expr, $source:expr, [$($field:ident),* $(,)?]) => {
        $(if $source.$field.is_some() {
            $target.$field = $source.$field.clone();
        })*
    };
}

impl Props {
    /// This node's props with `other`'s set fields laid over them, in variant order.
    pub fn overlaid(&self, other: &Props) -> Props {
        let mut merged = self.clone();
        overlay!(merged, other, [
            size, place, anchor, padding, margin, inset, align_self, slot, grow,
            fill, color, stroke, radius, radius_rule, shadow, material, opacity, blend,
            isolate, clip, hit, z,
            axis, gap, align, justify, wrap, min_cell, row_gap, cell_ratio,
            offset, scrollbar, content, origin, background,
            value, style, max_lines, truncate, limit,
            binding, placeholder, placeholder_color, selection_color, caret_color, caret_blink_ms,
            name, source, fit, sampling,
            start_deg, sweep_deg, progress, min_progress, track,
            grid, points, commands, from, to, spokes, period_ms,
        ]);
        for (key, value) in &other.params {
            merged.params.insert(key.clone(), value.clone());
        }
        merged
    }

    /// Shadows in paint order, however the document spelled them.
    pub fn shadows(&self) -> &[Shadow] {
        match &self.shadow {
            Some(Shadows::One(shadow)) => std::slice::from_ref(shadow),
            Some(Shadows::Many(shadows)) => shadows,
            None => &[],
        }
    }
}

// MARK: Enumerations

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    #[default]
    Y,
}

/// Cross-axis alignment. `leading` and `trailing` are accepted as `start` and `end`, so one
/// enum serves stacks, text and anchors.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
    Leading,
    Trailing,
}

impl Align {
    /// `start`, `center` or `end`, with `stretch` reported separately by the caller.
    pub fn fraction(self) -> f32 {
        match self {
            Self::Start | Self::Leading | Self::Stretch => 0.0,
            Self::Center => 0.5,
            Self::End | Self::Trailing => 1.0,
        }
    }

    pub fn is_stretch(self) -> bool {
        self == Self::Stretch
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Slot {
    Leading,
    Center,
    Trailing,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Blend {
    #[default]
    SrcOver,
    DstOut,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RadiusRule {
    #[default]
    Fixed,
    /// AppKit's button rule: a capsule at 28pt and above, a third of the height below.
    Control,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Truncate {
    #[default]
    None,
    Ellipsis,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    #[default]
    Cover,
    Contain,
    Fill,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Sampling {
    #[default]
    Linear,
    Nearest,
    Mipmap,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StrokeAlign {
    #[default]
    Center,
    Inside,
    Outside,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Cap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Join {
    #[default]
    Miter,
    Round,
    Bevel,
}

// MARK: Sizing

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum SizeSpec {
    /// A square box, which is how `symbol` and `spinner` are sized.
    Square(NumberSpec),
    Axes(Axes),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Axes {
    #[serde(default)]
    pub width: Option<Dimension>,
    #[serde(default)]
    pub height: Option<Dimension>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Dimension {
    Fixed(NumberSpec),
    Keyword(Keyword),
    Spec(DimSpec),
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Keyword {
    Hug,
    Fill,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimSpec {
    pub fixed: Option<NumberSpec>,
    pub hug: Option<bool>,
    pub fill: Option<FillSpec>,
    pub fraction: Option<f32>,
    pub ratio: Option<f32>,
    /// Measured text width, for a box sized to its label.
    pub text: Option<StringSpec>,
    pub style: Option<TextStyleSpec>,
    pub plus: Option<f32>,
    pub min: Option<f32>,
    pub max: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum FillSpec {
    All(bool),
    Clamped {
        #[serde(default)]
        min: Option<f32>,
        #[serde(default)]
        max: Option<f32>,
    },
}

// MARK: Geometry

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Place {
    pub left: Option<f32>,
    pub right: Option<f32>,
    pub top: Option<f32>,
    pub bottom: Option<f32>,
    pub center_x: Option<f32>,
    pub center_y: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    #[serde(default)]
    pub to: Option<Point>,
    #[serde(default)]
    pub align: Option<Align>,
    #[serde(default = "yes")]
    pub flip: bool,
    #[serde(default = "default_margin")]
    pub margin: f32,
}

fn yes() -> bool {
    true
}

fn default_margin() -> f32 {
    8.0
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Edges {
    Uniform(f32),
    /// Two values are vertical then horizontal; four are top, right, bottom, left.
    List(Vec<f32>),
    Sides(Sides),
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sides {
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
    pub left: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Radius {
    Fixed(f32),
    /// Only `"capsule"`.
    Keyword(String),
    Corners(Corners),
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Corners {
    pub tl: Option<f32>,
    pub tr: Option<f32>,
    pub br: Option<f32>,
    pub bl: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Point {
    Pair([NumberSpec; 2]),
    Named(NamedPoint),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedPoint {
    #[serde(default)]
    pub x: Option<NumberSpec>,
    #[serde(default)]
    pub y: Option<NumberSpec>,
    /// A bound point, such as a menu's anchor.
    #[serde(default, rename = "$")]
    pub path: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    #[serde(default)]
    pub x: Option<NumberSpec>,
    #[serde(default)]
    pub y: Option<NumberSpec>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum PathCommand {
    Move {
        #[serde(rename = "move")]
        to: Point,
    },
    Line {
        #[serde(rename = "line")]
        to: Point,
    },
    Cubic {
        cubic: [Point; 3],
    },
    Close {
        close: bool,
    },
}

// MARK: Paint

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum ColorSpec {
    /// A token name, or a literal `#RRGGBB` / `#AARRGGBB`.
    Named(String),
    Computed(Box<ComputedColor>),
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputedColor {
    pub token: Option<String>,
    pub hex: Option<String>,
    /// Multiplies the alpha, as `theme::faded` does.
    pub opacity: Option<NumberSpec>,
    #[serde(rename = "$")]
    pub path: Option<String>,
    pub mix: Option<(ColorSpec, ColorSpec, f32)>,
    pub gradient: Option<Gradient>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gradient {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    pub stops: Vec<ColorSpec>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    pub color: ColorSpec,
    #[serde(default = "one")]
    pub width: f32,
    #[serde(default)]
    pub align: StrokeAlign,
    #[serde(default)]
    pub cap: Cap,
    #[serde(default)]
    pub join: Join,
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Shadows {
    One(Shadow),
    Many(Vec<Shadow>),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shadow {
    pub color: ColorSpec,
    #[serde(default)]
    pub blur: f32,
    #[serde(default)]
    pub dx: f32,
    #[serde(default)]
    pub dy: f32,
    /// Clips the shape out from under its own shadow, so a translucent fill does not darken.
    #[serde(default = "yes")]
    pub knockout: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    #[serde(default = "default_blur")]
    pub blur: f32,
    #[serde(default)]
    pub tint: Option<ColorSpec>,
}

fn default_blur() -> f32 {
    24.0
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scrollbar {
    #[serde(default = "default_bar_width")]
    pub width: f32,
    #[serde(default = "default_bar_inset")]
    pub inset: f32,
    #[serde(default = "default_bar_min")]
    pub min_length: f32,
    #[serde(default)]
    pub color: Option<ColorSpec>,
    #[serde(default)]
    pub opacity: Option<NumberSpec>,
}

fn default_bar_width() -> f32 {
    6.0
}

fn default_bar_inset() -> f32 {
    2.0
}

fn default_bar_min() -> f32 {
    32.0
}

// MARK: Text

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum TextStyleSpec {
    Named(String),
    Spec(StyleOverrides),
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleOverrides {
    pub style: Option<String>,
    pub size: Option<NumberSpec>,
    pub line: Option<NumberSpec>,
    pub weight: Option<Weight>,
    pub rounded: Option<bool>,
    pub tabular: Option<bool>,
}

// MARK: Values

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum StringSpec {
    Literal(String),
    Bound(Box<BoundString>),
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundString {
    #[serde(rename = "$")]
    pub path: Option<String>,
    #[serde(rename = "else")]
    pub fallback: Option<String>,
    /// `"Audible {marketplace.name}"`.
    pub format: Option<String>,
    /// A count to choose between `zero`, `one` and `many`, where `{n}` is the count.
    pub count: Option<String>,
    pub zero: Option<String>,
    pub one: Option<String>,
    pub many: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum NumberSpec {
    Literal(f32),
    Bound(BoundNumber),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundNumber {
    #[serde(rename = "$")]
    pub path: String,
    #[serde(rename = "else", default)]
    pub fallback: Option<f32>,
}

// MARK: Interaction

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hit {
    pub action: Option<String>,
    #[serde(default)]
    pub args: BTreeMap<String, serde_json::Value>,
    pub secondary: Option<String>,
    #[serde(default)]
    pub secondary_args: BTreeMap<String, serde_json::Value>,
    pub tooltip: Option<StringSpec>,
    #[serde(default)]
    pub text_cursor: bool,
    /// Swallows the click, for modal backdrops and disabled controls.
    #[serde(default)]
    pub blocks: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Condition {
    /// A state flag, true when set and truthy.
    Flag(String),
    Always(bool),
    Expr(Box<CondExpr>),
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CondExpr {
    pub not: Option<Condition>,
    pub all: Option<Vec<Condition>>,
    pub any: Option<Vec<Condition>>,
    pub eq: Option<[serde_json::Value; 2]>,
    pub gt: Option<[serde_json::Value; 2]>,
    /// True when the named value is absent, empty or zero.
    pub empty: Option<String>,
}
