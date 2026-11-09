//! Pointer and keyboard state around a screen: which hit area is hovered and pressed, tooltips,
//! and turning raw input into actions. No window here, so tests can drive it directly.

use std::time::Duration;

use skia_safe::{Canvas, Point, Rect, Size};

use crate::shell::browser::BrowserRequest;
use crate::text::{self, TextSystem};
use crate::theme::Theme;
use crate::ui::{self, Action, Hit, Ui};

/// What a screen asks of the window system.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Close,
    Minimize,
    Zoom,
    Drag,
    Copy(String),
    OpenUrl(String),
    Reveal(std::path::PathBuf),
}

/// Where a caret move goes in a single-line field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Text(String),
    /// An input method's unconfirmed text, with the selection inside it as byte offsets; empty
    /// when it is withdrawn.
    Compose(String, Option<(usize, usize)>),
    /// Text an input method confirmed.
    Commit(String),
    Backspace {
        word: bool,
    },
    Delete {
        word: bool,
    },
    Move {
        to: Motion,
        select: bool,
    },
    Enter,
    Escape,
    SelectAll,
    Copy,
    Cut,
    Paste(String),
}

/// A primary button press: how many in quick succession, and whether Shift extends a selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    pub point: Point,
    pub clicks: u8,
    pub extend: bool,
}

pub trait Screen {
    fn render(&mut self, ui: &mut Ui, size: Size);
    fn handle(&mut self, action: Action, pointer: Point, text: &TextSystem) -> Option<Effect>;
    /// An action that acts as the button goes down, such as placing a caret.
    fn press(&mut self, action: Action, press: Press, text: &TextSystem) -> Option<Effect> {
        self.handle(action, press.point, text)
    }
    /// The pointer moved with the button held after a `press`, extending a text selection.
    fn drag(&mut self, _pointer: Point, _text: &TextSystem) {}
    fn key(&mut self, key: Key, text: &TextSystem) -> Option<Effect>;
    /// Where text is being typed, in window points, so input methods can place their panels.
    fn text_input_area(&self) -> Option<Rect> {
        None
    }
    fn scroll(&mut self, delta_y: f32);
    fn tick(&mut self, elapsed: Duration);
    /// How soon the screen needs another frame, or `None` when it is idle.
    fn next_frame(&self) -> Option<Duration>;
    /// The embedded browser this frame should show, if any.
    fn browser(&self) -> Option<BrowserRequest> {
        None
    }
    fn browser_callback(&mut self, _url: String) -> Option<Effect> {
        None
    }
    fn browser_failed(&mut self, _message: String) {}
    /// The user asked to close the window. True closes it now; false means the screen first
    /// asks them or stops its work, and reports through `is_ready_to_close`.
    fn close_requested(&mut self) -> bool {
        true
    }
    fn is_ready_to_close(&self) -> bool {
        false
    }
    /// The window came to the front.
    fn window_focused(&mut self) {}
}

const TOOLTIP_DELAY: Duration = Duration::from_millis(800);
/// AppKit's default double-click interval, and how far apart its clicks may land.
const MULTI_CLICK: Duration = Duration::from_millis(500);
const MULTI_CLICK_SLOP: f32 = 4.0;

pub struct Stage<S> {
    pub screen: S,
    pub theme: Theme,
    pub is_active: bool,
    text: TextSystem,
    hits: Vec<Hit>,
    pointer: Point,
    pressed: Option<Action>,
    /// A press placed a caret, so moving the pointer selects text until release.
    is_selecting: bool,
    last_click: Option<(Duration, Point, u8)>,
    time: Duration,
    hover_started: Duration,
}

impl<S: Screen> Stage<S> {
    pub fn new(screen: S, theme: Theme) -> Self {
        Self {
            screen,
            theme,
            is_active: true,
            text: TextSystem::default(),
            hits: vec![],
            pointer: Point::new(-1.0, -1.0),
            pressed: None,
            is_selecting: false,
            last_click: None,
            time: Duration::ZERO,
            hover_started: Duration::ZERO,
        }
    }

    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    pub fn advance(&mut self, elapsed: Duration) {
        self.time += elapsed;
        self.screen.tick(elapsed);
    }

