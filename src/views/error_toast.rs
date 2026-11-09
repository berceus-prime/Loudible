//! An error's readable guidance, its technical details, and copying them.

use skia_safe::{Color, Point, Rect};

use crate::icons::Icon;
use crate::model::AppError;
use crate::text;
use crate::ui::{Action, Ui, fill, outline, rounded};

pub const MAX_WIDTH: f32 = 440.0;
const PADDING: f32 = 16.0;
const ICON_WIDTH: f32 = 19.5;
const CLOSE: f32 = 24.0;
const GAP: f32 = 12.0;
const STACK: f32 = 8.0;
const DETAILS_MAX_HEIGHT: f32 = 140.0;
const DISCLOSURE: f32 = 22.0;

pub struct ToastProps<'a> {
    pub id: u64,
    pub error: &'a AppError,
    pub is_copied: bool,
    pub shows_details: bool,
}

/// Draws the toast at (`x`, `y`) and returns its height.
pub fn error_toast(ui: &mut Ui, props: &ToastProps, (x, y): (f32, f32), width: f32) -> f32 {
    let height = ui.measure(|ui| toast_column(ui, props, (x, y), width)) + PADDING * 2.0;
    let rect = Rect::from_xywh(x, y, width, height);
    let shape = rounded(rect, 12.0);
    ui.shadow(shape, Color::new(0x1F00_0000), 12.0, 4.0);
    ui.material(shape);
    ui.canvas.draw_rrect(
        rounded(rect.with_inset((0.5, 0.5)), 11.5),
        &outline(ui.theme.material_rim, 1.0),
    );
    ui.hit(rect, Action::Block);

    let icon = Point::new(x + PADDING + ICON_WIDTH / 2.0, y + PADDING + 9.5);
    ui.icon(Icon::ErrorFill, icon, 18.5, ui.theme.red);
    toast_column(ui, props, (x, y), width);

    let close = Rect::from_xywh(x + width - PADDING - CLOSE, y + PADDING, CLOSE, CLOSE);
    let center = Point::new(close.center_x(), close.center_y());
    ui.icon(Icon::XMark, center, 11.0, ui.theme.secondary);
    let action = Action::DismissToast(props.id);
    ui.hit_with(close, action, None, Some("Dismiss error"), false);
    height
}

/// The text column; returns its height.
fn toast_column(ui: &mut Ui, props: &ToastProps, (x, y): (f32, f32), width: f32) -> f32 {
    let theme = ui.theme;
    let left = x + PADDING + ICON_WIDTH + GAP;
    let column = width - PADDING * 2.0 - ICON_WIDTH - GAP * 2.0 - CLOSE;
    let top = y + PADDING;
    let mut cursor = top;
    let error = props.error;
    cursor += ui.paragraph(
        error.title(),
        text::HEADLINE,
        theme.on_material,
        Point::new(left, cursor),
        column,
        None,
    ) + STACK;
    let detail = error.detail();
    cursor += ui.paragraph(
        &detail,
        text::CALLOUT,
        theme.secondary,
        Point::new(left, cursor),
        column,
        None,
    ) + STACK;

    let row = Rect::from_xywh(left, cursor, column, DISCLOSURE);
    let chevron = if props.shows_details {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    ui.icon(
        chevron,
        Point::new(left + 3.5, row.center_y()),
        9.0,
        theme.tertiary,
    );
    let label_y = row.center_y() - text::CALLOUT.line / 2.0;
    ui.line(
        "Details",
        text::CALLOUT,
        theme.on_material,
        left + 12.0,
        label_y,
    );
    ui.hit(
        Rect::from_xywh(left, cursor, 60.0, DISCLOSURE),
        Action::ToggleToastDetails(props.id),
    );
    cursor += DISCLOSURE;

    if !props.shows_details {
        cursor += STACK;
    } else {
        let technical = error.technical();
        let lines = ui.text.wrap(technical, text::CAPTION, column, None);
        let height = (lines.len() as f32 * text::CAPTION.line).min(DETAILS_MAX_HEIGHT);
        ui.clip(Rect::from_xywh(left, cursor, column, height), |ui| {
            ui.paragraph(
                technical,
                text::CAPTION,
                theme.on_material,
                Point::new(left, cursor),
                column,
                None,
            );
        });
        cursor += height + STACK;
    }

    let title = if props.is_copied {
        "Copied"
    } else {
        "Copy Details"
    };
    let icon = if props.is_copied {
        Icon::Check
    } else {
        Icon::DocOnDoc
    };
    let label = crate::text::TextStyle::new(11.0, 14.0, crate::text::Weight::Regular);
    let width = ui.text.width(title, label) + 42.0;
    let button = Rect::from_xywh(left, cursor, width, 22.0);
    let shape = rounded(button, 7.0);
    ui.shadow(shape, Color::new(0x1400_0000), 6.0, 0.0);
    let action = Action::CopyToast(props.id);
    let background = if ui.is_pressed(&action) {
        theme.control_pressed
    } else {
        theme.toast_button
    };
    ui.canvas.draw_rrect(shape, &fill(background));
    ui.icon(
        icon,
        Point::new(button.left + 17.0, button.center_y()),
        15.0,
        theme.on_material,
    );
    ui.line(
        title,
        label,
        theme.on_material,
        button.left + 32.0,
        cursor + 4.0,
    );
    ui.hit_with(
        button,
        action,
        None,
        Some("Copy the full technical error to the clipboard"),
        false,
    );
    cursor + 22.0 - top
}
