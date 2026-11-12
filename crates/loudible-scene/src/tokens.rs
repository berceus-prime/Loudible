//! Turning token names, hex literals and bindings into colours and text styles.

use skia_safe::Color;

use crate::document::{ColorSpec, TextStyleSpec, Tokens, Weight};
use crate::ports::TextStyle;
use crate::scope::Scope;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    Light,
    Dark,
}

impl Appearance {
    pub fn is_dark(self) -> bool {
        self == Self::Dark
    }
}

/// Where a linear gradient runs. Only the two directions the app uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Vertical,
    Horizontal,
}

/// A resolved fill: one colour, or a gradient down or across the box.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    Solid(Color),
    Gradient {
        direction: Direction,
        stops: Vec<Color>,
    },
}

impl Fill {
    /// The colour to use where a gradient makes no sense, such as a stroke.
    pub fn flat(&self) -> Color {
        match self {
            Self::Solid(color) => *color,
            Self::Gradient { stops, .. } => stops.first().copied().unwrap_or(Color::TRANSPARENT),
        }
    }
}

pub struct Palette<'a> {
    tokens: &'a Tokens,
    appearance: Appearance,
}

impl<'a> Palette<'a> {
    pub fn new(tokens: &'a Tokens, appearance: Appearance) -> Self {
        Self { tokens, appearance }
    }

    /// `None` when the document asks for something the data has not supplied, which paints
    /// nothing rather than guessing a colour.
    pub fn fill(&self, spec: &ColorSpec, scope: &Scope) -> Option<Fill> {
        match spec {
            ColorSpec::Named(name) => self.named(name, scope).map(Fill::Solid),
            ColorSpec::Computed(computed) => {
                if let Some(gradient) = &computed.gradient {
                    let stops: Vec<Color> = gradient
                        .stops
                        .iter()
                        .filter_map(|stop| self.color(stop, scope))
                        .collect();
                    if stops.len() < 2 {
                        return None;
                    }
                    let horizontal = matches!(gradient.to.as_deref(), Some("right"))
                        || matches!(gradient.from.as_deref(), Some("left"));
                    return Some(Fill::Gradient {
                        direction: if horizontal {
                            Direction::Horizontal
                        } else {
                            Direction::Vertical
                        },
                        stops,
                    });
                }
                self.color(spec, scope).map(Fill::Solid)
            }
        }
    }

    pub fn color(&self, spec: &ColorSpec, scope: &Scope) -> Option<Color> {
        match spec {
            ColorSpec::Named(name) => self.named(name, scope),
            ColorSpec::Computed(computed) => {
                let base = if let Some((first, second, amount)) = &computed.mix {
                    let first = self.color(first, scope)?;
                    let second = self.color(second, scope)?;
                    mix(first, second, *amount)
                } else if let Some(token) = &computed.token {
                    self.token(token)?
                } else if let Some(hex) = &computed.hex {
                    parse_hex(hex)?
                } else if let Some(path) = &computed.path {
                    self.bound(path, scope)?
                } else if let Some(gradient) = &computed.gradient {
                    // A gradient asked for as a flat colour reads as its first stop.
                    self.color(gradient.stops.first()?, scope)?
                } else {
                    return None;
                };
                let opacity = computed
                    .opacity
                    .as_ref()
                    .and_then(|spec| scope.number(spec));
                Some(match opacity {
                    Some(opacity) => faded(base, opacity),
                    None => base,
                })
            }
        }
    }

    /// A `#` literal, a `$binding` or a token name.
    fn named(&self, name: &str, scope: &Scope) -> Option<Color> {
        if let Some(hex) = name.strip_prefix('#') {
            return parse_hex(hex);
        }
        if let Some(path) = name.strip_prefix('$') {
            return self.bound(path, scope);
        }
        self.token(name)
    }

