//! `loudible-scene` in a browser.
//!
//! One export draws a scene document onto a canvas; two more hand over the fonts and pictures
//! the document needs, which a browser cannot find for itself. Everything crosses the boundary
//! as plain JSON, so the document the page passes is the document `SCHEMA.md` describes.
//!
//! ```js
//! import Module from "./loudible_scene_wasm.js";
//! const scene = await Module();
//! await scene.registerFont(new Uint8Array(fontBytes));
//! const frame = scene.render(canvas, document, { dark: true, scale: 2, data });
//! ```
//!
//! The document is painted into a raster buffer and blitted with `putImageData`. TODO: a WebGL
//! backend through Ganesh, the way rust-skia's own `wasm-example` wires it up, for scenes that
//! animate rather than redraw on edit.

mod fields;
mod fonts;
mod frame;
mod images;
mod options;

use std::cell::Cell;

use js_sys::JSON;
use loudible_scene::{Renderer, Scene};
use serde_json::Value;
use skia_safe::{AlphaType, ColorType, ImageInfo, surfaces};
use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};

use frame::Painted;
use options::Options;

/// Emscripten calls this when the module is instantiated. The exports below do the work.
fn main() {}

fn fail(message: impl std::fmt::Display) -> JsValue {
    JsError::new(&message.to_string()).into()
}

/// A JS value as JSON. Going through `JSON.stringify` keeps one conversion for the document, the
/// options and the frame, and gives the page the same errors it would get from `JSON.parse`.
fn to_json(value: &JsValue) -> Result<Value, JsValue> {
    if value.is_undefined() || value.is_null() {
        return Ok(Value::Null);
    }
    let text = JSON::stringify(value)
        .map_err(|_| fail("that value cannot be turned into JSON"))?
        .as_string()
        .ok_or_else(|| fail("that value cannot be turned into JSON"))?;
    serde_json::from_str(&text).map_err(fail)
}

fn from_json<T: serde::Serialize>(value: &T) -> Result<JsValue, JsValue> {
    let text = serde_json::to_string(value).map_err(fail)?;
    JSON::parse(&text).map_err(|_| fail("the frame could not be handed back"))
}

/// Milliseconds, from the clock the page has.
fn now() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now())
        .unwrap_or_else(js_sys::Date::now)
}

thread_local! {
    static WARNED: Cell<bool> = const { Cell::new(false) };
}

/// A document full of text drawn with no font registered is a blank frame and a long afternoon,
/// so say it once.
fn warn_about_fonts() {
    if !WARNED.replace(true) {
        web_sys::console::warn_1(&JsValue::from_str(
            "loudible-scene: no font registered, so text will not draw. Call registerFont(bytes) \
             with a .ttf or .otf first.",
        ));
    }
}

/// Adds a face from the bytes of a `.ttf` or `.otf`, under `family` or the name it carries.
/// Returns the family a document's text styles should ask for.
#[wasm_bindgen(js_name = registerFont)]
pub fn register_font(bytes: &[u8], family: Option<String>) -> Result<String, JsValue> {
    fonts::register(bytes, family).map_err(fail)
}

/// The families registered so far, in the order they arrived.
#[wasm_bindgen(js_name = fontFamilies)]
pub fn font_families() -> Vec<String> {
    fonts::families()
}

/// Decodes a PNG or JPEG and keeps it under `name`, which is what an `image` node's `source`
/// names. A source that was never registered leaves the node to paint its fallback children.
#[wasm_bindgen(js_name = registerImage)]
pub fn register_image(name: &str, bytes: &[u8]) -> Result<(), JsValue> {
    images::register(name, bytes).map_err(fail)
}

/// Draws `document` onto `canvas` and gives back the frame: the regions that can be clicked, the
/// rects reserved for native views, what scrollers measured, and how long it took.
///
/// `document` is a complete scene document — `schema`, `tokens`, `symbols`, `components` and
/// `root` — exactly as `SCHEMA.md` describes it. Assembling one from the files under `scenes/`
/// is the caller's job.
#[wasm_bindgen]
pub fn render(
    canvas: &HtmlCanvasElement,
    document: JsValue,
    options: JsValue,
) -> Result<JsValue, JsValue> {
    let started = now();

    let scene = Scene::from_value(to_json(&document)?).map_err(fail)?;
    let options = match to_json(&options)? {
        Value::Null => Options::default(),
        given => serde_json::from_value::<Options>(given).map_err(fail)?,
    };

    let size = options.size(canvas.width(), canvas.height());
    let scale = options.scale.max(0.01);
    let (width, height) = (
        (size.width * scale).round().max(1.0) as i32,
        (size.height * scale).round().max(1.0) as i32,
    );
    canvas.set_width(width as u32);
    canvas.set_height(height as u32);

    let info = ImageInfo::new((width, height), ColorType::RGBA8888, AlphaType::Premul, None);
    let row = info.min_row_bytes();
    let mut pixels = vec![0_u8; row * height as usize];
    let background = options.background().map_err(fail)?;

    let painted = {
        let mut surface = surfaces::wrap_pixels(&info, &mut pixels, row, None)
            .ok_or_else(|| fail(format!("no {width}x{height} surface")))?;
        let skia = surface.canvas();
        // An opaque ground, so Skia's premultiplied pixels are what `ImageData` wants to read.
        skia.clear(background);
        skia.scale((scale, scale));

        fonts::with_engine(|book| {
            if book.is_empty() {
                warn_about_fonts();
            }
            images::with_store(|store| {
                let renderer = Renderer::new(book.engine(), store, &fields::NoFields);
                let frame = scene.render(&renderer, skia, &options.inputs(size));
                Painted::new(&frame, now() - started, book.families().len(), store.len())
            })
        })
    };

    let context: CanvasRenderingContext2d = canvas
        .get_context("2d")?
        .ok_or_else(|| fail("that canvas has no 2d context"))?
        .dyn_into()?;
    // The pixels go over as a `Uint8ClampedArray` we build ourselves, rather than through
    // `ImageData::new_with_u8_clamped_array_and_sh`: in emscripten mode wasm-bindgen hands that
    // constructor a view of `HEAPU8`, which is a `Uint8Array`, and the browser then reads the
    // call as `new ImageData(sw, sh, settings)` and refuses it.
    let array = js_sys::Uint8ClampedArray::new_with_length(pixels.len() as u32);
    array.copy_from(&pixels);
    let image = ImageData::new_with_js_u8_clamped_array_and_sh(&array, width as u32, height as u32)?;
    context.put_image_data(&image, 0.0, 0.0)?;

    from_json(&painted)
}
