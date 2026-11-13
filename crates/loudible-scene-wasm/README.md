# loudible-scene-wasm

`loudible-scene` in a browser. One export takes a scene document as a JavaScript object and paints
it onto a `<canvas>`; two more hand over the fonts and pictures a browser cannot find by itself.

```js
import Module from "./loudible_scene_wasm.js";

const scene = await Module();
scene.registerFont(new Uint8Array(fontBytes));          // wasm has no system fonts
const frame = scene.render(canvas, document, { dark: true, scale: 2, data });
// frame: { hits, hosts, content, nextFrameMs, ms, fonts, images }
```

`document` is a **complete** scene document — `schema`, `tokens`, `symbols`, `components`, `root` —
exactly as `../loudible-scene/SCHEMA.md` describes it. Nothing is merged in for you; assembling one
out of the one-thing-per-file layout under `../loudible-scene/scenes/` is the caller's job, and
`web/document.js` does it for the playground.

## Setting up

```sh
brew install emscripten                       # 7.0.0; rust-skia wants 5.0+, wasm-bindgen 6.0.10+
cargo install wasm-bindgen-cli --version 0.2.130   # must match Cargo.lock exactly
```

`rust-toolchain.toml` pins Rust 1.92 and `rustup` installs it on first use. The target comes with
it, and `.cargo/config.toml` selects it, so there is nothing to pass on the command line.

## Doing things

```sh
just fonts     # copy the system UI face to web/fonts/ui.ttf (gitignored)
just build     # cargo build; emscripten runs wasm-bindgen itself at link time
just smoke     # render every authored view under node, with a stand-in canvas
just test      # the unit tests, on the host
just serve     # http://localhost:8000/loudible-scene-wasm/web/
```

`just build` emits `target/wasm32-unknown-emscripten/debug/loudible_scene_wasm.{js,wasm}`. The
debug module is around 30MB of wasm; `just build-release` is the one to serve over a network.

## How it is put together

- **`src/main.rs`** — the three exports. Everything crosses the boundary as JSON, through
  `JSON.stringify`, so the document the page passes is the document the schema describes and the
  errors are the ones `SceneError` already reports, with their JSON pointers.
- **`src/fonts.rs`** — the registered faces, and the `TextEngine` built over them. A swap point:
  another text system replaces this file and nothing else.
- **`src/images.rs`** — decoded pictures by name, for the `Images` port. The other swap point, kept
  separate from the fonts on purpose. PNG and JPEG only: this Skia carries no WebP decoder.
- **`src/fields.rs`** — a no-op `Fields`. Editable text is out of scope, so a `field` node reserves
  its box and draws nothing.
- **`src/options.rs`, `src/frame.rs`** — the options object in, the painted frame out.
- **`web/`** — the playground: a view picker, two JSON editors that repaint as you type, light/dark
  and scale toggles, and hover highlighting driven by the hit regions the frame reports.
- **`tests/smoke.mjs`** — `wasm-bindgen-test` on this target only checks that bindings compile, so
  the built module is exercised under node instead, against every authored view.

## Three things to know

**Rust is pinned to 1.92 on purpose.** 1.93 changed this target's unwinding ABI from JS exceptions
to wasm exceptions and now passes `-fwasm-exceptions` to the linker. The prebuilt Skia archives are
compiled the other way — libpng, libjpeg and FreeType all use `setjmp`, which emscripten lowers to
its own trampolines — and `emcc` refuses the combination outright:
`SUPPORT_LONGJMP=emscripten is not compatible with -fwasm-exceptions`. Until rust-skia publishes
wasm-exception binaries, 1.92 is the last toolchain that links against the ones on crates.io.
Building Skia from source with `-sSUPPORT_LONGJMP=wasm` is the other way out, and a much longer one.

**`ImageData` is built by hand.** In emscripten mode wasm-bindgen's
`ImageData::new_with_u8_clamped_array_and_sh` hands the constructor a view of emscripten's
`HEAPU8`, which is a `Uint8Array` rather than a `Uint8ClampedArray`; the browser then reads the
call as `new ImageData(sw, sh, settings)` and refuses it with *"The provided value is not of type
'ImageDataSettings'"*. `render` builds the clamped array through `js_sys` instead. Node stubs do
not notice this, which is why the playground is also checked in a real browser.

**The painting is on the CPU.** The scene is rasterised into a buffer and blitted with
`putImageData`, which is simple, exact and costs a full-frame copy. TODO: a Ganesh/WebGL backend,
the way rust-skia's own `wasm-example` wires one up — the `gl` feature, `MAX_WEBGL_VERSION=2`,
`EXPORTED_RUNTIME_METHODS=GL` and the default framebuffer wrapped as a surface — for scenes that
animate rather than redraw on edit.
