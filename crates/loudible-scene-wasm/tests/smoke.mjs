// Renders every authored view through the wasm module under node, with a stand-in canvas.
//
// `wasm-bindgen-test` on this target only checks that bindings compile, so this is where the
// built module is actually exercised: the exports, the fonts, the paint and the frame that comes
// back. Run it with `just smoke`, after `just build` and `just fonts`.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { VIEWS, assemble, options, sidecar } from "../web/document.js";

const here = dirname(fileURLToPath(import.meta.url));
const scenes = join(here, "..", "..", "loudible-scene", "scenes");
const read = async (path) => JSON.parse(readFileSync(join(scenes, path), "utf8"));

// The browser objects web-sys reaches for, with only the parts `render` uses.
globalThis.CanvasRenderingContext2D = class CanvasRenderingContext2D {
  putImageData(image) {
    this.image = image;
  }
};
globalThis.HTMLCanvasElement = class HTMLCanvasElement {
  constructor() {
    this.width = 0;
    this.height = 0;
    this.context = new globalThis.CanvasRenderingContext2D();
  }
  getContext(kind) {
    return kind === "2d" ? this.context : null;
  }
};
globalThis.ImageData = class ImageData {
  constructor(data, width, height) {
    Object.assign(this, { data, width, height });
  }
};

let failures = 0;
function check(condition, message) {
  if (!condition) {
    failures += 1;
    console.error(`  FAILED: ${message}`);
  }
}

const Module = (await import("../target/wasm32-unknown-emscripten/debug/loudible_scene_wasm.js"))
  .default;
const scene = await Module();

const font = readFileSync(join(here, "..", "web", "fonts", "ui.ttf"));
const family = scene.registerFont(new Uint8Array(font));
console.log(`font: ${family}`);
check(scene.fontFamilies().includes(family), "the family is listed after registering");

for (const view of VIEWS) {
  const document = await assemble(read, view);
  const { data, render } = await sidecar(read, view);
  const canvas = new globalThis.HTMLCanvasElement();
  const frame = scene.render(canvas, document, { data, ...options(render) });

  const image = canvas.context.image;
  let ink = 0;
  for (let i = 0; i < image.data.length; i += 4) {
    if (image.data[i] !== 255 || image.data[i + 1] !== 255 || image.data[i + 2] !== 255) ink += 1;
  }
  console.log(
    `${view.padEnd(14)} ${String(image.width).padStart(5)}x${String(image.height).padEnd(5)}` +
      ` ${String(frame.hits.length).padStart(3)} regions  ${frame.ms.toFixed(1)}ms  ${ink} inked`,
  );
  check(image.width === canvas.width, `${view}: the image is the size of the canvas`);
  check(ink > 1000, `${view}: drew something`);
}

// A document that is not a document should come back as an error with its path, not a panic.
try {
  scene.render(new globalThis.HTMLCanvasElement(), { schema: "loudible.scene/1" }, {});
  check(false, "a document with no root is refused");
} catch (error) {
  console.log(`refused as expected: ${error.message ?? error}`);
}

console.log(failures === 0 ? "\nall good" : `\n${failures} failed`);
process.exit(failures === 0 ? 0 : 1);
