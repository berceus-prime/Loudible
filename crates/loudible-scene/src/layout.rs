//! Intrinsic sizes bottom-up, then absolute rects top-down.
//!
//! A node with no `size` of its own fills its parent if it is a container or draws in its own
//! coordinate space, and hugs its content if it is text, a field, a symbol or a spinner.

use skia_safe::{Rect, Size};

use crate::document::{Align, Axis, Justify, Slot};
use crate::kind::NodeType;
use crate::ports::TextEngine;
use crate::resolved::{Content, Dim, Insets, Resolved, Style, clamp};

/// A node with its box worked out, in paint order.
#[derive(Clone, Debug)]
pub struct Laid<'a> {
    pub node: &'a Resolved,
    pub rect: Rect,
    /// A symbol's children are laid out on its own grid; paint scales them into `rect`.
    pub grid: Option<f32>,
    pub children: Vec<Laid<'a>>,
}

pub struct Layout<'a> {
    pub text: &'a dyn TextEngine,
    /// The window, for keeping anchored overlays inside it.
    pub window: Size,
}

impl<'a> Layout<'a> {
    /// The root is sized like any other node, so a document that hugs its content produces a
    /// box the shell can measure rather than one stretched to the window.
    pub fn run<'n>(&self, node: &'n Resolved, bounds: Rect) -> Laid<'n> {
        let size = self.size_of(node, Size::new(bounds.width(), bounds.height()));
        let slot = Rect::from_xywh(bounds.left, bounds.top, size.width, size.height);
        self.place_node(node, slot, bounds)
    }

    /// `slot` is where the parent decided this node goes; `available` is what it may draw in.
    fn place_node<'n>(&self, node: &'n Resolved, slot: Rect, available: Rect) -> Laid<'n> {
        let rect = self.box_of(node, slot, available);
        let children = self.children(node, rect);
        Laid {
            node,
            rect,
            grid: match &node.content {
                Content::Symbol(symbol) => Some(symbol.grid),
                _ => None,
            },
            children,
        }
    }

    fn box_of(&self, node: &Resolved, slot: Rect, available: Rect) -> Rect {
        let style = &node.style;
        if let Some(anchor) = &style.anchor {
            let size = self.size_of(node, Size::new(available.width(), available.height()));
            let mut x = anchor.point.0 - size.width * anchor.align.fraction();
            let mut y = anchor.point.1;
            let margin = anchor.margin;
            x = clamp(
                x,
                Some(margin),
                Some((self.window.width - size.width - margin).max(margin)),
            );
            if anchor.flip && y + size.height > self.window.height - margin {
                y = (anchor.point.1 - size.height).max(margin);
            }
            return Rect::from_xywh(x, y, size.width, size.height);
        }
        if style.place.is_empty() {
            return slot;
        }
        // `place` measures from the parent's content box, and two opposing edges stretch.
        let measured = self.size_of(node, Size::new(available.width(), available.height()));
        let place = &style.place;
        let (left, width) = span(
            available.left,
            available.width(),
            place.left,
            place.right,
            place.center_x,
            measured.width,
        );
        let (top, height) = span(
            available.top,
            available.height(),
            place.top,
            place.bottom,
            place.center_y,
            measured.height,
        );
        Rect::from_xywh(left, top, width, height)
    }

    fn children<'n>(&self, node: &'n Resolved, rect: Rect) -> Vec<Laid<'n>> {
        let inner = inset(rect, node.style.padding);
        match (&node.content, node.kind) {
            (Content::Symbol(symbol), _) => {
                // Grid space: paint scales it into the node's box.
                let grid = Rect::from_wh(symbol.grid, symbol.grid);
                symbol
                    .children
                    .iter()
                    .map(|child| self.place_node(child, grid, grid))
                    .collect()
            }
            (Content::Image(image), _) => image
                .fallback
                .iter()
                .map(|child| self.place_node(child, inner, inner))
                .collect(),
            (Content::Children(children), NodeType::Stack) => {
                self.stack(children, inner, &node.style)
            }
            (Content::Children(children), NodeType::Bar) => self.bar(children, inner, &node.style),
            (Content::Children(children), NodeType::Grid) => {
                self.grid(children, inner, &node.style)
            }
            (Content::Children(children), NodeType::Scroll) => {
                self.scroll(children, inner, &node.style)
            }
            (Content::Children(children), _) => children
                .iter()
                .map(|child| self.place_node(child, inner, inner))
                .collect(),
            _ => vec![],
        }
    }

    // MARK: Containers

    fn stack<'n>(&self, children: &'n [Resolved], inner: Rect, style: &Style) -> Vec<Laid<'n>> {
        let horizontal = style.axis == Axis::X;
        let main_space = if horizontal {
            inner.width()
        } else {
            inner.height()
        };
        let cross_space = if horizontal {
            inner.height()
        } else {
            inner.width()
        };
        let gaps = style.gap * children.len().saturating_sub(1) as f32;

        // Fixed and hugging children first, then whatever is left goes to the ones that fill.
        let mut mains: Vec<Option<f32>> = vec![];
        let mut fixed_total = 0.0;
        let mut flexible = 0.0;
        for child in children {
            let margin = child.style.margin;
            let extra = if horizontal {
                margin.horizontal()
            } else {
                margin.vertical()
            };
            let dim = self.main_dim(child, horizontal);
            match dim {
                Dim::Fill { .. } => {
                    let grow = if child.style.grow > 0.0 {
                        child.style.grow
                    } else {
                        1.0
                    };
                    flexible += grow;
                    mains.push(None);
                }
                _ => {
                    let available = Size::new(
                        if horizontal { main_space } else { cross_space },
                        if horizontal { cross_space } else { main_space },
                    );
                    let size = self.main_size(child, available, horizontal);
                    let main = if horizontal { size.width } else { size.height };
                    fixed_total += main + extra;
                    mains.push(Some(main));
                }
            }
        }
        let leftover = (main_space - gaps - fixed_total).max(0.0);
        let share = if flexible > 0.0 {
            leftover / flexible
        } else {
            0.0
        };

        let content_main = if flexible > 0.0 {
            main_space
        } else {
            fixed_total + gaps
        };
        let free = (main_space - content_main).max(0.0);
        let mut cursor = match style.justify {
            Justify::Start | Justify::SpaceBetween => 0.0,
            Justify::Center => free / 2.0,
            Justify::End => free,
        };
        let between = match style.justify {
            Justify::SpaceBetween if children.len() > 1 => {
                free / (children.len() - 1) as f32
            }
            _ => 0.0,
        };

        let mut out = Vec::with_capacity(children.len());
        for (index, child) in children.iter().enumerate() {
            let margin = child.style.margin;
            let (lead, trail) = if horizontal {
                (margin.left, margin.right)
            } else {
                (margin.top, margin.bottom)
            };
            let grow = if child.style.grow > 0.0 {
                child.style.grow
            } else {
                1.0
            };
            let main = match mains[index] {
                Some(main) => main,
                None => {
                    match self.main_dim(child, horizontal) {
                        Dim::Fill { min, max } => clamp(share * grow, min, max),
                        _ => share * grow,
                    }
                }
            };
            cursor += lead;
            let cross = self.cross(child, cross_space, horizontal, style);
            let (cross_start, cross_size) = cross;
            let slot = if horizontal {
                Rect::from_xywh(inner.left + cursor, inner.top + cross_start, main, cross_size)
            } else {
                Rect::from_xywh(inner.left + cross_start, inner.top + cursor, cross_size, main)
            };
            out.push(self.place_node(child, slot, slot));
            cursor += main + trail + style.gap + between;
        }
        out
    }

    /// The cross-axis offset and size for one child of a stack.
    fn cross(
        &self,
        child: &Resolved,
        cross_space: f32,
        horizontal: bool,
        style: &Style,
    ) -> (f32, f32) {
        let margin = child.style.margin;
        let (lead, trail) = if horizontal {
            (margin.top, margin.bottom)
        } else {
            (margin.left, margin.right)
        };
        let room = (cross_space - lead - trail).max(0.0);
        let align = child
            .style
            .align_self
            .or(style.align)
            .unwrap_or(Align::Start);
        let dim = self.dim(child, !horizontal);
        let size = match dim {
            Dim::Fill { min, max } => clamp(room, min, max),
            Dim::Fraction(fraction) => room * fraction,
            _ if align.is_stretch() => room,
            _ => {
                let available = Size::new(
                    if horizontal { room } else { cross_space },
                    if horizontal { cross_space } else { room },
                );
                let measured = self.size_of(child, available);
                if horizontal {
                    measured.height
                } else {
                    measured.width
                }
            }
        };
        let offset = lead + (room - size).max(0.0) * align.fraction();
        (offset, size)
    }

    /// Leading, centre and trailing, where the centre is centred on the bar and then pushed
    /// aside by whichever end crowds it — the toolbar's search field.
    fn bar<'n>(&self, children: &'n [Resolved], inner: Rect, style: &Style) -> Vec<Laid<'n>> {
        let available = Size::new(inner.width(), inner.height());
        let mut leading_width = 0.0_f32;
        let mut trailing_width = 0.0_f32;
        for child in children {
            let size = self.main_size(child, available, true);
            match child.style.slot {
                Some(Slot::Leading) => leading_width = leading_width.max(size.width),
                Some(Slot::Trailing) => trailing_width = trailing_width.max(size.width),
                _ => {}
            }
        }
        let mut out = vec![];
        for child in children {
            let wanted = match self.main_dim(child, true) {
                Dim::Fill { min, max } => {
                    let room = inner.width() - leading_width - trailing_width - style.gap * 2.0;
                    clamp(room.max(0.0), min, max)
                }
                _ => self.main_size(child, available, true).width,
            };
            let left = match child.style.slot {
                Some(Slot::Leading) | None => inner.left,
                Some(Slot::Trailing) => inner.right - wanted,
                Some(Slot::Center) => {
                    let centred = inner.center_x() - wanted / 2.0;
                    let low = inner.left + leading_width + style.gap;
                    let high = (inner.right - trailing_width - style.gap - wanted).max(low);
                    centred.clamp(low, high)
                }
            };
            let (top, height) = self.bar_cross(child, inner, style, available);
            let slot = Rect::from_xywh(left, top, wanted, height);
            out.push(self.place_node(child, slot, slot));
        }
        out
    }

    fn bar_cross(
        &self,
        child: &Resolved,
        inner: Rect,
        style: &Style,
        available: Size,
    ) -> (f32, f32) {
        let align = child
            .style
            .align_self
            .or(style.align)
            .unwrap_or(Align::Center);
        let height = match self.dim(child, false) {
            Dim::Fill { min, max } => clamp(inner.height(), min, max),
            _ if align.is_stretch() => inner.height(),
            _ => self.main_size(child, available, false).height,
        };
        let top = inner.top + (inner.height() - height).max(0.0) * align.fraction();
        (top, height)
    }

    /// The library grid: as many columns as fit at `min_cell`, then the remainder shared evenly.
    fn grid<'n>(&self, children: &'n [Resolved], inner: Rect, style: &Style) -> Vec<Laid<'n>> {
        let gap = style.gap;
        let row_gap = style.row_gap.unwrap_or(gap);
        let min_cell = style.min_cell.max(1.0);
        let columns = (((inner.width() + gap) / (min_cell + gap)).floor() as usize).max(1);
        let cell = (inner.width() - gap * (columns - 1) as f32) / columns as f32;

        let mut out = vec![];
        let mut y = inner.top;
        for row in children.chunks(columns) {
            let mut tallest = 0.0_f32;
            for (column, child) in row.iter().enumerate() {
                let x = inner.left + column as f32 * (cell + gap);
                let available = Size::new(cell, inner.height());
                let height = match self.main_dim(child, false) {
                    Dim::Fixed(value) => value,
                    Dim::Ratio(ratio) => cell * ratio,
                    _ => self.main_size(child, available, false).height,
                };
                let slot = Rect::from_xywh(x, y, cell, height);
                out.push(self.place_node(child, slot, slot));
                tallest = tallest.max(height);
            }
            y += tallest + row_gap;
        }
        out
    }

    fn scroll<'n>(&self, children: &'n [Resolved], inner: Rect, style: &Style) -> Vec<Laid<'n>> {
        let shifted = match style.axis {
            Axis::Y => Rect::from_xywh(
                inner.left,
                inner.top - style.offset,
                inner.width(),
                inner.height(),
            ),
            Axis::X => Rect::from_xywh(
                inner.left - style.offset,
                inner.top,
                inner.width(),
                inner.height(),
            ),
        };
        children
            .iter()
            .map(|child| self.place_node(child, shifted, shifted))
            .collect()
    }

    // MARK: Measuring

    /// A child's size along the axis its parent arranges it on. Saying nothing means hugging:
    /// a row of controls is as wide as the controls, not as wide as the row. The cross axis
    /// still falls back to filling, so those controls stretch to the row's height.
    fn main_dim(&self, node: &Resolved, horizontal: bool) -> Dim {
        let declared = if horizontal {
            node.style.width
        } else {
            node.style.height
        };
        declared.unwrap_or(Dim::Hug)
    }

    /// This node's dimension on one axis, falling back to what its type does by default.
    fn dim(&self, node: &Resolved, horizontal: bool) -> Dim {
        let own = if horizontal {
            node.style.width
        } else {
            node.style.height
        };
        own.unwrap_or_else(|| default_dim(node))
    }

    /// How large this node wants to be, given the room it has.
    pub fn size_of(&self, node: &Resolved, available: Size) -> Size {
        self.sized(node, available, false, false)
    }

    /// Measuring for a hugging parent: a child that would fill is asked what it needs instead,
    /// or every hug would come out the size of the window.
    fn hug_size(&self, node: &Resolved, available: Size) -> Size {
        self.sized(node, available, true, true)
    }

    /// Measuring a child of a stack, bar or grid: it hugs along the axis it is arranged on and
    /// still fills across it, so a row of buttons keeps their widths and their height.
    fn main_size(&self, node: &Resolved, available: Size, horizontal: bool) -> Size {
        self.sized(node, available, horizontal, !horizontal)
    }

    fn sized(&self, node: &Resolved, available: Size, hug_width: bool, hug_height: bool) -> Size {
        let resolve = |dim: Dim, hug: bool| {
            if hug && matches!(dim, Dim::Fill { .. }) {
                Dim::Hug
            } else {
                dim
            }
        };
        let own_width = resolve(self.dim(node, true), hug_width);
        let own_height = resolve(self.dim(node, false), hug_height);
        let width = match own_width {
            Dim::Fixed(value) => value,
            Dim::Fraction(fraction) => available.width * fraction,
            Dim::Fill { min, max } => clamp(available.width, min, max),
            Dim::Hug => self.content_size(node, available).width,
            Dim::Ratio(_) => 0.0,
        };
        let height = match own_height {
            Dim::Fixed(value) => value,
            Dim::Fraction(fraction) => available.height * fraction,
            Dim::Fill { min, max } => clamp(available.height, min, max),
            Dim::Hug => {
                self.content_size(node, Size::new(width, available.height))
                    .height
            }
            Dim::Ratio(ratio) => width * ratio,
        };
        // A width given as a ratio reads the height, which is now known.
        let width = match own_width {
            Dim::Ratio(ratio) => height * ratio,
            _ => width,
        };
        Size::new(width.max(0.0), height.max(0.0))
    }

    /// What the content alone needs, before padding.
    fn content_size(&self, node: &Resolved, available: Size) -> Size {
        let padding = node.style.padding;
        let room = Size::new(
            (available.width - padding.horizontal()).max(0.0),
            (available.height - padding.vertical()).max(0.0),
        );
        let inner = match &node.content {
            Content::Text(text) => {
                if text.wrap {
                    let lines = self
                        .text
                        .wrap(&text.value, &text.style, room.width, text.max_lines);
                    Size::new(room.width, lines.len() as f32 * text.style.line)
                } else {
                    let width = self.text.width(&text.value, &text.style);
                    Size::new(text.limit.map_or(width, |limit| width.min(limit)), text.style.line)
                }
            }
            Content::Field(field) => Size::new(room.width, field.style.line),
            Content::Symbol(symbol) => Size::new(symbol.grid, symbol.grid),
            Content::Spinner(_) => Size::new(15.0, 15.0),
            Content::Children(children) => self.children_size(node, children, room),
            Content::Image(image) => self.children_size(node, &image.fallback, room),
            _ => Size::new(0.0, 0.0),
        };
        Size::new(
            inner.width + padding.horizontal(),
            inner.height + padding.vertical(),
        )
    }

    /// Children are always measured by what they need here: this only runs for a hug.
    fn children_size(&self, node: &Resolved, children: &[Resolved], room: Size) -> Size {
        if children.is_empty() {
            return Size::new(0.0, 0.0);
        }

        match node.kind {
            NodeType::Stack => {
                let horizontal = node.style.axis == Axis::X;
                let gaps = node.style.gap * children.len().saturating_sub(1) as f32;
                // In a row, a child that fills gets what its siblings leave, and its own hug
                // height depends on that width — so the fixed ones are measured first.
                let mut taken = gaps;
                if horizontal {
                    for child in children {
                        if !matches!(self.main_dim(child, true), Dim::Fill { .. }) {
                            taken += self.hug_size(child, room).width
                                + child.style.margin.horizontal();
                        }
                    }
                }
                let flexible = children
                    .iter()
                    .filter(|child| matches!(self.main_dim(child, true), Dim::Fill { .. }))
                    .count()
                    .max(1) as f32;
                let share = ((room.width - taken) / flexible).max(0.0);

                let mut main = gaps;
                let mut cross = 0.0_f32;
                for child in children {
                    let fills =
                        horizontal && matches!(self.main_dim(child, true), Dim::Fill { .. });
                    let room = if fills {
                        Size::new(share, room.height)
                    } else {
                        room
                    };
                    let size = self.hug_size(child, room);
                    let margin = child.style.margin;
                    if horizontal {
                        main += size.width + margin.horizontal();
                        cross = cross.max(size.height + margin.vertical());
                    } else {
                        main += size.height + margin.vertical();
                        cross = cross.max(size.width + margin.horizontal());
                    }
                }
                if horizontal {
                    Size::new(main, cross)
                } else {
                    Size::new(cross, main)
                }
            }
            NodeType::Bar => {
                let mut width = 0.0;
                let mut height = 0.0_f32;
                for child in children {
                    let size = self.hug_size(child, room);
                    width += size.width;
                    height = height.max(size.height);
                }
                Size::new(width, height)
            }
            _ => {
                // Overlapping children: as large as the furthest corner any of them reaches.
                let mut width = 0.0_f32;
                let mut height = 0.0_f32;
                for child in children {
                    let size = self.hug_size(child, room);
                    let place = child.style.place;
                    width = width.max(size.width + place.left.unwrap_or(0.0));
                    height = height.max(size.height + place.top.unwrap_or(0.0));
                }
                Size::new(width, height)
            }
        }
    }
}

