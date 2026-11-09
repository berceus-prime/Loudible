//! Immediate-mode drawing: views paint directly and register what clicking each area does.
//! A later hit area lies on top of earlier ones.

use std::{rc::Rc, time::Duration};

use skia_safe::{
    Canvas, ClipOp, Color, Color4f, MaskFilter, Paint, Point, RRect, Rect, Shader, TileMode,
    canvas::SaveLayerRec,
    gradient::{Colors as GradientColors, Gradient, Interpolation, shaders::linear_gradient},
    image_filters,
    paint::{Cap, Style},
};

use crate::icons::{self, Icon};
use crate::model::{Asin, BookAction, Marketplace};
use crate::text::{self, TextStyle, TextSystem};
use crate::theme::{Theme, faded};

/// An editable text field in the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Search,
    PastedUrl,
}

/// Everything a click can do. Views only name these; the shell carries them out.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Swallows the click, for modal backdrops and disabled controls.
    Block,
    DragWindow,
    CloseWindow,
    MinimizeWindow,
    ZoomWindow,
    ClearSelection,
    ToggleBook(Asin),
    OpenBookMenu(Asin, Point),
    Perform(BookAction, Asin),
    OnSelection(BookAction),
    Refresh,
    ToggleAccount,
    SignIn,
    SignOut,
    OpenMarketplaceMenu(Point),
    SetMarketplace(Marketplace),
    FocusField(Field, f32),
    ClearSearch,
    SimulateSignIn,
    OpenInBrowser,
    SubmitPastedUrl,
    CancelSignIn,
    ToggleBrowserFallback,
    ToggleToastDetails(u64),
    CopyToast(u64),
    DismissToast(u64),
    ConfirmDelete,
    CancelDelete,
    ConfirmQuit,
    CancelQuit,
    DismissPopup,
    Gallery(usize),
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub rect: Rect,
    pub action: Action,
    pub secondary: Option<Action>,
    pub tooltip: Option<Rc<str>>,
    pub is_text: bool,
}

pub struct Ui<'a> {
    pub canvas: &'a Canvas,
    pub theme: Theme,
    pub text: &'a TextSystem,
    pub hits: Vec<Hit>,
    pub hot: Option<Action>,
    pub pressed: Option<Action>,
    pub time: Duration,
    /// Whether the window is key; macOS greys title bar controls otherwise.
    pub is_active: bool,
    clips: Vec<Rect>,
}

pub fn fill(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true).set_color(color);
    paint
}

pub fn outline(color: Color, width: f32) -> Paint {
    let mut paint = fill(color);
    paint.set_style(Style::Stroke).set_stroke_width(width);
    paint
}

pub fn capsule(rect: Rect) -> RRect {
    let radius = rect.height().min(rect.width()) / 2.0;
    RRect::new_rect_xy(rect, radius, radius)
}

pub fn rounded(rect: Rect, radius: f32) -> RRect {
    RRect::new_rect_xy(rect, radius, radius)
}

/// How a push button is filled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle {
    Bordered,
    Prominent,
    /// Label only until pressed, like Liquid Glass on a plain window.
    Clear,
}

pub struct Button<'t> {
    pub title: &'t str,
    pub action: Action,
    pub style: ButtonStyle,
    pub enabled: bool,
    pub color: Option<Color>,
}

