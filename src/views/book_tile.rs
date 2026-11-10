//! A cover with its status badge, then the title row with the actions menu.

use skia_safe::{
    Color, FilterMode, Image, MipmapMode, Paint, PaintCap, Point, Rect, SamplingOptions,
};

use crate::icons::Icon;
use crate::model::{Audiobook, BookStatus, BookTileState, CoverColor, JobKind};
use crate::text::{self, TextStyle, Weight};
use crate::ui::{Action, Ui, capsule, fill, outline, rounded};

const COVER_RADIUS: f32 = 6.0;
const MENU_WIDTH: f32 = 13.0;
const TITLE: TextStyle = text::CALLOUT.weight(Weight::Medium);
const BADGE: TextStyle = TextStyle::new(10.0, 13.0, Weight::Bold).rounded();

/// Blends `color` towards white, as the synthetic covers' stripe does.
fn lightened(color: Color, amount: f32) -> Color {
    let mix = |c: u8| (f32::from(c) + (255.0 - f32::from(c)) * amount) as u8;
    Color::from_rgb(mix(color.r()), mix(color.g()), mix(color.b()))
}

pub fn draw_cover(ui: &mut Ui, cover: Option<CoverColor>, rect: Rect) {
    match cover {
        Some(CoverColor(rgb)) => {
            let color = Color::new(0xFF00_0000 | rgb);
            ui.canvas.draw_rect(rect, &fill(color));
            let stripe = Rect::from_xywh(
                rect.left,
                rect.top + rect.height() * 0.30,
                rect.width(),
                rect.height() * 0.08,
            );
            ui.canvas.draw_rect(stripe, &fill(lightened(color, 0.35)));
        }
        None => {
            ui.canvas.draw_rect(rect, &fill(ui.theme.placeholder_fill));
            let center = Point::new(rect.center_x(), rect.center_y());
            ui.icon(Icon::BookClosed, center, 30.0, ui.theme.secondary);
        }
    }
}

fn draw_status_badge(ui: &mut Ui, status: &BookStatus, center: Point) {
    let content_visible = !matches!(
        status,
        BookStatus::None | BookStatus::Failed { .. } | BookStatus::Exported
    );
    if !content_visible {
        return;
    }
    ui.canvas
        .draw_circle(center, 13.0, &fill(Color::new(0x9900_0000)));
    match status {
        BookStatus::Queued(_) => ui.icon(Icon::Clock, center, 15.0, Color::WHITE),
        BookStatus::Downloaded => ui.icon(Icon::ArrowDownCircleFill, center, 16.0, Color::WHITE),
        BookStatus::Running { kind, progress, .. } => {
            let color = if *kind == JobKind::Export {
                ui.theme.accent
            } else {
                Color::WHITE
            };
            let radius = 8.0 - 1.25;
            let track = outline(Color::new(0x40FF_FFFF), 2.5);
            ui.canvas.draw_circle(center, radius, &track);
            let mut arc = outline(color, 2.5);
            arc.set_stroke_cap(PaintCap::Round);
            let oval = Rect::from_xywh(
                center.x - radius,
                center.y - radius,
                radius * 2.0,
                radius * 2.0,
            );
            let sweep = progress.clamp(0.02, 1.0) * 360.0;
            ui.canvas.draw_arc(oval, -90.0, sweep, false, &arc);
        }
        _ => {}
    }
}

fn draw_vertical_ellipsis(ui: &Ui, center: Point, enabled: bool) {
    let paint = fill(if enabled {
        ui.theme.label
    } else {
        ui.theme.tertiary
    });
    for offset in [-5.5, 0.0, 5.5] {
        ui.canvas
            .draw_circle((center.x, center.y + offset), 1.6, &paint);
    }
}

/// Draws the tile at (`x`, `y`) and returns its height.
pub fn book_tile(
    ui: &mut Ui,
    book: &Audiobook,
    state: &BookTileState,
    can_select: bool,
    origin: (f32, f32),
    width: f32,
) -> f32 {
    book_tile_with_cover(ui, book, state, can_select, origin, width, None)
}

/// Fills `rect` with `image`, cropping whichever sides overflow, like `scaledToFill`.
fn draw_cover_image(ui: &mut Ui, image: &Image, rect: Rect) {
    let (width, height) = (image.width() as f32, image.height() as f32);
    let side = width.min(height);
    let source = Rect::from_xywh((width - side) / 2.0, (height - side) / 2.0, side, side);
    let sampling = SamplingOptions::new(FilterMode::Linear, MipmapMode::None);
    ui.canvas.draw_image_rect_with_sampling_options(
        image,
        Some((&source, skia_safe::canvas::SrcRectConstraint::Fast)),
        rect,
        sampling,
        &Paint::default(),
    );
}

