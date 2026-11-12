//! One pass from the document to the tree layout and paint walk.
//!
//! `repeat` expands, components and symbols are instantiated with their arguments bound, `when`
//! prunes, `variants` merge in order, bindings and tokens are read, and foreground colour is
//! threaded down. Everything after this is arithmetic and Skia calls.

use std::borrow::Cow;
use std::time::Duration;

use serde_json::Value;
use skia_safe::Color;

use crate::document::{
    Align, Axis, Blend, Corners, Dimension, Document, Edges, FillSpec, Keyword, Node, PathCommand,
    Point, Props, Radius, RadiusRule, SizeSpec, Truncate,
};
use crate::kind::NodeType;
use crate::ports::TextEngine;
use crate::resolved::{
    AnchorAt, ArcContent, Content, Dim, FieldContent, HitStyle, ImageContent, Insets,
    MaterialStyle, PathContent, Place, Radii, Resolved, ScrollbarStyle, Segment, ShadowStyle,
    SpinnerContent, Style, StrokeStyle, SymbolContent, TextContent,
};
use crate::scope::{Locals, Scope};
use crate::tokens::Palette;

/// A named action with its arguments, for comparing against what is hovered or pressed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionRef {
    pub action: String,
    pub args: Value,
}

impl ActionRef {
    pub fn new(action: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            args: Value::Null,
        }
    }

    pub fn with_args(action: impl Into<String>, args: Value) -> Self {
        Self {
            action: action.into(),
            args,
        }
    }
}

pub struct Resolver<'a> {
    pub document: &'a Document,
    pub palette: Palette<'a>,
    pub text: &'a dyn TextEngine,
    pub hot: Option<&'a ActionRef>,
    pub pressed: Option<&'a ActionRef>,
    pub focused_field: Option<&'a str>,
    pub time: Duration,
}

impl<'a> Resolver<'a> {
    /// The root node, or `None` when its own `when` excludes it.
    pub fn root(&self, data: &Value, environment: &Locals<'_>) -> Option<Resolved> {
        let scope = Scope::new(data, environment);
        let mut out = vec![];
        self.node(&self.document.root, scope, self.palette.default_color(), &mut out);
        out.into_iter().next()
    }

    /// Appends however many nodes this one becomes: none when pruned, several under a `repeat`.
    fn node(&self, node: &Node, scope: Scope<'_>, inherited: Color, out: &mut Vec<Resolved>) {
        if let Some(repeat) = &node.repeat {
            let Some(Value::Array(items)) = scope.lookup(repeat.source.trim_start_matches('$'))
            else {
                return;
            };
            for (index, item) in items.iter().enumerate() {
                let mut locals = scope.locals.child();
                locals.bind(repeat.binding.clone(), Cow::Borrowed(item));
                if let Some(name) = &repeat.index {
                    locals.bind_owned(name.clone(), Value::from(index));
                }
                let inner = scope.with(&locals);
                let mut once = node.clone();
                once.repeat = None;
                self.node(&once, inner, inherited, out);
            }
            return;
        }
        if let Some(when) = &node.when
            && !scope.test(when)
        {
            return;
        }

        // The interaction flags a variant may test come from the node's base action, as
        // `Ui::is_hot` reads the action a view registered before it styled itself.
        let base_hit = node.props.hit.as_ref().and_then(|hit| {
            hit.action.as_ref().map(|action| ActionRef::with_args(
                action.clone(),
                args_value(&hit.args, &scope),
            ))
        });
        let hovered = base_hit
            .as_ref()
            .is_some_and(|hit| self.hot == Some(hit));
        let pressed = hovered && base_hit.as_ref().is_some_and(|hit| self.pressed == Some(hit));

        let mut locals = scope.locals.child();
        if base_hit.is_some() {
            locals.bind_owned("hovered", Value::from(hovered));
            locals.bind_owned("pressed", Value::from(pressed));
        }
        let scope = scope.with(&locals);

        let mut props = node.props.clone();
        for variant in &node.variants {
            if scope.test(&variant.when) {
                props = props.overlaid(&variant.props);
            }
        }

        match NodeType::parse(&node.kind) {
            Some(kind) => {
                if let Some(resolved) = self.built_in(kind, node, &props, scope, inherited) {
                    out.push(resolved);
                }
            }
            None => self.component(node, &props, scope, inherited, out),
        }
    }