impl<'t> Button<'t> {
    pub fn new(title: &'t str, action: Action) -> Self {
        Self {
            title,
            action,
            style: ButtonStyle::Bordered,
            enabled: true,
            color: None,
        }
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

impl<'a> Ui<'a> {
    pub fn new(
        canvas: &'a Canvas,
        theme: Theme,
        text: &'a TextSystem,
        hot: Option<Action>,
        pressed: Option<Action>,
        time: Duration,
        is_active: bool,
    ) -> Self {
        Self {
            canvas,
            theme,
            text,
            hits: vec![],
            hot,
            pressed,
            time,
            is_active,
            clips: vec![],
        }
    }

    // MARK: Hit areas

    pub fn hit(&mut self, rect: Rect, action: Action) {
        self.hit_with(rect, action, None, None, false);
    }

    pub fn hit_with(
        &mut self,
        rect: Rect,
        action: Action,
        secondary: Option<Action>,
        tooltip: Option<&str>,
        is_text: bool,
    ) {
        let mut rect = rect;
        if let Some(clip) = self.clips.last()
            && !rect.intersect(clip)
        {
            return;
        }
        self.hits.push(Hit {
            rect,
            action,
            secondary,
            tooltip: tooltip.map(Rc::from),
            is_text,
        });
    }

    pub fn is_hot(&self, action: &Action) -> bool {
        self.hot.as_ref() == Some(action)
    }

    pub fn is_pressed(&self, action: &Action) -> bool {
        self.pressed.as_ref() == Some(action) && self.is_hot(action)
    }

    pub fn clip(&mut self, rect: Rect, draw: impl FnOnce(&mut Self)) {
        self.canvas.save();
        self.canvas.clip_rect(rect, None, true);
        let mut effective = rect;
        if let Some(outer) = self.clips.last()
            && !effective.intersect(outer)
        {
            effective = Rect::default();
        }
        self.clips.push(effective);
        draw(self);
        self.clips.pop();
        self.canvas.restore();
    }

    /// Runs a view without painting or registering hits, to find the size it returns.
    pub fn measure<R>(&mut self, view: impl FnOnce(&mut Self) -> R) -> R {
        let count = self.hits.len();
        self.canvas.save();
        self.canvas.clip_rect(Rect::default(), None, false);
        let result = view(self);
        self.canvas.restore();
        self.hits.truncate(count);
        result
    }

    // MARK: Text

    pub fn line(&self, text: &str, style: TextStyle, color: Color, x: f32, y: f32) {
        self.line_limited(text, style, color, x, y, None);
    }

    pub fn line_limited(
        &self,
        text: &str,
        style: TextStyle,
        color: Color,
        x: f32,
        y: f32,
        limit: Option<f32>,
    ) {
        self.text
            .draw_line(self.canvas, text, style, color, Point::new(x, y), limit);
    }

    pub fn line_centered(&self, text: &str, style: TextStyle, color: Color, center_x: f32, y: f32) {
        let x = center_x - self.text.width(text, style) / 2.0;
        self.line(text, style, color, x, y);
    }

    /// Draws wrapped text and returns its height.
    pub fn paragraph(
        &self,
        text: &str,
        style: TextStyle,
        color: Color,
        origin: Point,
        width: f32,
        max_lines: Option<usize>,
    ) -> f32 {
        let lines = self.text.wrap(text, style, width, max_lines);
        for (index, line) in lines.iter().enumerate() {
            let y = origin.y + index as f32 * style.line;
            self.line_limited(line, style, color, origin.x, y, Some(width));
        }
        lines.len() as f32 * style.line
    }

    // MARK: Shapes

    pub fn shadow(&self, shape: RRect, color: Color, radius: f32, y: f32) {
        let mut paint = fill(color);
        paint.set_mask_filter(MaskFilter::blur(
            skia_safe::BlurStyle::Normal,
            radius,
            false,
        ));
        // Knocked out under the shape, so translucent fills don't darken.
        self.canvas.save();
        self.canvas.clip_rrect(shape, ClipOp::Difference, true);
        self.canvas.draw_rrect(shape.with_offset((0.0, y)), &paint);
        self.canvas.restore();
    }

    pub fn vertical_gradient(&self, rect: Rect, top: Color, bottom: Color) -> Option<Shader> {
        let colors = [Color4f::from(top), Color4f::from(bottom)];
        let gradient = Gradient::new(
            GradientColors::new_evenly_spaced(&colors, TileMode::Clamp, None),
            Interpolation::default(),
        );
        linear_gradient(
            (
                Point::new(rect.left, rect.top),
                Point::new(rect.left, rect.bottom),
            ),
            &gradient,
            None,
        )
    }

    /// Blurs what is behind `shape` and tints it: macOS's window materials.
    pub fn material(&self, shape: RRect) {
        self.canvas.save();
        self.canvas.clip_rrect(shape, None, true);
        if let Some(blur) = image_filters::blur((24.0, 24.0), TileMode::Clamp, None, None) {
            self.canvas
                .save_layer(&SaveLayerRec::default().backdrop(&blur));
            self.canvas.restore();
        }
        self.canvas.restore();
        self.canvas.draw_rrect(shape, &fill(self.theme.material));
    }

    pub fn glass_capsule(&self, rect: Rect) {
        let shape = capsule(rect);
        self.canvas.draw_rrect(shape, &fill(self.theme.glass));
        self.canvas
            .draw_rrect(shape, &outline(self.theme.glass_rim, 0.75));
    }

    pub fn separator(&self, x: f32, y: f32, width: f32) {
        let rect = Rect::from_xywh(x, y, width, 1.0);
        self.canvas.draw_rect(rect, &fill(self.theme.separator));
    }

    pub fn icon(&self, icon: Icon, center: Point, size: f32, color: Color) {
        icons::draw(self.canvas, icon, center, size, color);
    }

    /// AppKit's indeterminate spinner: twelve fading spokes.
    pub fn spinner(&self, center: Point, size: f32) {
        const SPOKES: usize = 12;
        let phase = (self.time.as_secs_f32() * SPOKES as f32) as usize % SPOKES;
        let mut paint = fill(self.theme.label);
        paint
            .set_stroke_width(size * 0.09)
            .set_stroke_cap(Cap::Round);
        for spoke in 0..SPOKES {
            let age = (phase + SPOKES - spoke) % SPOKES;
            paint.set_color(faded(self.theme.label, 0.85 - 0.06 * age as f32));
            let angle = spoke as f32 / SPOKES as f32 * std::f32::consts::TAU;
            let (sin, cos) = angle.sin_cos();
            let (inner, outer) = (size * 0.25, size * 0.5);
            self.canvas.draw_line(
                (center.x + sin * inner, center.y - cos * inner),
                (center.x + sin * outer, center.y - cos * outer),
                &paint,
            );
        }
    }

    // MARK: Controls

    /// Draws a push button filling `rect`; height 28 and up are capsules like large controls.
    pub fn button(&mut self, button: Button, rect: Rect) {
        let theme = self.theme;
        let pressed = button.enabled && self.is_pressed(&button.action);
        let radius = if rect.height() >= 28.0 {
            rect.height() / 2.0
        } else {
            rect.height() / 3.0
        };
        let shape = rounded(rect, radius);
        let label = match button.style {
            ButtonStyle::Bordered | ButtonStyle::Clear => {
                let background = match (button.style, pressed) {
                    (_, true) => Some(theme.control_pressed),
                    (ButtonStyle::Bordered, false) => Some(theme.control),
                    _ => None,
                };
                if let Some(background) = background {
                    self.canvas.draw_rrect(shape, &fill(background));
                }
                button.color.unwrap_or(theme.label)
            }
            ButtonStyle::Prominent => {
                // AppKit drops the accent from default buttons in a background window.
                if button.enabled && !self.is_active {
                    self.canvas.draw_rrect(shape, &fill(theme.control));
                    let y = rect.center_y() - text::BODY.line / 2.0;
                    self.line_centered(button.title, text::BODY, theme.label, rect.center_x(), y);
                    self.hit(rect, button.action);
                    return;
                }
                let background = match (button.enabled, pressed) {
                    (false, _) => theme.control,
                    (true, true) => theme.accent.with_a(200),
                    (true, false) => theme.accent,
                };
                self.canvas.draw_rrect(shape, &fill(background));
                Color::WHITE
            }
        };
        let label = if button.enabled {
            label
        } else {
            theme.tertiary
        };
        let y = rect.center_y() - text::BODY.line / 2.0;
        self.line_centered(button.title, text::BODY, label, rect.center_x(), y);
        self.hit(
            rect,
            if button.enabled {
                button.action
            } else {
                Action::Block
            },
        );
    }

    /// Draws the `traffic lights`; `is_active` colours them.
    pub fn traffic_lights(&mut self) {
        const LIGHTS: [(f32, Color, Action); 3] = [
            (26.0, Color::new(0xFFFF_5F57), Action::CloseWindow),
            (49.0, Color::new(0xFFFE_BC2E), Action::MinimizeWindow),
            (72.0, Color::new(0xFF28_C840), Action::ZoomWindow),
        ];
        for (x, color, action) in LIGHTS {
            let center = Point::new(x, 26.0);
            let oval = Rect::from_xywh(x - 7.0, 19.0, 14.0, 14.0);
            let mut paint = fill(color);
            if !self.is_active {
                let (top, bottom) = if self.theme.is_dark {
                    (Color::new(0xFF55_5555), Color::new(0xFF4A_4A4A))
                } else {
                    (Color::new(0xFFEA_EAEC), Color::new(0xFFF7_F7F9))
                };
                paint.set_shader(self.vertical_gradient(oval, top, bottom));
            }
            self.canvas.draw_circle(center, 7.0, &paint);
            self.canvas
                .draw_circle(center, 6.75, &outline(self.theme.glass_rim, 0.75));
            self.hit(oval, action);
        }
    }
}
