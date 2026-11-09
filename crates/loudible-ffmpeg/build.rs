//! Compiles the C bridge against the FFmpeg built by `Scripts/build-ffmpeg.sh`. Platforms without
//! that build yet compile this crate empty, so the rest of the workspace still builds there.

use std::{env, path::PathBuf};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_ffmpeg)");
    println!("cargo::rerun-if-env-changed=LOUDIBLE_FFMPEG_DIR");

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("set by Cargo"));
    let workspace = manifest.join("../..");
    let ffmpeg = env::var_os("LOUDIBLE_FFMPEG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("Vendor/ffmpeg"));
    let bridge = manifest.join("bridge");
    println!("cargo::rerun-if-changed={}", bridge.display());
    println!("cargo::rerun-if-changed={}", ffmpeg.join("lib").display());

    if !ffmpeg.join("include/libavformat/avformat.h").is_file() {
        println!(
            "cargo::warning=no FFmpeg at {}; building without the bridge",
            ffmpeg.display()
        );
        return;
    }

    cc::Build::new()
        .file(bridge.join("ffmpeg_bridge.c"))
        .include(&bridge)
        .include(ffmpeg.join("include"))
        .warnings(true)
        .extra_warnings(true)
        .warnings_into_errors(true)
        .compile("ffmpeg_bridge");

    // Dynamic linking only: FFmpeg stays LGPL-2.1-or-later.
    println!(
        "cargo::rustc-link-search=native={}",
        ffmpeg.join("lib").display()
    );
    for library in ["avformat", "avcodec", "avutil"] {
        println!("cargo::rustc-link-lib=dylib={library}");
    }
    println!("cargo::rustc-cfg=has_ffmpeg");
}
