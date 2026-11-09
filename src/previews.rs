//! Every screen state, with the same ids and sizes as the Swift app's preview galleries, so the
//! renders can be compared one to one. Used by the gallery and the headless renderer.

use skia_safe::{Rect, Size};

use crate::model::{
    Account, Audiobook, BookStatus, BookTileState, JobKind, Marketplace, MockApp, MockOptions,
    TransferProgress,
    preview_data::{self as data, tile_state},
};
use crate::shell::app_view::AppView;
use crate::stage::Screen;
use crate::theme::Theme;
use crate::ui::Ui;
use crate::views::{
    account::{self, PopoverContent, SignInProps},
    book_tile::book_tile,
    error_toast::{self, ToastProps},
    library::{self, GridProps},
    login_sheet::{self, LoginProps},
    text_field::TextField,
};

pub type DrawPreview = Box<dyn Fn(&mut Ui, Size)>;

pub struct Preview {
    pub id: &'static str,
    /// The Swift source file the state comes from, e.g. "BookTile".
    pub gallery: &'static str,
    pub title: &'static str,
    pub width: f32,
    pub height: f32,
    pub theme: Theme,
    pub draw: DrawPreview,
}

fn preview(
    gallery: &'static str,
    id: &'static str,
    title: &'static str,
    (width, height): (f32, f32),
    draw: impl Fn(&mut Ui, Size) + 'static,
) -> Preview {
    Preview {
        id,
        gallery,
        title,
        width,
        height,
        theme: Theme::LIGHT,
        draw: Box::new(draw),
    }
}

/// Centres a view of known width vertically, and horizontally in the preview.
fn centered(ui: &mut Ui, size: Size, width: f32, view: impl Fn(&mut Ui, (f32, f32)) -> f32) {
    let x = (size.width - width) / 2.0;
    let height = ui.measure(|ui| view(ui, (x, 0.0)));
    view(ui, (x, (size.height - height) / 2.0));
}

fn book(index: usize) -> Audiobook {
    data::books()[index].clone()
}

fn tile_preview(
    id: &'static str,
    title: &'static str,
    index: usize,
    state: BookTileState,
    can_select: bool,
) -> Preview {
    let book = book(index);
    preview("BookTile", id, title, (200.0, 280.0), move |ui, size| {
        centered(ui, size, 150.0, |ui, origin| {
            book_tile(ui, &book, &state, can_select, origin, 150.0)
        });
    })
}

fn book_tiles() -> Vec<Preview> {
    let status = |status| tile_state(true, status);
    let selected = BookTileState {
        is_selected: true,
        ..BookTileState::new()
    };
    let plain = BookTileState::new;
    let transferred = TransferProgress {
        received: 132_000_000,
        expected: 288_000_000,
    };
    let other_store = BookTileState {
        marketplace_badge: Some("UK".into()),
        status: BookStatus::Downloaded,
        ..BookTileState::new()
    };
    vec![
        tile_preview("tileUnselected", "Tile – unselected", 0, plain(), true),
        tile_preview("tileSelected", "Tile – selected", 0, selected, true),
        tile_preview("tileNoCover", "Tile – no cover", 2, plain(), true),
        tile_preview(
            "tileLongTitle",
            "Tile – long title, several authors",
            4,
            plain(),
            true,
        ),
        tile_preview("tileNoRuntime", "Tile – no runtime", 7, plain(), true),
        tile_preview("tileDisabled", "Tile – disabled", 0, plain(), false),
        tile_preview(
            "tileNotInLibrary",
            "Tile – not in library",
            1,
            tile_state(false, BookStatus::Downloaded),
            true,
        ),
        tile_preview(
            "tileOtherMarketplace",
            "Tile – other marketplace",
            3,
            other_store,
            true,
        ),
        tile_preview(
            "tileQueued",
            "Tile – queued",
            1,
            status(BookStatus::Queued(JobKind::Download)),
            true,
        ),
        tile_preview(
            "tileDownloading",
            "Tile – downloading",
            1,
            status(BookStatus::Running {
                kind: JobKind::Download,
                progress: 0.4,
                transferred: Some(transferred),
            }),
            true,
        ),
        tile_preview(
            "tileExporting",
            "Tile – exporting",
            1,
            status(BookStatus::Running {
                kind: JobKind::Export,
                progress: 0.7,
                transferred: None,
            }),
            true,
        ),
        tile_preview(
            "tileFailed",
            "Tile – failed",
            1,
            status(BookStatus::Failed {
                kind: JobKind::Download,
                message: "HTTP 403: Forbidden".into(),
            }),
            true,
        ),
        tile_preview(
            "tileDownloaded",
            "Tile – downloaded",
            1,
            status(BookStatus::Downloaded),
            true,
        ),
        tile_preview(
            "tileExported",
            "Tile – exported",
            1,
            status(BookStatus::Exported),
            true,
        ),
    ]
}

