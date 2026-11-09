//! Things that float over the window: menus, the popover's frame, sheets and alerts.

use skia_safe::{Color, PathBuilder, Point, RRect, Rect, Size};

use crate::icons::Icon;
use crate::text;
use crate::ui::{Action, Button, ButtonStyle, Ui, fill, outline, rounded};

const FLOAT_SHADOW: Color = Color::new(0x4000_0000);

#[derive(Clone, Debug, PartialEq)]
pub struct MenuEntry {
    pub title: String,
    /// `None` shows the entry dimmed: a disabled item, or the line explaining one.
    pub action: Option<Action>,
    pub is_destructive: bool,
    pub is_checked: bool,
}

const MENU_ROW: f32 = 22.0;
const MENU_PADDING: f32 = 6.0;

/// Draws a menu at `anchor`, moved or flipped to stay inside the window.
pub fn popup_menu(ui: &mut Ui, entries: &[MenuEntry], anchor: Point, window: Size) {
    let theme = ui.theme;
    let leading = if entries.iter().any(|e| e.is_checked) {
        24.0
    } else {
        12.0
    };
    let widest = entries
        .iter()
        .map(|e| ui.text.width(&e.title, text::BODY))
        .fold(0.0, f32::max);
    let width = (widest + leading + 12.0 + MENU_PADDING * 2.0).max(120.0);
    let height = entries.len() as f32 * MENU_ROW + MENU_PADDING * 2.0;
    let x = anchor.x.min(window.width - width - 8.0).max(8.0);
    let y = if anchor.y + height > window.height - 8.0 {
        (anchor.y - height).max(8.0)
    } else {
        anchor.y
    };
    let rect = Rect::from_xywh(x, y, width, height);
    let shape = rounded(rect, 12.0);
    ui.shadow(shape, FLOAT_SHADOW, 20.0, 8.0);
    ui.material(shape);
    ui.canvas
        .draw_rrect(shape, &outline(theme.material_rim, 0.5));
    ui.hit(rect, Action::Block);
    for (index, entry) in entries.iter().enumerate() {
        let row = Rect::from_xywh(
            x + MENU_PADDING,
            y + MENU_PADDING + index as f32 * MENU_ROW,
            width - MENU_PADDING * 2.0,
            MENU_ROW,
        );
        let hovered = entry
            .action
            .as_ref()
            .is_some_and(|action| ui.is_hot(action));
        if hovered {
            ui.canvas.draw_rrect(rounded(row, 6.0), &fill(theme.accent));
        }
        let color = match (&entry.action, hovered, entry.is_destructive) {
            (_, true, _) => Color::WHITE,
            (None, ..) => theme.tertiary,
            (_, _, true) => theme.red,
            _ => theme.label,
        };
        if entry.is_checked {
            ui.icon(
                Icon::Check,
                Point::new(row.left + 12.0, row.center_y()),
                10.0,
                color,
            );
        }
        let text_y = row.center_y() - text::BODY.line / 2.0;
        ui.line(&entry.title, text::BODY, color, row.left + leading, text_y);
        if let Some(action) = &entry.action {
            ui.hit(row, action.clone());
        }
    }
}

/// The popover's rounded body with its arrow pointing up at `arrow_x`.
pub fn popover_frame(ui: &mut Ui, rect: Rect, arrow_x: f32) {
    const ARROW: f32 = 8.0;
    let shape = rounded(rect, 12.0);
    ui.shadow(shape, FLOAT_SHADOW, 24.0, 8.0);
    ui.material(shape);
    let mut arrow = PathBuilder::new();
    arrow
        .move_to((arrow_x - ARROW, rect.top + 0.5))
        .line_to((arrow_x, rect.top - ARROW))
        .line_to((arrow_x + ARROW, rect.top + 0.5))
        .close();
    ui.canvas
        .draw_path(&arrow.detach(), &fill(ui.theme.material.with_a(255)));
    ui.canvas
        .draw_rrect(shape, &outline(ui.theme.material_rim, 0.5));
    ui.hit(rect, Action::Block);
}

/// Dims the window and blocks everything beneath a sheet or alert.
pub fn modal_backdrop(ui: &mut Ui, window: Size) {
    let rect = Rect::from_size(window);
    ui.canvas.draw_rect(rect, &fill(Color::new(0x1A00_0000)));
    ui.hit(rect, Action::Block);
}

/// A sheet's card, drawn opaque.
pub fn sheet_card(ui: &mut Ui, rect: Rect) -> RRect {
    let shape = rounded(rect, 16.0);
    ui.shadow(shape, Color::new(0x5900_0000), 36.0, 12.0);
    ui.canvas.draw_rrect(shape, &fill(ui.theme.window));
    ui.canvas
        .draw_rrect(shape, &outline(ui.theme.material_rim, 0.5));
    shape
}

/// Asks before something that can't be undone.
pub fn confirm_alert(
    ui: &mut Ui,
    title: &str,
    message: &str,
    (confirm, confirm_action): (&str, Action),
    cancel_action: Action,
    window: Size,
) {
    const WIDTH: f32 = 280.0;
    const PADDING: f32 = 20.0;
    const BUTTON: f32 = 28.0;
    let theme = ui.theme;
    let inner = WIDTH - PADDING * 2.0;
    let title_style = text::HEADLINE;
    let title_height =
        ui.text.wrap(title, title_style, inner, None).len() as f32 * title_style.line;
    let message_height =
        ui.text.wrap(message, text::CALLOUT, inner, None).len() as f32 * text::CALLOUT.line;
    let height = PADDING * 2.0 + title_height + 6.0 + message_height + 16.0 + BUTTON * 2.0 + 8.0;
    let x = (window.width - WIDTH) / 2.0;
    let y = (window.height - height) / 2.0;
    sheet_card(ui, Rect::from_xywh(x, y, WIDTH, height));
    let left = x + PADDING;
    let mut cursor = y + PADDING;
    cursor += ui.paragraph(
        title,
        title_style,
        theme.label,
        Point::new(left, cursor),
        inner,
        None,
    ) + 6.0;
    cursor += ui.paragraph(
        message,
        text::CALLOUT,
        theme.secondary,
        Point::new(left, cursor),
        inner,
        None,
    ) + 16.0;
    let destructive = Button::new(confirm, confirm_action).color(theme.red);
    ui.button(destructive, Rect::from_xywh(left, cursor, inner, BUTTON));
    let cancel = Button::new("Cancel", cancel_action).style(ButtonStyle::Bordered);
    ui.button(
        cancel,
        Rect::from_xywh(left, cursor + BUTTON + 8.0, inner, BUTTON),
    );
}
