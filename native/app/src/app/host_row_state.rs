//! What a host row shows about its connection. Port of
//! `src/utils/hostRowState.js`.
//!
//! Two facts get two signals. `connected` is "there is a live session behind
//! one of this host's tabs"; `active` is "the tab you are looking at is this
//! host's". The old rule was "a tab exists", which stayed green after the
//! session dropped although the store already knew better.

use super::tabs::Tab;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostRowState {
    pub connected: bool,
    pub active: bool,
    pub connecting: bool,
    pub tab_count: usize,
}

pub fn host_row_state(
    tabs: &[Tab],
    active_tab_id: Option<&str>,
    connecting_host_id: Option<i64>,
    host_id: i64,
) -> HostRowState {
    let mine = tabs.iter().filter(|t| t.host_id == Some(host_id));
    let mut state = HostRowState {
        connecting: connecting_host_id == Some(host_id),
        ..Default::default()
    };
    for tab in mine {
        state.tab_count += 1;
        state.connected |= tab.connected;
        state.active |= Some(tab.id.as_str()) == active_tab_id;
    }
    state
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`), so no date crate is needed for one label.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The inverse, for the "Jan 4" label.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Milliseconds since the epoch for `YYYY-MM-DD[T ]HH:MM:SS[.fff][Z|±HH:MM]`.
///
/// SQLite's `CURRENT_TIMESTAMP` has no `T` and no zone and is UTC, which is
/// what the JS meant by appending `Z`. A `T` form without a zone is read as
/// UTC too; JS would have read it as local time, but nothing writes one.
pub fn parse_timestamp_ms(text: &str) -> Option<i64> {
    let s = text.trim();
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(r)?;
        part.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if s.get(4..5)? != "-" || s.get(7..8)? != "-" || !matches!(s.get(10..11)?, "T" | " ") {
        return None;
    }
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut rest = &s[19..];
    let mut millis = 0;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
        let ms: String = frac[..digits]
            .chars()
            .chain("000".chars())
            .take(3)
            .collect();
        millis = ms.parse().ok()?;
        rest = &frac[digits..];
    }
    let offset_min = match rest {
        "" | "Z" | "z" => 0,
        tz => {
            let sign = match tz.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let hhmm = tz[1..].replace(':', "");
            if hhmm.len() != 4 || !hhmm.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            sign * (hhmm[..2].parse::<i64>().ok()? * 60 + hhmm[2..].parse::<i64>().ok()?)
        }
    };
    let days = days_from_civil(y, mo, d);
    let secs = days * 86_400 + h * 3_600 + mi * 60 + sec - offset_min * 60;
    Some(secs * 1000 + millis)
}

/// How long ago, in the shortest form that is still unambiguous. Empty when
/// the host was never connected or the timestamp is nonsense.
pub fn format_age(iso: Option<&str>, now_ms: i64) -> String {
    let Some(then) = iso.filter(|s| !s.is_empty()).and_then(parse_timestamp_ms) else {
        return String::new();
    };
    let secs = (now_ms - then).div_euclid(1000);
    // A clock skewed into the future reads as now.
    if secs < 60 {
        return "just now".into();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    if days < 30 {
        return format!("{days}d ago");
    }
    // Past a month the exact age stops being useful; the date is more so.
    let (_, m, d) = civil_from_days(then.div_euclid(86_400_000));
    format!("{} {d}", MONTHS[(m - 1) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tab_kind::TabKind;

    fn tab(id: &str, host: i64, connected: bool) -> Tab {
        Tab {
            connected,
            ..Tab::new(id, Some(host), TabKind::Ssh, id)
        }
    }

    #[test]
    fn is_not_connected_with_no_tabs() {
        let s = host_row_state(&[], None, None, 1);
        assert!(!s.connected && !s.active);
        assert_eq!(s.tab_count, 0);
    }

    #[test]
    fn stops_calling_a_dropped_session_connected() {
        let s = host_row_state(&[tab("a", 1, false)], None, None, 1);
        assert!(!s.connected);
        assert_eq!(s.tab_count, 1);
    }

    #[test]
    fn counts_a_datastore_tab_as_connected_even_though_it_sets_no_flag() {
        let redis = Tab::new("redis-1", Some(1), TabKind::Redis, "cache");
        assert!(host_row_state(&[redis], None, None, 1).connected);
    }

    #[test]
    fn is_connected_when_any_one_of_several_tabs_is_live() {
        let s = host_row_state(&[tab("a", 1, false), tab("b", 1, true)], None, None, 1);
        assert!(s.connected);
        assert_eq!(s.tab_count, 2);
    }

    #[test]
    fn separates_active_from_connected() {
        let tabs = [tab("a", 1, true), tab("b", 2, true)];
        let one = host_row_state(&tabs, Some("a"), None, 1);
        assert!(one.connected && one.active);
        let two = host_row_state(&tabs, Some("a"), None, 2);
        assert!(two.connected && !two.active);
    }

    #[test]
    fn reports_connecting_from_the_model() {
        assert!(host_row_state(&[], None, Some(7), 7).connecting);
        assert!(!host_row_state(&[], None, Some(7), 8).connecting);
    }

    fn now() -> i64 {
        parse_timestamp_ms("2026-09-10T12:00:00Z").unwrap()
    }

    #[test]
    fn is_empty_when_a_host_has_never_been_connected() {
        assert_eq!(format_age(None, now()), "");
        assert_eq!(format_age(Some(""), now()), "");
    }

    #[test]
    fn reads_in_the_shortest_unambiguous_form() {
        assert_eq!(format_age(Some("2026-09-10T11:59:30Z"), now()), "just now");
        assert_eq!(format_age(Some("2026-09-10T11:30:00Z"), now()), "30m ago");
        assert_eq!(format_age(Some("2026-09-10T09:00:00Z"), now()), "3h ago");
        assert_eq!(format_age(Some("2026-09-05T12:00:00Z"), now()), "5d ago");
    }

    #[test]
    fn switches_to_a_date_once_the_age_stops_being_useful() {
        assert!(format_age(Some("2026-01-04T12:00:00Z"), now()).contains("Jan"));
    }

    #[test]
    fn reads_the_space_separated_form_sqlite_writes() {
        // db.rs stores CURRENT_TIMESTAMP, which has no T and no zone.
        assert_eq!(format_age(Some("2026-09-10 09:00:00"), now()), "3h ago");
    }

    #[test]
    fn does_not_throw_on_nonsense() {
        assert_eq!(format_age(Some("not a date"), now()), "");
    }

    #[test]
    fn treats_a_clock_skewed_into_the_future_as_now() {
        assert_eq!(format_age(Some("2026-09-10T12:05:00Z"), now()), "just now");
    }

    #[test]
    fn timestamps_round_trip_through_the_calendar() {
        assert_eq!(parse_timestamp_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_timestamp_ms("2026-09-10T14:00:00+02:00"),
            parse_timestamp_ms("2026-09-10T12:00:00Z")
        );
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }
}
