# Demos

Six documents that exercise the renderer rather than reproduce a view. Each has a sibling
`*.data.json` whose `$render` block sets the canvas, so they need no flags:

```sh
cargo run -p loudible-scene --features testing --example render -- demo/palette
cargo run -p loudible-scene --features testing --example render -- demo/poster /tmp/poster.png
cargo run -p loudible-scene --features testing --example render -- demo/palette --dark
```

| Scene | Shows |
| --- | --- |
| `palette` | every colour token, with each swatch's fill read from the token's own name in the data |
| `icons` | all 20 symbols, tinted per tile, with their knockouts intact |
| `shapes` | corner rules, ellipses, arcs, polylines, cubics, lines, stroke alignment, gradients and mixes |
| `effects` | shadows and their knockout, a material blurring what sits behind it, layer opacity, `dst-out` with and without `isolate` |
| `charts` | bar heights, ring sweeps and legend rows all driven by `repeat` over bound numbers |
| `poster` | a composition: gradients, a vinyl built from arcs, a `repeat`-driven waveform, a bound progress bar and hit regions on the transport |

Two limits shaped these, both worth knowing before writing more: `place` takes points rather than
bindings, so the progress thumb rides inside the filled bar instead of being positioned from data;
and a path's coordinates are its own, so a sparkline is authored at the size it draws at.
