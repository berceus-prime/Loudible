//! A single-line editable value and how to paint it.

use skia_safe::{Color, Rect};

use crate::text::TextStyle;
use crate::ui::{Ui, fill};

const BLINK_MS: u128 = 530;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextField {
    pub text: String,
    /// Byte offset of the insertion point, always on a character boundary.
    pub caret: usize,
    pub all_selected: bool,
}

impl TextField {
    pub fn set(&mut self, text: String) {
        self.caret = text.len();
        self.text = text;
        self.all_selected = false;
    }

    fn replace_selection(&mut self) {
        if std::mem::take(&mut self.all_selected) {
            self.text.clear();
            self.caret = 0;
        }
    }

    pub fn insert(&mut self, text: &str) {
        self.replace_selection();
        // A pasted address may end in a line break; the field is a single line.
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        self.text.insert_str(self.caret, &text);
        self.caret += text.len();
    }

    fn previous(&self) -> usize {
        self.text[..self.caret]
            .char_indices()
            .last()
            .map_or(0, |(i, _)| i)
    }

    fn next(&self) -> usize {
        self.text[self.caret..]
            .chars()
            .next()
            .map_or(self.caret, |c| self.caret + c.len_utf8())
    }

    pub fn backspace(&mut self) {
        if self.all_selected {
            self.replace_selection();
        } else if self.caret > 0 {
            let start = self.previous();
            self.text.replace_range(start..self.caret, "");
            self.caret = start;
        }
    }

    pub fn delete_forward(&mut self) {
        if self.all_selected {
            self.replace_selection();
        } else if self.caret < self.text.len() {
            let end = self.next();
            self.text.replace_range(self.caret..end, "");
        }
    }

    pub fn move_left(&mut self) {
        self.caret = if std::mem::take(&mut self.all_selected) {
            0
        } else {
            self.previous()
        };
    }

    pub fn move_right(&mut self) {
        self.caret = if std::mem::take(&mut self.all_selected) {
            self.text.len()
        } else {
            self.next()
        };
    }

    pub fn move_to(&mut self, caret: usize) {
        self.all_selected = false;
        self.caret = caret;
    }

    pub fn select_all(&mut self) {
        self.all_selected = !self.text.is_empty();
    }
}

/// Paints the value or placeholder on one line starting at (`x`, `y`), scrolled so the caret
/// stays inside `width`. The caret blinks while `focused`.
#[allow(clippy::too_many_arguments)]
pub fn draw_value(
    ui: &mut Ui,
    field: &TextField,
    placeholder: &str,
    placeholder_color: Color,
    style: TextStyle,
    (x, y): (f32, f32),
    width: f32,
    focused: bool,
) {
    let clip = Rect::from_xywh(x - 1.0, y - 2.0, width + 2.0, style.line + 4.0);
    ui.clip(clip, |ui| {
        if field.text.is_empty() {
            ui.line_limited(placeholder, style, placeholder_color, x, y, Some(width));
        } else {
            let caret_x = ui.text.width(&field.text[..field.caret], style);
            let offset = (caret_x - width + 2.0).max(0.0);
            if focused && field.all_selected {
                let selection =
                    Rect::from_xywh(x - offset, y, ui.text.width(&field.text, style), style.line);
                ui.canvas
                    .draw_rect(selection, &fill(ui.theme.accent.with_a(70)));
            }
            ui.line(&field.text, style, ui.theme.label, x - offset, y);
        }
        if focused && !field.all_selected && (ui.time.as_millis() / BLINK_MS).is_multiple_of(2) {
            let caret_x = ui.text.width(&field.text[..field.caret], style);
            let offset = (caret_x - width + 2.0).max(0.0);
            let caret = Rect::from_xywh(x + caret_x - offset, y, 1.0, style.line);
            ui.canvas.draw_rect(caret, &fill(ui.theme.accent));
        }
    });
}
