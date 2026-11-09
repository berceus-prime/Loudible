//! The whole window: maps an `AppModel` onto the views and carries out their actions.

use std::{collections::HashSet, time::Duration};

use skia_safe::{Point, Rect, Size};

use crate::model::Platform;
use crate::model::{AppModel, Asin, BookAction, Marketplace, MockApp, MockOptions, ModelEffect};
use crate::shell::browser::{BrowserPage, BrowserRequest};
use crate::shell::covers::CoverStore;
use crate::shell::webview::LoginWebView;
use crate::stage::{Effect, Key, Press, Screen};
use crate::text::{self, TextSystem};
use crate::ui::{Action, Field, Ui};
use crate::views::{
    account::{self, PopoverContent, SignInProps},
    dialogs::{self, MenuEntry},
    error_toast::{self, ToastProps},
    library::{self, GridProps},
    login_sheet::{self, LoginProps},
    text_field::TextField,
    toolbar::{self, AccountLabel, ToolbarProps},
};

const SCROLLER_VISIBLE_MS: f32 = 900.0;
const TOAST_MARGIN: f32 = 24.0;

#[derive(Clone, Debug, PartialEq)]
enum Popup {
    Account,
    Menu {
        entries: Vec<MenuEntry>,
        anchor: Point,
    },
}

pub type BrowserCallback = Box<dyn FnMut(&str) -> Option<Effect>>;

pub struct AppView<M = MockApp> {
    pub app: M,
    search: TextField,
    pasted: TextField,
    focus: Option<Field>,
    /// Where the focused field's text starts, from the press that focused it.
    field_origin: f32,
    popup: Option<Popup>,
    pending_delete: Option<Asin>,
    is_confirming_quit: bool,
    shows_fallback: bool,
    expanded_toasts: HashSet<u64>,
    scroll: f32,
    content_height: f32,
    viewport_height: f32,
    scroller_ms: f32,
    /// False in previews, which must match the Swift galleries.
    can_simulate: bool,
    /// A real page for the sheet's browser, in place of the stand-in.
    browser_page: Option<BrowserPage>,
    browser_rect: Option<Rect>,
    on_browser_callback: Option<BrowserCallback>,
    covers: Option<CoverStore>,
}

impl AppView<MockApp> {
    pub fn new(options: MockOptions) -> Self {
        Self::with_app(MockApp::new(options))
    }
}

impl<M: AppModel> AppView<M> {
    pub fn with_app(app: M) -> Self {
        Self {
            app,
            search: TextField::new(toolbar::SEARCH_TEXT),
            pasted: TextField::new(text::BODY),
            focus: None,
            field_origin: 0.0,
            popup: None,
            pending_delete: None,
            is_confirming_quit: false,
            shows_fallback: false,
            expanded_toasts: HashSet::new(),
            scroll: 0.0,
            content_height: 0.0,
            viewport_height: 0.0,
            scroller_ms: 0.0,
            can_simulate: true,
            browser_page: None,
            browser_rect: None,
            on_browser_callback: None,
            covers: None,
        }
    }

    /// Fetches real cover images for the books that have them.
    pub fn with_covers(mut self, covers: CoverStore) -> Self {
        self.covers = Some(covers);
        self
    }

    fn page(&self) -> Option<BrowserPage> {
        self.browser_page
            .clone()
            .or_else(|| self.app.browser_page())
    }

    fn model_effect(effect: Option<ModelEffect>) -> Option<Effect> {
        effect.map(|ModelEffect::Reveal(path)| Effect::Reveal(path))
    }

    /// Shows `page` in the sign-in sheet and reports the redirect it ends on.
    pub fn with_browser(mut self, page: BrowserPage, on_callback: BrowserCallback) -> Self {
        self.browser_page = Some(page);
        self.on_browser_callback = Some(on_callback);
        self
    }

    pub fn for_preview(mut self) -> Self {
        self.can_simulate = false;
        self
    }

