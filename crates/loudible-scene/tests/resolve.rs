//! Expanding, pruning, merging and binding: everything that happens between the document and
//! the tree layout walks.

use loudible_scene::document::Weight;
use loudible_scene::resolved::{Content, Dim, Resolved};
use loudible_scene::testing::{Builder, StubText, renderer};
use loudible_scene::tokens::Fill;
use loudible_scene::{ActionRef, Appearance, Inputs, Platform, Scene};
use serde_json::{Value, json};
use skia_safe::{Color, Size};

fn scene(root: Value) -> Scene {
    Builder::standard().root(root).parse()
}

fn resolve_with(root: Value, data: &Value, tune: impl Fn(Inputs) -> Inputs) -> Resolved {
    let scene = scene(root);
    let inputs = tune(Inputs::new(Size::new(1000.0, 700.0), data));
    scene
        .resolve(&renderer(), &inputs)
        .expect("the root was not pruned")
}

fn resolve(root: Value, data: &Value) -> Resolved {
    resolve_with(root, data, |inputs| inputs)
}

fn text_of(node: &Resolved) -> &str {
    match &node.content {
        Content::Text(text) => &text.value,
        other => panic!("expected text, found {other:?}"),
    }
}

fn texts(node: &Resolved) -> Vec<&str> {
    node.children().iter().map(text_of).collect()
}

#[test]
fn repeat_expands_once_per_element() {
    let data = json!({ "books": [{ "title": "One" }, { "title": "Two" }, { "title": "Three" }] });
    let root = resolve(
        json!({
            "type": "stack",
            "children": [{
                "type": "text",
                "repeat": { "for": "book", "in": "$books" },
                "props": { "value": { "$": "book.title" } }
            }]
        }),
        &data,
    );
    assert_eq!(texts(&root), ["One", "Two", "Three"]);
}

#[test]
fn repeat_can_name_its_index() {
    let data = json!({ "rows": ["a", "b"] });
    let root = resolve(
        json!({
            "type": "stack",
            "children": [{
                "type": "text",
                "repeat": { "for": "row", "in": "$rows", "index": "i" },
                "props": { "value": { "format": "{i}:{row}" } }
            }]
        }),
        &data,
    );
    assert_eq!(texts(&root), ["0:a", "1:b"]);
}

#[test]
fn repeat_over_nothing_draws_nothing() {
    let root = resolve(
        json!({
            "type": "stack",
            "children": [{
                "type": "text",
                "repeat": { "for": "book", "in": "$books" },
                "props": { "value": "x" }
            }]
        }),
        &json!({}),
    );
    assert!(root.children().is_empty());
}

#[test]
fn when_prunes_a_subtree() {
    let data = json!({ "selected": true, "busy": false });
    let root = resolve(
        json!({
            "type": "stack",
            "children": [
                { "type": "text", "when": "selected", "props": { "value": "kept" } },
                { "type": "text", "when": "busy", "props": { "value": "gone" } },
                { "type": "text", "when": { "not": "busy" }, "props": { "value": "also kept" } }
            ]
        }),
        &data,
    );
    assert_eq!(texts(&root), ["kept", "also kept"]);
}

#[test]
fn conditions_cover_the_whole_little_language() {
    let data = json!({
        "count": 3,
        "name": "Ada",
        "empty_list": [],
        "kind": "running"
    });
    let cases: [(Value, bool); 7] = [
        (json!({ "eq": ["kind", "running"] }), true),
        (json!({ "eq": ["kind", "queued"] }), false),
        (json!({ "gt": ["count", 2] }), true),
        (json!({ "gt": ["count", 3] }), false),
        (json!({ "empty": "empty_list" }), true),
        (json!({ "all": ["name", { "gt": ["count", 1] }] }), true),
        (json!({ "any": ["missing", { "empty": "missing" }] }), true),
    ];
    for (condition, expected) in cases {
        let root = resolve(
            json!({
                "type": "stack",
                "children": [{ "type": "text", "when": condition, "props": { "value": "y" } }]
            }),
            &data,
        );
        assert_eq!(
            !root.children().is_empty(),
            expected,
            "{condition} should be {expected}"
        );
    }
}

