//! macOS semantic colours.

use skia_safe::Color;

const fn rgba(r: u8, g: u8, b: u8, alpha: f32) -> Color {
    Color::new(((alpha * 255.0 + 0.5) as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub is_dark: bool,
    pub window: Color,
    pub label: Color,
    pub secondary: Color,
    pub tertiary: Color,
    pub placeholder_fill: Color,
    pub separator: Color,
    /// Fill of bordered buttons and pop-up pickers.
    pub control: Color,
    pub control_pressed: Color,
    /// Fill and rim of the toolbar's glass capsules.
    pub glass: Color,
    pub glass_rim: Color,
    pub cover_rim: Color,
    /// Laid over a blurred backdrop for toasts, popovers and menus.
    pub material: Color,
    pub material_rim: Color,
    /// Primary text on a material, which vibrancy softens.
    pub on_material: Color,
    pub toast_button: Color,
    pub accent: Color,
    pub red: Color,
    pub orange: Color,
}

impl Theme {
    pub const LIGHT: Self = Self {
        is_dark: false,
        window: Color::WHITE,
        label: rgba(0, 0, 0, 0.85),
        secondary: rgba(0, 0, 0, 0.5),
        tertiary: rgba(0, 0, 0, 0.3),
        placeholder_fill: rgba(0, 0, 0, 0.12),
        separator: rgba(0, 0, 0, 0.1),
        control: rgba(0, 0, 0, 0.08),
        control_pressed: rgba(0, 0, 0, 0.16),
        glass: rgba(255, 255, 255, 0.85),
        glass_rim: rgba(0, 0, 0, 0.16),
        cover_rim: rgba(0, 0, 0, 0.1),
        material: rgba(240, 240, 240, 0.9),
        material_rim: rgba(0, 0, 0, 0.12),
        on_material: rgba(0, 0, 0, 0.72),
        toast_button: rgba(255, 255, 255, 0.2),
        accent: Color::new(0xFF30_78F6),
        red: Color::new(0xFFFF_383C),
        orange: Color::new(0xFFFF_8D28),
    };

    pub const DARK: Self = Self {
        is_dark: true,
        window: Color::new(0xFF1E_1E1E),
        label: rgba(255, 255, 255, 0.85),
        secondary: rgba(255, 255, 255, 0.55),
        tertiary: rgba(255, 255, 255, 0.25),
        placeholder_fill: rgba(255, 255, 255, 0.1),
        separator: rgba(255, 255, 255, 0.12),
        control: rgba(255, 255, 255, 0.12),
        control_pressed: rgba(255, 255, 255, 0.22),
        glass: rgba(60, 60, 60, 0.7),
        glass_rim: rgba(255, 255, 255, 0.16),
        cover_rim: rgba(255, 255, 255, 0.1),
        material: rgba(36, 36, 36, 0.9),
        material_rim: rgba(255, 255, 255, 0.1),
        on_material: rgba(255, 255, 255, 0.85),
        toast_button: rgba(0, 0, 0, 0.3),
        accent: Color::new(0xFF0A_84FF),
        red: Color::new(0xFFFF_453A),
        orange: Color::new(0xFFFF_9F0A),
    };
}

/// `color` with its alpha multiplied by `opacity`.
pub fn faded(color: Color, opacity: f32) -> Color {
    color.with_a((f32::from(color.a()) * opacity) as u8)
}
