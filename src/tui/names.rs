//! Shared id → name cache, filled from list responses, so events, DLQ rows and
//! connections show names without extra requests.

use std::collections::HashMap;

const PREFIX_LENGTH: usize = 8;

#[derive(Debug, Clone, Default)]
pub struct NameCache {
    sources: HashMap<String, String>,
    destinations: HashMap<String, String>,
}

impl NameCache {
    pub fn remember_sources<'a>(&mut self, pairs: impl IntoIterator<Item = (&'a str, &'a str)>) {
        for (id, name) in pairs {
            self.sources.insert(id.to_string(), name.to_string());
        }
    }

    pub fn remember_destinations<'a>(
        &mut self,
        pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) {
        for (id, name) in pairs {
            self.destinations.insert(id.to_string(), name.to_string());
        }
    }

    pub fn source(&self, id: &str) -> String {
        self.sources
            .get(id)
            .cloned()
            .unwrap_or_else(|| short_id(id))
    }

    pub fn destination(&self, id: &str) -> String {
        self.destinations
            .get(id)
            .cloned()
            .unwrap_or_else(|| short_id(id))
    }
}

pub fn short_id(id: &str) -> String {
    id.chars().take(PREFIX_LENGTH).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_ids_resolve_and_unknown_ones_show_a_prefix() {
        let mut names = NameCache::default();
        names.remember_sources([("0198c9f0-aaaa", "stripe-prod")]);
        names.remember_destinations([("0198c9f0-bbbb", "billing")]);
        assert_eq!(names.source("0198c9f0-aaaa"), "stripe-prod");
        assert_eq!(names.destination("0198c9f0-bbbb"), "billing");
        assert_eq!(names.source("0198c9f0-cccc-7000"), "0198c9f0");
        assert_eq!(names.destination("short"), "short");
    }

    #[test]
    fn a_rename_replaces_the_cached_name() {
        let mut names = NameCache::default();
        names.remember_sources([("id", "old")]);
        names.remember_sources([("id", "new")]);
        assert_eq!(names.source("id"), "new");
    }
}
