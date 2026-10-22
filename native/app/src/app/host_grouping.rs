//! Filtering and grouping the host list. Port of `src/utils/hostGrouping.js`.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::backend::HostSummary;

/// The text a host can be found by.
///
/// Datastore hosts have empty `host` and `username` columns, so the address
/// you actually connect to has to be in here or searching for it returns
/// nothing. `address` is the *display* form, credentials masked, so a stored
/// password can never be matched against, nor confirmed by a match.
pub fn host_search_text(host: &HostSummary) -> String {
    [
        host.name.as_str(),
        host.host.as_str(),
        host.username.as_str(),
        host.group.as_str(),
        host.address.as_str(),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase()
}

/// Hosts matching a query. An empty or blank query matches everything.
pub fn filter_hosts<'a>(hosts: &'a [HostSummary], query: &str) -> Vec<&'a HostSummary> {
    let q = query.trim().to_lowercase();
    hosts
        .iter()
        .filter(|h| q.is_empty() || host_search_text(h).contains(&q))
        .collect()
}

/// Ungrouped (the empty name) sorts last; everything else alphabetically,
/// ignoring case the way `localeCompare` does for these names.
pub fn compare_group_names(a: &str, b: &str) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a
            .to_lowercase()
            .cmp(&b.to_lowercase())
            .then_with(|| a.cmp(b)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostGroup<'a> {
    pub name: String,
    pub hosts: Vec<&'a HostSummary>,
}

/// Hosts grouped for display, in display order. A `Vec`, not a map: the Vue
/// version learnt that an object hoists integer-like keys, so groups `2` and
/// `10` rendered in the wrong order. Groups the user created but has not
/// filled yet still show.
pub fn group_hosts<'a>(hosts: &[&'a HostSummary], custom_groups: &[String]) -> Vec<HostGroup<'a>> {
    let mut by_name: BTreeMap<&str, Vec<&'a HostSummary>> = BTreeMap::new();
    for host in hosts {
        by_name.entry(host.group.as_str()).or_default().push(host);
    }
    for name in custom_groups {
        by_name.entry(name.as_str()).or_default();
    }
    let mut groups: Vec<HostGroup<'a>> = by_name
        .into_iter()
        .map(|(name, hosts)| HostGroup {
            name: name.to_string(),
            hosts,
        })
        .collect();
    groups.sort_by(|a, b| compare_group_names(&a.name, &b.name));
    groups
}

