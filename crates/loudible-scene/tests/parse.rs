//! The authored documents parse, and the mistakes a hand-written one makes come back with a
//! pointer to the place at fault.

use loudible_scene::testing::Builder;
use loudible_scene::{ErrorKind, NodeType, Scene, SceneError, What};
use serde_json::{Value, json};

fn error(root: Value) -> SceneError {
    Builder::standard()
        .root(root)
        .build()
        .expect_err("expected this document to be rejected")
}

#[test]
fn the_shared_vocabulary_loads() {
    let scene = Builder::standard().parse();
    let document = scene.document();
    assert_eq!(document.tokens.colors.len(), 18, "every colour in theme.rs");
    assert_eq!(document.tokens.text.len(), 5, "body, headline, callout, caption, title3");
    assert!(document.symbols.contains_key("error.fill"));
    assert!(document.components.contains_key("book-tile"));
}

#[test]
fn the_authored_views_parse() {
    for file in ["toolbar.json", "error-toast.json"] {
        let scene = Builder::standard().root_scene(file).parse();
        assert!(!scene.document().root.children.is_empty(), "{file} has content");
    }
}

#[test]
fn a_component_takes_its_parameters_as_props() {
    let scene = Builder::standard()
        .root(json!({
            "type": "book-tile",
            "props": { "book": { "$": "book" }, "state": { "$": "state" }, "selectable": true }
        }))
        .parse();
    let params = &scene.document().root.props.params;
    assert_eq!(params.len(), 3);
    assert_eq!(params["selectable"], json!(true));
}

#[test]
fn every_symbol_is_a_tree_of_shapes() {
    let scene = Builder::standard().parse();
    for (name, symbol) in &scene.document().symbols {
        assert_eq!(symbol.grid, 24.0, "{name} is on the 24-unit grid");
        assert!(!symbol.children.is_empty(), "{name} draws something");
        for child in &symbol.children {
            let kind = NodeType::parse(&child.kind);
            assert!(kind.is_some(), "{name} draws a {:?}", child.kind);
        }
    }
}

#[test]
fn an_unknown_prop_names_its_node() {
    let found = error(json!({ "type": "box", "props": { "fil": "window" } }));
    assert_eq!(found.path, "/root/props");
    assert!(
        matches!(found.kind, ErrorKind::UnknownKey { ref key, .. } if key == "fil"),
        "{found}"
    );
}

#[test]
fn a_prop_on_the_wrong_type_says_so() {
    let found = error(json!({ "type": "ellipse", "props": { "min_cell": 130 } }));
    assert_eq!(found.path, "/root/props");
    assert_eq!(
        found.kind,
        ErrorKind::PropNotOnType {
            key: "min_cell".into(),
            node: "ellipse".into()
        }
    );
    assert!(format!("{found}").contains("not a prop of a \"ellipse\" node"));
}

#[test]
fn an_unknown_node_type_is_not_a_component() {
    let found = error(json!({ "type": "stak" }));
    assert_eq!(found.path, "/root/type");
    assert_eq!(found.kind, ErrorKind::UnknownNodeType("stak".into()));
}

#[test]
fn an_unknown_colour_token_is_rejected() {
    let found = error(json!({ "type": "box", "props": { "fill": "windo" } }));
    assert_eq!(found.path, "/root/props/fill");
    assert_eq!(
        found.kind,
        ErrorKind::Undeclared {
            what: What::ColorToken,
            name: "windo".into()
        }
    );
}

#[test]
fn an_unknown_symbol_is_rejected() {
    let found = error(json!({ "type": "symbol", "props": { "name": "sparkle" } }));
    assert_eq!(found.path, "/root/props/name");
    assert_eq!(
        found.kind,
        ErrorKind::Undeclared {
            what: What::Symbol,
            name: "sparkle".into()
        }
    );
}

#[test]
fn a_literal_colour_needs_no_token() {
    Builder::standard()
        .root(json!({ "type": "box", "props": { "fill": "#99000000" } }))
        .parse();
}

#[test]
fn a_leaf_may_not_have_children() {
    let found = error(json!({
        "type": "text",
        "props": { "value": "hi" },
        "children": [{ "type": "box" }]
    }));
    assert_eq!(found.path, "/root/children");
    assert_eq!(
        found.kind,
        ErrorKind::Conflict("this node type paints no children")
    );
}

#[test]
fn a_bar_child_needs_a_slot() {
    let found = error(json!({
        "type": "bar",
        "children": [{ "type": "box", "props": { "slot": "leading" } }, { "type": "box" }]
    }));
    assert_eq!(found.path, "/root/children/1");
    assert_eq!(
        found.kind,
        ErrorKind::Missing("every child of a `bar` needs a `slot`")
    );
}

#[test]
fn an_arc_needs_a_sweep() {
    let found = error(json!({ "type": "arc", "props": { "size": 16 } }));
    assert_eq!(found.path, "/root/props");
    assert_eq!(
        found.kind,
        ErrorKind::Missing("an `arc` needs `sweep_deg` or `progress`")
    );
}

#[test]
fn only_one_axis_may_be_a_ratio() {
    let found = error(json!({
        "type": "box",
        "props": { "size": { "width": { "ratio": 1 }, "height": { "ratio": 1 } } }
    }));
    assert_eq!(found.path, "/root/props/size");
    assert_eq!(
        found.kind,
        ErrorKind::Conflict("only one axis may be a `ratio` of the other")
    );
}

#[test]
fn a_component_may_not_reach_itself() {
    let found = Builder::new()
        .component_value(
            "loop",
            json!({ "body": { "type": "box", "children": [{ "type": "loop" }] } }),
        )
        .root(json!({ "type": "loop" }))
        .build()
        .expect_err("a cycle is rejected");
    assert!(
        matches!(found.kind, ErrorKind::ComponentCycle(_)),
        "{found}"
    );
}

#[test]
fn a_wrong_version_is_refused() {
    let document = json!({ "schema": "loudible.scene/2", "root": { "type": "box" } });
    let found = Scene::from_value(document).expect_err("a future version is refused");
    assert_eq!(found.path, "/schema");
    assert_eq!(
        found.kind,
        ErrorKind::UnknownVersion("loudible.scene/2".into())
    );
}

#[test]
fn malformed_json_is_not_a_panic() {
    let found = Scene::parse("{ not json").expect_err("rejected");
    assert!(matches!(found.kind, ErrorKind::NotJson(_)), "{found}");
}
