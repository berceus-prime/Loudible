//! Node types, and which keys each one accepts. The tables here are what turns a misplaced prop
//! into an error with a path instead of a field nobody reads.

/// The built-in node types. Anything else in a node's `type` names a component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeType {
    // Containers
    Stack,
    Bar,
    Grid,
    Layer,
    Scroll,
    Viewport,
    Box,
    // Paint leaves
    Text,
    Field,
    Symbol,
    Image,
    Ellipse,
    Arc,
    Path,
    Line,
    Spinner,
    Spacer,
    Host,
}

impl NodeType {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "stack" => Self::Stack,
            "bar" => Self::Bar,
            "grid" => Self::Grid,
            "layer" => Self::Layer,
            "scroll" => Self::Scroll,
            "viewport" => Self::Viewport,
            "box" => Self::Box,
            "text" => Self::Text,
            "field" => Self::Field,
            "symbol" => Self::Symbol,
            "image" => Self::Image,
            "ellipse" => Self::Ellipse,
            "arc" => Self::Arc,
            "path" => Self::Path,
            "line" => Self::Line,
            "spinner" => Self::Spinner,
            "spacer" => Self::Spacer,
            "host" => Self::Host,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Stack => "stack",
            Self::Bar => "bar",
            Self::Grid => "grid",
            Self::Layer => "layer",
            Self::Scroll => "scroll",
            Self::Viewport => "viewport",
            Self::Box => "box",
            Self::Text => "text",
            Self::Field => "field",
            Self::Symbol => "symbol",
            Self::Image => "image",
            Self::Ellipse => "ellipse",
            Self::Arc => "arc",
            Self::Path => "path",
            Self::Line => "line",
            Self::Spinner => "spinner",
            Self::Spacer => "spacer",
            Self::Host => "host",
        }
    }

    /// Whether children mean anything here. An `image`'s children are its fallback, painted
    /// while the source has not loaded.
    pub fn takes_children(self) -> bool {
        matches!(
            self,
            Self::Stack
                | Self::Bar
                | Self::Grid
                | Self::Layer
                | Self::Scroll
                | Self::Viewport
                | Self::Box
                | Self::Image
        )
    }

    /// Whether this node lays its children out by `place` rather than along an axis.
    pub fn places_children(self) -> bool {
        matches!(self, Self::Layer | Self::Viewport | Self::Box | Self::Image)
    }

    /// The props this type accepts on top of [`BOX_PROPS`].
    pub fn extra_props(self) -> &'static [&'static str] {
        match self {
            Self::Stack => &["axis", "gap", "align", "justify", "wrap"],
            Self::Bar => &["gap", "align"],
            Self::Grid => &["min_cell", "gap", "row_gap", "cell_ratio"],
            Self::Scroll => &["axis", "offset", "scrollbar"],
            Self::Viewport => &["content", "origin", "background"],
            Self::Layer | Self::Box => &[],
            Self::Text => &[
                "value",
                "style",
                "align",
                "wrap",
                "max_lines",
                "truncate",
                "limit",
            ],
            Self::Field => &[
                "binding",
                "placeholder",
                "placeholder_color",
                "style",
                "selection_color",
                "caret_color",
                "caret_blink_ms",
            ],
            Self::Symbol => &["name"],
            Self::Image => &["source", "fit", "sampling"],
            Self::Ellipse => &[],
            Self::Arc => &[
                "start_deg",
                "sweep_deg",
                "progress",
                "min_progress",
                "track",
            ],
            Self::Path => &["grid", "points", "commands"],
            Self::Line => &["from", "to"],
            Self::Spinner => &["spokes", "period_ms"],
            Self::Spacer => &[],
            Self::Host => &["name"],
        }
    }
}

/// Legal on every node. Mirrors the "Box properties" table in `SCHEMA.md`.
pub const BOX_PROPS: &[&str] = &[
    "size",
    "place",
    "anchor",
    "padding",
    "margin",
    "inset",
    "align_self",
    "slot",
    "grow",
    "fill",
    "color",
    "stroke",
    "radius",
    "radius_rule",
    "shadow",
    "material",
    "opacity",
    "blend",
    "isolate",
    "clip",
    "hit",
    "z",
];

/// The keys a node object itself may carry.
pub const NODE_KEYS: &[&str] = &[
    "$comment",
    "type",
    "id",
    "when",
    "repeat",
    "props",
    "variants",
    "children",
];

pub const NODE_TYPES: &[&str] = &[
    "stack",
    "bar",
    "grid",
    "layer",
    "scroll",
    "viewport",
    "box",
    "text",
    "field",
    "symbol",
    "image",
    "ellipse",
    "arc",
    "path",
    "line",
    "spinner",
    "spacer",
    "host",
];
