//! Canonical extended JSON made readable, and a page of documents made into
//! table columns. Ports of `src/utils/bsonDisplay.js` and
//! `src/utils/mongoTable.js`.
//!
//! The core returns each document as canonical extended JSON so no numeric
//! type is lost: an ObjectId arrives as `{"$oid": "..."}` and a date as
//! `{"$date": {"$numberLong": "..."}}`. It is unwrapped here, for display
//! only.
//!
//! Documents are parsed into [`Json`], which **keeps field order**
//! (`serde_json::Value` sorts keys unless a feature elsewhere in the build
//! happens to turn `preserve_order` on, which this must not depend on).

use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

/// Depth past which nested values are elided (`DEFAULT_MAX_DEPTH`).
pub const MAX_DEPTH: usize = 12;

/// Beyond this many columns a table stops being readable
/// (`DEFAULT_MAX_COLUMNS`).
pub const MAX_COLUMNS: usize = 12;

/// The collapsed row's summary length (`summarizeDocument`'s `limit`).
pub const SUMMARY_LIMIT: usize = 120;

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

// ---- bsonDisplay.js ------------------------------------------------------

/// Unwrap one extended-JSON wrapper into its display string, or `None` when
/// `value` is not one. Only the first key decides, as in the Vue helper.
pub fn unwrap_extended_json(value: &Json) -> Option<String> {
    let Json::Obj(fields) = value else {
        return None;
    };
    let (key, inner) = fields.first()?;
    let field = |name: &str| inner.get(name).map(Json::js_string);
    Some(match key.as_str() {
        // Single quotes: the string is quoted again by the pretty view, and
        // double quotes would come back escaped.
        "$oid" => format!("ObjectId('{}')", inner.js_string()),
        "$date" => format_date(inner),
        "$numberLong" | "$numberInt" | "$numberDouble" | "$numberDecimal" => inner.js_string(),
        "$binary" => format!("Binary('{}')", field("subType").unwrap_or_default()),
        "$timestamp" => format!(
            "Timestamp({}, {})",
            field("t").unwrap_or_else(|| "0".into()),
            field("i").unwrap_or_else(|| "0".into())
        ),
        "$regularExpression" => format!(
            "/{}/{}",
            field("pattern").unwrap_or_default(),
            field("options").unwrap_or_default()
        ),
        "$minKey" => "MinKey".into(),
        "$maxKey" => "MaxKey".into(),
        "$undefined" => "undefined".into(),
        _ => return None,
    })
}

/// `ISODate('...')`, or `Date(...)` when it is not a valid date.
fn format_date(raw: &Json) -> String {
    let ms = match raw {
        Json::Obj(_) => raw.get("$numberLong").map(Json::js_number),
        Json::Str(s) => {
            return match chrono::DateTime::parse_from_rfc3339(s) {
                Ok(d) => iso_date(d.timestamp_millis()).unwrap_or_else(|| format!("Date({s})")),
                Err(_) => format!("Date({s})"),
            };
        }
        other => Some(other.js_number()),
    };
    let ms = ms.unwrap_or(f64::NAN);
    // JavaScript dates span ±8.64e15 ms.
    if ms.is_finite()
        && ms.abs() <= 8.64e15
        && let Some(s) = iso_date(ms.trunc() as i64)
    {
        return s;
    }
    format!("Date({})", js_number(ms))
}

fn iso_date(ms: i64) -> Option<String> {
    let d = chrono::DateTime::from_timestamp_millis(ms)?;
    Some(format!("ISODate('{}')", d.format("%Y-%m-%dT%H:%M:%S%.3fZ")))
}

