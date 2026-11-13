// Assembles complete scene documents out of the one-thing-per-file layout under
// `loudible-scene/scenes/`.
//
// `render` takes a whole document — schema, tokens, symbols, components, root — and nothing else,
// so this is where the pieces are put together. It mirrors `testing::Builder::standard()` on the
// Rust side, which does the same for the crate's own tests and its PNG example.

export const SCHEMA = "loudible.scene/1";

/** The authored views, as paths under `scenes/`. */
export const VIEWS = [
  "demo/poster",
  "demo/charts",
  "demo/shapes",
  "demo/icons",
  "demo/effects",
  "demo/palette",
  "toolbar",
  "error-toast",
];

/** Declared on every document, because any view may reach for them. */
const COMPONENTS = {
  "account-avatar": "account-avatar.json",
  "book-tile": "book-tile.json",
};

/** A file's top-level `$comment` is for the reader, not for the renderer. */
function strip(value) {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const { $comment, ...rest } = value;
    return rest;
  }
  return value;
}

/**
 * Builds the document for one view. `read(path)` resolves a path under `scenes/` to parsed JSON.
 */
export async function assemble(read, view) {
  const names = Object.keys(COMPONENTS);
  const [tokens, symbols, root, ...bodies] = await Promise.all([
    read("tokens.json"),
    read("symbols.json"),
    read(`${view}.json`),
    ...names.map((name) => read(COMPONENTS[name])),
  ]);
  const components = {};
  names.forEach((name, index) => {
    components[name] = strip(bodies[index]);
  });
  return {
    schema: SCHEMA,
    tokens: strip(tokens),
    symbols: strip(symbols),
    components,
    root: strip(root),
  };
}

/**
 * The data beside a view, split into what the document binds to and how the file says to look at
 * it. `$render` carries the size, scale and appearance each scene was authored for.
 */
export async function sidecar(read, view) {
  let file;
  try {
    file = await read(`${view}.data.json`);
  } catch {
    return { data: {}, render: {} };
  }
  const { $render = {}, $comment, ...data } = file ?? {};
  return { data, render: $render };
}

/** `$render` as render options: `{ size: [w, h], scale, dark }`. */
export function options(render) {
  const given = {};
  if (Array.isArray(render.size)) {
    given.width = render.size[0];
    given.height = render.size[1];
  }
  if (typeof render.scale === "number") given.scale = render.scale;
  if (typeof render.dark === "boolean") given.dark = render.dark;
  return given;
}