    pub fn draw(&mut self, canvas: &Canvas, size: Size) {
        let hot = self.top().map(|hit| hit.action.clone());
        let mut ui = Ui::new(
            canvas,
            self.theme,
            &self.text,
            hot,
            self.pressed.clone(),
            self.time,
            self.is_active,
        );
        self.screen.render(&mut ui, size);
        self.hits = std::mem::take(&mut ui.hits);
        if self.time.saturating_sub(self.hover_started) >= TOOLTIP_DELAY
            && self.pressed.is_none()
            && let Some(tooltip) = self.top().and_then(|hit| hit.tooltip.clone())
        {
            let mut ui = Ui::new(
                canvas,
                self.theme,
                &self.text,
                None,
                None,
                self.time,
                self.is_active,
            );
            draw_tooltip(&mut ui, &tooltip, self.pointer, size);
        }
    }

    fn top(&self) -> Option<&Hit> {
        let p = self.pointer;
        self.hits.iter().rev().find(|hit| {
            p.x >= hit.rect.left
                && p.x < hit.rect.right
                && p.y >= hit.rect.top
                && p.y < hit.rect.bottom
        })
    }

    /// Whether this changed what is hovered, so the window needs a redraw.
    pub fn pointer_moved(&mut self, pointer: Point) -> bool {
        let before = self.top().map(|hit| hit.action.clone());
        self.pointer = pointer;
        let after = self.top().map(|hit| hit.action.clone());
        if self.is_selecting {
            self.screen.drag(pointer, &self.text);
            return true;
        }
        if before != after {
            self.hover_started = self.time;
        }
        before != after
    }

    pub fn is_over_text(&self) -> bool {
        self.top().is_some_and(|hit| hit.is_text)
    }

    pub fn tooltip_due(&self) -> Option<Duration> {
        self.top()?.tooltip.as_ref()?;
        (self.hover_started + TOOLTIP_DELAY).checked_sub(self.time)
    }

    pub fn pointer_down(&mut self, extend: bool) -> Option<Effect> {
        self.hover_started = self.time;
        let action = self.top()?.action.clone();
        // These act as the button goes down; everything else waits for release, like AppKit.
        if matches!(action, Action::DragWindow | Action::FocusField(..)) {
            let clicks = match self.last_click {
                Some((at, point, clicks))
                    if self.time.saturating_sub(at) <= MULTI_CLICK
                        && (point - self.pointer).length() <= MULTI_CLICK_SLOP =>
                {
                    clicks % 3 + 1
                }
                _ => 1,
            };
            self.last_click = Some((self.time, self.pointer, clicks));
            self.is_selecting = matches!(action, Action::FocusField(..));
            let press = Press {
                point: self.pointer,
                clicks,
                extend,
            };
            return self.screen.press(action, press, &self.text);
        }
        self.pressed = Some(action);
        None
    }

    pub fn key(&mut self, key: Key) -> Option<Effect> {
        self.screen.key(key, &self.text)
    }

    pub fn pointer_up(&mut self) -> Option<Effect> {
        self.is_selecting = false;
        let pressed = self.pressed.take()?;
        let released = self.top().map(|hit| hit.action.clone());
        (released.as_ref() == Some(&pressed))
            .then(|| self.screen.handle(pressed, self.pointer, &self.text))
            .flatten()
    }

    pub fn secondary_click(&mut self) -> Option<Effect> {
        let action = self.top()?.secondary.clone()?;
        self.screen.handle(action, self.pointer, &self.text)
    }

    /// A full press and release at `point`, for tests.
    pub fn click(&mut self, point: Point) -> Option<Effect> {
        self.pointer_moved(point);
        self.pointer_down(false).or_else(|| self.pointer_up())
    }
}

fn draw_tooltip(ui: &mut Ui, tooltip: &str, pointer: Point, size: Size) {
    const MAX_WIDTH: f32 = 320.0;
    let style = text::CALLOUT;
    let lines = ui.text.wrap(tooltip, style, MAX_WIDTH, None);
    let width = lines
        .iter()
        .map(|l| ui.text.width(l, style))
        .fold(0.0, f32::max)
        + 12.0;
    let height = lines.len() as f32 * style.line + 8.0;
    let x = pointer.x.min(size.width - width - 4.0).max(4.0);
    let y = if pointer.y + 22.0 + height > size.height {
        pointer.y - height - 6.0
    } else {
        pointer.y + 22.0
    };
    let rect = skia_safe::Rect::from_xywh(x, y, width, height);
    let shape = ui::rounded(rect, 4.0);
    ui.shadow(shape, skia_safe::Color::new(0x4000_0000), 8.0, 2.0);
    ui.material(shape);
    ui.canvas
        .draw_rrect(shape, &ui::outline(ui.theme.material_rim, 0.5));
    for (index, line) in lines.iter().enumerate() {
        ui.line(
            line,
            style,
            ui.theme.label,
            x + 6.0,
            y + 4.0 + index as f32 * style.line,
        );
    }
}
