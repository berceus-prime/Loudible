//! Two checks that `serde` cannot make on its own.
//!
//! The structural pass runs over the raw JSON *before* deserialising, because that is where the
//! key names still exist: a typo'd prop, a prop on the wrong node type or an unknown `type` comes
//! back as a JSON pointer rather than serde's "data did not match any variant". The reference pass
//! runs over the typed document, where tokens, symbols and components are easy to look up.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::document::{
    ColorSpec, Component, Dimension, Document, Node, Props, SizeSpec, StringSpec, TextStyleSpec,
    Tokens, VERSION,
};

use crate::error::{ErrorKind, SceneError, What};
use crate::kind::{BOX_PROPS, NODE_KEYS, NODE_TYPES, NodeType};

const DOCUMENT_KEYS: &[&str] = &["$comment", "schema", "tokens", "symbols", "components", "root"];

type Result<T> = std::result::Result<T, SceneError>;

fn at(path: &str, segment: impl std::fmt::Display) -> String {
    format!("{path}/{segment}")
}

fn type_name(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(_) => "a boolean".into(),
        Value::Number(_) => "a number".into(),
        Value::String(s) => format!("the string {s:?}"),
        Value::Array(_) => "an array".into(),
        Value::Object(_) => "an object".into(),
    }
}

fn unknown_key(path: &str, key: &str, expected: &'static str) -> SceneError {
    SceneError::new(
        path,
        ErrorKind::UnknownKey {
            key: key.to_string(),
            expected,
        },
    )
}

