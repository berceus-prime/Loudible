//! Browse every preview state in one window.

use std::time::Duration;

use skia_safe::{Color, Point, Rect, Size};

use crate::previews::{self, Preview};
use crate::stage::{Effect, Key, Screen};
use crate::text::{self, TextSystem};
use crate::ui::{Action, Ui, fill};

const SIDEBAR: f32 = 280.0;
const ROW: f32 = 22.0;

pub struct GalleryView {
    previews: Vec<Preview>,
    selected: usize,
    scroll: f32,
}

impl Default for GalleryView {
    fn default() -> Self {
        Self {
            previews: previews::all(),
            selected: 0,
            scroll: 0.0,
        }
    }
}

impl Screen for GalleryView {
    fn render(&mut self, ui: &mut Ui, size: Size) {
        let theme = ui.theme;
        ui.canvas.clear(theme.placeholder_fill.with_a(255));
        let list = Rect::from_wh(SIDEBAR, size.height);
        ui.canvas.draw_rect(list, &fill(theme.window));
        ui.hit(
            Rect::from_ltrb(0.0, 0.0, size.width, 28.0),
            Action::DragWindow,
        );
        ui.clip(Rect::from_ltrb(0.0, 28.0, SIDEBAR, size.height), |ui| {
            for (index, preview) in self.previews.iter().enumerate() {
                let y = 28.0 + index as f32 * ROW - self.scroll;
                let row = Rect::from_xywh(8.0, y, SIDEBAR - 16.0, ROW);
                let selected = index == self.selected;
                if selected {
                    ui.canvas
                        .draw_rrect(crate::ui::rounded(row, 6.0), &fill(theme.accent));
                }
                let color = if selected { Color::WHITE } else { theme.label };
                ui.line_limited(
                    preview.title,
                    text::BODY,
                    color,
                    16.0,
                    y + 3.0,
                    Some(SIDEBAR - 32.0),
                );
                ui.hit(row, Action::Gallery(index));
            }
        });

        let preview = &self.previews[self.selected];
        let (width, height) = (preview.width, preview.height);
        let x = SIDEBAR + ((size.width - SIDEBAR - width) / 2.0).max(8.0);
        let y = ((size.height - height) / 2.0).max(8.0);
        let hits = ui.hits.len();
        ui.canvas.save();
        ui.canvas.translate((x, y));
        ui.canvas
            .clip_rect(Rect::from_wh(width, height), None, true);
        ui.canvas.clear(theme.window);
        (preview.draw)(ui, Size::new(width, height));
        ui.canvas.restore();
        // A preview is a picture here; its controls have nothing behind them.
        ui.hits.truncate(hits);
    }

    fn handle(&mut self, action: Action, _: Point, _: &TextSystem) -> Option<Effect> {
        match action {
            Action::Gallery(index) => self.selected = index,
            Action::DragWindow => return Some(Effect::Drag),
            _ => {}
        }
        None
    }

    fn key(&mut self, _: Key) -> Option<Effect> {
        None
    }

    fn scroll(&mut self, delta_y: f32) {
        let max = (self.previews.len() as f32 * ROW - 400.0).max(0.0);
        self.scroll = (self.scroll - delta_y).clamp(0.0, max);
    }

    fn tick(&mut self, _: Duration) {}

    fn next_frame(&self) -> Option<Duration> {
        None
    }
}