    pub fn is_popup_open(&self) -> bool {
        self.popup.is_some()
    }

    fn field_mut(&mut self, field: Field) -> &mut TextField {
        match field {
            Field::Search => &mut self.search,
            Field::PastedUrl => &mut self.pasted,
        }
    }

    fn field(&self, field: Field) -> &TextField {
        match field {
            Field::Search => &self.search,
            Field::PastedUrl => &self.pasted,
        }
    }

    /// Moves focus, dropping the selection of the field it leaves.
    fn set_focus(&mut self, focus: Option<Field>, text: &TextSystem) {
        if let Some(previous) = self.focus
            && focus != Some(previous)
        {
            self.field_mut(previous).blur(text);
        }
        self.focus = focus;
    }

    fn press_field(&mut self, field: Field, origin: f32, press: Press, text: &TextSystem) {
        self.set_focus(Some(field), text);
        self.field_origin = origin;
        self.field_mut(field)
            .press(press.point.x - origin, press.clicks, press.extend, text);
    }

    fn sync_field(&mut self, field: Field) {
        match field {
            Field::Search => {
                self.app.set_search(self.search.text());
                self.scroll = 0.0;
            }
            Field::PastedUrl => self.app.set_pasted_url(self.pasted.text()),
        }
    }

    fn book_menu(&self, asin: &Asin) -> Vec<MenuEntry> {
        let Some(book) = self
            .app
            .visible_books()
            .into_iter()
            .find(|b| b.asin == *asin)
        else {
            return vec![];
        };
        let mut entries = vec![];
        for item in self.app.tile(book).menu_items.clone() {
            entries.push(MenuEntry {
                title: item.action.title().to_string(),
                action: item
                    .is_enabled()
                    .then(|| Action::Perform(item.action, asin.clone())),
                is_destructive: item.action == BookAction::DeleteDownload,
                is_checked: false,
            });
            if let Some(reason) = item.disabled_reason {
                entries.push(MenuEntry {
                    title: reason,
                    action: None,
                    is_destructive: false,
                    is_checked: false,
                });
            }
        }
        entries
    }

    fn marketplace_menu(&self) -> Vec<MenuEntry> {
        Marketplace::ALL
            .iter()
            .map(|marketplace| MenuEntry {
                title: marketplace.name.to_string(),
                action: Some(Action::SetMarketplace(*marketplace)),
                is_destructive: false,
                is_checked: *marketplace == self.app.marketplace(),
            })
            .collect()
    }

    fn sign_in_props(&self) -> SignInProps {
        SignInProps {
            needs_reauthentication: self.app.needs_reauthentication(),
            marketplace: self.app.marketplace(),
            can_change_marketplace: self.app.can_change_marketplace(),
            can_sign_in: self.app.can_sign_in(),
        }
    }

    /// The name of the downloaded book awaiting confirmation.
    fn pending_delete_info(&self) -> Option<String> {
        let asin = self.pending_delete.as_ref()?;
        let visible = self.app.visible_books();
        let book = visible.into_iter().find(|b| b.asin == *asin)?;
        Some(book.title.clone())
    }

    fn draw_toolbar(&self, ui: &mut Ui, width: f32) {
        let app = &self.app;
        let signed_out_and_empty = app.is_signed_out_and_empty();
        let name = app
            .account()
            .and_then(|account| account.customer_name.as_deref());
        toolbar::toolbar(
            ui,
            &ToolbarProps {
                account: (!signed_out_and_empty).then_some(AccountLabel {
                    name,
                    is_signed_in: app.is_signed_in(),
                    needs_reauthentication: app.needs_reauthentication(),
                }),
                search: &self.search,
                search_focused: self.focus == Some(Field::Search),
                search_enabled: !signed_out_and_empty,
                selection: (app.selected_count() > 0).then(|| {
                    (
                        app.selection_offering(BookAction::Download),
                        app.selection_offering(BookAction::Export),
                    )
                }),
                is_busy: app.is_loading() || app.is_checking_marketplaces(),
                can_refresh: app.can_refresh(),
                checking_marketplaces: app.is_checking_marketplaces() && !app.is_loading(),
                tint: library::messages_tint(&ui.theme, &app.messages(), &app.notices()),
            },
            width,
        );
    }