/// Every group name in play, for a "move to group" menu.
pub fn group_names(hosts: &[HostSummary], custom_groups: &[String]) -> Vec<String> {
    let mut names: Vec<String> = hosts.iter().map(|h| h.group.clone()).collect();
    names.extend(custom_groups.iter().cloned());
    names.sort_by(|a, b| compare_group_names(a, b));
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::HostRecord;

    fn ssh() -> HostSummary {
        HostRecord {
            id: 1,
            name: "web-01".into(),
            host: "10.0.1.4".into(),
            username: "deploy".into(),
            group: Some("prod".into()),
            ..Default::default()
        }
        .summary()
    }
    fn mongo() -> HostSummary {
        HostRecord {
            id: 2,
            name: "orders".into(),
            mongo_uri: Some("mongodb://db.internal:27017".into()),
            ..Default::default()
        }
        .summary()
    }
    fn redis() -> HostSummary {
        HostRecord {
            id: 3,
            name: "cache".into(),
            redis_uri: Some("redis://@10.0.9.9:6380/0".into()),
            ..Default::default()
        }
        .summary()
    }
    fn vnc() -> HostSummary {
        HostRecord {
            id: 4,
            name: "kiosk".into(),
            vnc_host: Some("10.0.9.9".into()),
            vnc_port: Some(5901),
            ..Default::default()
        }
        .summary()
    }
    fn in_group(id: i64, group: &str) -> HostSummary {
        HostRecord {
            id,
            group: Some(group.into()),
            ..Default::default()
        }
        .summary()
    }
    fn ids(hosts: &[&HostSummary]) -> Vec<i64> {
        hosts.iter().map(|h| h.id).collect()
    }

    #[test]
    fn includes_the_address_of_a_datastore_host() {
        assert!(host_search_text(&mongo()).contains("db.internal"));
        assert!(host_search_text(&redis()).contains("10.0.9.9"));
    }

    #[test]
    fn never_exposes_a_stored_password_to_the_search() {
        let with_secret = HostRecord {
            name: "x".into(),
            redis_uri: Some("redis://:hunter2@h:6379".into()),
            ..Default::default()
        }
        .summary();
        assert!(!host_search_text(&with_secret).contains("hunter2"));
    }

    #[test]
    fn finds_a_vnc_host_by_the_address_you_actually_connect_to() {
        assert!(host_search_text(&vnc()).contains("10.0.9.9:5901"));
        let hosts = [ssh(), vnc()];
        assert_eq!(ids(&filter_hosts(&hosts, "10.0.9.9")), [4]);
    }

    #[test]
    fn returns_everything_for_an_empty_query() {
        let hosts = [ssh(), mongo(), redis()];
        assert_eq!(filter_hosts(&hosts, "").len(), 3);
        assert_eq!(filter_hosts(&hosts, "   ").len(), 3);
    }

    #[test]
    fn finds_a_datastore_host_by_its_address() {
        let hosts = [ssh(), mongo(), redis()];
        assert_eq!(ids(&filter_hosts(&hosts, "10.0.9.9")), [3]);
        assert_eq!(ids(&filter_hosts(&hosts, "db.internal")), [2]);
    }

    #[test]
    fn still_finds_hosts_by_name_user_address_and_group() {
        let hosts = [ssh(), mongo(), redis()];
        for q in ["web", "deploy", "prod", "10.0.1.4"] {
            assert_eq!(ids(&filter_hosts(&hosts, q)), [1], "query {q}");
        }
    }

    #[test]
    fn ignores_case() {
        let hosts = [ssh(), mongo(), redis()];
        assert_eq!(ids(&filter_hosts(&hosts, "WEB-01")), [1]);
    }

    #[test]
    fn handles_missing_input() {
        assert!(filter_hosts(&[], "x").is_empty());
    }

    #[test]
    fn preserves_the_comparator_order_for_numeric_group_names() {
        // The bug this replaces: an object hoisted "2" before "10".
        let hosts = [in_group(1, "10"), in_group(2, "2"), in_group(3, "web")];
        let refs: Vec<_> = hosts.iter().collect();
        let names: Vec<_> = group_hosts(&refs, &[])
            .into_iter()
            .map(|g| g.name)
            .collect();
        assert_eq!(names, ["10", "2", "web"]);
    }

    #[test]
    fn puts_ungrouped_last() {
        let hosts = [in_group(1, ""), in_group(2, "web")];
        let refs: Vec<_> = hosts.iter().collect();
        let names: Vec<_> = group_hosts(&refs, &[])
            .into_iter()
            .map(|g| g.name)
            .collect();
        assert_eq!(names, ["web", ""]);
    }

    #[test]
    fn keeps_every_host_in_exactly_one_group() {
        let hosts = [in_group(1, "a"), in_group(2, "a"), in_group(3, "b")];
        let refs: Vec<_> = hosts.iter().collect();
        let groups = group_hosts(&refs, &[]);
        let mut all: Vec<i64> = groups.iter().flat_map(|g| ids(&g.hosts)).collect();
        all.sort();
        assert_eq!(all, [1, 2, 3]);
        assert_eq!(
            groups.iter().find(|g| g.name == "a").unwrap().hosts.len(),
            2
        );
    }

    #[test]
    fn shows_a_created_but_empty_group() {
        let hosts = [in_group(1, "web")];
        let refs: Vec<_> = hosts.iter().collect();
        let groups = group_hosts(&refs, &["staging".to_string()]);
        let names: Vec<_> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["staging", "web"]);
        assert!(
            groups
                .iter()
                .find(|g| g.name == "staging")
                .unwrap()
                .hosts
                .is_empty()
        );
    }

    #[test]
    fn handles_no_hosts_at_all() {
        assert!(group_hosts(&[], &[]).is_empty());
    }

    #[test]
    fn lists_every_name_with_ungrouped_last() {
        let hosts = [in_group(1, "b"), in_group(2, ""), in_group(3, "a")];
        assert_eq!(group_names(&hosts, &[]), ["a", "b", ""]);
    }

    #[test]
    fn sorts_ungrouped_last_whichever_side_it_is_on() {
        assert_eq!(compare_group_names("", "a"), Ordering::Greater);
        assert_eq!(compare_group_names("a", ""), Ordering::Less);
    }
}
