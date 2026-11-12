//! Walking the laid-out tree and emitting Skia calls, collecting hit regions as it goes.

use std::time::Duration;

use serde_json::Value;
use skia_safe::{
    BlendMode, Canvas, ClipOp, Color, Color4f, MaskFilter, Paint, PaintCap, PaintJoin, PathBuilder,
    Point, RRect, Rect, SamplingOptions, Shader, TileMode,
    canvas::{SaveLayerRec, SrcRectConstraint},
    gradient::{Colors as GradientColors, Gradient, Interpolation, shaders::linear_gradient},
    image_filters,
    paint::Style as PaintStyle,
};

use crate::document::{Align, Blend, Cap, Fit, Join, Sampling, StrokeAlign, Truncate};
use crate::kind::NodeType;
use crate::layout::Laid;
use crate::ports::FieldPaint;
use crate::resolved::{Content, Resolved, Segment, ShadowStyle, StrokeStyle, Style};
use crate::tokens::{Direction, Fill};

/// A named region in paint order; later entries sit on top, as `Ui::hit` arranges them.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub rect: Rect,
    pub action: String,
    pub args: Value,
    pub secondary: Option<String>,
    pub secondary_args: Value,
    pub tooltip: Option<String>,
    pub text_cursor: bool,
    pub blocks: bool,
}

/// A rect reserved for a native view, such as the sign-in browser.
#[derive(Clone, Debug, PartialEq)]
pub struct Host {
    pub name: String,
    pub rect: Rect,
}

/// What one painted frame gives back to the shell.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub hits: Vec<Region>,
    pub hosts: Vec<Host>,
    /// By node id, so a scroller can be clamped to what its content needed.
    pub content: Vec<(String, skia_safe::Size)>,
    /// `Some` while something animates.
    pub next_frame: Option<Duration>,
}

pub struct Painter<'a> {
    pub canvas: &'a Canvas,
    pub renderer: &'a crate::Renderer<'a>,
    pub time: Duration,
    pub frame: Frame,
    clips: Vec<Rect>,
}

impl<'a> Painter<'a> {
    pub fn new(canvas: &'a Canvas, renderer: &'a crate::Renderer<'a>, time: Duration) -> Self {
        Self {
            canvas,
            renderer,
            time,
            frame: Frame::default(),
            clips: vec![],
        }
    }

    pub fn finish(self) -> Frame {
        self.frame
    }

