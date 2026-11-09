//! The cover grid, its placeholders, and the messages bar above it.

use std::borrow::Cow;

use skia_safe::{Color, Point, Rect};

use crate::icons::Icon;
use crate::model::{Audiobook, BookTileState};
use crate::shell::covers::CoverStore;
use crate::text;
use crate::theme::{Theme, faded};
use crate::ui::{Action, Ui, capsule, fill};
use crate::views::book_tile::book_tile_with_cover;

const MIN_TILE: f32 = 130.0;
const SPACING: f32 = 18.0;
const ROW_SPACING: f32 = 24.0;
const PADDING: f32 = 20.0;

pub struct GridProps<'a> {
    pub books: Vec<(&'a Audiobook, Cow<'a, BookTileState>)>,
    pub is_library_empty: bool,
    pub is_signed_in: bool,
    pub is_loading: bool,
    pub is_checking_marketplaces: bool,
    pub can_select: bool,
    pub scroll: f32,
    /// Opacity of the overlay scroller, 0 once it has faded.
    pub scroller_opacity: f32,
    /// Real cover images; `None` shows each book's own synthetic cover.
    pub covers: Option<&'a CoverStore>,
}

impl GridProps<'_> {
    fn placeholder(&self) -> Option<&'static str> {
        if !self.is_library_empty || !self.is_signed_in {
            return None;
        }
        Some(if self.is_loading {
            "Loading library…"
        } else if self.is_checking_marketplaces {
            "Checking other marketplaces…"
        } else {
            "No purchased audiobooks"
        })
    }
}

/// Draws the grid into `rect` and returns the height of its content.
pub fn library_grid(ui: &mut Ui, props: &GridProps, rect: Rect) -> f32 {
    ui.hit(rect, Action::ClearSelection);
    let inner = rect.width() - PADDING * 2.0;
    let columns = (((inner + SPACING) / (MIN_TILE + SPACING)).floor() as usize).max(1);
    let tile = (inner - SPACING * (columns - 1) as f32) / columns as f32;

    let mut content = PADDING;
    ui.clip(rect, |ui| {
        for row in props.books.chunks(columns) {
            let top = rect.top + content - props.scroll;
            let mut height = 0.0_f32;
            for (column, (book, state)) in row.iter().enumerate() {
                let x = rect.left + PADDING + column as f32 * (tile + SPACING);
                let cover = book_cover(props, book);
                let h = book_tile_with_cover(
                    ui,
                    book,
                    state,
                    props.can_select,
                    (x, top),
                    tile,
                    cover.as_ref(),
                );
                height = height.max(h);
            }
            content += height + ROW_SPACING;
        }
    });
    let content = if props.books.is_empty() {
        0.0
    } else {
        content - ROW_SPACING + PADDING
    };

    if let Some(placeholder) = props.placeholder() {
        let y = rect.center_y() - text::BODY.line / 2.0;
        ui.line_centered(
            placeholder,
            text::BODY,
            ui.theme.secondary,
            rect.center_x(),
            y,
        );
    }
    if props.scroller_opacity > 0.0 && content > rect.height() {
        let track = rect.height() - 4.0;
        let height = (track * rect.height() / content).max(32.0);
        let progress = props.scroll / (content - rect.height());
        let thumb = Rect::from_xywh(
            rect.right - 9.0,
            rect.top + 2.0 + (track - height) * progress,
            6.0,
            height,
        );
        let color = faded(ui.theme.secondary, props.scroller_opacity);
        ui.canvas.draw_rrect(capsule(thumb), &fill(color));
    }
    content
}

fn book_cover(props: &GridProps, book: &Audiobook) -> Option<skia_safe::Image> {
    props.covers?.image(book.cover_url.as_deref()?)
}

/// The bar's background for these messages, if it shows at all.
pub fn messages_tint(theme: &Theme, messages: &[String], notices: &[String]) -> Option<Color> {
    match (messages.is_empty(), notices.is_empty()) {
        (true, true) => None,
        (true, false) => Some(faded(theme.secondary, 0.1)),
        (false, _) => Some(faded(theme.red, 0.08)),
    }
}

/// Draws error messages, then notices, from `y`, and returns the height; 0 when there are none.
pub fn messages_bar(
    ui: &mut Ui,
    messages: &[String],
    notices: &[String],
    x: f32,
    y: f32,
    width: f32,
) -> f32 {
    if messages.is_empty() && notices.is_empty() {
        return 0.0;
    }
    const ICON: f32 = 14.0;
    const ICON_GAP: f32 = 10.0;
    let theme = ui.theme;
    let text_width = width - 24.0 - ICON - ICON_GAP;
    let height_of = |ui: &Ui, entry: &str| {
        ui.text.wrap(entry, text::BODY, text_width, None).len() as f32 * text::BODY.line
    };
    let entries = messages.iter().chain(notices);
    let content: f32 =
        entries.clone().map(|e| height_of(ui, e)).sum::<f32>() + 4.0 * (entries.count() - 1) as f32;
    let total = content + 16.0;

    if let Some(tint) = messages_tint(&theme, messages, notices) {
        ui.canvas
            .draw_rect(Rect::from_xywh(x, y, width, total), &fill(tint));
    }

    let mut cursor = y + 8.0;
    let mut row = |ui: &mut Ui, entry: &str, icon, color| {
        let center = Point::new(x + 12.0 + ICON / 2.0, cursor + text::BODY.line / 2.0);
        ui.icon(icon, center, ICON, color);
        let origin = Point::new(x + 12.0 + ICON + ICON_GAP, cursor);
        cursor += ui.paragraph(entry, text::BODY, color, origin, text_width, None) + 4.0;
    };
    for message in messages {
        row(ui, message, Icon::WarningFill, theme.red);
    }
    for notice in notices {
        row(ui, notice, Icon::Info, theme.secondary);
    }
    total
}