    fn token(&self, name: &str) -> Option<Color> {
        let value = self.tokens.colors.get(name)?;
        let hex = match value {
            crate::document::TokenColor::Both(hex) => hex,
            crate::document::TokenColor::Pair { light, dark } => {
                if self.appearance.is_dark() { dark } else { light }
            }
        };
        parse_hex(hex)
    }

    /// Data may hand over a `#` literal, a token name, or a packed number as the model's own
    /// `CoverColor` does.
    fn bound(&self, path: &str, scope: &Scope) -> Option<Color> {
        match scope.lookup(path)? {
            serde_json::Value::String(text) => self.named(text, scope),
            serde_json::Value::Number(number) => {
                let packed = number.as_u64()? as u32;
                Some(if packed > 0x00FF_FFFF {
                    Color::new(packed)
                } else {
                    Color::new(0xFF00_0000 | packed)
                })
            }
            _ => None,
        }
    }

    /// A named style with any overrides applied. Falls back to the `body` token, then to the
    /// port's own default, so a document that forgets a style still draws.
    pub fn text_style(&self, spec: Option<&TextStyleSpec>, scope: &Scope) -> TextStyle {
        let (name, overrides) = match spec {
            Some(TextStyleSpec::Named(name)) => (Some(name.as_str()), None),
            Some(TextStyleSpec::Spec(overrides)) => {
                (overrides.style.as_deref(), Some(overrides))
            }
            None => (None, None),
        };
        let base = name
            .or(Some("body"))
            .and_then(|name| self.tokens.text.get(name))
            .copied();
        let mut style = base.map_or_else(TextStyle::default, |literal| TextStyle {
            size: literal.size,
            line: literal.line,
            weight: literal.weight,
            rounded: literal.rounded,
            tabular: literal.tabular,
        });
        if let Some(overrides) = overrides {
            if let Some(size) = overrides.size.as_ref().and_then(|spec| scope.number(spec)) {
                style.size = size;
                // A size with no line of its own keeps the proportion AppKit's styles use.
                if overrides.line.is_none() && base.is_none() {
                    style.line = size * 1.2;
                }
            }
            if let Some(line) = overrides.line.as_ref().and_then(|spec| scope.number(spec)) {
                style.line = line;
            }
            if let Some(weight) = overrides.weight {
                style.weight = weight;
            }
            if let Some(rounded) = overrides.rounded {
                style.rounded = rounded;
            }
            if let Some(tabular) = overrides.tabular {
                style.tabular = tabular;
            }
        }
        style
    }

    /// What a node inherits when nothing above it set a colour: the `label` token, or black.
    pub fn default_color(&self) -> Color {
        self.token("label").unwrap_or(Color::BLACK)
    }

    pub fn metric(&self, name: &str) -> Option<f32> {
        self.tokens.metrics.get(name).copied()
    }
}

/// `#RRGGBB` is opaque; `#AARRGGBB` carries its own alpha.
pub fn parse_hex(hex: &str) -> Option<Color> {
    let digits = hex.strip_prefix('#').unwrap_or(hex);
    let packed = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        6 => Some(Color::new(0xFF00_0000 | packed)),
        8 => Some(Color::new(packed)),
        _ => None,
    }
}

/// `color` with its alpha multiplied, as `theme::faded` does.
pub fn faded(color: Color, opacity: f32) -> Color {
    color.with_a((f32::from(color.a()) * opacity.clamp(0.0, 1.0)) as u8)
}

/// Blends towards `other`, as the synthetic covers' stripe does.
pub fn mix(color: Color, other: Color, amount: f32) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |from: u8, to: u8| {
        (f32::from(from) + (f32::from(to) - f32::from(from)) * amount) as u8
    };
    Color::from_argb(
        channel(color.a(), other.a()),
        channel(color.r(), other.r()),
        channel(color.g(), other.g()),
        channel(color.b(), other.b()),
    )
}

/// Skia's weight numbers, for a port that wants them.
pub fn weight_value(weight: Weight) -> i32 {
    weight as i32
}