    pub fn paint(&mut self, laid: &Laid<'_>) {
        let node = laid.node;
        let style = &node.style;
        let rect = laid.rect;
        let layered = style.opacity < 1.0 || style.isolate;
        if layered {
            if style.opacity < 1.0 {
                self.canvas.save_layer_alpha_f(None, style.opacity);
            } else {
                self.canvas.save_layer(&SaveLayerRec::default());
            }
        }

        let shape = self.shape(rect, style);
        for shadow in &style.shadows {
            self.shadow(shape, shadow);
        }
        if let Some(material) = &style.material {
            self.material(shape, material.blur, material.tint);
        }
        // A shape with neither fill nor stroke is painted in the colour it inherited, which is
        // how a row of dots or a filled glyph takes its parent's tint.
        let implicit = style.fill.clone().or_else(|| {
            let bare = style.stroke.is_none()
                && matches!(node.kind, NodeType::Ellipse | NodeType::Path);
            bare.then(|| Fill::Solid(style.color))
        });
        if let Some(fill) = &implicit {
            let mut paint = self.fill_paint(fill, rect);
            apply_blend(&mut paint, style.blend);
            self.draw_shape(node, shape, rect, &paint);
        }

        // Registered before the children, so a control inside a draggable bar wins the click.
        if let Some(hit) = &style.hit {
            let mut region = rect;
            let inside = match self.clips.last() {
                Some(clip) => region.intersect(clip),
                None => true,
            };
            if inside {
                self.frame.hits.push(Region {
                    rect: region,
                    action: hit.action.clone(),
                    args: hit.args.clone(),
                    secondary: hit.secondary.clone(),
                    secondary_args: hit.secondary_args.clone(),
                    tooltip: hit.tooltip.clone(),
                    text_cursor: hit.text_cursor,
                    blocks: hit.blocks,
                });
            }
        }

        self.content(laid, rect);

        if style.clip {
            self.canvas.save();
            self.canvas.clip_rrect(shape, None, true);
            self.clips.push(effective_clip(self.clips.last(), rect));
        }
        // A symbol paints its own children under its grid transform, and an image paints its
        // fallback only while nothing has loaded, so both are left to `content`.
        if matches!(node.content, Content::Children(_)) {
            for child in &laid.children {
                self.paint(child);
            }
        }
        if style.clip {
            self.clips.pop();
            self.canvas.restore();
        }

        if let Some(stroke) = &style.stroke
            && !matches!(node.content, Content::Path(_) | Content::Line { .. })
        {
            let inset_rect = match stroke.align {
                StrokeAlign::Inside => rect.with_inset((stroke.width / 2.0, stroke.width / 2.0)),
                StrokeAlign::Outside => {
                    rect.with_outset((stroke.width / 2.0, stroke.width / 2.0))
                }
                StrokeAlign::Center => rect,
            };
            let shrink = match stroke.align {
                StrokeAlign::Inside => -stroke.width / 2.0,
                StrokeAlign::Outside => stroke.width / 2.0,
                StrokeAlign::Center => 0.0,
            };
            let shape = self.shape_adjusted(inset_rect, style, shrink);
            let mut paint = stroke_paint(stroke);
            apply_blend(&mut paint, style.blend);
            self.draw_shape(node, shape, inset_rect, &paint);
        }

        if layered {
            self.canvas.restore();
        }

        if let Some(id) = &node.id {
            self.frame
                .content
                .push((id.clone(), skia_safe::Size::new(rect.width(), rect.height())));
        }
    }

    // MARK: Content

