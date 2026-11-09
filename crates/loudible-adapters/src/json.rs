use serde_json::Value;

/// Lenient, read-only view over parsed JSON. Missing keys and type mismatches yield `None`,
/// which suits Audible responses whose optional fields come and go.
#[derive(Clone, Copy)]
pub struct Json<'a>(Option<&'a Value>);

/// Parses `data`; anything unparsable reads as JSON null.
pub fn parse(data: &[u8]) -> Value {
    serde_json::from_slice(data).unwrap_or(Value::Null)
}

impl<'a> Json<'a> {
    pub fn new(value: &'a Value) -> Self {
        Self(Some(value))
    }

    pub fn get(self, key: &str) -> Json<'a> {
        Self(self.0.and_then(Value::as_object).and_then(|o| o.get(key)))
    }

    pub fn is_null(self) -> bool {
        self.0.is_none_or(Value::is_null)
    }

    pub fn string(self) -> Option<String> {
        match self.0? {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(match number.as_f64() {
                // `7.0` and `7` are the same number; Audible's versions are compared as text.
                Some(float) if number.is_f64() && float.fract() == 0.0 => format!("{float:.0}"),
                _ => number.to_string(),
            }),
            _ => None,
        }
    }

    /// Accepts numbers and numeric strings (Amazon sends `expires_in` as either).
    pub fn int(self) -> Option<i64> {
        match self.0? {
            Value::Number(number) => number.as_i64().or(number.as_f64().map(|f| f as i64)),
            Value::String(text) => text.parse().ok(),
            _ => None,
        }
    }

    pub fn bool(self) -> Option<bool> {
        match self.0? {
            Value::Bool(value) => Some(*value),
            Value::String(text) => match text.to_lowercase().as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn array(self) -> Option<Vec<Json<'a>>> {
        Some(self.0?.as_array()?.iter().map(Self::new).collect())
    }

    pub fn is_object(self) -> bool {
        self.0.is_some_and(Value::is_object)
    }

    /// The object's values in key order.
    pub fn values(self) -> Vec<Json<'a>> {
        self.0
            .and_then(Value::as_object)
            .map(|object| object.values().map(Self::new).collect())
            .unwrap_or_default()
    }
}