    fn draw_popups(&self, ui: &mut Ui, size: Size) {
        match &self.popup {
            Some(Popup::Account) => {
                ui.hit(Rect::from_size(size), Action::DismissPopup);
                self.draw_account_popover(ui, size);
            }
            Some(Popup::Menu { entries, anchor }) => {
                ui.hit(Rect::from_size(size), Action::DismissPopup);
                dialogs::popup_menu(ui, entries, *anchor, size);
            }
            None => {}
        }
    }

    fn draw_account_popover(&self, ui: &mut Ui, size: Size) {
        let others = self.app.other_marketplaces();
        let content = match self.app.account() {
            Some(account) => PopoverContent::SignedIn {
                account,
                other_marketplaces: &others,
            },
            None => PopoverContent::SignedOut(self.sign_in_props()),
        };
        let can_sign_out = self.app.can_sign_out();
        let height =
            ui.measure(|ui| account::account_popover(ui, &content, can_sign_out, (0.0, 0.0)));
        const ARROW_X: f32 = 148.0;
        const TOP: f32 = 54.0;
        let x = (ARROW_X - account::POPOVER_WIDTH / 2.0)
            .max(8.0)
            .min(size.width - account::POPOVER_WIDTH - 8.0);
        let rect = Rect::from_xywh(x, TOP, account::POPOVER_WIDTH, height);
        dialogs::popover_frame(ui, rect, ARROW_X);
        account::account_popover(ui, &content, can_sign_out, (x, TOP));
    }

    fn draw_login(&mut self, ui: &mut Ui, size: Size) {
        self.browser_rect = None;
        let Some(login) = self.app.pending_login() else {
            return;
        };
        let has_page = self.browser_page.is_some() || self.app.browser_page().is_some();
        const WIDTH: f32 = 520.0;
        dialogs::modal_backdrop(ui, size);
        let height = (size.height - 32.0).min(760.0);
        let rect = Rect::from_xywh(
            (size.width - WIDTH) / 2.0,
            (size.height - height) / 2.0,
            WIDTH,
            height,
        );
        dialogs::sheet_card(ui, rect);
        let browser = login_sheet::login_sheet(
            ui,
            &LoginProps {
                marketplace_name: login.marketplace.name,
                is_registering: login.is_registering,
                pasted_url: &self.pasted,
                pasted_url_focused: self.focus == Some(Field::PastedUrl),
                can_submit_pasted_url: self.app.can_submit_pasted_url(),
                error: login.error.as_deref(),
                shows_fallback: self.shows_fallback,
                shows_browser_label: self.browser_page.is_none()
                    && self.app.browser_page().is_none(),
                can_simulate: self.can_simulate && self.app.can_simulate() && !has_page,
            },
            rect,
        );
        self.browser_rect = Some(browser);
    }

    fn draw_toast(&self, ui: &mut Ui, size: Size) {
        let Some(toast) = self.app.toasts().first() else {
            return;
        };
        let props = ToastProps {
            id: toast.id,
            error: &toast.error,
            is_copied: toast.is_copied,
            shows_details: self.expanded_toasts.contains(&toast.id),
        };
        let width = error_toast::MAX_WIDTH.min(size.width - TOAST_MARGIN * 2.0);
        let x = size.width - TOAST_MARGIN - width;
        let height = ui.measure(|ui| error_toast::error_toast(ui, &props, (x, 0.0), width));
        error_toast::error_toast(ui, &props, (x, size.height - TOAST_MARGIN - height), width);
    }