/// `book_tile`, showing `image` in place of the placeholder when it has loaded.
pub fn book_tile_with_cover(
    ui: &mut Ui,
    book: &Audiobook,
    state: &BookTileState,
    can_select: bool,
    (x, y): (f32, f32),
    width: f32,
    image: Option<&Image>,
) -> f32 {
    let theme = ui.theme;
    if !can_select {
        // A disabled button dims its whole label.
        ui.canvas.save_layer_alpha_f(None, 0.5);
    }
    let rect = Rect::from_xywh(x, y, width, width);
    let shape = rounded(rect, COVER_RADIUS);
    ui.shadow(shape, Color::new(0x2600_0000), 3.0, 1.0);
    ui.canvas.save();
    ui.canvas.clip_rrect(shape, None, true);
    match image {
        Some(image) => draw_cover_image(ui, image, rect),
        None => draw_cover(ui, book.cover, rect),
    }
    ui.canvas.restore();
    if state.is_selected {
        let inset = rounded(rect.with_inset((1.5, 1.5)), COVER_RADIUS - 1.5);
        ui.canvas.draw_rrect(inset, &outline(theme.accent, 3.0));
        let center = Point::new(rect.right - 6.0 - 9.0, rect.top + 6.0 + 9.0);
        ui.canvas.draw_circle(center, 7.0, &fill(Color::WHITE));
        let disc = rounded(
            Rect::from_xywh(center.x - 8.0, center.y - 8.0, 16.0, 16.0),
            8.0,
        );
        ui.shadow(disc, Color::new(0x2600_0000), 3.0, 1.0);
        ui.icon(Icon::CheckCircleFill, center, 17.5, theme.accent);
    } else {
        let inset = rounded(rect.with_inset((0.5, 0.5)), COVER_RADIUS - 0.5);
        ui.canvas.draw_rrect(inset, &outline(theme.cover_rim, 1.0));
    }
    draw_status_badge(
        ui,
        &state.status,
        Point::new(rect.right - 19.0, rect.bottom - 19.0),
    );
    if let Some(code) = &state.marketplace_badge {
        let label_width = ui.text.width(code, BADGE);
        let badge = Rect::from_xywh(rect.left + 6.0, rect.top + 6.0, label_width + 10.0, 17.0);
        ui.canvas
            .draw_rrect(capsule(badge), &fill(Color::new(0x9900_0000)));
        ui.line(code, BADGE, Color::WHITE, badge.left + 5.0, badge.top + 2.0);
    }

    let text_width = width - 4.0 - MENU_WIDTH;
    let text_top = rect.bottom + 6.0;
    let mut cursor = text_top;
    cursor += ui.paragraph(
        &book.title,
        TITLE,
        theme.label,
        Point::new(x, cursor),
        text_width,
        Some(2),
    );
    cursor += 2.0;
    let authors = book.authors_text();
    ui.line_limited(
        &authors,
        text::CAPTION,
        theme.secondary,
        x,
        cursor,
        Some(text_width),
    );
    cursor += text::CAPTION.line;
    if !state.is_in_library {
        cursor += 2.0;
        ui.line("Not in library", text::CAPTION, theme.orange, x, cursor);
        cursor += text::CAPTION.line;
    }
    if let Some(transferred) = state.status.transferred() {
        cursor += 2.0;
        let style = text::CAPTION.tabular().weight(Weight::Medium);
        ui.line(&transferred.text(), style, theme.secondary, x, cursor);
        cursor += text::CAPTION.line;
    } else if let Some(runtime) = book.runtime_text() {
        cursor += 2.0;
        ui.line(&runtime, text::CAPTION, theme.tertiary, x, cursor);
        cursor += text::CAPTION.line;
    }

    if !can_select {
        ui.canvas.restore();
    }
    let toggle = if can_select {
        Action::ToggleBook(book.asin.clone())
    } else {
        Action::Block
    };
    let menu_point = Point::new(x + width - MENU_WIDTH, text_top + 20.0);
    let menu = Action::OpenBookMenu(book.asin.clone(), menu_point);
    let context = (!state.menu_items.is_empty()).then(|| menu.clone());
    let tooltip = book.tooltip();
    let selectable = Rect::from_ltrb(x, y, x + width, rect.bottom);
    ui.hit_with(
        selectable,
        toggle.clone(),
        context.clone(),
        Some(&tooltip),
        false,
    );
    let label = Rect::from_ltrb(x, text_top, x + text_width + 4.0, cursor);
    ui.hit_with(label, toggle, context, Some(&tooltip), false);

    let dots = Point::new(x + width - MENU_WIDTH / 2.0, text_top + 8.5);
    draw_vertical_ellipsis(ui, dots, !state.menu_items.is_empty());
    let target = Rect::from_xywh(x + width - MENU_WIDTH, text_top, MENU_WIDTH, 20.0);
    if !state.menu_items.is_empty() {
        ui.hit_with(target, menu, None, Some("More actions"), false);
    }
    cursor - y
}
