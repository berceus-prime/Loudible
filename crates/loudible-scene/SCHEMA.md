# Scene schema

A UI is a JSON document describing one frame's *design*: a tree of nodes, each with a `type`,
a `props` object and a `children` array. It carries no behaviour — nothing in here says what a
click does, only what the window looks like, including what it looks like while hovered,
pressed, selected or focused.

The schema is derived from the views in `src/views/`, `src/ui.rs` and `src/icons.rs`: every shape,
material, text style and layout idiom those files draw today has a representation here, and
nothing has been added that they don't need. `schema/scene.schema.json` is the machine-checkable
form; `scenes/` holds three real views transcribed into it, and `cargo run --example render` draws
one to a PNG.

## Document

```json
{
  "schema": "loudible.scene/1",
  "tokens":     { "colors": {}, "text": {}, "metrics": {} },
  "symbols":    { "<name>": { "grid": 24, "children": [] } },
  "components": { "<name>": { "params": [], "body": {} } },
  "root":       {}
}
```

| Field | Purpose |
| --- | --- |
| `schema` | Version string. The renderer refuses anything it doesn't know. |
| `tokens` | Named colours, text styles and metrics. Replaces `src/theme.rs` and the `text::BODY`-style constants. |
| `symbols` | Reusable vector drawings on a fixed grid — the 20 SF Symbol stand-ins in `src/icons.rs`. |
| `components` | Parameterised subtrees, so `book_tile` is declared once and instantiated per book. |
| `root` | The window's node. |

## Node

Every node has the same shape, and the same **box properties** are legal on all of them. `type`
only decides how a node's children are laid out, or what the node paints.

```json
{
  "type": "box",
  "id": "cover",
  "when": "selected",
  "repeat": { "for": "book", "in": "$books" },
  "props": {},
  "variants": [{ "when": "pressed", "props": { "fill": "control_pressed" } }],
  "children": []
}
```

### Box properties (any node)

| Prop | Type | Notes |
| --- | --- | --- |
| `size` | `{ "width": Size, "height": Size }` \| number | See **Sizing**. A bare number is a square box, which is how `symbol` and `spinner` are sized. |
| `place` | `{ left, right, top, bottom, center_x, center_y }` | Offsets from the parent's content box, in points. Only meaningful inside `layer`, `viewport` and `box`. Two opposing edges stretch the node. |
| `anchor` | `{ to: Point, align, flip, margin }` | Overlay placement: put the node at a point, keep it `margin` inside the window, and flip it across the anchor rather than let it hang off the bottom. This is the popup menu's clamp-and-flip (`src/views/dialogs.rs:36-42`) and the account popover's horizontal clamp (`src/shell/app_view.rs:287-291`). |
| `slot` | `"leading"` \| `"center"` \| `"trailing"` | Which slot of a `bar` parent this child occupies. |
| `padding` | number \| `[v, h]` \| `{ top, right, bottom, left }` | |
| `margin` | same as `padding` | Outside the box; `stack` adds it to `gap`. |
| `fill` | Color | The node's own background. |
| `color` | Color | Foreground, **inherited**: a descendant `text`, `symbol`, `path`, `line`, `arc` or `ellipse` with no `color` of its own uses the nearest ancestor's. One `variants` entry on a container therefore recolours a whole row — how a hovered menu entry flips its label and check mark to white (`src/views/dialogs.rs:56-68`), and how a tile's dots dim when it has no menu. On a shape it supplies `fill` when neither `fill` nor `stroke` is set. |
| `stroke` | Stroke | `align: "inside"` reproduces the `with_inset((0.5, 0.5))` + reduced-radius idiom every rim in the codebase uses. |
| `radius` | number \| `"capsule"` \| `{ tl, tr, br, bl }` | `"capsule"` is `min(w, h) / 2`, i.e. `ui::capsule`. |
| `radius_rule` | `"fixed"` \| `"control"` | `"control"` is the button rule: `h/2` at 28pt and above, else `h/3`. |
| `shadow` | Shadow \| Shadow[] | |
| `material` | `{ "blur": 24, "tint": Color }` | Backdrop blur then tint — `Ui::material`. |
| `opacity` | number | Isolating layer, as `save_layer_alpha_f` does for a disabled tile. |
| `blend` | `"src-over"` \| `"dst-out"` | `dst-out` punches the node out of its isolated ancestor. |
| `isolate` | bool | Starts a layer so `dst-out` children knock out of this node only. |
| `clip` | bool | Clips children to this node's rounded box. Also clips the hit regions inside it. |
| `hit` | Hit | A named region. See **Hit regions**. |
| `z` | number | Paint order override within a parent; default is document order. |