    fn clamp_scroll(&mut self) {
        let max = (self.content_height - self.viewport_height).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
    }
}

impl<M: AppModel> Screen for AppView<M> {
    fn browser(&self) -> Option<BrowserRequest> {
        if !LoginWebView::IS_AVAILABLE {
            return None;
        }
        let login = self.app.pending_login()?;
        Some(BrowserRequest {
            page: self.page()?,
            rect: self.browser_rect?,
            // A native view covers Skia, so it steps aside for the spinner and for toasts.
            is_visible: !login.is_registering
                && self.app.toasts().is_empty()
                && !self.is_confirming_quit,
        })
    }

    fn browser_callback(&mut self, url: String) -> Option<Effect> {
        self.app.complete_sign_in(Some(url.clone()));
        self.on_browser_callback.as_mut()?(&url)
    }

    fn browser_failed(&mut self, message: String) {
        eprintln!("Sign-in browser failed: {message}");
        self.app.browser_failed(message);
    }

    fn close_requested(&mut self) -> bool {
        if !self.app.has_active_jobs() {
            return true;
        }
        if self.app.has_work_suspend_loses() {
            self.is_confirming_quit = true;
        } else {
            self.app.begin_suspend();
        }
        false
    }

    fn is_ready_to_close(&self) -> bool {
        self.app.has_finished_suspending()
    }

    fn window_focused(&mut self) {
        self.app.rescan();
    }

    fn render(&mut self, ui: &mut Ui, size: Size) {
        ui.canvas.clear(ui.theme.window);
        self.clamp_scroll();
        let mut grid_top = toolbar::HEIGHT;
        let (messages, notices) = (self.app.messages(), self.app.notices());
        grid_top += library::messages_bar(ui, &messages, &notices, 0.0, grid_top, size.width);

        let app = &self.app;
        let books = app
            .visible_books()
            .into_iter()
            .map(|book| (book, app.tile(book)))
            .collect();
        let grid = GridProps {
            books,
            is_library_empty: app.is_library_empty(),
            is_signed_in: app.is_signed_in(),
            is_loading: app.is_loading(),
            is_checking_marketplaces: app.is_checking_marketplaces(),
            can_select: app.can_select(),
            scroll: self.scroll,
            scroller_opacity: (self.scroller_ms / 200.0).min(1.0),
            covers: self.covers.as_ref(),
        };
        let rect = Rect::from_ltrb(0.0, grid_top, size.width, size.height);
        self.content_height = library::library_grid(ui, &grid, rect);
        self.viewport_height = rect.height();

        if self.app.is_signed_out_and_empty() && self.app.has_restored() {
            let props = self.sign_in_props();
            let x = (size.width - account::POPOVER_WIDTH) / 2.0;
            let height = ui.measure(|ui| account::sign_in_panel(ui, &props, (x, 0.0)));
            let y = rect.center_y() - height / 2.0;
            account::sign_in_panel(ui, &props, (x, y));
        }
        self.draw_toolbar(ui, size.width);
        self.draw_toast(ui, size);
        self.draw_login(ui, size);
        if self.app.pending_login().is_some() {
            // Toasts also present above the sign-in sheet, as in the Swift app.
            self.draw_toast(ui, size);
        }
        if let Some(title) = self.pending_delete_info() {
            dialogs::modal_backdrop(ui, size);
            let trash = Platform::CURRENT.trash();
            let heading = format!("Move “{title}” to the {trash}?");
            let confirm = format!("Move to {trash}");
            let message = "The exported M4B is not affected.";
            dialogs::confirm_alert(
                ui,
                &heading,
                message,
                (&confirm, Action::ConfirmDelete),
                Action::CancelDelete,
                size,
            );
        }
        if self.is_confirming_quit {
            dialogs::modal_backdrop(ui, size);
            dialogs::confirm_alert(
                ui,
                "Quit and cancel the exports and queued jobs?",
                "Exports in progress and queued jobs are cancelled. Downloads restart next time. Finished files are kept.",
                ("Quit", Action::ConfirmQuit),
                Action::CancelQuit,
                size,
            );
        }
        self.draw_popups(ui, size);
    }