/// Replace every extended-JSON wrapper with its display string, eliding
/// below `max_depth` (`toDisplayValue`).
pub fn to_display(value: &Json, depth: usize, max_depth: usize) -> Json {
    if depth > max_depth {
        return Json::Str("\u{2026}".into());
    }
    if let Some(s) = unwrap_extended_json(value) {
        return Json::Str(s);
    }
    match value {
        Json::Arr(items) => Json::Arr(
            items
                .iter()
                .map(|v| to_display(v, depth + 1, max_depth))
                .collect(),
        ),
        Json::Obj(fields) => Json::Obj(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), to_display(v, depth + 1, max_depth)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// A document as indented, readable JSON; the input itself when it does not
/// parse (`prettyPrintDocument`).
pub fn pretty_print(json: &str) -> String {
    match Json::parse(json) {
        Some(v) => to_display(&v, 0, MAX_DEPTH).stringify_pretty(),
        None => json.to_string(),
    }
}

/// One line for a collapsed row, `_id` first, cut at `limit` characters
/// (`summarizeDocument`).
pub fn summarize(json: &str, limit: usize) -> String {
    let Some(parsed) = Json::parse(json).map(|v| to_display(&v, 0, MAX_DEPTH)) else {
        return json.chars().take(limit).collect();
    };
    let mut entries: Vec<(String, &Json)> = match &parsed {
        Json::Obj(fields) => fields.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Json::Arr(items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        scalar => return scalar.js_string().chars().take(limit).collect(),
    };
    // Stable: only `_id` moves.
    entries.sort_by_key(|(k, _)| k != "_id");

    let mut parts: Vec<String> = Vec::new();
    for (k, v) in entries {
        parts.push(format!("{k}: {}", v.js_string()));
        if parts.join(", ").chars().count() > limit {
            break;
        }
    }
    let line = parts.join(", ");
    if line.chars().count() > limit {
        let mut cut: String = line.chars().take(limit.saturating_sub(1)).collect();
        cut.push('\u{2026}');
        cut
    } else {
        line
    }
}

// ---- mongoTable.js -------------------------------------------------------

/// A page parsed once for the table. A document that is not an object is
/// `None`, so one bad row cannot blank the page (`parseDocuments`).
pub fn parse_documents(docs: &[String]) -> Vec<Option<Vec<(String, Json)>>> {
    docs.iter()
        .map(
            |d| match Json::parse(d).map(|v| to_display(&v, 0, MAX_DEPTH)) {
                Some(Json::Obj(fields)) => Some(fields),
                _ => None,
            },
        )
        .collect()
}

/// The table's columns: `_id` first, then the fields most documents share,
/// first-seen order breaking ties; and how many did not fit
/// (`deriveColumns`).
pub fn derive_columns(
    docs: &[Option<Vec<(String, Json)>>],
    max_columns: usize,
) -> (Vec<String>, usize) {
    let mut seen: Vec<(String, usize)> = Vec::new();
    for doc in docs.iter().flatten() {
        for (field, _) in doc {
            match seen.iter_mut().find(|(f, _)| f == field) {
                Some(slot) => slot.1 += 1,
                None => seen.push((field.clone(), 1)),
            }
        }
    }
    let has_id = seen.iter().any(|(f, _)| f == "_id");
    let mut fields: Vec<(String, usize)> = seen.into_iter().filter(|(f, _)| f != "_id").collect();
    // Stable sort: equal counts keep first-seen order.
    fields.sort_by_key(|f| std::cmp::Reverse(f.1));
    let mut ordered: Vec<String> = Vec::with_capacity(fields.len() + 1);
    if has_id {
        ordered.push("_id".into());
    }
    ordered.extend(fields.into_iter().map(|(f, _)| f));
    let limit = max_columns.max(1);
    let hidden = ordered.len().saturating_sub(limit);
    ordered.truncate(limit);
    (ordered, hidden)
}

/// One cell: blank when the document lacks the field, nested values
/// collapsed (`cellText`).
pub fn cell_text(doc: Option<&Vec<(String, Json)>>, column: &str) -> String {
    doc.and_then(|fields| fields.iter().find(|(k, _)| k == column))
        .map(|(_, v)| v.js_string())
        .unwrap_or_default()
}

/// An index spec's name for its row header: `options.name`, else its keys.
pub fn index_name(spec: &str) -> String {
    let Some(v) = Json::parse(spec) else {
        return String::new();
    };
    if let Some(Json::Str(name)) = v.get("options").and_then(|o| o.get("name")) {
        return name.clone();
    }
    match v.get("keys") {
        Some(Json::Obj(keys)) => keys
            .iter()
            .map(|(k, dir)| {
                let dir = unwrap_extended_json(dir).unwrap_or_else(|| dir.js_string());
                format!("{k}_{dir}")
            })
            .collect::<Vec<_>>()
            .join("_"),
        _ => String::new(),
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
    fn wrappers_unwrap_as_the_vue_helper_does() {
        let u = |s: &str| unwrap_extended_json(&Json::parse(s).unwrap());
        assert_eq!(u(r#"{"$oid":"abc"}"#).unwrap(), "ObjectId('abc')");
        assert_eq!(
            u(r#"{"$date":{"$numberLong":"0"}}"#).unwrap(),
            "ISODate('1970-01-01T00:00:00.000Z')"
        );
        assert_eq!(
            u(r#"{"$date":"2024-05-06T07:08:09.010Z"}"#).unwrap(),
            "ISODate('2024-05-06T07:08:09.010Z')"
        );
        assert_eq!(u(r#"{"$date":{"$numberLong":"x"}}"#).unwrap(), "Date(NaN)");
        assert_eq!(
            u(r#"{"$numberLong":"9007199254740993"}"#).unwrap(),
            "9007199254740993"
        );
        assert_eq!(u(r#"{"$numberDecimal":"1.10"}"#).unwrap(), "1.10");
        assert_eq!(
            u(r#"{"$binary":{"base64":"AA==","subType":"04"}}"#).unwrap(),
            "Binary('04')"
        );
        assert_eq!(
            u(r#"{"$timestamp":{"t":5,"i":2}}"#).unwrap(),
            "Timestamp(5, 2)"
        );
        assert_eq!(
            u(r#"{"$regularExpression":{"pattern":"^a","options":"i"}}"#).unwrap(),
            "/^a/i"
        );
        assert_eq!(u(r#"{"$minKey":1}"#).unwrap(), "MinKey");
        assert_eq!(u(r#"{"a":{"$oid":"x"}}"#), None);
        assert_eq!(u(r#"{}"#), None);
        assert_eq!(u(r#"[1]"#), None);
    }

    #[test]
    fn pretty_print_matches_json_stringify() {
        let doc = r#"{"_id":{"$oid":"6650"},"n":{"$numberInt":"3"},"tags":["a"],"e":{},"l":[],"s":"q\"x"}"#;
        assert_eq!(
            pretty_print(doc),
            "{\n  \"_id\": \"ObjectId('6650')\",\n  \"n\": \"3\",\n  \"tags\": [\n    \"a\"\n  ],\n  \"e\": {},\n  \"l\": [],\n  \"s\": \"q\\\"x\"\n}"
        );
        assert_eq!(pretty_print("not json"), "not json");
        assert_eq!(pretty_print("1.5"), "1.5");
    }

    #[test]
    fn deep_values_are_elided() {
        let mut s = String::from("1");
        for _ in 0..20 {
            s = format!("[{s}]");
        }
        let out = pretty_print(&s);
        assert!(out.contains("\"\u{2026}\""));
    }

    #[test]
    fn summary_leads_with_id_and_is_cut() {
        let doc = r#"{"name":"x","_id":{"$oid":"1"},"arr":[1,2],"o":{"a":1},"n":null}"#;
        assert_eq!(
            summarize(doc, 120),
            "_id: ObjectId('1'), name: x, arr: [2], o: {\u{2026}}, n: null"
        );
        let long = format!(r#"{{"a":"{}"}}"#, "x".repeat(200));
        let s = summarize(&long, 120);
        assert_eq!(s.chars().count(), 120);
        assert!(s.ends_with('\u{2026}'));
        assert_eq!(summarize("not json at all", 3), "not");
        assert_eq!(summarize("42", 120), "42");
    }

    #[test]
    fn columns_put_id_first_then_the_common_fields() {
        let docs = parse_documents(&[
            r#"{"b":1,"_id":1,"a":1}"#.into(),
            r#"{"a":2,"c":2}"#.into(),
            "[1]".into(),
            "bad".into(),
        ]);
        assert!(docs[2].is_none() && docs[3].is_none());
        let (cols, hidden) = derive_columns(&docs, 12);
        assert_eq!(cols, ["_id", "a", "b", "c"]);
        assert_eq!(hidden, 0);
        let (cols, hidden) = derive_columns(&docs, 2);
        assert_eq!(cols, ["_id", "a"]);
        assert_eq!(hidden, 2);
        assert_eq!(derive_columns(&[], 12), (vec![], 0));
    }

    #[test]
    fn cells_collapse_nested_values() {
        let docs = parse_documents(&[
            r#"{"_id":{"$oid":"9"},"a":[1,2,3],"o":{"x":1},"n":null,"f":1.5,"t":true}"#.into(),
        ]);
        let d = docs[0].as_ref();
        assert_eq!(cell_text(d, "_id"), "ObjectId('9')");
        assert_eq!(cell_text(d, "a"), "[3]");
        assert_eq!(cell_text(d, "o"), "{\u{2026}}");
        assert_eq!(cell_text(d, "n"), "null");
        assert_eq!(cell_text(d, "f"), "1.5");
        assert_eq!(cell_text(d, "t"), "true");
        assert_eq!(cell_text(d, "missing"), "");
        assert_eq!(cell_text(None, "a"), "");
    }

    #[test]
    fn numbers_print_like_javascript() {
        assert_eq!(js_number(3.0), "3");
        assert_eq!(js_number(-0.5), "-0.5");
        assert_eq!(js_number(f64::NAN), "NaN");
        assert_eq!(Json::Str(" 12 ".into()).js_number(), 12.0);
        assert!(Json::Str("x".into()).js_number().is_nan());
    }

    #[test]
    fn indexes_are_named_by_their_options_or_keys() {
        assert_eq!(
            index_name(r#"{"keys":{"_id":{"$numberInt":"1"}},"options":{"name":"_id_"}}"#),
            "_id_"
        );
        assert_eq!(
            index_name(r#"{"keys":{"a":{"$numberInt":"1"},"b":{"$numberInt":"-1"}}}"#),
            "a_1_b_-1"
        );
        assert_eq!(index_name("bad"), "");
    }
}