#[test]
fn variants_apply_in_order_and_the_last_one_wins() {
    let data = json!({ "a": true, "b": true });
    let root = resolve(
        json!({
            "type": "box",
            "props": { "fill": "window" },
            "variants": [
                { "when": "a", "props": { "fill": "red" } },
                { "when": "b", "props": { "fill": "accent" } },
                { "when": "missing", "props": { "fill": "orange" } }
            ]
        }),
        &data,
    );
    assert_eq!(root.style.fill, Some(Fill::Solid(Color::new(0xFF3078F6))));
}

#[test]
fn a_variant_can_turn_a_region_into_a_blocker() {
    let data = json!({ "enabled": false });
    let root = resolve(
        json!({
            "type": "box",
            "props": { "hit": { "action": "refresh" } },
            "variants": [
                { "when": { "not": "enabled" }, "props": { "hit": { "blocks": true } } }
            ]
        }),
        &data,
    );
    let hit = root.style.hit.expect("a region");
    assert!(hit.blocks);
    assert_eq!(hit.action, "");
}

#[test]
fn hovering_a_region_styles_its_whole_subtree() {
    let root = resolve_with(
        json!({
            "type": "box",
            "props": { "hit": { "action": "open-book-menu" }, "color": "label" },
            "variants": [{ "when": "hovered", "props": { "fill": "accent", "color": "window" } }],
            "children": [{ "type": "text", "props": { "value": "Download" } }]
        }),
        &json!({}),
        |inputs| inputs.hot(ActionRef::new("open-book-menu")),
    );
    assert_eq!(root.style.fill, Some(Fill::Solid(Color::new(0xFF3078F6))));
    // The label inherits the variant's colour without naming it.
    assert_eq!(root.children()[0].style.color, Color::WHITE);
}

#[test]
fn pressing_needs_the_pointer_to_still_be_over_it() {
    let root = json!({
        "type": "box",
        "props": { "hit": { "action": "copy-toast" } },
        "variants": [{ "when": "pressed", "props": { "fill": "control_pressed" } }]
    });
    let elsewhere = resolve_with(root.clone(), &json!({}), |inputs| {
        inputs.pressed(ActionRef::new("copy-toast"))
    });
    assert_eq!(elsewhere.style.fill, None, "pressed but not hovered");

    let under = resolve_with(root, &json!({}), |inputs| {
        inputs
            .hot(ActionRef::new("copy-toast"))
            .pressed(ActionRef::new("copy-toast"))
    });
    assert!(under.style.fill.is_some(), "hovered and pressed");
}

#[test]
fn a_region_carries_its_arguments() {
    let data = json!({ "book": { "asin": "B0001" } });
    let root = resolve(
        json!({
            "type": "box",
            "props": {
                "hit": {
                    "action": "toggle-book",
                    "args": { "asin": { "$": "book.asin" } },
                    "secondary": "open-book-menu",
                    "tooltip": { "format": "{book.asin} by Ada" }
                }
            }
        }),
        &data,
    );
    let hit = root.style.hit.expect("a region");
    assert_eq!(hit.action, "toggle-book");
    assert_eq!(hit.args, json!({ "asin": "B0001" }));
    assert_eq!(hit.secondary.as_deref(), Some("open-book-menu"));
    assert_eq!(hit.tooltip.as_deref(), Some("B0001 by Ada"));
}

#[test]
fn the_environment_is_readable_like_any_other_data() {
    let root = json!({
        "type": "stack",
        "children": [
            {
                "type": "text",
                "when": { "eq": ["platform", "macos"] },
                "props": { "value": "lights" }
            },
            {
                "type": "text",
                "when": { "not": "window_active" },
                "props": { "value": "grey" }
            }
        ]
    });
    let mac = resolve_with(root.clone(), &json!({}), |inputs| {
        inputs.platform(Platform::MacOs).window_active(false)
    });
    assert_eq!(texts(&mac), ["lights", "grey"]);

    let linux = resolve_with(root, &json!({}), |inputs| {
        inputs.platform(Platform::Linux)
    });
    assert_eq!(texts(&linux), Vec::<&str>::new());
}

