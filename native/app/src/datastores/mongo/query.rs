//! The document browser's query controls and header line. Port of
//! `src/utils/mongoQuery.js` and the stats line of `MongoDocumentsView.vue`.

use termdrop_core::mongodb::MAX_FIND_LIMIT;

use super::bson::{Json, js_number};
use crate::datastores::common::format_count;

/// The core's cap on a page (`MAX_PAGE_SIZE`, pinned by a test to
/// `mongodb::MAX_FIND_LIMIT`).
pub const MAX_PAGE_SIZE: u64 = 200;
pub const DEFAULT_PAGE_SIZE: u64 = 25;
/// The page-size menu.
pub const PAGE_SIZES: [u64; 4] = [25, 50, 100, 200];

/// Check a filter or sort box before it reaches the core, so a typo is
/// reported next to the field. `None` when valid (empty is valid).
pub fn validate_json_input(text: &str, field: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    match serde_json::from_str::<Json>(trimmed) {
        Err(e) => Some(format!("{field} is not valid JSON: {e}")),
        Ok(Json::Obj(_)) => None,
        Ok(_) => Some(format!(
            "{field} must be a JSON object, e.g. {{\"field\": \"value\"}}"
        )),
    }
}

/// A requested page size clamped to what the core returns.
pub fn clamp_page_size(size: u64) -> u64 {
    if size < 1 {
        DEFAULT_PAGE_SIZE
    } else {
        size.min(MAX_PAGE_SIZE)
    }
}

/// How many documents to skip for a zero-based page.
pub fn skip_for(page: u64, page_size: u64) -> u64 {
    page.saturating_mul(clamp_page_size(page_size))
}

/// The last zero-based page for a total; 0 for an empty result, so Prev and
/// Next have something to clamp against.
pub fn last_page(total: u64, page_size: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    total.div_ceil(clamp_page_size(page_size)) - 1
}

/// "1–25 of ~10,000,000", or the empty wording.
pub fn describe_range(
    page: u64,
    page_size: u64,
    returned: usize,
    total: u64,
    estimated: bool,
) -> String {
    if returned == 0 {
        return if total == 0 {
            "No documents".into()
        } else {
            "No documents on this page".into()
        };
    }
    let start = skip_for(page, page_size) + 1;
    let end = start + returned as u64 - 1;
    format!(
        "{}\u{2013}{} of {}{}",
        format_count(start),
        format_count(end),
        if estimated { "~" } else { "" },
        format_count(total)
    )
}

/// The header's "12 KB, 2 indexes" from `mongodb_collection_stats`, or
/// `None` when the size is not there (stats are a nicety).
pub fn stats_summary(raw: &str) -> Option<String> {
    let stats = Json::parse(raw)?;
    // A number may come wrapped ({"$numberInt": "3"}): take its first value.
    let num = |key: &str| -> f64 {
        match stats.get(key) {
            None => f64::NAN,
            Some(Json::Obj(fields)) => fields.first().map_or(f64::NAN, |(_, v)| v.js_number()),
            Some(v) => v.js_number(),
        }
    };
    let size = num("size");
    if !size.is_finite() {
        return None;
    }
    let indexes = num("nindexes");
    Some(format!(
        "{}, {} index{}",
        stats_bytes(size),
        js_number(indexes),
        if indexes == 1.0 { "" } else { "es" }
    ))
}

/// The documents view's own `formatBytes`: "512 B", "1.5 KB", "12 KB".
/// Not `common::format_bytes`, which always keeps one decimal.
pub fn stats_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 && unit > 0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{} {}", js_number(value.round()), UNITS[unit])
    }
}

/// The trimmed text of a query box, `None` when empty (`.trim() || null`).
pub fn query_arg(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_string())
}

const _: () = assert!(MAX_PAGE_SIZE as i64 == MAX_FIND_LIMIT);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_inputs_must_be_objects() {
        assert_eq!(validate_json_input("", "filter"), None);
        assert_eq!(validate_json_input("  ", "filter"), None);
        assert_eq!(validate_json_input(r#"{"a": 1}"#, "filter"), None);
        assert!(
            validate_json_input("{a: 1}", "filter")
                .unwrap()
                .starts_with("filter is not valid JSON: ")
        );
        assert_eq!(
            validate_json_input("[1]", "sort").unwrap(),
            r#"sort must be a JSON object, e.g. {"field": "value"}"#
        );
        assert!(validate_json_input("null", "sort").is_some());
    }

    #[test]
    fn pages_clamp_and_count() {
        assert_eq!(clamp_page_size(0), DEFAULT_PAGE_SIZE);
        assert_eq!(clamp_page_size(500), 200);
        assert_eq!(skip_for(3, 25), 75);
        assert_eq!(skip_for(1, 1000), 200);
        assert_eq!(last_page(0, 25), 0);
        assert_eq!(last_page(25, 25), 0);
        assert_eq!(last_page(26, 25), 1);
        assert_eq!(last_page(10_000_000, 200), 49_999);
    }

    #[test]
    fn ranges_read_like_the_vue_footer() {
        assert_eq!(describe_range(0, 25, 0, 0, false), "No documents");
        assert_eq!(
            describe_range(4, 25, 0, 10, false),
            "No documents on this page"
        );
        assert_eq!(
            describe_range(0, 25, 25, 10_000_000, true),
            "1\u{2013}25 of ~10,000,000"
        );
        assert_eq!(
            describe_range(2, 50, 7, 107, false),
            "101\u{2013}107 of 107"
        );
    }

    #[test]
    fn stats_read_wrapped_or_plain_numbers() {
        assert_eq!(
            stats_summary(r#"{"size":{"$numberInt":"12800"},"nindexes":{"$numberInt":"2"}}"#)
                .unwrap(),
            "13 KB, 2 indexes"
        );
        assert_eq!(
            stats_summary(r#"{"size":512,"nindexes":1}"#).unwrap(),
            "512 B, 1 index"
        );
        assert_eq!(
            stats_summary(r#"{"size":{"$numberDouble":"3221225472.0"}}"#).unwrap(),
            "3.0 GB, NaN indexes"
        );
        assert_eq!(stats_summary(r#"{"count":1}"#), None);
        assert_eq!(stats_summary("nope"), None);
    }

    #[test]
    fn stats_bytes_match_the_view() {
        assert_eq!(stats_bytes(0.0), "0 B");
        assert_eq!(stats_bytes(1023.0), "1023 B");
        assert_eq!(stats_bytes(1536.0), "1.5 KB");
        assert_eq!(stats_bytes(20.0 * 1024.0), "20 KB");
        assert_eq!(stats_bytes(5.0 * 1024f64.powi(5)), "5120 TB");
    }

    #[test]
    fn empty_query_boxes_send_nothing() {
        assert_eq!(query_arg("  "), None);
        assert_eq!(query_arg(" {} "), Some("{}".into()));
    }
}