#[derive(Clone, Copy)]
struct GridState {
    is_signed_in: bool,
    is_loading: bool,
    is_checking: bool,
    is_library_empty: bool,
}

const EMPTY: GridState = GridState {
    is_signed_in: true,
    is_loading: false,
    is_checking: false,
    is_library_empty: true,
};

fn empty_grid(id: &'static str, title: &'static str, state: GridState) -> Preview {
    preview(
        "LibraryViews",
        id,
        title,
        (900.0, 560.0),
        move |ui, size| {
            let props = GridProps {
                books: vec![],
                is_library_empty: state.is_library_empty,
                is_signed_in: state.is_signed_in,
                is_loading: state.is_loading,
                is_checking_marketplaces: state.is_checking,
                can_select: false,
                scroll: 0.0,
                covers: None,
                scroller_opacity: 0.0,
            };
            library::library_grid(ui, &props, Rect::from_size(size));
        },
    )
}

fn book_grid(id: &'static str, title: &'static str, selected: &'static [usize]) -> Preview {
    let books = data::books();
    preview(
        "LibraryViews",
        id,
        title,
        (900.0, 560.0),
        move |ui, size| {
            let entries = books
                .iter()
                .enumerate()
                .map(|(index, book)| {
                    let tile = data::grid_tile(book, selected.contains(&index));
                    (book, std::borrow::Cow::Owned(tile))
                })
                .collect();
            let props = GridProps {
                books: entries,
                is_library_empty: false,
                is_signed_in: true,
                is_loading: false,
                is_checking_marketplaces: false,
                can_select: true,
                scroll: 0.0,
                covers: None,
                scroller_opacity: 0.0,
            };
            library::library_grid(ui, &props, Rect::from_size(size));
        },
    )
}

fn messages(
    id: &'static str,
    title: &'static str,
    errors: &'static [&'static str],
    notices: &'static [&'static str],
) -> Preview {
    preview(
        "LibraryViews",
        id,
        title,
        (700.0, 110.0),
        move |ui, size| {
            let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
            let notices: Vec<String> = notices.iter().map(ToString::to_string).collect();
            centered(ui, size, size.width, |ui, (x, y)| {
                library::messages_bar(ui, &errors, &notices, x, y, size.width)
            });
        },
    )
}

