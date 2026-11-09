//! The title bar: traffic lights, account, search, selection actions and refresh.

use skia_safe::{Point, Rect};

use crate::icons::Icon;
use crate::model::BookAction;
use crate::text::{self, Weight};
use crate::ui::{Action, Field, Ui, capsule, fill, outline};
use crate::views::account::account_button_label;
use crate::views::text_field::{self, TextField};

pub const HEIGHT: f32 = 52.0;
const CONTROL_Y: f32 = 8.0;
const CONTROL_HEIGHT: f32 = 36.0;
/// Clears the window controls on macOS, where they share the title bar with the toolbar.
const ACCOUNT_X: f32 = if cfg!(target_os = "macos") {
    95.0
} else {
    12.0
};
const ACCOUNT_MAX_WIDTH: f32 = 200.0;
const SEARCH_WIDTH: f32 = 452.0;
const TRAILING: f32 = 8.0;
const CENTER_Y: f32 = CONTROL_Y + CONTROL_HEIGHT / 2.0;
const SEARCH_TEXT: text::TextStyle = text::BODY.weight(Weight::Medium);

pub struct AccountLabel<'a> {
    pub name: Option<&'a str>,
    pub is_signed_in: bool,
    pub needs_reauthentication: bool,
}

pub struct ToolbarProps<'a> {
    /// `None` while signed out with nothing to show: the window has its own prompt.
    pub account: Option<AccountLabel<'a>>,
    pub search: &'a TextField,
    pub search_focused: bool,
    pub search_enabled: bool,
    /// Books the selection can download and export, when anything is selected.
    pub selection: Option<(usize, usize)>,
    pub is_busy: bool,
    pub can_refresh: bool,
    pub checking_marketplaces: bool,
    /// The messages bar's colour, which runs up behind the title bar.
    pub tint: Option<skia_safe::Color>,
}

fn help(count: usize, action: &str, none: &str) -> String {
    match count {
        0 => format!("None of the selected books can be {none}"),
        1 => format!("{action} 1 book"),
        _ => format!("{action} {count} books"),
    }
}

pub fn toolbar(ui: &mut Ui, props: &ToolbarProps, width: f32) {
    let bar = Rect::from_wh(width, HEIGHT);
    ui.canvas.draw_rect(bar, &fill(ui.theme.window));
    match props.tint {
        Some(tint) => {
            ui.canvas.draw_rect(bar, &fill(tint));
        }
        None => ui.separator(0.0, HEIGHT - 0.5, width),
    }
    ui.hit(bar, Action::DragWindow);
    // Other systems keep their standard title bar and its controls.
    if cfg!(target_os = "macos") {
        ui.traffic_lights();
    }

    if let Some(account) = &props.account {
        let label = ui.measure(|ui| draw_account(ui, account));
        let rect = Rect::from_xywh(ACCOUNT_X, CONTROL_Y, label, CONTROL_HEIGHT);
        ui.glass_capsule(rect);
        draw_account(ui, account);
        let tip = if account.is_signed_in {
            "Account"
        } else {
            "Sign in to Audible"
        };
        ui.hit_with(rect, Action::ToggleAccount, None, Some(tip), false);
    }
    let trailing = draw_trailing(ui, props, width);
    draw_search(ui, props, width, trailing);
}

fn draw_account(ui: &mut Ui, account: &AccountLabel) -> f32 {
    account_button_label(
        ui,
        account.name,
        account.is_signed_in,
        account.needs_reauthentication,
        (ACCOUNT_X, CENTER_Y),
        ACCOUNT_MAX_WIDTH,
    )
}

fn draw_search(ui: &mut Ui, props: &ToolbarProps, width: f32, trailing_left: f32) {
    let theme = ui.theme;
    let leading = ACCOUNT_X + ACCOUNT_MAX_WIDTH / 2.0;
    let available = (trailing_left - leading - 16.0).max(120.0);
    let search_width = SEARCH_WIDTH.min(available);
    let centered = (width - search_width) / 2.0;
    let left = centered.clamp(leading, (trailing_left - 8.0 - search_width).max(leading));
    let rect = Rect::from_xywh(left, CONTROL_Y, search_width, CONTROL_HEIGHT);
    ui.canvas.draw_rrect(capsule(rect), &fill(ui.theme.glass));
    let rim = if props.search_focused {
        outline(theme.accent, 2.0)
    } else {
        outline(theme.glass_rim, 0.75)
    };
    ui.canvas.draw_rrect(capsule(rect), &rim);

    let color = if props.search_enabled {
        theme.label
    } else {
        theme.secondary
    };
    ui.icon(Icon::Search, Point::new(left + 18.0, CENTER_Y), 13.5, color);
    let text_x = left + 33.0;
    let clear = !props.search.text.is_empty();
    let text_width = search_width - 33.0 - if clear { 34.0 } else { 14.0 };
    text_field::draw_value(
        ui,
        props.search,
        "Search title, author, narrator",
        color,
        SEARCH_TEXT,
        (text_x, CENTER_Y - SEARCH_TEXT.line / 2.0),
        text_width,
        props.search_focused,
    );
    let action = if props.search_enabled {
        Action::FocusField(Field::Search, text_x)
    } else {
        Action::Block
    };
    ui.hit_with(rect, action, None, None, props.search_enabled);
    if clear {
        let center = Point::new(rect.right - 18.0, CENTER_Y);
        ui.canvas.draw_circle(center, 7.0, &fill(theme.tertiary));
        ui.icon(Icon::XMark, center, 7.0, theme.window);
        let target = Rect::from_xywh(center.x - 10.0, CENTER_Y - 10.0, 20.0, 20.0);
        ui.hit(target, Action::ClearSearch);
    }
}