    fn built_in(
        &self,
        kind: NodeType,
        node: &Node,
        props: &Props,
        scope: Scope<'_>,
        inherited: Color,
    ) -> Option<Resolved> {
        let mut style = self.style(props, scope, inherited);
        let color = style.color;
        let content = match kind {
            NodeType::Text => Content::Text(TextContent {
                value: props
                    .value
                    .as_ref()
                    .map(|spec| scope.string(spec))
                    .unwrap_or_default(),
                style: self.palette.text_style(props.style.as_ref(), &scope),
                align: props.align.unwrap_or(Align::Leading),
                wrap: props.wrap.unwrap_or(false),
                max_lines: props.max_lines,
                truncate: props.truncate.unwrap_or(Truncate::None),
                limit: props.limit,
            }),
            NodeType::Field => {
                let binding = props.binding.clone().unwrap_or_default();
                let blink = props.caret_blink_ms.unwrap_or(600.0).max(1.0);
                Content::Field(FieldContent {
                    focused: self.focused_field == Some(binding.as_str()),
                    caret_visible: (self.time.as_millis() as f32 / blink) as u64 % 2 == 0,
                    binding,
                    style: self.palette.text_style(props.style.as_ref(), &scope),
                    placeholder: props
                        .placeholder
                        .as_ref()
                        .map(|spec| scope.string(spec))
                        .unwrap_or_default(),
                    placeholder_color: self
                        .color_or(props.placeholder_color.as_ref(), &scope, color),
                    selection_color: self.color_or(
                        props.selection_color.as_ref(),
                        &scope,
                        crate::tokens::faded(color, 0.3),
                    ),
                    caret_color: self.color_or(props.caret_color.as_ref(), &scope, color),
                })
            }
            NodeType::Symbol => {
                let name = props
                    .name
                    .as_ref()
                    .map(|spec| scope.string(spec))
                    .unwrap_or_default();
                let symbol = self.document.symbols.get(&name)?;
                let size = match (&style.width, &style.height) {
                    (Some(Dim::Fixed(width)), _) => *width,
                    (_, Some(Dim::Fixed(height))) => *height,
                    _ => symbol.grid,
                };
                let mut locals = scope.locals.child();
                locals.bind_owned("color", Value::from(hex_of(color)));
                locals.bind_owned("size", Value::from(size));
                let inner = scope.with(&locals);
                // A symbol's own props are its drawing defaults: a stroke written once at the
                // top applies to every shape that does not bring its own, and an `isolate`
                // there starts the layer its knockouts cut into.
                style.isolate |= symbol.props.isolate.unwrap_or(false);
                let mut children = vec![];
                for child in &symbol.children {
                    let mut child = child.clone();
                    if child.props.stroke.is_none() {
                        child.props.stroke = symbol.props.stroke.clone();
                    }
                    self.node(&child, inner, color, &mut children);
                }
                Content::Symbol(SymbolContent {
                    name,
                    grid: symbol.grid,
                    children,
                })
            }
            NodeType::Image => Content::Image(ImageContent {
                source: props
                    .source
                    .as_ref()
                    .map(|spec| scope.string(spec))
                    .filter(|source| !source.is_empty()),
                fit: props.fit.unwrap_or_default(),
                sampling: props.sampling.unwrap_or_default(),
                fallback: self.resolve_children(node, scope, color),
            }),
            NodeType::Ellipse => Content::Ellipse,
            NodeType::Arc => {
                let sweep = match (&props.sweep_deg, &props.progress) {
                    (Some(sweep), _) => scope.number(sweep).unwrap_or(0.0),
                    (None, Some(progress)) => {
                        let value = scope.number(progress).unwrap_or(0.0);
                        let floor = props.min_progress.unwrap_or(0.02);
                        value.clamp(floor, 1.0) * 360.0
                    }
                    (None, None) => 0.0,
                };
                Content::Arc(ArcContent {
                    start_deg: props.start_deg.unwrap_or(-90.0),
                    sweep_deg: sweep,
                    track: props
                        .track
                        .as_ref()
                        .and_then(|stroke| self.stroke(stroke, &scope, color)),
                })
            }
            NodeType::Path => Content::Path(PathContent {
                grid: props.grid,
                commands: self.segments(props, &scope),
            }),
            NodeType::Line => Content::Line {
                from: props
                    .from
                    .as_ref()
                    .map(|point| self.point(point, &scope))
                    .unwrap_or((0.0, 0.0)),
                to: props
                    .to
                    .as_ref()
                    .map(|point| self.point(point, &scope))
                    .unwrap_or((0.0, 0.0)),
            },
            NodeType::Spinner => Content::Spinner(SpinnerContent {
                spokes: props.spokes.unwrap_or(12).max(1),
                period_ms: props.period_ms.unwrap_or(1000.0).max(1.0),
            }),
            NodeType::Spacer => Content::Spacer,
            NodeType::Host => Content::Host(
                props
                    .name
                    .as_ref()
                    .map(|spec| scope.string(spec))
                    .unwrap_or_default(),
            ),
            _ => Content::Children(self.resolve_children(node, scope, color)),
        };
        Some(Resolved {
            kind,
            id: node.id.clone(),
            style,
            content,
        })
    }

