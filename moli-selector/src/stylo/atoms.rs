use std::{cell::RefCell, collections::HashMap};

use style::{Atom, LocalName, Namespace, values::AtomIdent};

pub(super) fn lang_matches(actual: &str, expected: &str) -> bool {
    style::servo::selector_parser::extended_filtering(actual, expected)
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
    use super::QueryAtomCache;

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