#[test]
fn strings_interpolate_and_count() {
    let data = json!({ "store": { "name": "US" }, "downloads": 0, "exports": 1, "both": 7 });
    let root = resolve(
        json!({
            "type": "stack",
            "children": [
                { "type": "text", "props": { "value": { "format": "Audible {store.name}" } } },
                { "type": "text", "props": { "value": {
                    "count": "downloads", "zero": "None can be downloaded",
                    "one": "Download 1 book", "many": "Download {n} books" } } },
                { "type": "text", "props": { "value": {
                    "count": "exports", "zero": "z", "one": "Export 1 book", "many": "m" } } },
                { "type": "text", "props": { "value": {
                    "count": "both", "zero": "z", "one": "o", "many": "Export {n} books" } } },
                { "type": "text", "props": { "value": { "$": "missing", "else": "Account" } } }
            ]
        }),
        &data,
    );
    assert_eq!(
        texts(&root),
        [
            "Audible US",
            "None can be downloaded",
            "Export 1 book",
            "Export 7 books",
            "Account"
        ]
    );
}

#[test]
fn colour_is_inherited_until_something_sets_its_own() {
    let root = resolve(
        json!({
            "type": "stack",
            "props": { "color": "secondary" },
            "children": [
                { "type": "text", "props": { "value": "inherited" } },
                { "type": "text", "props": { "value": "own", "color": "red" } },
                {
                    "type": "stack",
                    "children": [{ "type": "text", "props": { "value": "deep" } }]
                }
            ]
        }),
        &json!({}),
    );
    let children = root.children();
    assert_eq!(children[0].style.color, Color::new(0x80000000));
    assert_eq!(children[1].style.color, Color::new(0xFFFF383C));
    assert_eq!(children[2].children()[0].style.color, Color::new(0x80000000));
}

#[test]
fn colours_fade_mix_and_come_from_data() {
    let data = json!({ "cover": "#FF6E7CF7" });
    let root = resolve(
        json!({
            "type": "stack",
            "children": [
                { "type": "box", "props": { "fill": { "token": "red", "opacity": 0.5 } } },
                { "type": "box", "props": { "fill": { "$": "cover" } } },
                { "type": "box", "props": {
                    "fill": { "mix": [{ "$": "cover" }, "#FFFFFFFF", 0.5] } } },
                { "type": "box", "props": {
                    "fill": { "gradient": { "stops": ["#FFEAEAEC", "#FFF7F7F9"] } } } }
            ]
        }),
        &data,
    );
    let fills: Vec<_> = root.children().iter().map(|c| c.style.fill.clone()).collect();
    assert_eq!(fills[0], Some(Fill::Solid(Color::new(0x7FFF383C))));
    assert_eq!(fills[1], Some(Fill::Solid(Color::new(0xFF6E7CF7))));
    // Halfway to white on each channel, as the synthetic covers' stripe is.
    assert_eq!(fills[2], Some(Fill::Solid(Color::new(0xFFB6BDFB))));
    assert!(matches!(fills[3], Some(Fill::Gradient { .. })));
}

#[test]
fn appearance_picks_the_other_side_of_every_token() {
    let root = json!({ "type": "box", "props": { "fill": "window" } });
    let light = resolve_with(root.clone(), &json!({}), |inputs| inputs);
    let dark = resolve_with(root, &json!({}), |inputs| {
        inputs.appearance(Appearance::Dark)
    });
    assert_eq!(light.style.fill, Some(Fill::Solid(Color::WHITE)));
    assert_eq!(dark.style.fill, Some(Fill::Solid(Color::new(0xFF1E1E1E))));
}

