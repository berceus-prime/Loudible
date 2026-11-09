//! A single-line editable value and how to paint it. Parley's `PlainEditor` does the editing;
//! this maps the app's keys and clicks onto it and paints its layout through Skia.

use std::cell::{Cell, RefCell};

use parley::{PlainEditor, PlainEditorDriver};
use skia_safe::{Color, Point, Rect};

use crate::stage::Motion;
use crate::text::{TextStyle, TextSystem};
use crate::ui::{Ui, fill};

const BLINK_MS: u128 = 530;

pub struct TextField {
    /// Behind a `RefCell` so painting, which only borrows the field, can bring its layout up to
    /// date after edits.
    editor: RefCell<PlainEditor<()>>,
    style: TextStyle,
    /// Whether the editor has `style`'s fonts yet, which takes a `TextSystem` to work out.
    is_styled: Cell<bool>,
    /// How far the value is scrolled left to keep the caret in view, as last painted.
    scroll: Cell<f32>,
    /// The editing area in window points, as last painted while focused.
    input_area: Cell<Option<Rect>>,
}

impl TextField {
    pub fn new(style: TextStyle) -> Self {
        let mut editor = PlainEditor::new(style.size);
        editor.set_width(None);
        Self {
            editor: RefCell::new(editor),
            style,
            is_styled: Cell::new(false),
            scroll: Cell::new(0.0),
            input_area: Cell::new(None),
        }
    }

    /// The value, without any unconfirmed input method text.
    pub fn text(&self) -> String {
        self.editor.borrow().text().chars().collect()
    }

    pub fn selected_text(&self) -> Option<String> {
        self.editor.borrow().selected_text().map(str::to_string)
    }

    pub fn input_area(&self) -> Option<Rect> {
        self.input_area.get()
    }

    /// Replaces the value with the caret at its end.
    pub fn set(&mut self, text: &str, system: &TextSystem) {
        self.editor.get_mut().set_text(text);
        self.drive(system, |driver| driver.move_to_text_end());
    }

    pub fn move_to_start(&mut self, system: &TextSystem) {
        self.drive(system, |driver| driver.move_to_text_start());
    }

    /// Gives the editor `style`'s fonts the first time a `TextSystem` is at hand.
    fn prepare(
        editor: &mut PlainEditor<()>,
        is_styled: &Cell<bool>,
        style: TextStyle,
        system: &TextSystem,
    ) {
        if !is_styled.replace(true) {
            let styles = editor.edit_styles();
            for property in system.edit_styles(style) {
                styles.insert(property);
            }
        }
    }