    fn content(&mut self, laid: &Laid<'_>, rect: Rect) {
        let node = laid.node;
        match &node.content {
            Content::Text(text) => {
                let color = node.style.color;
                if text.wrap {
                    let lines =
                        self.renderer
                            .text
                            .wrap(&text.value, &text.style, rect.width(), text.max_lines);
                    for (index, line) in lines.iter().enumerate() {
                        let y = rect.top + index as f32 * text.style.line;
                        self.line(line, text, color, rect, y, Some(rect.width()));
                    }
                } else {
                    let limit = text
                        .limit
                        .or((text.truncate == Truncate::Ellipsis).then_some(rect.width()));
                    self.line(&text.value, text, color, rect, rect.top, limit);
                }
            }
            Content::Field(field) => {
                self.renderer.fields.draw(
                    self.canvas,
                    FieldPaint {
                        binding: &field.binding,
                        rect,
                        style: field.style,
                        color: node.style.color,
                        placeholder: &field.placeholder,
                        placeholder_color: field.placeholder_color,
                        selection_color: field.selection_color,
                        caret_color: field.caret_color,
                        focused: field.focused,
                        caret_visible: field.caret_visible,
                    },
                );
                if field.focused {
                    self.frame.next_frame = Some(Duration::from_millis(100));
                }
            }
            Content::Symbol(symbol) => {
                // The symbol's children were laid out on its grid.
                self.canvas.save();
                self.canvas.translate((rect.left, rect.top));
                let scale = (rect.width() / symbol.grid, rect.height() / symbol.grid);
                self.canvas.scale(scale);
                for child in &laid.children {
                    self.paint(child);
                }
                self.canvas.restore();
            }
            Content::Image(image) => {
                let loaded = image
                    .source
                    .as_deref()
                    .and_then(|source| self.renderer.images.image(source));
                match loaded {
                    Some(bitmap) => self.image(&bitmap, image.fit, image.sampling, rect),
                    None => {
                        for child in &laid.children {
                            self.paint(child);
                        }
                    }
                }
            }
            Content::Arc(arc) => {
                if let Some(track) = &arc.track {
                    let paint = stroke_paint(track);
                    self.canvas.draw_oval(oval_in(rect, track.width), &paint);
                }
                if let Some(stroke) = &node.style.stroke {
                    let paint = stroke_paint(stroke);
                    self.canvas.draw_arc(
                        oval_in(rect, stroke.width),
                        arc.start_deg,
                        arc.sweep_deg,
                        false,
                        &paint,
                    );
                }
            }
            Content::Path(path) => {
                let built = self.build_path(path, rect);
                let fill = node.style.fill.clone().or_else(|| {
                    node.style
                        .stroke
                        .is_none()
                        .then(|| Fill::Solid(node.style.color))
                });
                if let Some(fill) = &fill {
                    let mut paint = self.fill_paint(fill, rect);
                    apply_blend(&mut paint, node.style.blend);
                    self.canvas.draw_path(&built, &paint);
                }
                if let Some(stroke) = &node.style.stroke {
                    let mut paint = stroke_paint(stroke);
                    apply_blend(&mut paint, node.style.blend);
                    self.canvas.draw_path(&built, &paint);
                }
            }
            Content::Line { from, to } => {
                if let Some(stroke) = &node.style.stroke {
                    let mut paint = stroke_paint(stroke);
                    apply_blend(&mut paint, node.style.blend);
                    self.canvas.draw_line(
                        (rect.left + from.0, rect.top + from.1),
                        (rect.left + to.0, rect.top + to.1),
                        &paint,
                    );
                }
            }
            Content::Spinner(spinner) => {
                self.spinner(rect, node.style.color, spinner.spokes, spinner.period_ms);
                self.frame.next_frame = Some(Duration::from_millis(
                    (spinner.period_ms / spinner.spokes as f32) as u64,
                ));
            }
            Content::Host(name) => self.frame.hosts.push(Host {
                name: name.clone(),
                rect,
            }),
            Content::Children(_) | Content::Ellipse | Content::Spacer => {}
        }
    }

    fn line(
        &self,
        text: &str,
        content: &crate::resolved::TextContent,
        color: Color,
        rect: Rect,
        y: f32,
        limit: Option<f32>,
    ) {
        let width = self.renderer.text.width(text, &content.style);
        let x = match content.align {
            Align::Center => rect.center_x() - width.min(rect.width()) / 2.0,
            Align::End | Align::Trailing => rect.right - width.min(rect.width()),
            _ => rect.left,
        };
        self.renderer.text.draw_line(
            self.canvas,
            text,
            &content.style,
            color,
            Point::new(x, y),
            limit.filter(|limit| width > *limit),
        );
    }

    fn image(&self, bitmap: &skia_safe::Image, fit: Fit, sampling: Sampling, rect: Rect) {
        let (width, height) = (bitmap.width() as f32, bitmap.height() as f32);
        let source = match fit {
            // Crop whichever sides overflow, like `scaledToFill`.
            Fit::Cover => {
                let scale = (rect.width() / width).max(rect.height() / height);
                let visible = (rect.width() / scale, rect.height() / scale);
                Some(Rect::from_xywh(
                    (width - visible.0) / 2.0,
                    (height - visible.1) / 2.0,
                    visible.0,
                    visible.1,
                ))
            }
            Fit::Contain | Fit::Fill => None,
        };
        let destination = match fit {
            Fit::Contain => {
                let scale = (rect.width() / width).min(rect.height() / height);
                let size = (width * scale, height * scale);
                Rect::from_xywh(
                    rect.center_x() - size.0 / 2.0,
                    rect.center_y() - size.1 / 2.0,
                    size.0,
                    size.1,
                )
            }
            _ => rect,
        };
        let options = match sampling {
            Sampling::Nearest => SamplingOptions::new(
                skia_safe::FilterMode::Nearest,
                skia_safe::MipmapMode::None,
            ),
            Sampling::Mipmap => SamplingOptions::new(
                skia_safe::FilterMode::Linear,
                skia_safe::MipmapMode::Linear,
            ),
            Sampling::Linear => SamplingOptions::new(
                skia_safe::FilterMode::Linear,
                skia_safe::MipmapMode::None,
            ),
        };
        self.canvas.draw_image_rect_with_sampling_options(
            bitmap,
            source.as_ref().map(|source| (source, SrcRectConstraint::Fast)),
            destination,
            options,
            &Paint::default(),
        );
    }

