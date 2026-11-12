//! The tree layout and paint actually walk: every token looked up, every binding read, every
//! variant merged, every `repeat` expanded. No JSON and no conditions survive this far.

use serde_json::Value;
use skia_safe::Color;

use crate::document::{Align, Axis, Blend, Cap, Fit, Join, Justify, Sampling, Slot, StrokeAlign, Truncate};
use crate::kind::NodeType;
use crate::ports::TextStyle;
use crate::tokens::Fill;

#[derive(Clone, Debug)]
pub struct Resolved {
    pub kind: NodeType,
    pub id: Option<String>,
    pub style: Style,
    pub content: Content,
}

impl Resolved {
    pub fn children(&self) -> &[Resolved] {
        match &self.content {
            Content::Children(children) => children,
            Content::Symbol(symbol) => &symbol.children,
            Content::Image(image) => &image.fallback,
            _ => &[],
        }
    }
}

/// How one axis of a box is sized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dim {
    Fixed(f32),
    /// As large as the content needs.
    Hug,
    Fill {
        min: Option<f32>,
        max: Option<f32>,
    },
    Fraction(f32),
    /// A multiple of this box's other axis.
    Ratio(f32),
}

impl Dim {
    /// Applies `min`/`max` written beside the dimension, keeping any the dimension already
    /// carried: `{ "fill": { "max": 452 }, "min": 120 }` means both.
    pub fn clamped(self, min: Option<f32>, max: Option<f32>) -> Self {
        match self {
            Self::Fixed(value) => Self::Fixed(clamp(value, min, max)),
            Self::Fill {
                min: own_min,
                max: own_max,
            } => Self::Fill {
                min: own_min.or(min),
                max: own_max.or(max),
            },
            other => other,
        }
    }
}

