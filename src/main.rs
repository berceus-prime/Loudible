use skia_ui::{
    composition,
    model::MockOptions,
    shell::{app_view::AppView, gallery::GalleryView, window},
};

const WINDOW: (f64, f64) = (1100.0, 652.0);

/// Used by the packaging checks: proves the bundled FFmpeg loads, is LGPL, and is complete.
fn self_test() -> i32 {
    let (license, missing) = (
        loudible_ffmpeg::license(),
        loudible_ffmpeg::missing_components(),
    );
    println!(
        "Loudible self-test: libavformat {}",
        loudible_ffmpeg::libavformat_version()
    );
    println!("license: {license}");
    let listed = if missing.is_empty() {
        "none".to_string()
    } else {
        missing.join(", ")
    };
    println!("missing components: {listed}");
    i32::from(!(missing.is_empty() && license.contains("LGPL")))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|arg| arg == flag);
    if has("--self-test") {
        std::process::exit(self_test());
    }
    if has("gallery") {
        window::run(GalleryView::default(), "Loudible previews", (1200.0, 820.0));
    } else if has("--mock") {
        let options = MockOptions {
            signed_in: has("--signed-in"),
            fail_load: has("--fail-load"),
        };
        window::run(AppView::new(options), "Loudible", WINDOW);
    } else {
        window::run_with(
            |wake| composition::make_app_view(wake).expect("start the app"),
            "Loudible",
            WINDOW,
        );
    }
}
