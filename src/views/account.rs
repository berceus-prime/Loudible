//! The account button, its popover, and the signed-out sign-in prompt.

use skia_safe::{Point, Rect};

use crate::icons::Icon;
use crate::model::{Account, Marketplace};
use crate::text::{self, TextStyle, Weight};
use crate::theme::Theme;
use crate::ui::{Action, Button, ButtonStyle, Ui, capsule, fill};

pub const POPOVER_WIDTH: f32 = 280.0;
const CONTROL_HEIGHT: f32 = 28.0;
const SPACING: f32 = 14.0;
const PICKER_WIDTH: f32 = 153.0;
const LABEL: TextStyle = text::BODY.weight(Weight::Semibold);

/// A circle with the account's initial, or a person symbol when there is no name.
pub fn account_avatar(ui: &mut Ui, name: Option<&str>, center: Point, size: f32) {
    let radius = size / 2.0;
    let oval = Rect::from_xywh(center.x - radius, center.y - radius, size, size);
    let mut paint = fill(skia_safe::Color::WHITE);
    paint.set_shader(ui.vertical_gradient(
        oval,
        skia_safe::Color::from_rgb(179, 191, 230),
        skia_safe::Color::from_rgb(135, 148, 199),
    ));
    ui.canvas.draw_circle(center, radius, &paint);
    let white = skia_safe::Color::WHITE;
    match name.and_then(|n| n.trim().chars().next()) {
        Some(initial) => {
            let glyph = initial.to_uppercase().to_string();
            let size = size * 0.5;
            let style = TextStyle::new(size, size * 1.2, Weight::Semibold).rounded();
            ui.line_centered(&glyph, style, white, center.x, center.y - style.line / 2.0);
        }
        None => ui.icon(Icon::PersonFill, center, size * 0.5, white),
    }
}

/// The avatar, name and re-authentication warning, centred vertically on `cy`, returning its
/// width. Names longer than `max_width` are truncated.
pub fn account_button_label(
    ui: &mut Ui,
    name: Option<&str>,
    is_signed_in: bool,
    needs_reauthentication: bool,
    (x, cy): (f32, f32),
    max_width: f32,
) -> f32 {
    const WARNING: f32 = 16.0;
    let title = match (is_signed_in, name) {
        (false, _) => "Sign In",
        (true, Some(name)) => name,
        (true, None) => "Account",
    };
    let avatar_name = if is_signed_in { name } else { None };
    let mut chrome = 4.0 + 24.0 + 8.0 + 10.0;
    if needs_reauthentication {
        chrome += 8.0 + WARNING;
    }
    let text_width = ui.text.width(title, LABEL).min(max_width - chrome);
    account_avatar(ui, avatar_name, Point::new(x + 4.0 + 12.0, cy), 24.0);
    let text_x = x + 4.0 + 24.0 + 8.0;
    let y = cy - LABEL.line / 2.0;
    ui.line_limited(title, LABEL, ui.theme.label, text_x, y, Some(text_width));
    if needs_reauthentication {
        let center = Point::new(text_x + text_width + 8.0 + WARNING / 2.0, cy);
        ui.icon(Icon::WarningFill, center, WARNING, ui.theme.orange);
    }
    chrome + text_width
}

pub struct SignInProps {
    pub needs_reauthentication: bool,
    pub marketplace: Marketplace,
    pub can_change_marketplace: bool,
    pub can_sign_in: bool,
}

impl SignInProps {
    pub fn ready(marketplace: Marketplace) -> Self {
        Self {
            needs_reauthentication: false,
            marketplace,
            can_change_marketplace: true,
            can_sign_in: true,
        }
    }
}

fn picker(
    ui: &mut Ui,
    marketplace: Marketplace,
    enabled: bool,
    (x, y): (f32, f32),
    width: f32,
) -> f32 {
    let theme = ui.theme;
    let label = "Marketplace";
    let text_y = y + (CONTROL_HEIGHT - text::BODY.line) / 2.0;
    let color = if enabled { theme.label } else { theme.tertiary };
    ui.line(label, text::BODY, color, x, text_y);
    let left = x + ui.text.width(label, text::BODY) + 8.0;
    let rect = Rect::from_xywh(left, y, PICKER_WIDTH.min(x + width - left), CONTROL_HEIGHT);
    ui.canvas.draw_rrect(capsule(rect), &fill(theme.control));
    ui.line(
        marketplace.name,
        text::BODY,
        color,
        rect.left + 13.0,
        text_y,
    );
    ui.icon(
        Icon::UpDown,
        Point::new(rect.right - 15.0, rect.center_y()),
        13.0,
        color,
    );
    let action = if enabled {
        Action::OpenMarketplaceMenu(Point::new(rect.left, rect.bottom + 4.0))
    } else {
        Action::Block
    };
    ui.hit(rect, action);
    CONTROL_HEIGHT
}

