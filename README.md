# Loudible

Loudible's window drawn directly with [Skia](https://github.com/rust-skia/rust-skia), over the Rust
engine in `crates/`. `--mock` runs the same window over synthetic data, without contacting
Audible.

```sh
cargo run                          # the real app: signs in to Audible and uses your files
cargo run -- --mock                # the mock app: sign in, browse, select, download, export
cargo run -- --mock --signed-in    # start with the library loaded
cargo run -- --mock --fail-load    # loading the library fails
cargo run -- gallery               # browse all 58 preview states
cargo test                         # model tests, UI flow tests, headless render
cargo test --workspace             # also the crates under crates/
cargo test --release --test render_perf -- --ignored --nocapture   # frame times, large library
cargo run --bin webview_check      # the real sign-in browser against a loopback server
cargo run --bin webview_check -- --amazon   # Amazon's real page, by hand; never exchanges the code
```

The port of the Swift app is growing here. The workspace's crates mirror the SwiftPM targets, so
Cargo enforces the same layering. `MockApp` stays for the previews and `--mock`.

Ported so far: `loudible-core`, `loudible-adapters`, `loudible-ffmpeg` and
`loudible-presentation`, with their tests. Shared test doubles are
in `loudible-test-support`.

FFmpeg and packaging, from this directory:

```sh
Scripts/build-ffmpeg.sh        # LGPL FFmpeg into Vendor/ (once; the other scripts run it too)
Scripts/build-app.sh           # macOS: build/Loudible.app, ad-hoc signed
Scripts/verify-app.sh          # architecture, signatures, bundled FFmpeg, notices, self-test
Scripts/make-dmg.sh            # macOS: build/Loudible.dmg from the app, notarized with credentials
Scripts/package.sh             # Linux or Windows (MSYS2): a portable folder in build/
```

Pushing a tag like `v1.2.0` runs `.github/workflows/release.yml`, which publishes a GitHub release
with the DMG. The signing and notarization secrets and variables it needs are listed at its top.

Without `Vendor/ffmpeg` (or `LOUDIBLE_FFMPEG_DIR`), the build fails with setup instructions. To upgrade FFmpeg, follow the steps at the top of `Scripts/ffmpeg-version.sh`.

## Layout

| Path | Contents |
| --- | --- |
| `src/model/` | Types the views draw, the `AppModel` trait behind the window, `LiveApp` over the real engine, and `MockApp` with synthetic data, where time only moves through `tick`. |
| `src/ui.rs` | Immediate-mode drawing: shapes, materials, buttons, and hit areas naming an `Action`. |
| `src/text.rs` | Skia paragraph layout, AppKit-style wrapping, truncation and San Francisco's tracking. |
| `src/icons.rs` | Vector versions of the SF Symbols the Swift app uses. |
| `src/stage.rs` | Hover, press, tooltips and input routing, independent of any window. |
| `src/views/` | Content views: values in, painting and hit areas out. They never touch a model. |
| `src/shell/` | `AppView` maps an `AppModel` onto the views; `covers.rs` fetches and caches cover images; `GalleryView` lists the previews; `window.rs` hosts a screen with winit and softbuffer; `webview.rs` lays the native sign-in browser over it. |
| `src/composition.rs` | Picks the real adapters and folders. |
| `src/previews.rs` | Every state, for the gallery. |
| `tests/ui_flow.rs` | Paints real frames and clicks through hit areas of the mock. |
| `tests/live_app.rs` | The same, over the real engine with Audible, downloads and FFmpeg replaced. |

## Notes

* Rendering is on the CPU; a window redraws only when something changes or animates.
* The window draws its own traffic lights, because AppKit's standard buttons can't sit where the
  Swift app's toolbar puts them. Previews show them grey, like the Swift PNGs of an inactive window.
* The sign-in browser is a native view above Skia, so it hides while a toast or spinner must show
  there. macOS and Windows embed it; Linux has only the browser-and-paste fallback for now, shown open from the start.
* Only the system font is used on macOS. Other platforms fall back to a common sans-serif and have
  not been run.