    fn resolve_children(&self, node: &Node, scope: Scope<'_>, color: Color) -> Vec<Resolved> {
        let mut children = vec![];
        for child in &node.children {
            self.node(child, scope, color, &mut children);
        }
        children
    }

    /// A component instance: its arguments become locals, and the box props on the instance are
    /// laid over the body's own, so a caller can place what it instantiates.
    fn component(
        &self,
        node: &Node,
        props: &Props,
        scope: Scope<'_>,
        inherited: Color,
        out: &mut Vec<Resolved>,
    ) {
        let Some(component) = self.document.components.get(&node.kind) else {
            return;
        };
        let mut locals = scope.locals.child();
        for param in &component.params {
            let given = props.params.get(&param.name);
            match given.or(param.default.as_ref()) {
                Some(Value::Object(object)) if object.contains_key("$") => {
                    let path = object["$"].as_str().unwrap_or_default();
                    match scope.lookup(path) {
                        Some(value) => locals.bind(param.name.clone(), Cow::Borrowed(value)),
                        None => locals.bind_owned(param.name.clone(), Value::Null),
                    }
                }
                Some(value) => locals.bind(param.name.clone(), Cow::Borrowed(value)),
                None => locals.bind_owned(param.name.clone(), Value::Null),
            }
        }
        let inner = scope.with(&locals);

        let mut body = (*component.body).clone();
        // Box props on the instance win, so `{ "type": "book-tile", "props": { "size": … } }`
        // sizes the tile rather than being ignored.
        body.props = body.props.overlaid(&box_props_only(props));
        self.node(&body, inner, inherited, out);
    }

    // MARK: Style