#[test]
fn text_styles_come_from_tokens_and_take_overrides() {
    let root = resolve(
        json!({
            "type": "stack",
            "children": [
                { "type": "text", "props": { "value": "a", "style": "headline" } },
                { "type": "text", "props": { "value": "b", "style": {
                    "style": "callout", "weight": "medium" } } },
                { "type": "text", "props": { "value": "c", "style": {
                    "size": 11, "line": 14 } } }
            ]
        }),
        &json!({}),
    );
    let styles: Vec<_> = root
        .children()
        .iter()
        .map(|child| match &child.content {
            Content::Text(text) => text.style,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!((styles[0].size, styles[0].weight), (13.0, Weight::Bold));
    assert_eq!(
        (styles[1].size, styles[1].line, styles[1].weight),
        (12.0, 15.0, Weight::Medium)
    );
    assert_eq!((styles[2].size, styles[2].line), (11.0, 14.0));
}

#[test]
fn a_box_can_size_itself_to_its_label() {
    let title = "Open in Browser";
    let root = resolve(
        json!({
            "type": "box",
            "props": { "size": { "width": { "text": title, "style": "body", "plus": 24 } } }
        }),
        &json!({}),
    );
    let expected = (title.chars().count() as f32 * 13.0 * StubText::ADVANCE).ceil() + 24.0;
    assert_eq!(root.style.width, Some(Dim::Fixed(expected)));
}

#[test]
fn the_sizing_keywords_all_resolve() {
    let root = resolve(
        json!({
            "type": "stack",
            "children": [
                { "type": "box", "props": { "size": { "width": "hug", "height": "fill" } } },
                { "type": "box", "props": { "size": { "width": { "fill": { "min": 120, "max": 452 } } } } },
                { "type": "box", "props": { "size": { "width": "fill", "height": { "ratio": 1 } } } },
                { "type": "box", "props": { "size": { "width": { "fraction": 0.5 } } } },
                { "type": "box", "props": { "size": 18.5 } }
            ]
        }),
        &json!({}),
    );
    let dims: Vec<_> = root
        .children()
        .iter()
        .map(|child| (child.style.width, child.style.height))
        .collect();
    assert_eq!(
        dims[0],
        (Some(Dim::Hug), Some(Dim::Fill { min: None, max: None }))
    );
    assert_eq!(
        dims[1].0,
        Some(Dim::Fill {
            min: Some(120.0),
            max: Some(452.0)
        })
    );
    assert_eq!(dims[2].1, Some(Dim::Ratio(1.0)));
    assert_eq!(dims[3].0, Some(Dim::Fraction(0.5)));
    assert_eq!(dims[4], (Some(Dim::Fixed(18.5)), Some(Dim::Fixed(18.5))));
}

#[test]
fn a_component_binds_its_arguments_and_its_defaults() {
    let data = json!({ "account": { "initial": "A" } });
    let root = resolve(
        json!({
            "type": "account-avatar",
            "props": { "initial": { "$": "account.initial" } }
        }),
        &data,
    );
    // The default `size` of 24 became the body's box, and the initial reached the label.
    assert_eq!(root.style.width, Some(Dim::Fixed(24.0)));
    assert_eq!(text_of(&root.children()[0]), "A");
}

#[test]
fn a_component_falls_back_to_its_other_branch() {
    let root = resolve(
        json!({ "type": "account-avatar", "props": { "initial": "" } }),
        &json!({}),
    );
    let child = &root.children()[0];
    match &child.content {
        Content::Symbol(symbol) => assert_eq!(symbol.name, "person.fill"),
        other => panic!("expected the person symbol, found {other:?}"),
    }
}

#[test]
fn an_instance_can_place_what_it_instantiates() {
    let root = resolve(
        json!({
            "type": "account-avatar",
            "props": { "initial": "A", "size": 44, "place": { "left": 16, "top": 8 } }
        }),
        &json!({}),
    );
    assert_eq!(root.style.width, Some(Dim::Fixed(44.0)));
    assert_eq!(root.style.place.left, Some(16.0));
    assert_eq!(root.style.place.top, Some(8.0));
}

#[test]
fn a_symbol_is_instantiated_with_the_colour_it_is_drawn_in() {
    let root = resolve(
        json!({ "type": "symbol", "props": { "name": "search", "size": 13.5, "color": "accent" } }),
        &json!({}),
    );
    let symbol = match &root.content {
        Content::Symbol(symbol) => symbol,
        other => panic!("expected a symbol, found {other:?}"),
    };
    assert_eq!(symbol.grid, 24.0);
    assert_eq!(symbol.children.len(), 2, "a ring and a handle");
    let accent = Color::new(0xFF3078F6);
    let ring = &symbol.children[0];
    assert_eq!(
        ring.style.stroke.expect("the ring is stroked").color,
        accent,
        "$color reached the symbol's own shapes"
    );
}

#[test]
fn an_arc_turns_progress_into_a_sweep() {
    let data = json!({ "progress": 0.25, "tiny": 0.0 });
    let root = resolve(
        json!({
            "type": "stack",
            "children": [
                { "type": "arc", "props": { "size": 16, "progress": { "$": "progress" } } },
                { "type": "arc", "props": { "size": 16, "progress": { "$": "tiny" } } },
                { "type": "arc", "props": { "size": 16, "sweep_deg": 300, "start_deg": -40 } }
            ]
        }),
        &data,
    );
    let arcs: Vec<_> = root
        .children()
        .iter()
        .map(|child| match &child.content {
            Content::Arc(arc) => (arc.start_deg, arc.sweep_deg),
            other => panic!("expected an arc, found {other:?}"),
        })
        .collect();
    assert_eq!(arcs[0], (-90.0, 90.0));
    assert_eq!(arcs[1], (-90.0, 0.02 * 360.0), "a just-started job still shows");
    assert_eq!(arcs[2], (-40.0, 300.0));
}

#[test]
fn a_field_knows_whether_it_has_focus() {
    let root = json!({
        "type": "field",
        "props": { "binding": "search", "placeholder": "Search title, author, narrator" }
    });
    let idle = resolve(root.clone(), &json!({}));
    let focused = resolve_with(root, &json!({}), |inputs| inputs.focused_field("search"));
    for (node, expected) in [(idle, false), (focused, true)] {
        match &node.content {
            Content::Field(field) => {
                assert_eq!(field.focused, expected);
                assert_eq!(field.placeholder, "Search title, author, narrator");
            }
            other => panic!("expected a field, found {other:?}"),
        }
    }
}

#[test]
fn the_authored_views_resolve_against_real_looking_data() {
    let data = json!({
        "account": { "name": "Ada Lovelace", "initial": "A", "is_signed_in": true,
                     "needs_reauthentication": false },
        "search": { "text": "" },
        "search_focused": false,
        "search_enabled": true,
        "selection": { "count": 2, "downloads": 2, "exports": 0 },
        "is_busy": false,
        "can_refresh": true,
        "checking_marketplaces": false,
        "signed_out_and_empty": false,
        "messages": { "tint": null }
    });
    let toolbar = resolve_with(
        loudible_scene::testing::read_scene("toolbar.json"),
        &data,
        |inputs| inputs.platform(Platform::MacOs),
    );
    assert_eq!(toolbar.children().len(), 3, "leading, centre and trailing");

    let toast = json!({
        "id": 7,
        "error": { "title": "Could not export", "detail": "FFmpeg stopped.",
                   "technical": "exit status 1" },
        "shows_details": false,
        "is_copied": false
    });
    let node = resolve(
        loudible_scene::testing::read_scene("error-toast.json"),
        &toast,
    );
    assert!(node.style.material.is_some(), "the toast sits on a material");
    assert_eq!(node.children().len(), 3, "icon, column and close button");
}

#[test]
fn a_grid_of_tiles_resolves_one_per_book() {
    let data = json!({
        "books": [
            { "asin": "B01", "title": "One", "authors_text": "Ada", "cover": "#FF6E7CF7",
              "runtime_text": "10 hrs" },
            { "asin": "B02", "title": "Two", "authors_text": "Bea", "cover": null,
              "runtime_text": null }
        ],
        "tiles": [
            { "is_selected": true, "is_in_library": true, "menu_items": ["download"],
              "status": { "kind": "none", "shows_badge": false } },
            { "is_selected": false, "is_in_library": false, "menu_items": [],
              "status": { "kind": "running", "shows_badge": true, "progress": 0.4,
                          "job": "export" } }
        ]
    });
    let root = resolve(
        json!({
            "type": "grid",
            "props": { "min_cell": 130, "gap": 18, "row_gap": 24, "padding": 20 },
            "children": [{
                "type": "book-tile",
                "repeat": { "for": "book", "in": "$books", "index": "i" },
                "props": { "book": { "$": "book" }, "state": { "$": "tiles.0" } }
            }]
        }),
        &data,
    );
    assert_eq!(root.children().len(), 2, "one tile per book");
    assert_eq!(root.style.min_cell, 130.0);
    let first = &root.children()[0];
    let hit = first.style.hit.as_ref().expect("a tile is clickable");
    assert_eq!(hit.action, "toggle-book");
    assert_eq!(hit.args, json!({ "asin": "B01" }));
}