pub fn clamp(value: f32, min: Option<f32>, max: Option<f32>) -> f32 {
    let value = min.map_or(value, |min| value.max(min));
    max.map_or(value, |max| value.min(max))
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Insets {
    pub fn uniform(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

/// Offsets from the parent's content box. `None` on an axis means "wherever the parent puts it".
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Place {
    pub left: Option<f32>,
    pub right: Option<f32>,
    pub top: Option<f32>,
    pub bottom: Option<f32>,
    pub center_x: Option<f32>,
    pub center_y: Option<f32>,
}

impl Place {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Overlay placement: at a point, kept inside the window, flipped rather than hanging off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnchorAt {
    pub point: (f32, f32),
    pub align: Align,
    pub flip: bool,
    pub margin: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Radii {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
    /// Half the shorter side, worked out once the box is known.
    pub capsule: bool,
    /// AppKit's control rule, likewise.
    pub control: bool,
}

impl Radii {
    pub fn uniform(value: f32) -> Self {
        Self {
            top_left: value,
            top_right: value,
            bottom_right: value,
            bottom_left: value,
            capsule: false,
            control: false,
        }
    }

    pub fn is_zero(&self) -> bool {
        !self.capsule
            && !self.control
            && self.top_left == 0.0
            && self.top_right == 0.0
            && self.bottom_right == 0.0
            && self.bottom_left == 0.0
    }

    /// The corner radii for a box of this size, applying the capsule and control rules.
    pub fn for_box(&self, width: f32, height: f32) -> [f32; 4] {
        if self.capsule {
            let radius = width.min(height) / 2.0;
            return [radius; 4];
        }
        if self.control {
            let radius = if height >= 28.0 {
                height / 2.0
            } else {
                height / 3.0
            };
            return [radius; 4];
        }
        [
            self.top_left,
            self.top_right,
            self.bottom_right,
            self.bottom_left,
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeStyle {
    pub color: Color,
    pub width: f32,
    pub align: StrokeAlign,
    pub cap: Cap,
    pub join: Join,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowStyle {
    pub color: Color,
    pub blur: f32,
    pub dx: f32,
    pub dy: f32,
    pub knockout: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialStyle {
    pub blur: f32,
    pub tint: Option<Color>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarStyle {
    pub width: f32,
    pub inset: f32,
    pub min_length: f32,
    pub color: Color,
}

/// A named region, in paint order. Behaviour stays with the host: this is only a rect and a name.
#[derive(Clone, Debug, PartialEq)]
pub struct HitStyle {
    pub action: String,
    pub args: Value,
    pub secondary: Option<String>,
    pub secondary_args: Value,
    pub tooltip: Option<String>,
    pub text_cursor: bool,
    pub blocks: bool,
}

#[derive(Clone, Debug)]
pub struct Style {
    pub width: Option<Dim>,
    pub height: Option<Dim>,
    pub place: Place,
    pub anchor: Option<AnchorAt>,
    pub padding: Insets,
    pub margin: Insets,
    pub inset: Insets,
    pub align_self: Option<Align>,
    pub slot: Option<Slot>,
    pub grow: f32,

    pub fill: Option<Fill>,
    /// Foreground, inherited by descendants that set none of their own.
    pub color: Color,
    pub stroke: Option<StrokeStyle>,
    pub radius: Radii,
    pub shadows: Vec<ShadowStyle>,
    pub material: Option<MaterialStyle>,
    pub opacity: f32,
    pub blend: Blend,
    pub isolate: bool,
    pub clip: bool,
    pub hit: Option<HitStyle>,
    pub z: f32,

    pub axis: Axis,
    pub gap: f32,
    pub align: Option<Align>,
    pub justify: Justify,
    pub wrap: bool,
    pub min_cell: f32,
    pub row_gap: Option<f32>,
    pub cell_ratio: Option<f32>,
    pub offset: f32,
    pub scrollbar: Option<ScrollbarStyle>,
    pub content: Option<(Option<Dim>, Option<Dim>)>,
    pub origin: (f32, f32),
    pub background: Option<Color>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            place: Place::default(),
            anchor: None,
            padding: Insets::default(),
            margin: Insets::default(),
            inset: Insets::default(),
            align_self: None,
            slot: None,
            grow: 0.0,
            fill: None,
            color: Color::BLACK,
            stroke: None,
            radius: Radii::default(),
            shadows: vec![],
            material: None,
            opacity: 1.0,
            blend: Blend::SrcOver,
            isolate: false,
            clip: false,
            hit: None,
            z: 0.0,
            axis: Axis::Y,
            gap: 0.0,
            align: None,
            justify: Justify::Start,
            wrap: false,
            min_cell: 0.0,
            row_gap: None,
            cell_ratio: None,
            offset: 0.0,
            scrollbar: None,
            content: None,
            origin: (0.0, 0.0),
            background: None,
        }
    }
}

// MARK: Content

#[derive(Clone, Debug)]
pub enum Content {
    Children(Vec<Resolved>),
    Text(TextContent),
    Field(FieldContent),
    Symbol(SymbolContent),
    Image(ImageContent),
    Ellipse,
    Arc(ArcContent),
    Path(PathContent),
    Line {
        from: (f32, f32),
        to: (f32, f32),
    },
    Spinner(SpinnerContent),
    Spacer,
    Host(String),
}

#[derive(Clone, Debug)]
pub struct TextContent {
    pub value: String,
    pub style: TextStyle,
    pub align: Align,
    pub wrap: bool,
    pub max_lines: Option<usize>,
    pub truncate: Truncate,
    pub limit: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct FieldContent {
    pub binding: String,
    pub style: TextStyle,
    pub placeholder: String,
    pub placeholder_color: Color,
    pub selection_color: Color,
    pub caret_color: Color,
    pub focused: bool,
    pub caret_visible: bool,
}

#[derive(Clone, Debug)]
pub struct SymbolContent {
    pub name: String,
    pub grid: f32,
    pub children: Vec<Resolved>,
}

#[derive(Clone, Debug)]
pub struct ImageContent {
    pub source: Option<String>,
    pub fit: Fit,
    pub sampling: Sampling,
    pub fallback: Vec<Resolved>,
}

#[derive(Clone, Copy, Debug)]
pub struct ArcContent {
    pub start_deg: f32,
    pub sweep_deg: f32,
    pub track: Option<StrokeStyle>,
}

#[derive(Clone, Debug)]
pub struct PathContent {
    /// Coordinates are in this grid, scaled to the node's box. `None` uses the box directly.
    pub grid: Option<f32>,
    pub commands: Vec<Segment>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    Move((f32, f32)),
    Line((f32, f32)),
    Cubic([(f32, f32); 3]),
    Close,
}

#[derive(Clone, Copy, Debug)]
pub struct SpinnerContent {
    pub spokes: usize,
    pub period_ms: f32,
}