/// Selection actions and refresh at the trailing edge; returns the left edge they occupy.
fn draw_trailing(ui: &mut Ui, props: &ToolbarProps, width: f32) -> f32 {
    const CIRCLE: f32 = 36.5;
    let theme = ui.theme;
    let right = width - TRAILING;
    // An inactive window greys its toolbar labels, which is how the Swift previews appear.
    let enabled_color = if ui.is_active {
        theme.label
    } else {
        theme.secondary
    };
    let refresh_color = if props.is_busy || !props.can_refresh {
        theme.tertiary
    } else {
        enabled_color
    };
    let tip = if props.checking_marketplaces {
        "Checking other Audible marketplaces for more titles…"
    } else {
        "Refresh library"
    };
    let refresh_action = if !props.is_busy && props.can_refresh {
        Action::Refresh
    } else {
        Action::Block
    };

    let Some((downloads, exports)) = props.selection else {
        let rect = Rect::from_xywh(right - CIRCLE, CONTROL_Y, CIRCLE, CONTROL_HEIGHT);
        ui.glass_capsule(rect);
        let center = Point::new(rect.center_x(), CENTER_Y);
        draw_refresh(ui, props, center, refresh_color);
        ui.hit_with(rect, refresh_action, None, Some(tip), false);
        return rect.left;
    };

    const ICON: f32 = 18.0;
    let label = text::BODY.weight(Weight::Medium);
    let download_width = ui.text.width("Download", label);
    let export_width = ui.text.width("Export", label);
    let total = 10.0 + ICON + 8.0 + download_width + 17.0 + ICON + 8.0 + export_width + 26.0 + 19.5;
    let group = Rect::from_xywh(right - total, CONTROL_Y, total, CONTROL_HEIGHT);
    ui.glass_capsule(group);
    let mut x = group.left + 10.0;
    let mut item = |ui: &mut Ui,
                    title: &str,
                    icon,
                    text_width: f32,
                    count: usize,
                    action: BookAction,
                    tip: String| {
        let color = if count == 0 {
            theme.tertiary
        } else {
            enabled_color
        };
        ui.icon(icon, Point::new(x + ICON / 2.0, CENTER_Y), ICON, color);
        ui.line(
            title,
            label,
            color,
            x + ICON + 8.0,
            CENTER_Y - label.line / 2.0,
        );
        let target = Rect::from_xywh(
            x - 6.0,
            CONTROL_Y,
            ICON + 8.0 + text_width + 12.0,
            CONTROL_HEIGHT,
        );
        let hit = if count == 0 {
            Action::Block
        } else {
            Action::OnSelection(action)
        };
        ui.hit_with(target, hit, None, Some(&tip), false);
        x += ICON + 8.0 + text_width + 17.0;
    };
    let download_tip = help(downloads, "Download", "downloaded");
    let export_tip = help(exports, "Export to M4B", "exported");
    item(
        ui,
        "Download",
        Icon::ArrowDownCircle,
        download_width,
        downloads,
        BookAction::Download,
        download_tip,
    );
    item(
        ui,
        "Export",
        Icon::Share,
        export_width,
        exports,
        BookAction::Export,
        export_tip,
    );
    let center = Point::new(right - 19.5, CENTER_Y);
    draw_refresh(ui, props, center, refresh_color);
    let target = Rect::from_xywh(center.x - 18.0, CONTROL_Y, 36.0, CONTROL_HEIGHT);
    ui.hit_with(target, refresh_action, None, Some(tip), false);
    group.left
}

fn draw_refresh(ui: &mut Ui, props: &ToolbarProps, center: Point, color: skia_safe::Color) {
    if props.is_busy {
        ui.spinner(center, 15.0);
    } else {
        ui.icon(Icon::Refresh, center, 19.0, color);
    }
}
