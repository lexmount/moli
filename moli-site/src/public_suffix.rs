use std::{net::IpAddr, sync::Arc};

use psl_types::{Info, List as Psl, Type};

const LEAF_FLAG: u8 = 1;
const EXCEPTION_FLAG: u8 = 2;
const EDGE_START_MASK: u64 = u32::MAX as u64;
const EDGE_COUNT_MASK: u64 = u16::MAX as u64;

mod tables {
    include!(concat!(env!("OUT_DIR"), "/public_suffix_tables.rs"));
}

/// Allocation-free view of Moli's vendored Public Suffix List snapshot.
///
/// The rule trie is generated into compact read-only tables at build time. An
/// instance is zero-sized; clones share the executable's static table pages.
#[derive(Clone, Copy, Debug, Default)]
pub struct StaticPublicSuffixList;

static PUBLIC_SUFFIX_LIST: std::sync::LazyLock<Arc<StaticPublicSuffixList>> =
    std::sync::LazyLock::new(|| Arc::new(StaticPublicSuffixList));

impl StaticPublicSuffixList {
    fn child(node_index: usize, label: &[u8]) -> Option<usize> {
        let metadata = *tables::NODE_METADATA.get(node_index)?;
        let edge_start = (metadata & EDGE_START_MASK) as usize;
        let edge_count = ((metadata >> 32) & EDGE_COUNT_MASK) as usize;
        let edges = tables::EDGES.get(edge_start..edge_start + edge_count)?;

        edges
            .binary_search_by(|encoded| edge_label(*encoded).cmp(label))
            .ok()
            .map(|relative_index| edge_start + relative_index + 1)
    }

    fn flags(node_index: usize) -> u8 {
        (tables::NODE_METADATA[node_index] >> 48) as u8
    }
}

impl Psl for StaticPublicSuffixList {
    fn find<'a, T>(&self, mut labels: T) -> Info
    where
        T: Iterator<Item = &'a [u8]>,
    {
        let Some(first_label) = labels.next() else {
            return Info { len: 0, typ: None };
        };
        let mut info = Info {
            len: first_label.len(),
            typ: None,
        };
        let Some(mut node_index) = Self::child(0, first_label) else {
            return info;
        };
        if Self::flags(node_index) & LEAF_FLAG != 0 {
            info.typ = Some(Type::Icann);
        }

        let mut len_so_far = info.len;
        for label in labels {
            let label_plus_dot = label.len() + 1;

            // A wildcard at this level remains a valid candidate even when an
            // exact child exists only to hold a deeper rule. Keep the wildcard
            // result, then continue down the exact branch in search of a
            // longer match. Choosing the exact branch up front would make a
            // deeper rule incorrectly shadow its parent's wildcard.
            if let Some(wildcard_node) = Self::child(node_index, b"*") {
                let flags = Self::flags(wildcard_node);
                if flags & LEAF_FLAG != 0 {
                    info.typ = Some(Type::Icann);
                    info.len = len_so_far + label_plus_dot;
                }
            }

            let Some(next_node) = Self::child(node_index, label) else {
                break;
            };
            node_index = next_node;

            let flags = Self::flags(node_index);
            if flags & LEAF_FLAG != 0 {
                info.typ = Some(Type::Icann);
                if flags & EXCEPTION_FLAG != 0 {
                    info.len = len_so_far;
                    break;
                }
                info.len = len_so_far + label_plus_dot;
            }
            len_so_far += label_plus_dot;
        }

        info
    }
}

fn edge_label(encoded: u32) -> &'static [u8] {
    let start = (encoded >> 8) as usize;
    let len = (encoded & u8::MAX as u32) as usize;
    &tables::LABELS[start..start + len]
}

pub(crate) fn is_ip_host(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
}

/// Returns the registrable-site key used for cookie site-data grouping.
pub fn site_key_for_host(host: &str) -> Option<String> {
    let host = host.trim().trim_start_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(registrable_site_host(&host).to_owned())
}

/// Returns whether a host is an explicitly known public suffix.
pub fn host_is_public_suffix(host: &str) -> bool {
    let host = host
        .trim()
        .trim_start_matches('.')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() || is_ip_host(&host) {
        return false;
    }
    StaticPublicSuffixList
        .suffix(host.as_bytes())
        .is_some_and(|suffix| suffix.is_known() && suffix == host.as_bytes())
}

/// Returns the registrable domain for `host`, or the host itself when Chromium
/// would not find an eTLD+1, such as IP literals, localhost, and public suffixes.
pub fn registrable_site_host(host: &str) -> &str {
    if is_ip_host(host) {
        return host;
    }

    let host = host.trim_start_matches('.');
    StaticPublicSuffixList
        .domain(host.as_bytes())
        .and_then(|domain| std::str::from_utf8(domain.as_bytes()).ok())
        .unwrap_or(host)
}

/// Compares two hosts using registrable-domain site semantics.
pub fn same_site_hosts(host_a: &str, host_b: &str) -> bool {
    if is_ip_host(host_a) || is_ip_host(host_b) {
        return host_a == host_b;
    }

    registrable_site_host(host_a) == registrable_site_host(host_b)
}

/// Returns the shared vendored public suffix list used by cookie stores.
pub fn public_suffix_list() -> Arc<StaticPublicSuffixList> {
    Arc::clone(&PUBLIC_SUFFIX_LIST)
}

#[cfg(test)]
pub(super) fn static_table_size_bytes() -> usize {
    std::mem::size_of_val(&tables::NODE_METADATA)
        + std::mem::size_of_val(&tables::EDGES)
        + std::mem::size_of_val(&tables::LABELS)
}

#[cfg(test)]
pub(super) const STATIC_RULE_COUNT: usize = tables::RULE_COUNT;