/// The expired-session warning, marketplace picker and Sign In button; returns the height.
fn sign_in_controls(
    ui: &mut Ui,
    props: &SignInProps,
    uses_glass: bool,
    (x, y): (f32, f32),
    width: f32,
) -> f32 {
    let theme = ui.theme;
    let mut cursor = y;
    if props.needs_reauthentication {
        const ICON: f32 = 15.0;
        let message = "Your session expired. Sign in again to continue.";
        let center = Point::new(x + ICON / 2.0, cursor + text::BODY.line / 2.0);
        ui.icon(Icon::WarningFill, center, ICON, theme.orange);
        let origin = Point::new(x + ICON + 6.0, cursor);
        cursor += ui.paragraph(
            message,
            text::BODY,
            theme.orange,
            origin,
            width - ICON - 6.0,
            None,
        ) + SPACING;
    }
    cursor += picker(
        ui,
        props.marketplace,
        props.can_change_marketplace,
        (x, cursor),
        width,
    ) + SPACING;
    let style = if uses_glass {
        ButtonStyle::Clear
    } else {
        ButtonStyle::Prominent
    };
    let rect = Rect::from_xywh(x, cursor, width, CONTROL_HEIGHT);
    ui.button(
        Button::new("Sign In", Action::SignIn)
            .style(style)
            .enabled(props.can_sign_in),
        rect,
    );
    cursor + CONTROL_HEIGHT - y
}

pub enum PopoverContent<'a> {
    SignedIn {
        account: &'a Account,
        other_marketplaces: &'a [(Marketplace, usize)],
    },
    SignedOut(SignInProps),
}

/// The popover's content, `POPOVER_WIDTH` wide; returns the height.
pub fn account_popover(
    ui: &mut Ui,
    content: &PopoverContent,
    can_sign_out: bool,
    (x, y): (f32, f32),
) -> f32 {
    const PADDING: f32 = 16.0;
    let theme: Theme = ui.theme;
    let width = POPOVER_WIDTH - PADDING * 2.0;
    let left = x + PADDING;
    let mut cursor = y + PADDING;
    match content {
        PopoverContent::SignedIn {
            account,
            other_marketplaces,
        } => {
            let name = account.customer_name.as_deref();
            account_avatar(ui, name, Point::new(left + 22.0, cursor + 22.0), 44.0);
            let text_x = left + 44.0 + 12.0;
            let top = cursor + (44.0 - 34.0) / 2.0;
            ui.line(
                name.unwrap_or("Signed in"),
                text::HEADLINE,
                theme.label,
                text_x,
                top,
            );
            let store = format!("Audible {}", account.marketplace.name);
            ui.line(&store, text::BODY, theme.secondary, text_x, top + 18.0);
            cursor += 44.0;
            if !other_marketplaces.is_empty() {
                let list = other_marketplaces
                    .iter()
                    .map(|(store, count)| format!("{} ({count})", store.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let note = format!("Also has titles in {list}");
                let origin = Point::new(left, cursor + SPACING);
                cursor += SPACING
                    + ui.paragraph(&note, text::CALLOUT, theme.secondary, origin, width, None);
            }
        }
        PopoverContent::SignedOut(props) => {
            cursor += sign_in_controls(ui, props, false, (left, cursor), width);
        }
    }
    if can_sign_out {
        ui.separator(left, cursor + SPACING - 0.5, width);
        cursor += SPACING * 2.0 + 1.0;
        let rect = Rect::from_xywh(left, cursor, width, CONTROL_HEIGHT);
        ui.button(Button::new("Sign Out", Action::SignOut), rect);
        cursor += CONTROL_HEIGHT;
    }
    cursor + PADDING - y
}

/// The empty window's prompt, `POPOVER_WIDTH` wide; returns the height.
pub fn sign_in_panel(ui: &mut Ui, props: &SignInProps, (x, y): (f32, f32)) -> f32 {
    let title = text::TITLE3.weight(Weight::Semibold);
    let prompt = "Sign in to see your library";
    ui.line_centered(prompt, title, ui.theme.label, x + POPOVER_WIDTH / 2.0, y);
    let controls = y + title.line + SPACING;
    title.line + SPACING + sign_in_controls(ui, props, true, (x, controls), POPOVER_WIDTH)
}
