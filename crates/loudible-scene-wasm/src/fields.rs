//! Editable text, which this host does not have.
//!
//! A `field` node reserves its box and draws nothing. The other swap point: a browser-side editor
//! would implement `Fields` here and leave the render path alone.

use loudible_scene::{Fields, ports::FieldPaint};
use skia_safe::{Canvas, Rect};

pub struct NoFields;

impl Fields for NoFields {
    fn draw(&self, _canvas: &Canvas, _field: FieldPaint<'_>) {}

    fn input_area(&self, _binding: &str) -> Option<Rect> {
        None
    }
}
