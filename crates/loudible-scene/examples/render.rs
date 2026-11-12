//! Renders a scene document to a PNG, so a change can be seen without running the app.
//!
//! ```sh
//! cargo run -p loudible-scene --features testing --example render -- toolbar
//! cargo run -p loudible-scene --features testing --example render -- error-toast /tmp/toast.png
//! cargo run -p loudible-scene --features testing --example render -- toolbar --dark --size 900x52
//! ```
//!
//! The scene is a file under `scenes/`, by name or by path. Data comes from a sibling
//! `<name>.data.json` unless `--data` names another, and that file's optional `$render` block
//! sets the window size, scale and appearance, so each scene can describe how to look at it.

use std::path::{Path, PathBuf};

use loudible_scene::testing::{Builder, FIELDS, IMAGES};
use loudible_scene::{Appearance, Inputs, Platform, Renderer, Scene, TextSystem};
use serde_json::{Value, json};
use skia_safe::{Color, EncodedImageFormat, Size, surfaces};

struct Options {
    scene: PathBuf,
    out: PathBuf,
    data: Option<PathBuf>,
    size: Option<Size>,
    scale: Option<f32>,
    dark: Option<bool>,
}

fn usage() -> ! {
    eprintln!(
        "usage: render <scene> [out.png] [--data file] [--size WxH] [--scale N] [--dark|--light]"
    );
    std::process::exit(2);
}

fn scenes_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scenes")
}

/// A bare name means a file in `scenes/`, with or without the extension.
fn find_scene(name: &str) -> PathBuf {
    let direct = PathBuf::from(name);
    if direct.exists() {
        return direct;
    }
    for candidate in [name.to_string(), format!("{name}.json")] {
        let path = scenes_dir().join(&candidate);
        if path.exists() {
            return path;
        }
    }
    eprintln!("no scene called {name:?} in {}", scenes_dir().display());
    std::process::exit(1);
}

fn parse_args() -> Options {
    let mut positional: Vec<String> = vec![];
    let mut options = Options {
        scene: PathBuf::new(),
        out: PathBuf::new(),
        data: None,
        size: None,
        scale: None,
        dark: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dark" => options.dark = Some(true),
            "--light" => options.dark = Some(false),
            "--data" => options.data = args.next().map(PathBuf::from),
            "--scale" => options.scale = args.next().and_then(|value| value.parse().ok()),
            "--size" => {
                let Some(value) = args.next() else { usage() };
                let (width, height) = value.split_once(['x', 'X']).unwrap_or_else(|| usage());
                match (width.parse::<f32>(), height.parse::<f32>()) {
                    (Ok(width), Ok(height)) => options.size = Some(Size::new(width, height)),
                    _ => usage(),
                }
            }
            "-h" | "--help" => usage(),
            other if other.starts_with('-') => usage(),
            other => positional.push(other.to_string()),
        }
    }
    let Some(name) = positional.first() else { usage() };
    options.scene = find_scene(name);
    options.out = positional
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{name}.png")));
    options
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

/// The data beside the scene, unless another was named.
fn load_data(options: &Options) -> Value {
    if let Some(path) = &options.data {
        return read_json(path);
    }
    let sibling = options.scene.with_extension("data.json");
    if sibling.exists() {
        return read_json(&sibling);
    }
    json!({})
}

fn main() {
    let options = parse_args();
    let stem = options
        .scene
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("scene")
        .to_string();

    let mut data = load_data(&options);
    let directions = data
        .as_object_mut()
        .and_then(|object| object.remove("$render"))
        .unwrap_or(Value::Null);

    let document = read_json(&options.scene);
    let is_component = document.get("body").is_some();
    let mut builder = Builder::standard();
    let root = if is_component {
        // A component file is instantiated, taking the data file's `props` as its arguments.
        builder = builder.component_value(&stem, document);
        json!({
            "type": stem,
            "props": data.get("props").cloned().unwrap_or_else(|| json!({}))
        })
    } else {
        document
    };
    let scene: Scene = builder.root(root).build().unwrap_or_else(|error| {
        eprintln!("{}: {error}", options.scene.display());
        std::process::exit(1);
    });

    let size = options
        .size
        .or_else(|| {
            let pair = directions.get("size")?;
            Some(Size::new(
                pair.get(0)?.as_f64()? as f32,
                pair.get(1)?.as_f64()? as f32,
            ))
        })
        .unwrap_or(Size::new(1100.0, 652.0));
    let scale = options
        .scale
        .or_else(|| directions.get("scale")?.as_f64().map(|n| n as f32))
        .unwrap_or(2.0);
    let dark = options
        .dark
        .or_else(|| directions.get("dark")?.as_bool())
        .unwrap_or(false);
    let appearance = if dark {
        Appearance::Dark
    } else {
        Appearance::Light
    };

    let pixels = (
        (size.width * scale).round() as i32,
        (size.height * scale).round() as i32,
    );
    let mut surface = surfaces::raster_n32_premul(pixels).expect("a raster surface");
    let canvas = surface.canvas();
    canvas.scale((scale, scale));
    canvas.clear(if dark {
        Color::new(0xFF1E1E1E)
    } else {
        Color::WHITE
    });

    let inputs = Inputs::new(size, &data)
        .appearance(appearance)
        .platform(Platform::MacOs);
    let system = TextSystem::default();
    let renderer = Renderer::new(&system, &IMAGES, &FIELDS);
    let frame = scene.render(&renderer, canvas, &inputs);

    let encoded = surface
        .image_snapshot()
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("PNG encoding");
    std::fs::write(&options.out, encoded.as_bytes())
        .unwrap_or_else(|error| panic!("write {}: {error}", options.out.display()));

    println!(
        "{} -> {} ({}x{} at {scale}x, {})",
        options.scene.display(),
        options.out.display(),
        size.width,
        size.height,
        if dark { "dark" } else { "light" },
    );
    println!("{} hit regions, {} native views", frame.hits.len(), frame.hosts.len());
    for region in &frame.hits {
        let rect = region.rect;
        let name = if region.blocks && region.action.is_empty() {
            "<blocks>"
        } else {
            &region.action
        };
        println!(
            "  {name:<24} {:>7.1} {:>7.1} {:>7.1} x {:>6.1}{}",
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            region
                .tooltip
                .as_deref()
                .map(|tip| format!("  \"{tip}\""))
                .unwrap_or_default()
        );
    }
}
