// The playground: pick a view, edit its JSON, watch it redraw.
//
// Everything the module needs arrives from here — the font, the assembled document, the data —
// and everything it reports comes back as the frame, which is what the hover highlighting reads.

import { VIEWS, assemble, options as renderOptions, sidecar } from "./document.js";

const SCENES = "../../loudible-scene/scenes/";
const FONT = "fonts/ui.ttf";

const view = document.getElementById("view");
const scaleField = document.getElementById("scale");
const darkField = document.getElementById("dark");
const documentField = document.getElementById("document");
const dataField = document.getElementById("data");
const canvas = document.getElementById("canvas");
const status = document.getElementById("status");
const error = document.getElementById("error");
const hoverLine = document.getElementById("hover");

const state = { render: {}, hot: null, pressed: null, frame: null };
let scene;

/** The module, debug or release, whichever was built. */
async function load() {
  const tried = [];
  for (const build of ["debug", "release"]) {
    const path = `../target/wasm32-unknown-emscripten/${build}/loudible_scene_wasm.js`;
    try {
      const Module = (await import(path)).default;
      return await Module();
    } catch (failure) {
      tried.push(`${build}: ${failure.message ?? failure}`);
    }
  }
  throw new Error(`no module to load — run \`just build\` first.\n${tried.join("\n")}`);
}

const read = async (path) => {
  const response = await fetch(SCENES + path);
  if (!response.ok) throw new Error(`${path}: ${response.status}`);
  return response.json();
};

function show(message) {
  error.hidden = !message;
  error.textContent = message ?? "";
}

/** Draws whatever is in the two editors. */
function paint() {
  let document_;
  let data;
  try {
    document_ = JSON.parse(documentField.value);
    data = dataField.value.trim() ? JSON.parse(dataField.value) : {};
  } catch (failure) {
    show(`JSON: ${failure.message}`);
    return;
  }

  const scale = Number(scaleField.value);
  const options = {
    ...state.render,
    data,
    scale,
    dark: darkField.checked,
    hot: state.hot,
    pressed: state.pressed,
  };

  try {
    const frame = scene.render(canvas, document_, options);
    state.frame = frame;
    canvas.style.width = `${canvas.width / scale}px`;
    canvas.style.height = `${canvas.height / scale}px`;
    show(null);
    status.textContent =
      `${frame.ms.toFixed(1)} ms · ${frame.hits.length} regions · ` +
      `${frame.hosts.length} views · ${frame.fonts} fonts`;
  } catch (failure) {
    show(failure.message ?? String(failure));
  }
}

let pending;
function repaint() {
  clearTimeout(pending);
  pending = setTimeout(paint, 200);
}

/** The region under a point, in window points: the last one wins, as it is the one on top. */
function regionAt(x, y) {
  let found = null;
  for (const hit of state.frame?.hits ?? []) {
    const [left, top, width, height] = hit.rect;
    if (x >= left && x < left + width && y >= top && y < top + height && hit.action) {
      found = hit;
    }
  }
  return found;
}

function pointer(event) {
  const region = regionAt(event.offsetX, event.offsetY);
  const action = region ? { action: region.action, args: region.args ?? null } : null;
  hoverLine.textContent = region
    ? `${region.action}${region.tooltip ? ` — ${region.tooltip}` : ""} ` +
      `[${region.rect.map((n) => n.toFixed(0)).join(", ")}]`
    : " ";
  const changed = JSON.stringify(action) !== JSON.stringify(state.hot);
  state.hot = action;
  if (state.pressed && !action) state.pressed = null;
  if (changed) paint();
}

async function choose(name) {
  show(null);
  state.hot = null;
  state.pressed = null;
  const [document_, beside] = await Promise.all([assemble(read, name), sidecar(read, name)]);
  state.render = renderOptions(beside.render);
  if (typeof state.render.scale === "number") {
    scaleField.value = String(state.render.scale);
    delete state.render.scale;
  }
  if (typeof state.render.dark === "boolean") {
    darkField.checked = state.render.dark;
    delete state.render.dark;
  }
  documentField.value = JSON.stringify(document_, null, 2);
  dataField.value = JSON.stringify(beside.data, null, 2);
  paint();
}

async function start() {
  for (const name of VIEWS) {
    const option = document.createElement("option");
    option.value = name;
    option.textContent = name;
    view.append(option);
  }

  scene = await load();
  try {
    const font = await fetch(FONT).then((response) => {
      if (!response.ok) throw new Error(`${response.status} — run \`just fonts\``);
      return response.arrayBuffer();
    });
    scene.registerFont(new Uint8Array(font));
  } catch (failure) {
    show(`No font: ${failure.message ?? failure}. Text will not draw.`);
  }

  view.addEventListener("change", () => choose(view.value));
  scaleField.addEventListener("change", paint);
  darkField.addEventListener("change", paint);
  documentField.addEventListener("input", repaint);
  dataField.addEventListener("input", repaint);
  canvas.addEventListener("mousemove", pointer);
  canvas.addEventListener("mouseleave", () => {
    if (state.hot || state.pressed) {
      state.hot = null;
      state.pressed = null;
      hoverLine.textContent = " ";
      paint();
    }
  });
  canvas.addEventListener("mousedown", () => {
    state.pressed = state.hot;
    paint();
  });
  canvas.addEventListener("mouseup", () => {
    state.pressed = null;
    paint();
  });

  await choose(VIEWS[0]);
}

start().catch((failure) => {
  status.textContent = "failed";
  show(failure.message ?? String(failure));
});