/// What a component accepts as props: the names of its parameters.
fn component_params(value: &Value) -> BTreeMap<String, BTreeSet<String>> {
    let mut map = BTreeMap::new();
    let Some(components) = value.get("components").and_then(Value::as_object) else {
        return map;
    };
    for (name, component) in components {
        let params = component
            .get("params")
            .and_then(Value::as_array)
            .map(|params| {
                params
                    .iter()
                    .filter_map(|param| param.get("name").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        map.insert(name.clone(), params);
    }
    map
}

/// Keys, node types and prop placement, straight off the JSON.
pub(crate) fn check_structure(value: &Value) -> Result<()> {
    let Some(root) = value.as_object() else {
        return Err(SceneError::new(
            "",
            ErrorKind::BadValue {
                expected: "an object",
                found: type_name(value),
            },
        ));
    };
    for key in root.keys() {
        if !DOCUMENT_KEYS.contains(&key.as_str()) {
            return Err(unknown_key(
                "",
                key,
                "$comment, schema, tokens, symbols, components, root",
            ));
        }
    }
    match root.get("schema").and_then(Value::as_str) {
        Some(VERSION) => {}
        Some(other) => {
            return Err(SceneError::new(
                "/schema",
                ErrorKind::UnknownVersion(other.to_string()),
            ));
        }
        None => {
            return Err(SceneError::new(
                "/schema",
                ErrorKind::Missing("every document needs a `schema` version"),
            ));
        }
    }
    if !root.contains_key("root") {
        return Err(SceneError::new(
            "",
            ErrorKind::Missing("every document needs a `root` node"),
        ));
    }

    let params = component_params(value);
    if let Some(symbols) = root.get("symbols").and_then(Value::as_object) {
        for (name, symbol) in symbols {
            let path = at("/symbols", name);
            for key in symbol.as_object().into_iter().flat_map(|o| o.keys()) {
                if !["$comment", "grid", "props", "children"].contains(&key.as_str()) {
                    return Err(unknown_key(&path, key, "$comment, grid, props, children"));
                }
            }
            for (index, child) in children_of(symbol).iter().enumerate() {
                check_node(child, &at(&at(&path, "children"), index), &params)?;
            }
        }
    }
    if let Some(components) = root.get("components").and_then(Value::as_object) {
        for (name, component) in components {
            let path = at("/components", name);
            for key in component.as_object().into_iter().flat_map(|o| o.keys()) {
                if !["$comment", "params", "body"].contains(&key.as_str()) {
                    return Err(unknown_key(&path, key, "$comment, params, body"));
                }
            }
            match component.get("body") {
                Some(body) => check_node(body, &at(&path, "body"), &params)?,
                None => {
                    return Err(SceneError::new(
                        path,
                        ErrorKind::Missing("a component needs a `body`"),
                    ));
                }
            }
        }
    }
    check_node(&root["root"], "/root", &params)
}

fn children_of(value: &Value) -> &[Value] {
    value
        .get("children")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn check_node(
    value: &Value,
    path: &str,
    params: &BTreeMap<String, BTreeSet<String>>,
) -> Result<()> {
    let Some(object) = value.as_object() else {
        return Err(SceneError::new(
            path,
            ErrorKind::BadValue {
                expected: "a node object",
                found: type_name(value),
            },
        ));
    };
    for key in object.keys() {
        if !NODE_KEYS.contains(&key.as_str()) {
            return Err(unknown_key(
                path,
                key,
                "$comment, type, id, when, repeat, props, variants, children",
            ));
        }
    }
    let Some(kind) = object.get("type").and_then(Value::as_str) else {
        return Err(SceneError::new(
            path,
            ErrorKind::Missing("a node needs a `type`"),
        ));
    };

    let node_type = NodeType::parse(kind);
    let allowed: BTreeSet<String> = match (node_type, params.get(kind)) {
        (Some(node_type), _) => BOX_PROPS
            .iter()
            .chain(node_type.extra_props())
            .map(|key| key.to_string())
            .collect(),
        // A component takes its parameters as props, plus the box props that position it.
        (None, Some(declared)) => BOX_PROPS
            .iter()
            .map(|key| key.to_string())
            .chain(declared.iter().cloned())
            .collect(),
        (None, None) => {
            return Err(SceneError::new(
                at(path, "type"),
                ErrorKind::UnknownNodeType(kind.to_string()),
            ));
        }
    };

    check_props(object.get("props"), &at(path, "props"), kind, &allowed)?;
    for (index, variant) in object
        .get("variants")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .enumerate()
    {
        let path = at(&at(path, "variants"), index);
        for key in variant.as_object().into_iter().flat_map(|o| o.keys()) {
            if !["when", "props"].contains(&key.as_str()) {
                return Err(unknown_key(&path, key, "when, props"));
            }
        }
        if !variant.get("when").is_some() {
            return Err(SceneError::new(
                &path,
                ErrorKind::Missing("a variant needs a `when`"),
            ));
        }
        check_props(variant.get("props"), &at(&path, "props"), kind, &allowed)?;
    }

    let children = children_of(value);
    if !children.is_empty() {
        match node_type {
            Some(node_type) if !node_type.takes_children() => {
                return Err(SceneError::new(
                    at(path, "children"),
                    ErrorKind::Conflict("this node type paints no children"),
                ));
            }
            None => {
                return Err(SceneError::new(
                    at(path, "children"),
                    ErrorKind::Conflict("a component instance takes props, not children"),
                ));
            }
            _ => {}
        }
    }
    if node_type == Some(NodeType::Bar) {
        for (index, child) in children.iter().enumerate() {
            let has_slot = child
                .get("props")
                .and_then(|props| props.get("slot"))
                .is_some();
            let conditional = child
                .get("variants")
                .and_then(Value::as_array)
                .is_some_and(|variants| {
                    variants
                        .iter()
                        .any(|variant| variant.pointer("/props/slot").is_some())
                });
            if !has_slot && !conditional {
                return Err(SceneError::new(
                    at(&at(path, "children"), index),
                    ErrorKind::Missing("every child of a `bar` needs a `slot`"),
                ));
            }
        }
    }
    for (index, child) in children.iter().enumerate() {
        check_node(child, &at(&at(path, "children"), index), params)?;
    }
    Ok(())
}

fn check_props(
    value: Option<&Value>,
    path: &str,
    node: &str,
    allowed: &BTreeSet<String>,
) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let Some(object) = value.as_object() else {
        return Err(SceneError::new(
            path,
            ErrorKind::BadValue {
                expected: "an object of props",
                found: type_name(value),
            },
        ));
    };
    for key in object.keys() {
        if allowed.contains(key.as_str()) {
            continue;
        }
        let known_elsewhere = BOX_PROPS.contains(&key.as_str())
            || NODE_TYPES
                .iter()
                .filter_map(|name| NodeType::parse(name))
                .any(|node_type| node_type.extra_props().contains(&key.as_str()));
        return Err(if known_elsewhere {
            SceneError::new(
                path,
                ErrorKind::PropNotOnType {
                    key: key.clone(),
                    node: node.to_string(),
                },
            )
        } else {
            unknown_key(path, key, "a box property or one of this type's own")
        });
    }
    Ok(())
}

// MARK: References

/// Tokens, symbols, component parameters and cycles, over the typed document.
pub(crate) fn check_references(document: &Document) -> Result<()> {
    for (name, symbol) in &document.symbols {
        let path = at("/symbols", name);
        check_props_references(document, &symbol.props, None, &path)?;
        for (index, child) in symbol.children.iter().enumerate() {
            check_tree(document, child, &at(&at(&path, "children"), index))?;
        }
    }
    for (name, component) in &document.components {
        let path = at(&at("/components", name), "body");
        check_tree(document, &component.body, &path)?;
    }
    check_cycles(&document.components)?;
    check_tree(document, &document.root, "/root")
}

fn check_tree(document: &Document, node: &Node, path: &str) -> Result<()> {
    let node_type = NodeType::parse(&node.kind);
    check_props_references(document, &node.props, node_type, path)?;
    for variant in &node.variants {
        check_props_references(document, &variant.props, node_type, path)?;
    }
    if node_type.is_none() && !document.components.contains_key(&node.kind) {
        return Err(SceneError::new(
            at(path, "type"),
            ErrorKind::UnknownNodeType(node.kind.clone()),
        ));
    }
    if node_type == Some(NodeType::Arc)
        && node.props.sweep_deg.is_none()
        && node.props.progress.is_none()
        && !node
            .variants
            .iter()
            .any(|v| v.props.sweep_deg.is_some() || v.props.progress.is_some())
    {
        return Err(SceneError::new(
            at(path, "props"),
            ErrorKind::Missing("an `arc` needs `sweep_deg` or `progress`"),
        ));
    }
    if let Some(SizeSpec::Axes(axes)) = &node.props.size
        && matches!(&axes.width, Some(Dimension::Spec(spec)) if spec.ratio.is_some())
        && matches!(&axes.height, Some(Dimension::Spec(spec)) if spec.ratio.is_some())
    {
        return Err(SceneError::new(
            at(path, "props/size"),
            ErrorKind::Conflict("only one axis may be a `ratio` of the other"),
        ));
    }
    for (index, child) in node.children.iter().enumerate() {
        check_tree(document, child, &at(&at(path, "children"), index))?;
    }
    Ok(())
}

fn check_props_references(
    document: &Document,
    props: &Props,
    node_type: Option<NodeType>,
    path: &str,
) -> Result<()> {
    let path = at(path, "props");
    let colors = [
        ("fill", &props.fill),
        ("color", &props.color),
        ("background", &props.background),
        ("placeholder_color", &props.placeholder_color),
        ("selection_color", &props.selection_color),
        ("caret_color", &props.caret_color),
    ];
    for (key, color) in colors {
        if let Some(color) = color {
            check_color(&document.tokens, color, &at(&path, key))?;
        }
    }
    for (key, stroke) in [("stroke", &props.stroke), ("track", &props.track)] {
        if let Some(stroke) = stroke {
            check_color(&document.tokens, &stroke.color, &at(&at(&path, key), "color"))?;
        }
    }
    if let Some(material) = &props.material
        && let Some(tint) = &material.tint
    {
        check_color(&document.tokens, tint, &at(&at(&path, "material"), "tint"))?;
    }
    if let Some(bar) = &props.scrollbar
        && let Some(color) = &bar.color
    {
        check_color(&document.tokens, color, &at(&at(&path, "scrollbar"), "color"))?;
    }
    for shadow in props.shadows() {
        check_color(&document.tokens, &shadow.color, &at(&path, "shadow"))?;
    }
    if let Some(style) = &props.style {
        check_text_style(&document.tokens, style, &at(&path, "style"))?;
    }
    if let Some(SizeSpec::Axes(axes)) = &props.size {
        for (key, dimension) in [("width", &axes.width), ("height", &axes.height)] {
            if let Some(Dimension::Spec(spec)) = dimension
                && let Some(style) = &spec.style
            {
                check_text_style(
                    &document.tokens,
                    style,
                    &at(&at(&at(&path, "size"), key), "style"),
                )?;
            }
        }
    }
    // A `host`'s name is a native view, not a symbol, so only `symbol` is checked here. A bound
    // name can only be checked when the data arrives.
    if node_type == Some(NodeType::Symbol)
        && let Some(StringSpec::Literal(name)) = &props.name
        && !document.symbols.contains_key(name)
    {
        return Err(SceneError::new(
            at(&path, "name"),
            ErrorKind::Undeclared {
                what: What::Symbol,
                name: name.clone(),
            },
        ));
    }
    Ok(())
}

fn check_color(tokens: &Tokens, color: &ColorSpec, path: &str) -> Result<()> {
    match color {
        ColorSpec::Named(name) => {
            // `#AARRGGBB` is a literal and `$param` is a binding; anything else is a token.
            if name.starts_with('#') || name.starts_with('$') || tokens.colors.contains_key(name) {
                Ok(())
            } else {
                Err(SceneError::new(
                    path,
                    ErrorKind::Undeclared {
                        what: What::ColorToken,
                        name: name.clone(),
                    },
                ))
            }
        }
        ColorSpec::Computed(computed) => {
            if let Some(token) = &computed.token
                && !tokens.colors.contains_key(token)
            {
                return Err(SceneError::new(
                    at(path, "token"),
                    ErrorKind::Undeclared {
                        what: What::ColorToken,
                        name: token.clone(),
                    },
                ));
            }
            if let Some((a, b, _)) = &computed.mix {
                check_color(tokens, a, &at(path, "mix/0"))?;
                check_color(tokens, b, &at(path, "mix/1"))?;
            }
            if let Some(gradient) = &computed.gradient {
                for (index, stop) in gradient.stops.iter().enumerate() {
                    check_color(tokens, stop, &at(&at(path, "gradient/stops"), index))?;
                }
            }
            Ok(())
        }
    }
}

fn check_text_style(tokens: &Tokens, style: &TextStyleSpec, path: &str) -> Result<()> {
    let named = match style {
        TextStyleSpec::Named(name) => Some(name),
        TextStyleSpec::Spec(overrides) => overrides.style.as_ref(),
    };
    match named {
        Some(name) if !tokens.text.contains_key(name) => Err(SceneError::new(
            path,
            ErrorKind::Undeclared {
                what: What::TextToken,
                name: name.clone(),
            },
        )),
        _ => Ok(()),
    }
}

/// A component may not reach itself, however long the way round.
fn check_cycles(components: &BTreeMap<String, Component>) -> Result<()> {
    fn walk(
        components: &BTreeMap<String, Component>,
        node: &Node,
        chain: &mut Vec<String>,
    ) -> Result<()> {
        if components.contains_key(&node.kind) {
            if chain.contains(&node.kind) {
                let mut chain = chain.clone();
                chain.push(node.kind.clone());
                return Err(SceneError::new(
                    at("/components", &node.kind),
                    ErrorKind::ComponentCycle(chain),
                ));
            }
            chain.push(node.kind.clone());
            walk(components, &components[&node.kind].body, chain)?;
            chain.pop();
        }
        for child in &node.children {
            walk(components, child, chain)?;
        }
        Ok(())
    }

    for name in components.keys() {
        let mut chain = vec![name.clone()];
        walk(components, &components[name].body, &mut chain)?;
    }
    Ok(())
}