    fn style(&self, props: &Props, scope: Scope<'_>, inherited: Color) -> Style {
        let color = props
            .color
            .as_ref()
            .and_then(|spec| self.palette.color(spec, &scope))
            .unwrap_or(inherited);
        let (width, height) = self.size(props, &scope);
        Style {
            width,
            height,
            place: self.place(props),
            anchor: self.anchor(props, &scope),
            padding: edges(props.padding.as_ref()),
            margin: edges(props.margin.as_ref()),
            inset: edges(props.inset.as_ref()),
            align_self: props.align_self,
            slot: props.slot,
            grow: props.grow.unwrap_or(0.0),
            fill: props
                .fill
                .as_ref()
                .and_then(|spec| self.palette.fill(spec, &scope)),
            color,
            stroke: props
                .stroke
                .as_ref()
                .and_then(|stroke| self.stroke(stroke, &scope, color)),
            radius: self.radius(props),
            shadows: props
                .shadows()
                .iter()
                .filter_map(|shadow| {
                    Some(ShadowStyle {
                        color: self.palette.color(&shadow.color, &scope)?,
                        blur: shadow.blur,
                        dx: shadow.dx,
                        dy: shadow.dy,
                        knockout: shadow.knockout,
                    })
                })
                .collect(),
            material: props.material.as_ref().map(|material| MaterialStyle {
                blur: material.blur,
                tint: material
                    .tint
                    .as_ref()
                    .and_then(|tint| self.palette.color(tint, &scope)),
            }),
            opacity: props
                .opacity
                .as_ref()
                .and_then(|spec| scope.number(spec))
                .unwrap_or(1.0)
                .clamp(0.0, 1.0),
            blend: props.blend.unwrap_or(Blend::SrcOver),
            isolate: props.isolate.unwrap_or(false),
            clip: props.clip.unwrap_or(false),
            hit: self.hit(props, &scope),
            z: props.z.unwrap_or(0.0),
            axis: props.axis.unwrap_or(Axis::Y),
            gap: props.gap.unwrap_or(0.0),
            align: props.align,
            justify: props.justify.unwrap_or_default(),
            wrap: props.wrap.unwrap_or(false),
            min_cell: props.min_cell.unwrap_or(0.0),
            row_gap: props.row_gap,
            cell_ratio: props.cell_ratio,
            offset: props
                .offset
                .as_ref()
                .and_then(|spec| scope.number(spec))
                .unwrap_or(0.0),
            scrollbar: props.scrollbar.as_ref().map(|bar| {
                let base = bar
                    .color
                    .as_ref()
                    .and_then(|spec| self.palette.color(spec, &scope))
                    .unwrap_or(color);
                let opacity = bar
                    .opacity
                    .as_ref()
                    .and_then(|spec| scope.number(spec))
                    .unwrap_or(1.0);
                ScrollbarStyle {
                    width: bar.width,
                    inset: bar.inset,
                    min_length: bar.min_length,
                    color: crate::tokens::faded(base, opacity),
                }
            }),
            content: props
                .content
                .as_ref()
                .map(|size| self.dimensions(size, &scope)),
            origin: props
                .origin
                .as_ref()
                .map(|origin| {
                    (
                        origin.x.as_ref().and_then(|x| scope.number(x)).unwrap_or(0.0),
                        origin.y.as_ref().and_then(|y| scope.number(y)).unwrap_or(0.0),
                    )
                })
                .unwrap_or((0.0, 0.0)),
            background: props
                .background
                .as_ref()
                .and_then(|spec| self.palette.color(spec, &scope)),
        }
    }

    fn size(&self, props: &Props, scope: &Scope<'_>) -> (Option<Dim>, Option<Dim>) {
        match &props.size {
            Some(size) => self.dimensions(size, scope),
            None => (None, None),
        }
    }

    fn dimensions(&self, size: &SizeSpec, scope: &Scope<'_>) -> (Option<Dim>, Option<Dim>) {
        match size {
            SizeSpec::Square(value) => {
                let side = scope.number(value).map(Dim::Fixed);
                (side, side)
            }
            SizeSpec::Axes(axes) => (
                axes.width
                    .as_ref()
                    .and_then(|dimension| self.dimension(dimension, scope)),
                axes.height
                    .as_ref()
                    .and_then(|dimension| self.dimension(dimension, scope)),
            ),
        }
    }