fn library_views() -> Vec<Preview> {
    vec![
        empty_grid(
            "gridSignedOut",
            "Grid – signed out",
            GridState {
                is_signed_in: false,
                ..EMPTY
            },
        ),
        empty_grid(
            "gridLoading",
            "Grid – loading",
            GridState {
                is_loading: true,
                ..EMPTY
            },
        ),
        empty_grid(
            "gridCheckingMarketplaces",
            "Grid – checking marketplaces",
            GridState {
                is_checking: true,
                ..EMPTY
            },
        ),
        empty_grid("gridEmpty", "Grid – empty library", EMPTY),
        book_grid("gridBooks", "Grid – books", &[]),
        book_grid("gridSelection", "Grid – books with selection", &[1, 3, 6]),
        empty_grid(
            "gridNoSearchResults",
            "Grid – no search results",
            GridState {
                is_library_empty: false,
                ..EMPTY
            },
        ),
        messages("messagesNone", "Messages – none", &[], &[]),
        messages(
            "messagesOne",
            "Messages – one",
            &["Could not reach Audible: The Internet connection appears to be offline."],
            &[],
        ),
        messages(
            "messagesSeveral",
            "Messages – several long",
            &[
                "Your session expired. Sign in again to continue.",
                "Audible returned HTTP 503 (Service Unavailable) while loading page 3 of your library. Try refreshing in a few minutes.",
                "The download folder could not be created or is not writable: /Users/jane/Documents/Loudible",
            ],
            &[],
        ),
        messages(
            "messagesNotice",
            "Messages – notice",
            &[],
            &["Couldn’t check the Audible Japan, India marketplaces for more titles."],
        ),
    ]
}

fn account_preview(
    id: &'static str,
    title: &'static str,
    size: (f32, f32),
    draw: impl Fn(&mut Ui, Size) + 'static,
) -> Preview {
    preview("AccountViews", id, title, size, draw)
}

fn avatar(
    id: &'static str,
    title: &'static str,
    name: Option<&'static str>,
    diameter: f32,
) -> Preview {
    account_preview(id, title, (80.0, 80.0), move |ui, size| {
        account::account_avatar(
            ui,
            name,
            (size.width / 2.0, size.height / 2.0).into(),
            diameter,
        );
    })
}

fn button_label(
    id: &'static str,
    title: &'static str,
    name: Option<&'static str>,
    is_signed_in: bool,
    needs_reauthentication: bool,
) -> Preview {
    account_preview(id, title, (260.0, 60.0), move |ui, size| {
        let center = (0.0, size.height / 2.0);
        let width = ui.measure(|ui| {
            account::account_button_label(
                ui,
                name,
                is_signed_in,
                needs_reauthentication,
                center,
                size.width,
            )
        });
        let x = (size.width - width) / 2.0;
        account::account_button_label(
            ui,
            name,
            is_signed_in,
            needs_reauthentication,
            (x, size.height / 2.0),
            size.width,
        );
    })
}

fn signed_out(can_act: bool, needs_reauthentication: bool) -> SignInProps {
    SignInProps {
        needs_reauthentication,
        marketplace: Marketplace::US,
        can_change_marketplace: can_act,
        can_sign_in: can_act,
    }
}

fn popover(
    id: &'static str,
    title: &'static str,
    can_sign_out: bool,
    content: impl Fn(&Account) -> PopoverContent<'static> + 'static,
) -> Preview {
    let account_data = Box::leak(Box::new(data::account()));
    account_preview(id, title, (280.0, 240.0), move |ui, size| {
        let content = content(account_data);
        centered(ui, size, account::POPOVER_WIDTH, |ui, origin| {
            account::account_popover(ui, &content, can_sign_out, origin)
        });
    })
}

fn signed_in_popover(
    id: &'static str,
    title: &'static str,
    others: &'static [(Marketplace, usize)],
) -> Preview {
    let account_data: &'static Account = Box::leak(Box::new(data::account()));
    popover(id, title, true, move |_| PopoverContent::SignedIn {
        account: account_data,
        other_marketplaces: others,
    })
}

fn panel(id: &'static str, title: &'static str, needs_reauthentication: bool) -> Preview {
    account_preview(id, title, (400.0, 260.0), move |ui, size| {
        let props = signed_out(true, needs_reauthentication);
        centered(ui, size, account::POPOVER_WIDTH, |ui, origin| {
            account::sign_in_panel(ui, &props, origin)
        });
    })
}