    fn drive(&mut self, system: &TextSystem, edit: impl FnOnce(&mut PlainEditorDriver<'_, ()>)) {
        let editor = self.editor.get_mut();
        Self::prepare(editor, &self.is_styled, self.style, system);
        system.edit(|fonts, layouts| edit(&mut editor.driver(fonts, layouts)));
    }

    pub fn insert(&mut self, text: &str, system: &TextSystem) {
        // A pasted address may end in a line break; the field is a single line.
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if !text.is_empty() && !self.editor.get_mut().is_composing() {
            self.drive(system, |driver| driver.insert_or_replace_selection(&text));
        }
    }

    pub fn backspace(&mut self, word: bool, system: &TextSystem) {
        self.drive(system, |driver| match word {
            true => driver.backdelete_word(),
            false => driver.backdelete(),
        });
    }

    pub fn delete(&mut self, word: bool, system: &TextSystem) {
        self.drive(system, |driver| match word {
            true => driver.delete_word(),
            false => driver.delete(),
        });
    }

    pub fn delete_selection(&mut self, system: &TextSystem) {
        self.drive(system, |driver| driver.delete_selection());
    }

    pub fn move_caret(&mut self, to: Motion, select: bool, system: &TextSystem) {
        self.drive(system, |driver| match (to, select) {
            (Motion::Left, false) => driver.move_left(),
            (Motion::Right, false) => driver.move_right(),
            (Motion::WordLeft, false) => driver.move_word_left(),
            (Motion::WordRight, false) => driver.move_word_right(),
            (Motion::Start, false) => driver.move_to_text_start(),
            (Motion::End, false) => driver.move_to_text_end(),
            (Motion::Left, true) => driver.select_left(),
            (Motion::Right, true) => driver.select_right(),
            (Motion::WordLeft, true) => driver.select_word_left(),
            (Motion::WordRight, true) => driver.select_word_right(),
            (Motion::Start, true) => driver.select_to_text_start(),
            (Motion::End, true) => driver.select_to_text_end(),
        });
    }

    pub fn select_all(&mut self, system: &TextSystem) {
        self.drive(system, |driver| driver.select_all());
    }

    /// Shows or withdraws an input method's unconfirmed text.
    pub fn compose(&mut self, text: &str, cursor: Option<(usize, usize)>, system: &TextSystem) {
        self.drive(system, |driver| match text.is_empty() {
            true => driver.clear_compose(),
            false => driver.set_compose(text, cursor),
        });
    }

    pub fn commit(&mut self, text: &str, system: &TextSystem) {
        self.drive(system, |driver| {
            driver.clear_compose();
            driver.insert_or_replace_selection(text);
        });
    }

    /// Keeps any unconfirmed text and drops the selection, as focus leaves the field.
    pub fn blur(&mut self, system: &TextSystem) {
        self.drive(system, |driver| {
            driver.finish_compose();
            driver.collapse_selection();
        });
        self.input_area.set(None);
    }

    /// A press `x` points right of where the value is painted from: places the caret, or with
    /// more clicks selects a word or everything; with `extend`, selects up to it.
    pub fn press(&mut self, x: f32, clicks: u8, extend: bool, system: &TextSystem) {
        let (x, y) = (x + self.scroll.get(), self.style.line / 2.0);
        self.drive(system, |driver| match (extend, clicks) {
            (true, _) => driver.shift_click_extension(x, y),
            (false, 2) => driver.select_word_at_point(x, y),
            (false, 3..) => driver.select_all(),
            (false, _) => driver.move_to_point(x, y),
        });
    }

    /// The pointer moved to `x` while pressed: selects from the press to it.
    pub fn drag(&mut self, x: f32, system: &TextSystem) {
        let (x, y) = (x + self.scroll.get(), self.style.line / 2.0);
        self.drive(system, |driver| driver.extend_selection_to_point(x, y));
    }

    /// Paints the value or placeholder on one line starting at (`x`, `y`), scrolled so the
    /// caret stays inside `width`. The caret blinks while `focused`.
    pub fn draw(
        &self,
        ui: &mut Ui,
        placeholder: &str,
        placeholder_color: Color,
        (x, y): (f32, f32),
        width: f32,
        focused: bool,
    ) {
        let style = self.style;
        let mut editor = self.editor.borrow_mut();
        Self::prepare(&mut editor, &self.is_styled, style, ui.text);
        ui.text
            .edit(|fonts, layouts| editor.refresh_layout(fonts, layouts));
        let caret = editor.cursor_geometry(1.0);
        let caret_x = caret.map_or(0.0, |caret| caret.x0 as f32);
        let offset = (caret_x - width + 2.0).max(0.0);
        self.scroll.set(offset);
        let left = x - offset;
        if focused {
            let area = editor.ime_cursor_area();
            self.input_area.set(Some(Rect::from_ltrb(
                left + area.x0 as f32,
                y + area.y0 as f32,
                left + area.x1 as f32,
                y + area.y1 as f32,
            )));
        }
        let clip = Rect::from_xywh(x - 1.0, y - 2.0, width + 2.0, style.line + 4.0);
        ui.clip(clip, |ui| {
            if editor.raw_text().is_empty() {
                ui.line_limited(placeholder, style, placeholder_color, x, y, Some(width));
            } else {
                if focused {
                    let paint = fill(ui.theme.accent.with_a(70));
                    for (rect, _) in editor.selection_geometry() {
                        let rect = Rect::from_ltrb(
                            left + rect.x0 as f32,
                            y + rect.y0 as f32,
                            left + rect.x1 as f32,
                            y + rect.y1 as f32,
                        );
                        ui.canvas.draw_rect(rect, &paint);
                    }
                }
                if let Some(layout) = editor.try_layout() {
                    let top = y + ui.text.edit_baseline_shift(style);
                    ui.text
                        .draw_layout(ui.canvas, layout, Point::new(left, top), ui.theme.label);
                }
            }
            let blink_on = (ui.time.as_millis() / BLINK_MS).is_multiple_of(2);
            if focused
                && blink_on
                && editor.selected_text().is_none()
                && let Some(caret) = caret
            {
                let caret = Rect::from_xywh(left + caret.x0 as f32, y, 1.0, style.line);
                ui.canvas.draw_rect(caret, &fill(ui.theme.accent));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use skia_safe::surfaces;

    use super::*;
    use crate::text::BODY;
    use crate::theme::Theme;

    fn field(text: &str, system: &TextSystem) -> TextField {
        let mut field = TextField::new(BODY);
        field.set(text, system);
        field
    }

    #[test]
    fn option_backspace_deletes_a_word() {
        let system = TextSystem::default();
        let mut field = field("Henrik Ibsen", &system);
        field.backspace(true, &system);
        assert_eq!(field.text(), "Henrik ");
    }

    #[test]
    fn typing_replaces_a_shift_selection() {
        let system = TextSystem::default();
        let mut field = field("Ibsen", &system);
        field.move_caret(Motion::Left, true, &system);
        field.move_caret(Motion::Left, true, &system);
        assert_eq!(field.selected_text().as_deref(), Some("en"));
        field.insert("EN", &system);
        assert_eq!(field.text(), "IbsEN");
    }

    #[test]
    fn a_pasted_line_break_is_dropped() {
        let system = TextSystem::default();
        let mut field = field("", &system);
        field.insert("https://example.com/\n", &system);
        assert_eq!(field.text(), "https://example.com/");
    }

    #[test]
    fn a_double_click_selects_the_word() {
        let system = TextSystem::default();
        let mut field = field("Henrik Ibsen plays", &system);
        let x = system.width("Henrik Ib", BODY);
        field.press(x, 2, false, &system);
        assert_eq!(field.selected_text().as_deref(), Some("Ibsen"));
    }

    #[test]
    fn composing_text_is_not_the_value_until_committed() {
        let system = TextSystem::default();
        let mut field = field("Ibsen ", &system);
        field.compose("にほん", Some((9, 9)), &system);
        assert_eq!(field.text(), "Ibsen ");
        field.insert("x", &system);
        field.compose("", None, &system);
        field.commit("日本", &system);
        assert_eq!(field.text(), "Ibsen 日本");
    }

    #[test]
    fn a_click_lands_in_the_scrolled_text() {
        let system = TextSystem::default();
        let mut field = field("A rather long search that cannot fit in the box", &system);
        let mut surface = surfaces::raster_n32_premul((200, 40)).expect("surface");
        let mut ui = Ui::new(
            surface.canvas(),
            Theme::LIGHT,
            &system,
            None,
            None,
            Duration::ZERO,
            true,
        );
        field.draw(&mut ui, "", Color::BLACK, (0.0, 0.0), 80.0, true);
        field.press(79.0, 1, false, &system);
        field.insert("!", &system);
        assert!(field.text().ends_with("box!"), "{}", field.text());
    }
}
