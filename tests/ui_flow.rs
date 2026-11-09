//! Clicks through the real `AppView`: paint a frame, find a control's hit area, click its centre.

use std::time::Duration;

use skia_safe::{Canvas, Point, Size, surfaces};
use skia_ui::{
    model::{BookAction, BookStatus, MockOptions},
    shell::app_view::AppView,
    stage::{Effect, Key, Screen, Stage},
    theme::Theme,
    ui::{Action, Field},
};

const SIZE: Size = Size::new(1100.0, 652.0);

struct Flow(Stage<AppView>);

impl Flow {
    fn new(options: MockOptions) -> Self {
        Self(Stage::new(AppView::new(options), Theme::LIGHT))
    }

    fn frame(&mut self) {
        let mut surface = surfaces::raster_n32_premul((1100, 652)).unwrap();
        let canvas: &Canvas = surface.canvas();
        self.0.draw(canvas, SIZE);
    }

    fn wait(&mut self, ms: u64) {
        self.0.advance(Duration::from_millis(ms));
        self.frame();
    }

    fn center_of(&mut self, matches: impl Fn(&Action) -> bool) -> Point {
        self.frame();
        let hit = self
            .0
            .hits()
            .iter()
            .rev()
            .find(|hit| matches(&hit.action))
            .expect("control is on screen");
        Point::new(hit.rect.center_x(), hit.rect.center_y())
    }

    fn click(&mut self, matches: impl Fn(&Action) -> bool) -> Option<Effect> {
        let point = self.center_of(matches);
        let effect = self.0.click(point);
        self.frame();
        effect
    }

    fn app(&self) -> &skia_ui::model::MockApp {
        &self.0.screen.app
    }

    fn status(&self, asin: &str) -> BookStatus {
        let book = *self
            .app()
            .visible_books()
            .iter()
            .find(|b| &*b.asin == asin)
            .unwrap();
        self.app().status(book)
    }
}

fn signed_in() -> Flow {
    let mut flow = Flow::new(MockOptions {
        signed_in: true,
        fail_load: false,
    });
    flow.wait(100);
    flow.wait(100);
    for _ in 0..20 {
        flow.wait(100);
    }
    assert!(!flow.app().is_busy());
    flow
}

#[test]
fn signing_in_through_the_sheet_loads_the_library() {
    let mut flow = Flow::new(MockOptions::default());
    flow.click(|a| *a == Action::SignIn);
    assert!(flow.app().pending_login().is_some());
    flow.click(|a| *a == Action::SimulateSignIn);
    for _ in 0..40 {
        flow.wait(100);
    }
    assert!(flow.app().is_signed_in());
    assert_eq!(flow.app().visible_books().len(), 10);
}

#[test]
fn a_sheet_blocks_the_window_beneath_it() {
    let mut flow = Flow::new(MockOptions::default());
    flow.click(|a| *a == Action::SignIn);
    let refresh_area = Point::new(1080.0, 26.0);
    flow.0.click(refresh_area);
    assert!(flow.app().pending_login().is_some());
    assert_eq!(flow.0.key(Key::Escape), None);
    assert!(flow.app().pending_login().is_none());
}

#[test]
fn clicking_covers_selects_and_the_background_clears() {
    let mut flow = signed_in();
    flow.click(|a| *a == Action::ToggleBook("PREVIEW001".into()));
    flow.click(|a| *a == Action::ToggleBook("PREVIEW002".into()));
    assert_eq!(flow.app().selected_count(), 2);
    flow.click(|a| *a == Action::OnSelection(BookAction::Download));
    assert_eq!(flow.app().selected_count(), 0);
    assert!(flow.app().is_busy());
}

#[test]
fn the_dots_menu_starts_a_download_without_selecting() {
    let mut flow = signed_in();
    flow.click(|a| matches!(a, Action::OpenBookMenu(asin, _) if &**asin == "PREVIEW002"));
    assert!(flow.0.screen.is_popup_open());
    assert_eq!(flow.app().selected_count(), 0);
    flow.click(|a| *a == Action::Perform(BookAction::Download, "PREVIEW002".into()));
    assert!(!flow.0.screen.is_popup_open());
    flow.wait(50);
    assert!(matches!(
        flow.status("PREVIEW002"),
        BookStatus::Running { .. }
    ));
}

#[test]
fn typing_in_search_filters_the_grid() {
    let mut flow = signed_in();
    flow.click(|a| matches!(a, Action::FocusField(Field::Search, _)));
    flow.0.key(Key::Text("henrik".into()));
    assert_eq!(flow.app().visible_books().len(), 1);
    flow.0.key(Key::SelectAll);
    flow.0.key(Key::Backspace { word: false });
    assert_eq!(flow.app().visible_books().len(), 10);
}

#[test]
fn dragging_across_search_text_selects_it_for_copying() {
    let mut flow = signed_in();
    flow.click(|a| matches!(a, Action::FocusField(Field::Search, _)));
    flow.0.key(Key::Text("henrik ibsen".into()));
    flow.frame();
    let hit = flow
        .0
        .hits()
        .iter()
        .find(|hit| matches!(hit.action, Action::FocusField(Field::Search, _)))
        .expect("search is on screen");
    let Action::FocusField(_, text_x) = hit.action else {
        unreachable!()
    };
    let y = hit.rect.center_y();
    // Waits out the double-click interval so this press places a caret.
    flow.wait(600);
    flow.0.pointer_moved(Point::new(text_x + 1.0, y));
    flow.0.pointer_down(false);
    flow.0.pointer_moved(Point::new(text_x + 300.0, y));
    flow.0.pointer_up();
    let effect = flow.0.key(Key::Copy);
    assert_eq!(effect, Some(Effect::Copy("henrik ibsen".into())));
}

#[test]
fn a_failed_download_shows_a_toast_that_copies_and_dismisses() {
    let mut flow = signed_in();
    flow.0.screen.handle(
        Action::Perform(BookAction::Download, "PREVIEW008".into()),
        Point::default(),
        &skia_ui::text::TextSystem::default(),
    );
    for _ in 0..10 {
        flow.wait(100);
    }
    assert_eq!(flow.app().toasts().len(), 1);
    let id = flow.app().toasts()[0].id;
    let effect = flow.click(|a| *a == Action::CopyToast(id));
    assert!(matches!(effect, Some(Effect::Copy(text)) if text.contains("license was denied")));
    flow.click(|a| *a == Action::DismissToast(id));
    assert!(flow.app().toasts().is_empty());
}

#[test]
fn deleting_a_download_asks_first() {
    let mut flow = signed_in();
    flow.0.screen.handle(
        Action::Perform(BookAction::Download, "PREVIEW002".into()),
        Point::default(),
        &skia_ui::text::TextSystem::default(),
    );
    for _ in 0..50 {
        flow.wait(100);
    }
    assert_eq!(flow.status("PREVIEW002"), BookStatus::Downloaded);
    flow.click(|a| matches!(a, Action::OpenBookMenu(asin, _) if &**asin == "PREVIEW002"));
    flow.click(|a| *a == Action::Perform(BookAction::DeleteDownload, "PREVIEW002".into()));
    assert_eq!(flow.status("PREVIEW002"), BookStatus::Downloaded);
    flow.click(|a| *a == Action::ConfirmDelete);
    assert_eq!(flow.status("PREVIEW002"), BookStatus::None);
}
