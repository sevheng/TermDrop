//! A stable colour per host group. Port of `src/utils/groupAccent.js`.
//!
//! Hashed rather than assigned in order, so a group keeps its colour when
//! the ones above it are renamed, deleted or re-sorted: the colour is a
//! property of the name, not of the position. The hash is the JS one, over
//! UTF-16 code units with 32-bit wrap-around, so a group has the same hue in
//! both apps.

/// How many hues there are: `--td-tag-1` .. `--td-tag-8`.
pub const GROUP_ACCENT_COUNT: usize = 8;

/// Which hue a group name gets, `0..GROUP_ACCENT_COUNT`.
pub fn group_accent_index(name: &str) -> usize {
    let mut hash: i32 = 0;
    for unit in name.encode_utf16() {
        // `(hash << 5) - hash + charCode`, then `hash |= 0`.
        hash = (hash << 5).wrapping_sub(hash).wrapping_add(i32::from(unit));
    }
    // `Math.abs` on a double, so i32::MIN does not overflow.
    (i64::from(hash).unsigned_abs() % GROUP_ACCENT_COUNT as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_lands_in_range() {
        let long = "x".repeat(500);
        for n in [
            "",
            "a",
            "Production",
            "staging-eu-west-1",
            "日本語",
            "1",
            &long,
        ] {
            assert!(group_accent_index(n) < GROUP_ACCENT_COUNT);
        }
    }

    #[test]
    fn is_stable_for_the_same_name() {
        assert_eq!(
            group_accent_index("Production"),
            group_accent_index("Production")
        );
    }

    #[test]
    fn is_a_property_of_the_name_not_the_position() {
        let before = group_accent_index("databases");
        for other in ["web", "cache", "staging"] {
            group_accent_index(other);
        }
        assert_eq!(group_accent_index("databases"), before);
    }

    #[test]
    fn spreads_a_realistic_set_of_names_over_several_hues() {
        let names = ["web", "db", "cache", "staging", "prod", "eu", "us", "dev"];
        let used: std::collections::HashSet<_> =
            names.iter().map(|n| group_accent_index(n)).collect();
        assert!(used.len() > 1);
    }

    /// Values computed with `groupAccentIndex` in the Vue app, so a group is
    /// the same colour in both.
    #[test]
    fn matches_the_js_hash() {
        assert_eq!(group_accent_index(""), 0);
        assert_eq!(group_accent_index("a"), 97 % 8);
        let long = "x".repeat(500);
        for (name, js) in [
            ("Production", 7),
            ("staging-eu-west-1", 6),
            ("日本語", 7),
            (long.as_str(), 0),
            ("web", 4),
            ("db", 6),
            ("cache", 2),
            // Outside the BMP: two UTF-16 units, as JS counts it.
            ("🚀 rockets", 6),
            ("databases", 0),
        ] {
            assert_eq!(group_accent_index(name), js, "{name}");
        }
    }
}