    fn dimension(&self, dimension: &Dimension, scope: &Scope<'_>) -> Option<Dim> {
        match dimension {
            Dimension::Fixed(value) => scope.number(value).map(Dim::Fixed),
            Dimension::Keyword(Keyword::Hug) => Some(Dim::Hug),
            Dimension::Keyword(Keyword::Fill) => Some(Dim::Fill {
                min: None,
                max: None,
            }),
            Dimension::Spec(spec) => {
                let base = if let Some(fixed) = &spec.fixed {
                    Dim::Fixed(scope.number(fixed)?)
                } else if spec.hug == Some(true) {
                    Dim::Hug
                } else if let Some(fill) = &spec.fill {
                    match fill {
                        FillSpec::All(_) => Dim::Fill {
                            min: spec.min,
                            max: spec.max,
                        },
                        FillSpec::Clamped { min, max } => Dim::Fill {
                            min: min.or(spec.min),
                            max: max.or(spec.max),
                        },
                    }
                } else if let Some(fraction) = spec.fraction {
                    Dim::Fraction(fraction)
                } else if let Some(ratio) = spec.ratio {
                    Dim::Ratio(ratio)
                } else if let Some(text) = &spec.text {
                    // A box sized to its label, as every button width in the app is.
                    let style = self.palette.text_style(spec.style.as_ref(), scope);
                    let measured = self.text.width(&scope.string(text), &style);
                    Dim::Fixed(measured + spec.plus.unwrap_or(0.0))
                } else {
                    return None;
                };
                Some(base.clamped(spec.min, spec.max))
            }
        }
    }

    fn place(&self, props: &Props) -> Place {
        props.place.map_or_else(Place::default, |place| Place {
            left: place.left,
            right: place.right,
            top: place.top,
            bottom: place.bottom,
            center_x: place.center_x,
            center_y: place.center_y,
        })
    }

    fn anchor(&self, props: &Props, scope: &Scope<'_>) -> Option<AnchorAt> {
        let anchor = props.anchor.as_ref()?;
        Some(AnchorAt {
            point: anchor
                .to
                .as_ref()
                .map(|point| self.point(point, scope))
                .unwrap_or((0.0, 0.0)),
            align: anchor.align.unwrap_or(Align::Leading),
            flip: anchor.flip,
            margin: anchor.margin,
        })
    }

    fn radius(&self, props: &Props) -> Radii {
        let mut radii = match &props.radius {
            Some(Radius::Fixed(value)) => Radii::uniform(*value),
            Some(Radius::Keyword(_)) => Radii {
                capsule: true,
                ..Radii::default()
            },
            Some(Radius::Corners(Corners { tl, tr, br, bl })) => Radii {
                top_left: tl.unwrap_or(0.0),
                top_right: tr.unwrap_or(0.0),
                bottom_right: br.unwrap_or(0.0),
                bottom_left: bl.unwrap_or(0.0),
                capsule: false,
                control: false,
            },
            None => Radii::default(),
        };
        if props.radius_rule == Some(RadiusRule::Control) {
            radii.control = true;
        }
        radii
    }

    fn stroke(
        &self,
        stroke: &crate::document::Stroke,
        scope: &Scope<'_>,
        inherited: Color,
    ) -> Option<StrokeStyle> {
        Some(StrokeStyle {
            color: self
                .palette
                .color(&stroke.color, scope)
                .unwrap_or(inherited),
            width: stroke.width,
            align: stroke.align,
            cap: stroke.cap,
            join: stroke.join,
        })
    }

    fn color_or(
        &self,
        spec: Option<&crate::document::ColorSpec>,
        scope: &Scope<'_>,
        fallback: Color,
    ) -> Color {
        spec.and_then(|spec| self.palette.color(spec, scope))
            .unwrap_or(fallback)
    }

    fn hit(&self, props: &Props, scope: &Scope<'_>) -> Option<HitStyle> {
        let hit = props.hit.as_ref()?;
        if hit.action.is_none() && !hit.blocks {
            return None;
        }
        Some(HitStyle {
            action: hit.action.clone().unwrap_or_default(),
            args: args_value(&hit.args, scope),
            secondary: hit.secondary.clone(),
            secondary_args: args_value(&hit.secondary_args, scope),
            tooltip: hit
                .tooltip
                .as_ref()
                .map(|spec| scope.string(spec))
                .filter(|text| !text.is_empty()),
            text_cursor: hit.text_cursor,
            blocks: hit.blocks,
        })
    }