    fn press(&mut self, action: Action, press: Press, text: &TextSystem) -> Option<Effect> {
        match action {
            Action::FocusField(field, origin) => {
                self.press_field(field, origin, press, text);
                None
            }
            action => self.handle(action, press.point, text),
        }
    }

    fn drag(&mut self, pointer: Point, text: &TextSystem) {
        if let Some(field) = self.focus {
            let origin = self.field_origin;
            self.field_mut(field).drag(pointer.x - origin, text);
        }
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.field(self.focus?).input_area()
    }

    fn handle(&mut self, action: Action, pointer: Point, text: &TextSystem) -> Option<Effect> {
        if !matches!(action, Action::FocusField(..)) {
            self.set_focus(None, text);
        }
        match action {
            Action::Block => {}
            Action::DragWindow => return Some(Effect::Drag),
            Action::CloseWindow => return Some(Effect::Close),
            Action::MinimizeWindow => return Some(Effect::Minimize),
            Action::ZoomWindow => return Some(Effect::Zoom),
            Action::ClearSelection => self.app.clear_selection(),
            Action::ToggleBook(asin) => self.app.toggle_selection(&asin),
            Action::OpenBookMenu(asin, anchor) => {
                let entries = self.book_menu(&asin);
                // A right click opens at the pointer; the dots open below themselves.
                let anchor = if pointer.y > anchor.y - 22.0 && pointer.x >= anchor.x - 1.0 {
                    anchor
                } else {
                    pointer
                };
                self.popup = Some(Popup::Menu { entries, anchor });
            }
            Action::Perform(BookAction::DeleteDownload, asin) => {
                self.popup = None;
                self.pending_delete = Some(asin);
            }
            Action::Perform(action, asin) => {
                self.popup = None;
                return Self::model_effect(self.app.perform(action, &asin));
            }
            Action::OnSelection(action) => self.app.perform_on_selection(action),
            Action::Refresh => self.app.refresh(),
            Action::ToggleAccount => {
                self.popup = if self.popup == Some(Popup::Account) {
                    None
                } else {
                    Some(Popup::Account)
                };
            }
            Action::SignIn => {
                self.popup = None;
                self.shows_fallback = !LoginWebView::IS_AVAILABLE;
                self.pasted = TextField::new(text::BODY);
                self.app.sign_in();
            }
            Action::SignOut => {
                self.popup = None;
                self.search = TextField::new(toolbar::SEARCH_TEXT);
                self.app.sign_out();
            }
            Action::OpenMarketplaceMenu(anchor) => {
                self.popup = Some(Popup::Menu {
                    entries: self.marketplace_menu(),
                    anchor,
                });
            }
            Action::SetMarketplace(marketplace) => {
                self.popup = None;
                self.app.set_marketplace(marketplace);
            }
            Action::FocusField(field, origin) => {
                let press = Press {
                    point: pointer,
                    clicks: 1,
                    extend: false,
                };
                self.press_field(field, origin, press, text);
            }
            Action::ClearSearch => {
                self.search = TextField::new(toolbar::SEARCH_TEXT);
                self.sync_field(Field::Search);
                self.focus = Some(Field::Search);
            }
            Action::SimulateSignIn => self.app.complete_sign_in(None),
            Action::OpenInBrowser => {
                if let Some(url) = self.app.login_url() {
                    return Some(Effect::OpenUrl(url));
                }
                self.pasted.set(
                    "https://www.amazon.com/ap/maplanding?openid.oa2.authorization_code=MOCK",
                    text,
                );
                self.sync_field(Field::PastedUrl);
            }
            Action::SubmitPastedUrl => self.app.submit_pasted_url(),
            Action::CancelSignIn => self.app.cancel_sign_in(),
            Action::ToggleBrowserFallback => self.shows_fallback = !self.shows_fallback,
            Action::ToggleToastDetails(id) => {
                if !self.expanded_toasts.remove(&id) {
                    self.expanded_toasts.insert(id);
                }
            }
            Action::CopyToast(id) => return self.app.copy_toast(id).map(Effect::Copy),
            Action::DismissToast(id) => {
                self.expanded_toasts.remove(&id);
                self.app.dismiss_toast(id);
            }
            Action::ConfirmDelete => {
                if let Some(asin) = self.pending_delete.take() {
                    return Self::model_effect(self.app.perform(BookAction::DeleteDownload, &asin));
                }
            }
            Action::CancelDelete => self.pending_delete = None,
            Action::ConfirmQuit => {
                self.is_confirming_quit = false;
                self.app.begin_suspend();
            }
            Action::CancelQuit => self.is_confirming_quit = false,
            Action::DismissPopup => self.popup = None,
            Action::Gallery(_) => {}
        }
        None
    }