### Containers

| `type` | Children laid out |
| --- | --- |
| `stack` | One axis. `axis: "y" \| "x"`, `gap`, `align` (cross axis: `start`/`center`/`end`/`stretch`), `justify` (main axis: `start`/`center`/`end`/`space-between`), `wrap`. The cursor-and-`SPACING` idiom in every view. |
| `bar` | Three slots — `leading`, `center`, `trailing` — where `center` is centred on the *container*, then clamped so it can't overlap the other two. This is exactly the toolbar's search field (`src/views/toolbar.rs:95-101`), which is window-centred until the account label or selection group crowds it. |
| `grid` | `min_cell`, `gap`, `row_gap`, `padding`. Column count is `floor((inner + gap) / (min_cell + gap))`, cell side is the remainder divided evenly — the library grid's formula (`src/views/library.rs:50-53`). |
| `layer` | Children overlap, each positioned by `place`. Badges on a cover, the toast's close button, the scroller thumb, a modal backdrop. |
| `scroll` | `axis`, `offset` (bindable), `scrollbar: { width, inset, min_length, opacity }`. Content height is reported back so the host can clamp. |
| `viewport` | A fixed logical size with its own origin, clipped. The gallery's preview pane and the render tests. |
| `box` | No layout of its own: a decorated box whose children are placed like `layer`. The generic container. |

### Paint leaves

| `type` | Props | Replaces |
| --- | --- | --- |
| `text` | `value`, `style`, `color`, `align`, `wrap`, `max_lines`, `truncate: "ellipsis"`, `limit` | `Ui::line`, `line_limited`, `line_centered`, `paragraph`. `wrap: false` is a single line; `max_lines` with `wrap: true` is a paragraph. |
| `field` | `binding`, `placeholder`, `placeholder_color`, `style`, `caret_blink_ms` | `TextField::draw` — the Parley-backed editable run, selection highlight and blinking caret. Opaque: the renderer owns it. |
| `symbol` | `name`, `color` (size comes from the box) | `Ui::icon` / `icons::draw`. |
| `image` | `source`, `fit: "cover" \| "contain" \| "fill"`, `sampling` | The cover blit, including the centre-crop. A `children` fallback paints while `source` is absent. |
| `ellipse` | `fill`, `stroke` | `draw_circle` in a square box, `draw_oval` otherwise. |
| `arc` | `start_deg`, `sweep_deg` \| `progress`, `stroke`, `track` | The progress ring, with its background circle as `track`. |
| `path` | `grid`, `commands` (`move`/`line`/`cubic`/`close`) or `points`, `fill`, `stroke` | Icon geometry and the popover arrow. |
| `line` | `from`, `to`, `stroke` | Spinner spokes. A separator is a 1pt-tall `box` with a `fill`. |
| `spinner` | `color`, `spokes`, `period_ms` | `Ui::spinner`. Animated, so it stays a primitive. |
| `spacer` | `size`, `grow` | Flexible gap inside a `stack`. |
| `host` | `name` | Reserves a rect for a native view and reports it back — the sign-in webview (`src/views/login_sheet.rs:31-43`). Paints nothing. |

## Sizing

Each of `size.width` / `size.height` is one of:

| Value | Meaning |
| --- | --- |
| `36` or `{ "fixed": 36 }` | Points. |
| `"hug"` | Intrinsic: as large as the children need. Removes every `ui.measure(...)`-then-place round trip in the current code. |
| `"fill"` | All the space the parent offers. |
| `{ "fill": { "min": 120, "max": 452 } }` | Fill, clamped. The search field. |
| `{ "fraction": 0.5 }` | Of the parent's content box. |
| `{ "ratio": 1.0 }` | Of this node's own other axis. Square covers. |
| `{ "text": "Download", "style": "body", "plus": 24 }` | Measured text width plus padding — the many `ui.text.width(title, style) + N` button widths. |

Any value also accepts `min` / `max` siblings.

## Values

### Color

```json
"label"                                   // token
{ "token": "label", "opacity": 0.5 }      // theme::faded
{ "hex": "#99000000" }                    // literal ARGB, for scrims and shadows
{ "mix": ["$cover", "#FFFFFFFF", 0.35] }  // book_tile::lightened
{ "gradient": { "kind": "linear", "from": "top", "to": "bottom",
                "stops": ["#FFEAEAEC", "#FFF7F7F9"] } }
```

Tokens resolve per appearance: `tokens.colors.label = { "light": "#D9000000", "dark": "#D9FFFFFF" }`.

### Stroke

```json
{ "color": "glass_rim", "width": 0.75, "align": "center", "cap": "round", "join": "round" }
```

`align` is `"center"` (default), `"inside"` or `"outside"`.

### Shadow

```json
{ "color": "#26000000", "blur": 3, "dy": 1, "dx": 0, "knockout": true }
```

`knockout` defaults to `true`, matching `Ui::shadow`, which clips the shape out from under its own
shadow so translucent cards don't self-darken.

### Text style

```json
"body"
{ "style": "callout", "weight": "medium", "rounded": true, "tabular": true }
{ "size": 11, "line": 14, "weight": "regular" }
```

Named styles come from `tokens.text` (`body`, `headline`, `callout`, `caption`, `title3`).

## Bindings and repetition

Content comes from a data scope the host supplies; the tree describes shape only.

| Form | Meaning |
| --- | --- |
| `{ "$": "book.title" }` | Read a path. |
| `{ "$": "account.name", "else": "Account" }` | With a fallback. |
| `{ "format": "Audible {marketplace.name}" }` | Interpolation. |
| `{ "count": "$selection.downloads", "zero": "…", "one": "…", "many": "{n} books" }` | The plural helpers in `toolbar::help`. |
| `"repeat": { "for": "book", "in": "$books", "index": "i" }` | Instantiates the node once per element. |

## State and variants

`state` is a flat bag of flags and numbers the host sets each frame: `hovered`, `pressed`,
`selected`, `focused`, `enabled`, `window_active`, `dark`, `platform`, plus anything the data
implies. Appearance may depend on it; nothing else may.

- `when` on a node includes or excludes its subtree:
  `"when": "selected"`, `{ "not": "enabled" }`, `{ "all": [...] }`, `{ "any": [...] }`,
  `{ "eq": ["platform", "macos"] }`.
- `variants` is an ordered list of `{ when, props }` overrides merged onto the node's own props.

A node carrying `hit` opens an **interaction scope**: `hovered` and `pressed` inside its subtree
refer to that region, which is how a menu row tints its background *and* flips its label to white
from one condition (`src/views/dialogs.rs:56-68`).

## Hit regions

The one concession to interactivity, and deliberately inert: a `hit` names a region and an
opaque action id. It contains no logic and the renderer never interprets it — it collects regions
in paint order and hands the list back, exactly as `Ui::hit` does today, so later regions sit on
top of earlier ones.

```json
"hit": {
  "action": "toggle-book",
  "args": { "asin": { "$": "book.asin" } },
  "secondary": "open-book-menu",
  "tooltip": { "$": "book.tooltip" },
  "text_cursor": false,
  "blocks": false
}
```

`"blocks": true` is `Action::Block`: swallow the click, for modal backdrops and disabled controls.

## Symbols

An icon is a subtree on a fixed grid, scaled at draw time — `src/icons.rs`'s 24-unit grid, verbatim.
Knockouts work because `isolate` plus `blend: "dst-out"` is available to any node.

