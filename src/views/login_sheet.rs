//! The sign-in sheet around a stand-in for Amazon's page.

use skia_safe::{Point, Rect};

use crate::icons::Icon;
use crate::text;
use crate::ui::{Action, Button, Field, Ui, fill, rounded};
use crate::views::text_field::TextField;

const PADDING: f32 = 16.0;
const SPACING: f32 = 10.0;
const BUTTON_HEIGHT: f32 = 24.0;
const URL_HINT: &str = "https://www.amazon…/ap/maplanding?…";
const FALLBACK_HELP: &str = "Open the sign-in page in your browser. After signing in you will land on an error or blank page; copy that page's full address and paste it here.";

pub struct LoginProps<'a> {
    pub marketplace_name: &'a str,
    pub is_registering: bool,
    pub pasted_url: &'a TextField,
    pub pasted_url_focused: bool,
    pub can_submit_pasted_url: bool,
    pub error: Option<&'a str>,
    pub shows_fallback: bool,
    /// Labels the browser stand-in; the Swift gallery's browser-failure state leaves it blank.
    pub shows_browser_label: bool,
    /// Offers the button that stands in for finishing Amazon's page. Off in previews.
    pub can_simulate: bool,
}

/// Fills `rect` with the sheet; the browser takes whatever height the rest leaves. Returns where
/// the browser goes.
pub fn login_sheet(ui: &mut Ui, props: &LoginProps, rect: Rect) -> Rect {
    let theme = ui.theme;
    let x = rect.left + PADDING;
    let width = rect.width() - PADDING * 2.0;
    let title = format!("Sign in to Audible ({})", props.marketplace_name);
    ui.line(&title, text::HEADLINE, theme.label, x, rect.top + PADDING);

    let browser_top = rect.top + PADDING + text::HEADLINE.line + SPACING;
    let tail_height = ui.measure(|ui| tail(ui, props, x, 0.0, width));
    let browser_bottom = rect.bottom - PADDING - tail_height - SPACING;
    let browser = Rect::from_ltrb(x, browser_top, x + width, browser_bottom);
    draw_browser(ui, props, browser);
    tail(ui, props, x, browser_bottom + SPACING, width);
    browser
}

fn draw_browser(ui: &mut Ui, props: &LoginProps, browser: Rect) {
    let theme = ui.theme;
    ui.canvas.draw_rect(
        browser,
        &fill(skia_safe::Color::new(if theme.is_dark {
            0x1AFF_FFFF
        } else {
            0x1A00_0000
        })),
    );
    let center = Point::new(browser.center_x(), browser.center_y());
    if props.is_registering {
        ui.spinner(Point::new(center.x, center.y - 12.0), 20.0);
        ui.line_centered(
            "Registering device…",
            text::BODY,
            theme.secondary,
            center.x,
            center.y + 6.0,
        );
    } else if props.shows_browser_label {
        let label = "Amazon sign-in page";
        let label_width = ui.text.width(label, text::BODY);
        let left = center.x - (label_width + 22.0) / 2.0;
        ui.icon(
            Icon::Globe,
            Point::new(left + 7.0, center.y),
            14.0,
            theme.secondary,
        );
        ui.line(
            label,
            text::BODY,
            theme.secondary,
            left + 22.0,
            center.y - text::BODY.line / 2.0,
        );
        if props.can_simulate {
            let button = Rect::from_xywh(center.x - 80.0, center.y + 24.0, 160.0, BUTTON_HEIGHT);
            ui.button(
                Button::new("Simulate signing in", Action::SimulateSignIn),
                button,
            );
        }
    }
    if props.is_registering {
        ui.hit(browser, Action::Block);
    }
}

/// Everything under the browser; returns its height.
fn tail(ui: &mut Ui, props: &LoginProps, x: f32, y: f32, width: f32) -> f32 {
    let theme = ui.theme;
    let mut cursor = y;
    let chevron = if props.shows_fallback {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    ui.icon(
        chevron,
        Point::new(x + 4.0, cursor + 12.0),
        10.0,
        theme.tertiary,
    );
    let disclosure = "Trouble signing in? Use your web browser";
    ui.line(disclosure, text::BODY, theme.label, x + 11.5, cursor + 4.0);
    let row = Rect::from_xywh(
        x,
        cursor,
        ui.text.width(disclosure, text::BODY) + 12.0,
        24.0,
    );
    ui.hit(row, Action::ToggleBrowserFallback);
    cursor += 24.0;

    if props.shows_fallback {
        cursor += 4.0;
        let origin = Point::new(x, cursor);
        cursor += ui.paragraph(
            FALLBACK_HELP,
            text::CALLOUT,
            theme.secondary,
            origin,
            width,
            None,
        ) + 6.0;
        cursor += fallback_row(ui, props, x, cursor, width);
    }
    if let Some(error) = props.error {
        cursor += SPACING;
        cursor += ui.paragraph(
            error,
            text::BODY,
            theme.red,
            Point::new(x, cursor),
            width,
            None,
        );
    }
    cursor += SPACING;
    let cancel_width = 66.0;
    let cancel = Rect::from_xywh(
        x + width - cancel_width,
        cursor,
        cancel_width,
        BUTTON_HEIGHT,
    );
    ui.button(Button::new("Cancel", Action::CancelSignIn), cancel);
    cursor + BUTTON_HEIGHT - y
}

fn fallback_row(ui: &mut Ui, props: &LoginProps, x: f32, y: f32, width: f32) -> f32 {
    const GAP: f32 = 8.0;
    let theme = ui.theme;
    let open_width = ui.text.width("Open in Browser", text::BODY) + 24.0;
    let submit_width = ui.text.width("Submit", text::BODY) + 24.0;
    let open = Rect::from_xywh(x, y, open_width, BUTTON_HEIGHT);
    ui.button(Button::new("Open in Browser", Action::OpenInBrowser), open);
    let submit = Rect::from_xywh(x + width - submit_width, y, submit_width, BUTTON_HEIGHT);
    let button =
        Button::new("Submit", Action::SubmitPastedUrl).enabled(props.can_submit_pasted_url);
    ui.button(button, submit);

    let field = Rect::from_ltrb(open.right + GAP, y, submit.left - GAP, y + BUTTON_HEIGHT);
    let shape = rounded(field, 6.0);
    ui.canvas.draw_rrect(
        shape,
        &fill(if theme.is_dark {
            theme.control
        } else {
            theme.window
        }),
    );
    let border = if props.pasted_url_focused {
        crate::ui::outline(theme.accent, 2.0)
    } else {
        crate::ui::outline(theme.glass_rim, 1.0)
    };
    ui.canvas.draw_rrect(shape, &border);
    let text_x = field.left + 8.0;
    let text_y = y + (BUTTON_HEIGHT - text::BODY.line) / 2.0;
    props.pasted_url.draw(
        ui,
        URL_HINT,
        theme.tertiary,
        (text_x, text_y),
        field.width() - 16.0,
        props.pasted_url_focused,
    );
    ui.hit_with(
        field,
        Action::FocusField(Field::PastedUrl, text_x),
        None,
        None,
        true,
    );
    BUTTON_HEIGHT
}