fn account_views() -> Vec<Preview> {
    const OTHERS: &[(Marketplace, usize)] = &[(Marketplace::UK, 12), (Marketplace::JAPAN, 3)];
    vec![
        avatar("avatarInitial", "Avatar – initial", Some("Jane Doe"), 24.0),
        avatar("avatarNoName", "Avatar – no name", None, 24.0),
        avatar("avatarLarge", "Avatar – 44 pt", Some("Jane Doe"), 44.0),
        button_label(
            "accountButtonSignedOut",
            "Account button – signed out",
            None,
            false,
            false,
        ),
        button_label(
            "accountButtonSignedIn",
            "Account button – signed in",
            Some("Jane Doe"),
            true,
            false,
        ),
        button_label(
            "accountButtonLongName",
            "Account button – long name",
            Some("Maximiliane Alexandra Featherstonehaugh-Smythe"),
            true,
            false,
        ),
        button_label(
            "accountButtonNeedsReauthentication",
            "Account button – needs re-auth",
            None,
            false,
            true,
        ),
        popover(
            "accountPopoverSignedOut",
            "Account popover – signed out",
            false,
            |_| PopoverContent::SignedOut(signed_out(true, false)),
        ),
        popover(
            "accountPopoverWorking",
            "Account popover – working",
            false,
            |_| PopoverContent::SignedOut(signed_out(false, false)),
        ),
        popover(
            "accountPopoverExpired",
            "Account popover – session expired",
            true,
            |_| PopoverContent::SignedOut(signed_out(true, true)),
        ),
        signed_in_popover("accountPopoverSignedIn", "Account popover – signed in", &[]),
        signed_in_popover(
            "accountPopoverOtherMarketplaces",
            "Account popover – other marketplaces",
            OTHERS,
        ),
        panel("signInPanel", "Sign-in panel", false),
        panel(
            "signInPanelExpired",
            "Sign-in panel – session expired",
            true,
        ),
    ]
}

#[derive(Clone, Default)]
struct LoginState {
    is_registering: bool,
    pasted: Option<&'static str>,
    can_submit: bool,
    error: Option<&'static str>,
    shows_fallback: bool,
    no_browser_label: bool,
    toast: bool,
}

fn login(id: &'static str, title: &'static str, state: LoginState) -> Preview {
    preview("LoginSheet", id, title, (520.0, 760.0), move |ui, size| {
        let mut pasted = TextField::new(crate::text::BODY);
        if let Some(url) = state.pasted {
            pasted.set(url, ui.text);
            pasted.move_to_start(ui.text);
        }
        login_sheet::login_sheet(
            ui,
            &LoginProps {
                marketplace_name: Marketplace::US.name,
                is_registering: state.is_registering,
                pasted_url: &pasted,
                pasted_url_focused: false,
                can_submit_pasted_url: state.can_submit,
                error: state.error,
                shows_fallback: state.shows_fallback,
                shows_browser_label: !state.no_browser_label,
                can_simulate: false,
            },
            Rect::from_size(size),
        );
        if state.toast {
            let error = crate::model::AppError::SignInBrowser {
                technical: "NSURLErrorDomain Code=-1009: Offline.".into(),
            };
            let props = ToastProps {
                id: 0,
                error: &error,
                is_copied: false,
                shows_details: false,
            };
            let width = error_toast::MAX_WIDTH.min(size.width - 48.0);
            let x = size.width - 24.0 - width;
            let height = ui.measure(|ui| error_toast::error_toast(ui, &props, (x, 0.0), width));
            error_toast::error_toast(ui, &props, (x, size.height - 24.0 - height), width);
        }
    })
}

