//! The whole pipeline: read, parse, lay out, paint.

use loudible_scene::testing::{Builder, renderer};
use loudible_scene::{Inputs, Platform};
use serde_json::{Value, json};
use skia_safe::{Color, Size, surfaces};

/// Renders at 1x into a raster surface and hands back the frame and a pixel probe.
fn render(root: Value, data: &Value, size: (i32, i32)) -> (loudible_scene::Frame, Vec<u8>) {
    let scene = Builder::standard().root(root).parse();
    let mut surface = surfaces::raster_n32_premul(size).expect("a surface");
    surface.canvas().clear(Color::WHITE);
    let inputs = Inputs::new(Size::new(size.0 as f32, size.1 as f32), data)
        .platform(Platform::MacOs);
    let frame = scene.render(&renderer(), surface.canvas(), &inputs);

    let info = surface.image_info();
    let row = info.min_row_bytes();
    let mut pixels = vec![0_u8; row * size.1 as usize];
    assert!(
        surface.read_pixels(&info, &mut pixels, row, (0, 0)),
        "read the pixels back"
    );
    (frame, pixels)
}

fn at(pixels: &[u8], size: (i32, i32), x: i32, y: i32) -> (u8, u8, u8) {
    let index = (y as usize * size.0 as usize + x as usize) * 4;
    (pixels[index], pixels[index + 1], pixels[index + 2])
}

#[test]
fn a_filled_box_lands_where_layout_put_it() {
    let size = (100, 100);
    let (_, pixels) = render(
        json!({
            "type": "layer",
            "children": [{
                "type": "box",
                "props": {
                    "place": { "left": 20, "top": 30 },
                    "size": { "width": 40, "height": 20 },
                    "fill": "red"
                }
            }]
        }),
        &json!({}),
        size,
    );
    assert_eq!(at(&pixels, size, 40, 40), (255, 56, 60), "inside the box");
    assert_eq!(at(&pixels, size, 10, 40), (255, 255, 255), "to its left");
    assert_eq!(at(&pixels, size, 40, 20), (255, 255, 255), "above it");
}

#[test]
fn a_stack_lays_its_children_out_along_its_axis() {
    let size = (100, 100);
    let (_, pixels) = render(
        json!({
            "type": "stack",
            "props": { "axis": "y", "gap": 10, "size": { "width": 100, "height": 100 } },
            "children": [
                { "type": "box", "props": { "size": { "height": 20 }, "fill": "red" } },
                { "type": "box", "props": { "size": { "height": 20 }, "fill": "accent" } }
            ]
        }),
        &json!({}),
        size,
    );
    assert_eq!(at(&pixels, size, 50, 10), (255, 56, 60), "the first row");
    assert_eq!(at(&pixels, size, 50, 25), (255, 255, 255), "the gap");
    assert_eq!(at(&pixels, size, 50, 40), (48, 120, 246), "the second row");
}

#[test]
fn opacity_dims_a_whole_subtree() {
    let size = (40, 40);
    let (_, pixels) = render(
        json!({
            "type": "box",
            "props": { "opacity": 0.5 },
            "children": [{ "type": "box", "props": { "fill": "#FF000000" } }]
        }),
        &json!({}),
        size,
    );
    let (r, g, b) = at(&pixels, size, 20, 20);
    assert!(
        (120..=135).contains(&r) && r == g && g == b,
        "half-black over white, found {r},{g},{b}"
    );
}

#[test]
fn a_knockout_leaves_a_hole() {
    let size = (40, 40);
    let (_, pixels) = render(
        json!({
            "type": "box",
            "props": { "isolate": true },
            "children": [
                { "type": "box", "props": { "fill": "red" } },
                {
                    "type": "box",
                    "props": {
                        "place": { "left": 10, "top": 10 },
                        "size": { "width": 20, "height": 20 },
                        "fill": "#FF000000",
                        "blend": "dst-out"
                    }
                }
            ]
        }),
        &json!({}),
        size,
    );
    assert_eq!(at(&pixels, size, 5, 5), (255, 56, 60), "the disc");
    assert_eq!(at(&pixels, size, 20, 20), (255, 255, 255), "the hole");
}

#[test]
fn regions_come_back_in_paint_order() {
    let (frame, _) = render(
        json!({
            "type": "layer",
            "props": { "hit": { "action": "drag-window" } },
            "children": [{
                "type": "box",
                "props": {
                    "place": { "left": 10, "top": 10 },
                    "size": 20,
                    "hit": { "action": "close-window" }
                }
            }]
        }),
        &json!({}),
        (100, 100),
    );
    let actions: Vec<&str> = frame.hits.iter().map(|hit| hit.action.as_str()).collect();
    assert_eq!(
        actions,
        ["drag-window", "close-window"],
        "the control sits on top of the bar it is in"
    );
}

#[test]
fn a_clip_trims_the_regions_inside_it() {
    let (frame, _) = render(
        json!({
            "type": "box",
            "props": { "size": { "width": 50, "height": 50 }, "clip": true },
            "children": [{
                "type": "box",
                "props": {
                    "place": { "left": 25, "top": 25 },
                    "size": 60,
                    "hit": { "action": "toggle-book" }
                }
            }]
        }),
        &json!({}),
        (100, 100),
    );
    let region = frame.hits.first().expect("one region");
    assert_eq!(
        (region.rect.width(), region.rect.height()),
        (25.0, 25.0),
        "clipped to what is visible"
    );
}

#[test]
fn a_host_reserves_its_rect() {
    let (frame, _) = render(
        json!({
            "type": "stack",
            "props": { "axis": "y", "size": { "width": 200, "height": 100 } },
            "children": [
                { "type": "box", "props": { "size": { "height": 20 } } },
                { "type": "host", "props": { "name": "sign-in", "size": { "height": 60 } } }
            ]
        }),
        &json!({}),
        (200, 100),
    );
    let host = frame.hosts.first().expect("one native view");
    assert_eq!(host.name, "sign-in");
    assert_eq!((host.rect.top, host.rect.height()), (20.0, 60.0));
}

#[test]
fn the_authored_views_render() {
    for (file, size) in [("toolbar.json", (1100, 52)), ("error-toast.json", (480, 260))] {
        let data = loudible_scene::testing::read_scene(&file.replace(".json", ".data.json"));
        let root = loudible_scene::testing::read_scene(file);
        let (frame, _) = render(root, &data, size);
        assert!(!frame.hits.is_empty(), "{file} registers regions");
    }
}