    fn point(&self, point: &Point, scope: &Scope<'_>) -> (f32, f32) {
        match point {
            Point::Pair([x, y]) => (
                scope.number(x).unwrap_or(0.0),
                scope.number(y).unwrap_or(0.0),
            ),
            Point::Named(named) => {
                if let Some(path) = &named.path
                    && let Some(value) = scope.lookup(path)
                {
                    let read = |key: &str, index: usize| {
                        value
                            .get(key)
                            .or_else(|| value.get(index))
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0) as f32
                    };
                    return (read("x", 0), read("y", 1));
                }
                (
                    named.x.as_ref().and_then(|x| scope.number(x)).unwrap_or(0.0),
                    named.y.as_ref().and_then(|y| scope.number(y)).unwrap_or(0.0),
                )
            }
        }
    }

    fn segments(&self, props: &Props, scope: &Scope<'_>) -> Vec<Segment> {
        if let Some(points) = &props.points {
            let mut segments = Vec::with_capacity(points.len());
            for (index, point) in points.iter().enumerate() {
                let at = self.point(point, scope);
                segments.push(if index == 0 {
                    Segment::Move(at)
                } else {
                    Segment::Line(at)
                });
            }
            return segments;
        }
        props
            .commands
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|command| match command {
                PathCommand::Move { to } => Segment::Move(self.point(to, scope)),
                PathCommand::Line { to } => Segment::Line(self.point(to, scope)),
                PathCommand::Cubic { cubic } => Segment::Cubic([
                    self.point(&cubic[0], scope),
                    self.point(&cubic[1], scope),
                    self.point(&cubic[2], scope),
                ]),
                PathCommand::Close { .. } => Segment::Close,
            })
            .collect()
    }
}

/// Only the props that position and decorate a box, for laying an instance over a component body.
fn box_props_only(props: &Props) -> Props {
    Props {
        size: props.size.clone(),
        place: props.place,
        anchor: props.anchor.clone(),
        margin: props.margin.clone(),
        align_self: props.align_self,
        slot: props.slot,
        grow: props.grow,
        opacity: props.opacity.clone(),
        z: props.z,
        hit: props.hit.clone(),
        ..Props::default()
    }
}

fn args_value(
    args: &std::collections::BTreeMap<String, Value>,
    scope: &Scope<'_>,
) -> Value {
    if args.is_empty() {
        return Value::Null;
    }
    let mut out = serde_json::Map::new();
    for (key, value) in args {
        let resolved = match value {
            Value::Object(object) if object.contains_key("$") => {
                let path = object["$"].as_str().unwrap_or_default();
                scope.lookup(path).cloned().unwrap_or(Value::Null)
            }
            other => other.clone(),
        };
        out.insert(key.clone(), resolved);
    }
    Value::Object(out)
}

fn edges(edges: Option<&Edges>) -> Insets {
    match edges {
        None => Insets::default(),
        Some(Edges::Uniform(value)) => Insets::uniform(*value),
        Some(Edges::List(values)) => match values.as_slice() {
            [vertical, horizontal] => Insets {
                top: *vertical,
                right: *horizontal,
                bottom: *vertical,
                left: *horizontal,
            },
            [top, right, bottom, left] => Insets {
                top: *top,
                right: *right,
                bottom: *bottom,
                left: *left,
            },
            [all] => Insets::uniform(*all),
            _ => Insets::default(),
        },
        Some(Edges::Sides(sides)) => Insets {
            top: sides.top.unwrap_or(0.0),
            right: sides.right.unwrap_or(0.0),
            bottom: sides.bottom.unwrap_or(0.0),
            left: sides.left.unwrap_or(0.0),
        },
    }
}

/// `#AARRGGBB`, so a symbol's `$color` reads back as a literal.
pub fn hex_of(color: Color) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        color.a(),
        color.r(),
        color.g(),
        color.b()
    )
}