/// What a node sizes itself as when the document says nothing: containers and the shapes that
/// draw in their own coordinate space fill, and content hugs.
fn default_dim(node: &Resolved) -> Dim {
    match node.kind {
        NodeType::Text | NodeType::Symbol | NodeType::Spinner => Dim::Hug,
        NodeType::Spacer => Dim::Fill {
            min: None,
            max: None,
        },
        _ => Dim::Fill {
            min: None,
            max: None,
        },
    }
}

/// One axis of a `place`: an offset from each edge, a centre, or a measured size.
fn span(
    origin: f32,
    extent: f32,
    near: Option<f32>,
    far: Option<f32>,
    centre: Option<f32>,
    measured: f32,
) -> (f32, f32) {
    match (near, far, centre) {
        (Some(near), Some(far), _) => (origin + near, (extent - near - far).max(0.0)),
        (Some(near), None, _) => (origin + near, measured),
        (None, Some(far), _) => (origin + extent - far - measured, measured),
        (None, None, Some(centre)) => (
            origin + extent / 2.0 + centre - measured / 2.0,
            measured,
        ),
        (None, None, None) => (origin, measured),
    }
}

pub fn inset(rect: Rect, insets: Insets) -> Rect {
    Rect::from_ltrb(
        rect.left + insets.left,
        rect.top + insets.top,
        rect.right - insets.right,
        rect.bottom - insets.bottom,
    )
}