    /// AppKit's indeterminate spinner: spokes that fade with age.
    fn spinner(&self, rect: Rect, color: Color, spokes: usize, period_ms: f32) {
        let size = rect.width().min(rect.height());
        let center = (rect.center_x(), rect.center_y());
        let step = period_ms / spokes as f32;
        let phase = (self.time.as_millis() as f32 / step) as usize % spokes;
        let mut paint = Paint::default();
        paint
            .set_anti_alias(true)
            .set_stroke_width(size * 0.09)
            .set_style(PaintStyle::Stroke)
            .set_stroke_cap(PaintCap::Round);
        for spoke in 0..spokes {
            let age = (phase + spokes - spoke) % spokes;
            paint.set_color(crate::tokens::faded(
                color,
                (0.85 - 0.06 * age as f32).max(0.0),
            ));
            let angle = spoke as f32 / spokes as f32 * std::f32::consts::TAU;
            let (sin, cos) = angle.sin_cos();
            let (inner, outer) = (size * 0.25, size * 0.5);
            self.canvas.draw_line(
                (center.0 + sin * inner, center.1 - cos * inner),
                (center.0 + sin * outer, center.1 - cos * outer),
                &paint,
            );
        }
    }

    // MARK: Shapes

    fn shape(&self, rect: Rect, style: &Style) -> RRect {
        self.shape_adjusted(rect, style, 0.0)
    }

    fn shape_adjusted(&self, rect: Rect, style: &Style, shrink: f32) -> RRect {
        let radii = style.radius.for_box(rect.width(), rect.height());
        let corner = |value: f32| (value + shrink).max(0.0);
        RRect::new_rect_radii(
            rect,
            &[
                Point::new(corner(radii[0]), corner(radii[0])),
                Point::new(corner(radii[1]), corner(radii[1])),
                Point::new(corner(radii[2]), corner(radii[2])),
                Point::new(corner(radii[3]), corner(radii[3])),
            ],
        )
    }

    /// An ellipse node draws an oval; everything else draws its rounded box.
    fn draw_shape(&self, node: &Resolved, shape: RRect, rect: Rect, paint: &Paint) {
        match node.kind {
            NodeType::Ellipse => {
                self.canvas.draw_oval(rect, paint);
            }
            NodeType::Arc | NodeType::Path | NodeType::Line => {}
            _ if node.style.radius.is_zero() => {
                self.canvas.draw_rect(rect, paint);
            }
            _ => {
                self.canvas.draw_rrect(shape, paint);
            }
        }
    }

    fn fill_paint(&self, fill: &Fill, rect: Rect) -> Paint {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        match fill {
            Fill::Solid(color) => {
                paint.set_color(*color);
            }
            Fill::Gradient { direction, stops } => {
                paint.set_color(stops[0]);
                if let Some(shader) = gradient(rect, *direction, stops) {
                    paint.set_shader(shader);
                }
            }
        }
        paint
    }

