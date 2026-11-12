//! Parse and validation failures, each pointing at the place in the document that caused it.

use std::fmt;

/// What went wrong, and where. `path` is a JSON pointer such as
/// `/root/children/2/props/fill`, so an editor can jump to it.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneError {
    pub path: String,
    pub kind: ErrorKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ErrorKind {
    /// The document is not JSON at all.
    NotJson(String),
    /// `schema` is missing or names a version this crate does not implement.
    UnknownVersion(String),
    /// A key no node or prop accepts, usually a typo.
    UnknownKey { key: String, expected: &'static str },
    /// A prop that exists, but not on this node type.
    PropNotOnType { key: String, node: String },
    /// `type` names neither a node type nor a declared component.
    UnknownNodeType(String),
    /// A reference to something the document never declares.
    Undeclared { what: What, name: String },
    /// A component that instantiates itself, directly or through others.
    ComponentCycle(Vec<String>),
    /// The value is the wrong shape for where it sits.
    BadValue { expected: &'static str, found: String },
    /// Two props that cannot both be set, or one that needs another.
    Conflict(&'static str),
    /// Something structurally required is absent.
    Missing(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    ColorToken,
    TextToken,
    MetricToken,
    Symbol,
    Component,
    ComponentParam,
}

impl SceneError {
    pub(crate) fn new(path: impl Into<String>, kind: ErrorKind) -> Self {
        Self {
            path: path.into(),
            kind,
        }
    }
}

impl fmt::Display for What {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::ColorToken => "colour token",
            Self::TextToken => "text style",
            Self::MetricToken => "metric",
            Self::Symbol => "symbol",
            Self::Component => "component",
            Self::ComponentParam => "component parameter",
        };
        f.write_str(name)
    }
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = if self.path.is_empty() { "/" } else { &self.path };
        match &self.kind {
            ErrorKind::NotJson(message) => write!(f, "{at}: not valid JSON: {message}"),
            ErrorKind::UnknownVersion(found) => {
                write!(f, "{at}: unknown schema version {found:?}")
            }
            ErrorKind::UnknownKey { key, expected } => {
                write!(f, "{at}: unknown key {key:?}; expected one of {expected}")
            }
            ErrorKind::PropNotOnType { key, node } => {
                write!(f, "{at}: {key:?} is not a prop of a {node:?} node")
            }
            ErrorKind::UnknownNodeType(found) => {
                write!(f, "{at}: {found:?} is not a node type or a component")
            }
            ErrorKind::Undeclared { what, name } => {
                write!(f, "{at}: no {what} named {name:?}")
            }
            ErrorKind::ComponentCycle(chain) => {
                write!(f, "{at}: components instantiate each other: {}", chain.join(" -> "))
            }
            ErrorKind::BadValue { expected, found } => {
                write!(f, "{at}: expected {expected}, found {found}")
            }
            ErrorKind::Conflict(message) => write!(f, "{at}: {message}"),
            ErrorKind::Missing(message) => write!(f, "{at}: {message}"),
        }
    }
}

impl std::error::Error for SceneError {}