    fn key(&mut self, key: Key, text: &TextSystem) -> Option<Effect> {
        if key == Key::Escape {
            if self.popup.take().is_none() {
                if std::mem::take(&mut self.is_confirming_quit)
                    || self.pending_delete.take().is_some()
                {
                } else if self.app.pending_login().is_some() {
                    self.app.cancel_sign_in();
                } else if self.app.selected_count() > 0 {
                    self.app.clear_selection();
                } else {
                    self.set_focus(None, text);
                }
            }
            return None;
        }
        let Some(field) = self.focus else {
            if key == Key::Enter
                && self.app.is_signed_out_and_empty()
                && self.app.pending_login().is_none()
            {
                self.app.sign_in();
            }
            return None;
        };
        let mut effect = None;
        let target = self.field_mut(field);
        let before = target.text();
        match key {
            Key::Text(typed) | Key::Paste(typed) => target.insert(&typed, text),
            Key::Compose(composed, cursor) => target.compose(&composed, cursor, text),
            Key::Commit(committed) => target.commit(&committed, text),
            Key::Backspace { word } => target.backspace(word, text),
            Key::Delete { word } => target.delete(word, text),
            Key::Move { to, select } => target.move_caret(to, select, text),
            Key::SelectAll => target.select_all(text),
            Key::Copy => effect = target.selected_text().map(Effect::Copy),
            Key::Cut => {
                if let Some(selected) = target.selected_text() {
                    effect = Some(Effect::Copy(selected));
                    target.delete_selection(text);
                }
            }
            Key::Enter => {
                if field == Field::PastedUrl {
                    self.app.submit_pasted_url();
                }
                return None;
            }
            Key::Escape => {}
        }
        if target.text() != before {
            self.sync_field(field);
        }
        effect
    }

    fn scroll(&mut self, delta_y: f32) {
        if self.popup.is_some() || self.app.pending_login().is_some() {
            return;
        }
        self.scroll -= delta_y;
        self.clamp_scroll();
        if self.content_height > self.viewport_height {
            self.scroller_ms = SCROLLER_VISIBLE_MS;
        }
    }

    fn tick(&mut self, elapsed: Duration) {
        let ms = elapsed.as_millis().min(100) as u32;
        self.app.tick(ms);
        if let Some(covers) = &mut self.covers {
            covers.receive();
        }
        self.scroller_ms = (self.scroller_ms - ms as f32).max(0.0);
    }

    fn next_frame(&self) -> Option<Duration> {
        if self.app.is_busy() || self.scroller_ms > 0.0 {
            Some(Duration::from_millis(16))
        } else if self.focus.is_some() {
            Some(Duration::from_millis(265))
        } else {
            None
        }
    }
}