```json
"error.fill": {
  "grid": 24,
  "props": { "isolate": true },
  "children": [
    { "type": "ellipse", "props": { "place": { "center_x": 0, "center_y": 0 },
                                    "size": { "width": 22, "height": 22 }, "fill": "$color" } },
    { "type": "box", "props": { "place": { "left": 10.8, "top": 5.6 },
                                "size": { "width": 2.4, "height": 8.6 },
                                "radius": 1.2, "fill": { "hex": "#FFFFFFFF" },
                                "blend": "dst-out" } },
    { "type": "ellipse", "props": { "place": { "center_x": 0, "top": 15.9 },
                                    "size": { "width": 3, "height": 3 },
                                    "fill": { "hex": "#FFFFFFFF" }, "blend": "dst-out" } }
  ]
}
```

`$color` and `$size` are the parameters every symbol receives from the `symbol` node.

## Components

```json
"components": {
  "book-tile": {
    "params": [
      { "name": "book", "type": "object" },
      { "name": "state", "type": "object" },
      { "name": "selectable", "type": "bool", "default": true }
    ],
    "body": { "type": "stack", "props": { "axis": "y" }, "children": [] }
  }
}
```

Instantiated as a node: `{ "type": "book-tile", "props": { "book": { "$": "book" } } }`. A
component's `body` sees only its params, which is the current rule that views never touch a model.

## Coverage

Every drawing path in the app, and the node that expresses it:

| Existing code | Expressed as |
| --- | --- |
| `ui::fill` / `outline` / `capsule` / `rounded` | `fill`, `stroke`, `radius` box props |
| `Ui::shadow` (blur + difference clip) | `shadow` |
| `Ui::material` (backdrop blur + tint) | `material` |
| `Ui::vertical_gradient` | `fill: { gradient }` |
| `Ui::separator` | 1pt `box` with `fill` |
| `Ui::glass_capsule` | `box`, `radius: "capsule"`, `fill` + `stroke` |
| `Ui::button` (3 styles, pressed, disabled, inactive window) | `box` + `text`, `radius_rule: "control"`, `variants` on `pressed`/`enabled`/`window_active` |
| `Ui::traffic_lights` | `layer` of three `ellipse` pairs, `variants` on `window_active` |
| `Ui::spinner` | `spinner` |
| `Ui::icon` | `symbol` |
| `Ui::line*` / `paragraph` | `text` |
| `Ui::clip` | `clip: true` |
| `Ui::measure` | `size: "hug"` — no longer needed |
| `toolbar` | `bar` with `leading`/`center`/`trailing` slots |
| `library_grid` | `scroll` > `grid` of `book-tile`, plus a `layer` scroller thumb |
| `messages_bar` | `stack` of icon/paragraph rows with a tinted `box` behind |
| `book_tile` | `stack` > square `box` (`clip`, `image` + placeholder) + badge `layer` + text `stack` |
| `error_toast` | `box` (`material`, `shadow`, `hug` height) > `layer` of icon, text `stack`, close button |
| `login_sheet` | `stack` with a `host` for the webview and a `hug` tail |
| `account_popover` / `sign_in_panel` | `stack`, shared via a component |
| `account_avatar` | `ellipse` with `fill: { gradient }` + `text` or `symbol` |
| `popup_menu` | `anchor` with `flip`, `stack` of rows, `variants` on `hovered` |
| `popover_frame` | `box` + `path` arrow |
| `modal_backdrop` / `sheet_card` / `confirm_alert` | `layer` + `box` + `stack` |
| `TextField::draw` | `field` |
| `GalleryView::render` | `stack` > list + `viewport` |

## Deliberately out of scope

- **Behaviour.** No actions, handlers, transitions or conditions beyond appearance-per-state.
- **Animation**, beyond the two primitives that cannot be static (`spinner`, and `field`'s caret).
  Hover and press are state changes the host re-renders, as they are today.
- **Text layout internals.** Wrapping stays AppKit's greedy-with-orphan-pullback rule in the
  renderer, not something the JSON can tune.
