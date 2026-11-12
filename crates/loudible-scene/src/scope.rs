//! The data a document reads, and the conditions it branches on.
//!
//! One namespace, looked up in order: the locals a `repeat`, a component or a symbol introduced,
//! then the host's data. Conditions read the same names, so `when` and `{ "$": … }` never
//! disagree about what `selected` means.

use std::borrow::Cow;

use serde_json::Value;

use crate::document::{BoundNumber, Condition, NumberSpec, StringSpec};

/// Names bound by an enclosing `repeat`, component instance or symbol, innermost first.
pub struct Locals<'a> {
    parent: Option<&'a Locals<'a>>,
    entries: Vec<(String, Cow<'a, Value>)>,
}

impl<'a> Locals<'a> {
    pub fn root() -> Self {
        Self {
            parent: None,
            entries: vec![],
        }
    }

    /// A scope nested inside this one, which sees everything this one does.
    pub fn child(&'a self) -> Self {
        Self {
            parent: Some(self),
            entries: vec![],
        }
    }

    pub fn bind(&mut self, name: impl Into<String>, value: Cow<'a, Value>) {
        self.entries.push((name.into(), value));
    }

    pub fn bind_owned(&mut self, name: impl Into<String>, value: Value) {
        self.bind(name, Cow::Owned(value));
    }

    /// Borrowed for as long as the chain lives, so a lookup can hand the value straight back.
    fn get(&'a self, name: &str) -> Option<&'a Value> {
        self.entries
            .iter()
            .rev()
            .find(|(bound, _)| bound == name)
            .map(|(_, value)| value.as_ref())
            .or_else(|| self.parent.and_then(|parent| parent.get(name)))
    }
}

/// Everything a value in the document can be resolved against.
#[derive(Clone, Copy)]
pub struct Scope<'a> {
    pub data: &'a Value,
    pub locals: &'a Locals<'a>,
}

impl<'a> Scope<'a> {
    pub fn new(data: &'a Value, locals: &'a Locals<'a>) -> Self {
        Self { data, locals }
    }

    pub fn with(&self, locals: &'a Locals<'a>) -> Self {
        Self {
            data: self.data,
            locals,
        }
    }

    /// `"book.authors.0"` walks locals first, then the host's data. Missing is `None`, never a
    /// panic: a view that outruns its data draws nothing rather than crashing.
    pub fn lookup(&self, path: &str) -> Option<&'a Value> {
        let mut segments = path.split('.');
        let head = segments.next()?;
        let mut current: &'a Value = match self.locals.get(head) {
            Some(value) => value,
            None => self.data.get(head)?,
        };
        for segment in segments {
            current = match segment.parse::<usize>() {
                Ok(index) => current.get(index)?,
                Err(_) => current.get(segment)?,
            };
        }
        Some(current)
    }

    pub fn string(&self, spec: &StringSpec) -> String {
        match spec {
            StringSpec::Literal(text) => text.clone(),
            StringSpec::Bound(bound) => {
                if let Some(format) = &bound.format {
                    return self.interpolate(format);
                }
                if let Some(count) = &bound.count {
                    let n = self.number_at(count).unwrap_or(0.0);
                    let template = match n as i64 {
                        0 => bound.zero.as_deref(),
                        1 => bound.one.as_deref(),
                        _ => bound.many.as_deref(),
                    };
                    return template
                        .unwrap_or_default()
                        .replace("{n}", &render_number(n));
                }
                let found = bound
                    .path
                    .as_deref()
                    .and_then(|path| self.lookup(path))
                    .map(render_value);
                match found {
                    Some(text) if !text.is_empty() => text,
                    _ => bound.fallback.clone().unwrap_or_default(),
                }
            }
        }
    }

    pub fn number(&self, spec: &NumberSpec) -> Option<f32> {
        match spec {
            NumberSpec::Literal(value) => Some(*value),
            NumberSpec::Bound(BoundNumber { path, fallback }) => {
                self.number_at(path).or(*fallback)
            }
        }
    }

    pub fn number_or(&self, spec: &Option<NumberSpec>, fallback: f32) -> f32 {
        spec.as_ref()
            .and_then(|spec| self.number(spec))
            .unwrap_or(fallback)
    }

    fn number_at(&self, path: &str) -> Option<f32> {
        match self.lookup(path)? {
            Value::Number(number) => number.as_f64().map(|n| n as f32),
            Value::Bool(true) => Some(1.0),
            Value::Bool(false) => Some(0.0),
            Value::String(text) => text.parse().ok(),
            Value::Array(items) => Some(items.len() as f32),
            _ => None,
        }
    }

    fn interpolate(&self, format: &str) -> String {
        let mut out = String::with_capacity(format.len());
        let mut rest = format;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let Some(end) = rest[start..].find('}').map(|offset| start + offset) else {
                break;
            };
            let path = &rest[start + 1..end];
            out.push_str(&self.lookup(path).map(render_value).unwrap_or_default());
            rest = &rest[end + 1..];
        }
        out.push_str(rest);
        out
    }

    /// Whether a name holds something worth showing: present, non-empty and non-zero.
    pub fn truthy(&self, name: &str) -> bool {
        self.lookup(name).is_some_and(is_truthy)
    }

    pub fn test(&self, condition: &Condition) -> bool {
        match condition {
            Condition::Flag(name) => self.truthy(name),
            Condition::Always(value) => *value,
            Condition::Expr(expr) => {
                if let Some(inner) = &expr.not {
                    return !self.test(inner);
                }
                if let Some(all) = &expr.all {
                    return all.iter().all(|inner| self.test(inner));
                }
                if let Some(any) = &expr.any {
                    return any.iter().any(|inner| self.test(inner));
                }
                if let Some([left, right]) = &expr.eq {
                    return self.operand(left) == self.operand(right);
                }
                if let Some([left, right]) = &expr.gt {
                    let (left, right) = (self.operand(left), self.operand(right));
                    return match (as_number(&left), as_number(&right)) {
                        (Some(left), Some(right)) => left > right,
                        _ => false,
                    };
                }
                if let Some(name) = &expr.empty {
                    return !self.truthy(name);
                }
                false
            }
        }
    }

    /// A condition's operand: a string is a name to look up when something is bound to it, and
    /// the literal string otherwise. Anything else is already a literal.
    fn operand(&self, value: &Value) -> Value {
        match value {
            Value::String(name) => self
                .lookup(name)
                .cloned()
                .unwrap_or_else(|| value.clone()),
            other => other.clone(),
        }
    }
}

fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::Array(items) => Some(items.len() as f64),
        Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        _ => None,
    }
}

pub fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

/// How a bound value reads when it lands in a string.
pub fn render_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Number(number) => number
            .as_f64()
            .map(|n| render_number(n as f32))
            .unwrap_or_default(),
        Value::Bool(flag) => flag.to_string(),
        other => other.to_string(),
    }
}

fn render_number(value: f32) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}
