//! Searching history (`history-and-bookmarks.md` sections 2.4 and 2.10):
//! the rows that answer a query, inline completion from what was typed,
//! and suggestions.

use std::collections::BTreeMap;

use crate::address;

use super::{History, Row};

/// The search that names one host, as Chrome's history page writes "More
/// from this site" (`history_list.ts`: `'host:' + domain`).
pub const HOST_SEARCH: &str = "host:";
/// The schemes an inline completion leaves out when the text typed has none.
const SCHEMES: [&str; 2] = ["https://", "http://"];

impl Row {
    /// The address without its scheme, for completion and channel matching.
    fn bare(&self) -> &str {
        SCHEMES.iter().find_map(|scheme| self.address.strip_prefix(scheme)).unwrap_or(&self.address)
    }
}

impl History {
    /// The rows that answer `query`, newest first: those whose address,
    /// channel title or page title holds it, ignoring case.
    #[must_use]
    pub fn query(&self, query: &str) -> Vec<&Row> {
        let query = query.to_lowercase();
        // `host:` names one host, as Chrome's history search does.
        let host = query.strip_prefix(HOST_SEARCH).map(str::trim);
        let mut rows: Vec<&Row> = self
            .rows
            .iter()
            .filter(|row| match host {
                Some(host) => address::parse(&row.address).is_ok_and(|address| address.host.eq_ignore_ascii_case(host)),
                None => query.is_empty() || [&row.address, &row.channel, &row.title].iter().any(|text| text.to_lowercase().contains(&query)),
            })
            .collect();
        rows.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.address.cmp(&b.address)));
        rows
    }

    /// Whether any day has a row of `address`.
    #[must_use]
    pub fn has_address(&self, address: &str) -> bool {
        self.rows.iter().any(|row| row.address == address)
    }

    /// The address to complete the field with as `typed` is typed: the most
    /// recent address the user typed before that starts with it, ignoring
    /// case, and without its scheme when `typed` has none (section 2.10).
    #[must_use]
    pub fn completion(&self, typed: &str) -> Option<String> {
        if typed.is_empty() {
            return None;
        }
        let lower = typed.to_lowercase();
        let with_scheme = SCHEMES.iter().any(|scheme| lower.starts_with(scheme) || scheme.starts_with(lower.as_str()));
        self.rows
            .iter()
            .filter(|row| row.typed > 0)
            .filter_map(|row| {
                let candidate = if with_scheme { row.address.as_str() } else { row.bare() };
                candidate.to_lowercase().starts_with(&lower).then_some((row.last, candidate))
            })
            .max_by_key(|(last, _)| *last)
            .map(|(_, candidate)| format!("{typed}{}", candidate.get(typed.len()..).unwrap_or_default()))
    }

    /// The rows that answer `typed` for suggestions: one per address, the
    /// most visited first, then the most recent (section 2.10).
    #[must_use]
    pub fn suggest(&self, typed: &str, max: usize) -> Vec<Row> {
        if typed.is_empty() {
            return Vec::new();
        }
        let mut by_address: BTreeMap<&str, Row> = BTreeMap::new();
        for row in self.query(typed) {
            let entry = by_address.entry(row.address.as_str()).or_insert_with(|| Row { visits: 0, ..row.clone() });
            entry.visits = entry.visits.saturating_add(row.visits);
            if row.last > entry.last {
                entry.last = row.last;
            }
        }
        let mut rows: Vec<Row> = by_address.into_values().collect();
        rows.sort_by(|a, b| b.visits.cmp(&a.visits).then_with(|| b.last.cmp(&a.last)));
        rows.truncate(max);
        rows
    }
}