    /// A blurred copy of the shape, with the shape itself clipped out from under it so a
    /// translucent fill does not darken.
    fn shadow(&self, shape: RRect, shadow: &ShadowStyle) {
        let mut paint = Paint::default();
        paint.set_anti_alias(true).set_color(shadow.color);
        if shadow.blur > 0.0 {
            paint.set_mask_filter(MaskFilter::blur(
                skia_safe::BlurStyle::Normal,
                shadow.blur,
                false,
            ));
        }
        self.canvas.save();
        if shadow.knockout {
            self.canvas.clip_rrect(shape, ClipOp::Difference, true);
        }
        self.canvas
            .draw_rrect(shape.with_offset((shadow.dx, shadow.dy)), &paint);
        self.canvas.restore();
    }

    /// Blurs what is behind the shape and tints it: macOS's window materials.
    fn material(&self, shape: RRect, blur: f32, tint: Option<Color>) {
        self.canvas.save();
        self.canvas.clip_rrect(shape, None, true);
        if let Some(filter) = image_filters::blur((blur, blur), TileMode::Clamp, None, None) {
            self.canvas
                .save_layer(&SaveLayerRec::default().backdrop(&filter));
            self.canvas.restore();
        }
        self.canvas.restore();
        if let Some(tint) = tint {
            let mut paint = Paint::default();
            paint.set_anti_alias(true).set_color(tint);
            self.canvas.draw_rrect(shape, &paint);
        }
    }

    fn build_path(&self, path: &crate::resolved::PathContent, rect: Rect) -> skia_safe::Path {
        let scale = path.grid.map_or(1.0, |grid| rect.width() / grid);
        let at = |point: (f32, f32)| {
            Point::new(rect.left + point.0 * scale, rect.top + point.1 * scale)
        };
        let mut builder = PathBuilder::new();
        for segment in &path.commands {
            match segment {
                Segment::Move(point) => {
                    builder.move_to(at(*point));
                }
                Segment::Line(point) => {
                    builder.line_to(at(*point));
                }
                Segment::Cubic([a, b, c]) => {
                    builder.cubic_to(at(*a), at(*b), at(*c));
                }
                Segment::Close => {
                    builder.close();
                }
            }
        }
        builder.detach()
    }
}

fn effective_clip(outer: Option<&Rect>, rect: Rect) -> Rect {
    let mut effective = rect;
    if let Some(outer) = outer
        && !effective.intersect(outer)
    {
        effective = Rect::default();
    }
    effective
}

fn oval_in(rect: Rect, stroke_width: f32) -> Rect {
    rect.with_inset((stroke_width / 2.0, stroke_width / 2.0))
}

fn stroke_paint(stroke: &StrokeStyle) -> Paint {
    let mut paint = Paint::default();
    paint
        .set_anti_alias(true)
        .set_color(stroke.color)
        .set_style(PaintStyle::Stroke)
        .set_stroke_width(stroke.width)
        .set_stroke_cap(match stroke.cap {
            Cap::Round => PaintCap::Round,
            Cap::Square => PaintCap::Square,
            Cap::Butt => PaintCap::Butt,
        })
        .set_stroke_join(match stroke.join {
            Join::Round => PaintJoin::Round,
            Join::Bevel => PaintJoin::Bevel,
            Join::Miter => PaintJoin::Miter,
        });
    paint
}

fn apply_blend(paint: &mut Paint, blend: Blend) {
    if blend == Blend::DstOut {
        paint.set_blend_mode(BlendMode::DstOut);
    }
}

fn gradient(rect: Rect, direction: Direction, stops: &[Color]) -> Option<Shader> {
    let colors: Vec<Color4f> = stops.iter().map(|color| Color4f::from(*color)).collect();
    let gradient = Gradient::new(
        GradientColors::new_evenly_spaced(&colors, TileMode::Clamp, None),
        Interpolation::default(),
    );
    let ends = match direction {
        Direction::Vertical => (
            Point::new(rect.left, rect.top),
            Point::new(rect.left, rect.bottom),
        ),
        Direction::Horizontal => (
            Point::new(rect.left, rect.top),
            Point::new(rect.right, rect.top),
        ),
    };
    linear_gradient(ends, &gradient, None)
}
