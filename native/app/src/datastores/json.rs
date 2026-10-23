//! A JSON value that **keeps field order**, for anything that shows a
//! document as the server sent it. `serde_json::Value` sorts object keys
//! unless some crate elsewhere in the build happens to turn serde_json's
//! `preserve_order` feature on, which nothing here may depend on. Parsing
//! follows `JSON.parse`: a repeated key keeps its first position and its last
//! value. Printing follows `JSON.stringify(value, null, 2)` and JavaScript's
//! number formatting, because the Vue tabs' output is what is matched.
//!
//! Hoisted from the MongoDB tab; re-exported by [`super::common`].

use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

/// A JSON value with object fields in document order.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn parse(text: &str) -> Option<Json> {
        serde_json::from_str(text).ok()
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// JavaScript's `String(value)` for a scalar; containers as JS would
    /// print them is never wanted here, so they read as their collapsed
    /// form.
    pub fn js_string(&self) -> String {
        match self {
            Json::Null => "null".into(),
            Json::Bool(b) => b.to_string(),
            Json::Int(n) => n.to_string(),
            Json::UInt(n) => n.to_string(),
            Json::Float(f) => js_number(*f),
            Json::Str(s) => s.clone(),
            Json::Arr(a) => format!("[{}]", a.len()),
            Json::Obj(_) => "{\u{2026}}".into(),
        }
    }

    /// JavaScript's `Number(value)`.
    pub fn js_number(&self) -> f64 {
        match self {
            Json::Null => 0.0,
            Json::Bool(b) => f64::from(u8::from(*b)),
            Json::Int(n) => *n as f64,
            Json::UInt(n) => *n as f64,
            Json::Float(f) => *f,
            Json::Str(s) => {
                let t = s.trim();
                if t.is_empty() {
                    0.0
                } else {
                    t.parse().unwrap_or(f64::NAN)
                }
            }
            Json::Arr(_) | Json::Obj(_) => f64::NAN,
        }
    }

    /// `JSON.stringify(value, null, 2)`.
    pub fn stringify_pretty(&self) -> String {
        let mut out = String::new();
        write_pretty(self, 0, &mut out);
        out
    }
}

/// A number as JavaScript prints it: integers without a fraction, `NaN`,
/// `Infinity`.
pub fn js_number(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else if f.fract() == 0.0 && f.abs() < 1e21 {
        format!("{f:.0}")
    } else {
        f.to_string()
    }
}

fn write_pretty(v: &Json, indent: usize, out: &mut String) {
    let pad = |n: usize, out: &mut String| out.push_str(&"  ".repeat(n));
    match v {
        Json::Arr(items) if items.is_empty() => out.push_str("[]"),
        Json::Obj(fields) if fields.is_empty() => out.push_str("{}"),
        Json::Arr(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                pad(indent + 1, out);
                write_pretty(item, indent + 1, out);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            pad(indent, out);
            out.push(']');
        }
        Json::Obj(fields) => {
            out.push_str("{\n");
            for (i, (k, val)) in fields.iter().enumerate() {
                pad(indent + 1, out);
                out.push_str(&quote(k));
                out.push_str(": ");
                write_pretty(val, indent + 1, out);
                if i + 1 < fields.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            pad(indent, out);
            out.push('}');
        }
        Json::Str(s) => out.push_str(&quote(s)),
        // JSON.stringify writes a non-finite number as null.
        Json::Float(f) if !f.is_finite() => out.push_str("null"),
        other => out.push_str(&other.js_string()),
    }
}

fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Json;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_unit<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }
    fn visit_none<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }
    fn visit_bool<E>(self, b: bool) -> Result<Json, E> {
        Ok(Json::Bool(b))
    }
    fn visit_i64<E>(self, n: i64) -> Result<Json, E> {
        Ok(Json::Int(n))
    }
    fn visit_u64<E>(self, n: u64) -> Result<Json, E> {
        Ok(Json::UInt(n))
    }
    fn visit_f64<E>(self, f: f64) -> Result<Json, E> {
        Ok(Json::Float(f))
    }
    fn visit_str<E: de::Error>(self, s: &str) -> Result<Json, E> {
        Ok(Json::Str(s.to_string()))
    }
    fn visit_string<E>(self, s: String) -> Result<Json, E> {
        Ok(Json::Str(s))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(v) = seq.next_element()? {
            items.push(v);
        }
        Ok(Json::Arr(items))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let mut fields: Vec<(String, Json)> = Vec::new();
        while let Some((k, v)) = map.next_entry::<String, Json>()? {
            // JSON.parse keeps the first position and the last value.
            match fields.iter_mut().find(|(key, _)| *key == k) {
                Some(slot) => slot.1 = v,
                None => fields.push((k, v)),
            }
        }
        Ok(Json::Obj(fields))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_keeps_field_order_and_the_last_duplicate() {
        let v = Json::parse(r#"{"z":1,"a":2,"m":{"y":true,"b":null},"z":3}"#).unwrap();
        let Json::Obj(fields) = &v else { panic!() };
        let keys: Vec<_> = fields.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["z", "a", "m"]);
        assert_eq!(v.get("z"), Some(&Json::UInt(3)));
        assert!(Json::parse("{oops").is_none());
    }

    #[test]
    fn numbers_print_like_javascript() {
        assert_eq!(js_number(3.0), "3");
        assert_eq!(js_number(-0.5), "-0.5");
        assert_eq!(js_number(f64::NAN), "NaN");
        assert_eq!(Json::Str(" 12 ".into()).js_number(), 12.0);
        assert!(Json::Str("x".into()).js_number().is_nan());
    }
}
