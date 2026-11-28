use std::{cell::RefCell, collections::HashMap};

use style::{Atom, LocalName, Namespace, values::AtomIdent};

pub(super) fn lang_matches(tag: &str, range: &str) -> bool {
    if range.is_empty() {
        return tag.is_empty();
    }

    let mut tag_subtags = tag.split('-');
    let language = tag_subtags.next().unwrap_or_default();
    // Validate the whole tag before matching. A short range cannot match a tag
    // whose remaining subtags contain invalid characters or lengths.
    if !(1..=8).contains(&language.len())
        || !language.bytes().all(|byte| byte.is_ascii_alphabetic())
        || !tag_subtags.clone().all(valid_subtag)
    {
        return false;
    }

    let mut range_subtags = range.split('-');
    let first_range_subtag = range_subtags.next().unwrap_or_default();
    if first_range_subtag != "*" && !first_range_subtag.eq_ignore_ascii_case(language) {
        return false;
    }

    // RFC 4647 extended filtering skips unmatched subtags until the same range
    // subtag matches. A nonmatching singleton ends the search.
    for range_subtag in range_subtags.filter(|subtag| *subtag != "*") {
        if !valid_subtag(range_subtag) {
            return false;
        }
        let matched = tag_subtags
            .by_ref()
            .find(|subtag| subtag.len() == 1 || subtag.eq_ignore_ascii_case(range_subtag));
        if !matched.is_some_and(|subtag| subtag.eq_ignore_ascii_case(range_subtag)) {
            return false;
        }
    }
    true
}

fn valid_subtag(subtag: &str) -> bool {
    (1..=8).contains(&subtag.len()) && subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

#[derive(Debug, Default)]
pub(super) struct QueryAtomCache {
    atoms: RefCell<HashMap<String, Box<Atom>>>,
    atom_idents: RefCell<HashMap<String, Box<AtomIdent>>>,
    local_names: RefCell<HashMap<String, Box<LocalName>>>,
    namespaces: RefCell<HashMap<String, Box<Namespace>>>,
}

impl QueryAtomCache {
    pub(super) fn atom(&self, value: &str) -> &Atom {
        intern_cached(value, &self.atoms, |value| Atom::from(value))
    }

    pub(super) fn atom_ident(&self, value: &str) -> &AtomIdent {
        intern_cached(value, &self.atom_idents, |value| AtomIdent::from(value))
    }

    pub(super) fn local_name(&self, value: &str) -> &LocalName {
        intern_cached(value, &self.local_names, |value| LocalName::from(value))
    }

    pub(super) fn namespace(&self, value: &str) -> &Namespace {
        intern_cached(value, &self.namespaces, |value| Namespace::from(value))
    }

    #[cfg(test)]
    fn local_name_len(&self) -> usize {
        self.local_names.borrow().len()
    }
}

fn intern_cached<'cache, T, F>(
    value: &str,
    cache: &'cache RefCell<HashMap<String, Box<T>>>,
    make: F,
) -> &'cache T
where
    F: FnOnce(&str) -> T,
{
    let mut guard = cache.borrow_mut();
    let entry = guard
        .entry(value.to_owned())
        .or_insert_with(|| Box::new(make(value)));
    let ptr = &**entry as *const T;
    drop(guard);

    // SAFETY: QueryAtomCache never removes entries. A returned reference is
    // tied to &self, so the cache cannot be dropped while callers can still use
    // the reference. HashMap rehashing may move the Box values, but the heap
    // allocation containing T remains stable.
    unsafe { &*ptr }
}

#[cfg(test)]
mod tests {
    use super::{QueryAtomCache, lang_matches};

    #[test]
    fn language_filter_matches_subtags_in_order() {
        for (tag, range, matches) in [
            ("en-Latn-US", "en-US", true),
            ("en-Latn-US-posix", "en-posix", true),
            ("en-Latn-US", "en-GB", false),
            ("en-Latn-US", "*-us", true),
            ("en-US-posix", "en-*-posix", true),
            ("en-US-x-private", "en-private", false),
            ("en-US-x-private", "en-x-private", true),
            ("fr-ninechars", "fr", false),
            ("fr-ninechars", "*", false),
            ("en--US", "en", false),
            ("fr-é", "fr", false),
            ("en_US", "*", false),
            ("en-US", "en,-US", false),
            ("en-US", "en--US", false),
            ("und", "*", true),
            ("", "*", false),
            ("", "", true),
        ] {
            assert_eq!(lang_matches(tag, range), matches, "{tag:?} and {range:?}");
        }
    }

    #[test]
    fn query_atom_cache_reuses_entries_without_global_interning() {
        let cache = QueryAtomCache::default();
        cache.local_name("custom-one");
        cache.local_name("custom-one");
        assert_eq!(cache.local_name_len(), 1);

        cache.local_name("custom-two");
        assert_eq!(cache.local_name_len(), 2);
    }
}