fn login_views() -> Vec<Preview> {
    vec![
        login("loginIdle", "Sign-in – idle", LoginState::default()),
        login(
            "loginRegistering",
            "Sign-in – registering",
            LoginState {
                is_registering: true,
                ..Default::default()
            },
        ),
        login(
            "loginError",
            "Sign-in – error",
            LoginState {
                error: Some(data::LOGIN_EXPIRED),
                ..Default::default()
            },
        ),
        login(
            "loginPastedURL",
            "Sign-in – pasted URL",
            LoginState {
                pasted: Some(
                    "https://www.amazon.com/ap/maplanding?openid.oa2.authorization_code=PREVIEW",
                ),
                can_submit: true,
                shows_fallback: true,
                ..Default::default()
            },
        ),
        login(
            "loginBrowserFailed",
            "Sign-in – browser error toast",
            LoginState {
                no_browser_label: true,
                toast: true,
                ..Default::default()
            },
        ),
    ]
}

fn toast(
    id: &'static str,
    title: &'static str,
    size: (f32, f32),
    error: crate::model::AppError,
    is_copied: bool,
    shows_details: bool,
) -> Preview {
    preview("ErrorToast", id, title, size, move |ui, size| {
        let props = ToastProps {
            id: 0,
            error: &error,
            is_copied,
            shows_details,
        };
        let width = error_toast::MAX_WIDTH.min(size.width - 48.0);
        centered(ui, size, width, |ui, origin| {
            error_toast::error_toast(ui, &props, origin, width)
        });
    })
}

fn toast_views() -> Vec<Preview> {
    vec![
        toast(
            "errorToastLicenseDenied",
            "Error toast – license denial details",
            (500.0, 390.0),
            data::license_denied(),
            false,
            true,
        ),
        toast(
            "errorToastDownload",
            "Error toast – download failed",
            (500.0, 240.0),
            data::download_failed(),
            false,
            false,
        ),
        toast(
            "errorToastLong",
            "Error toast – long title",
            (500.0, 300.0),
            data::export_long_title(),
            false,
            false,
        ),
        toast(
            "errorToastNarrow",
            "Error toast – credential storage",
            (340.0, 300.0),
            data::credential_save(),
            false,
            false,
        ),
        toast(
            "errorToastCopied",
            "Error toast – copied",
            (500.0, 240.0),
            data::credential_delete(),
            true,
            false,
        ),
        Preview {
            theme: Theme::DARK,
            ..toast(
                "errorToastDark",
                "Error toast – sign-in browser, dark",
                (500.0, 210.0),
                data::sign_in_browser(),
                false,
                false,
            )
        },
    ]
}

/// A signed-in app with the library loaded, past the marketplace check.
fn loaded(fail_load: bool) -> MockApp {
    let mut app = MockApp::new(MockOptions {
        signed_in: true,
        fail_load,
    });
    app.tick(1_000);
    app.tick(1_000);
    app
}

fn window(id: &'static str, title: &'static str, make: fn() -> MockApp) -> Preview {
    preview(
        "ContentView",
        id,
        title,
        (1100.0, 652.0),
        move |ui, size| {
            AppView::with_app(make()).for_preview().render(ui, size);
        },
    )
}

fn window_views() -> Vec<Preview> {
    vec![
        window("windowSignedOut", "Window – signed out", || {
            MockApp::new(MockOptions::default())
        }),
        window("windowLibrary", "Window – library", || loaded(false)),
        window("windowSelection", "Window – selection", || {
            let mut app = loaded(false);
            app.toggle_selection("PREVIEW001");
            app.toggle_selection("PREVIEW005");
            app
        }),
        window("windowLoading", "Window – loading", || {
            MockApp::new(MockOptions {
                signed_in: true,
                fail_load: false,
            })
        }),
        window("windowLoadFailed", "Window – load failed", || {
            loaded(true)
        }),
        window(
            "windowExportFailed",
            "Window – export error toast",
            || {
                let mut app = loaded(false);
                app.present(data::export_permission());
                app
            },
        ),
    ]
}

/// Every preview, grouped by the Swift file its states come from.
pub fn all() -> Vec<Preview> {
    [
        account_views(),
        book_tiles(),
        window_views(),
        toast_views(),
        library_views(),
        login_views(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
