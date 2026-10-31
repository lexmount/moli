//! Renderer-independent Web Storage data, mutation events, and SQLite persistence.

use std::{
    collections::{HashMap, VecDeque},
    fmt,
    sync::{Arc, Weak},
};

use anyhow::Result;
use moli_storage_key::{
    MoliStorageKey, storage_key_for_origin_and_top_level_site, storage_key_prefix_for_origin,
};
use parking_lot::Mutex;

fn utf16_units(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

fn string_from_utf16_units_lossy(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

mod sqlite;
mod store;
pub use sqlite::{
    SqliteWebStorageOwner, WebStorageAreaLease, import_sqlite_web_storage,
    new_lazy_sqlite_web_storage_store, new_sqlite_web_storage_store,
};
pub use store::{deep_clone_shared_web_storage_store, new_shared_web_storage_store};

const WEB_STORAGE_QUOTA_BYTES: usize = 5 * 1024 * 1024;

pub fn web_storage_partitioned_area_key(origin: &str, top_level_site: &str) -> String {
    storage_key_for_origin_and_top_level_site(origin, top_level_site)
}

pub fn web_storage_area_key_for_storage_key(storage_key: &MoliStorageKey) -> String {
    storage_key.serialized_storage_key()
}

fn web_storage_area_key_prefix_for_origin(origin: &str) -> String {
    storage_key_prefix_for_origin(origin)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WebStorageString {
    units: Arc<[u16]>,
}

#[derive(Debug, Default, Clone)]
struct MemoryWebStorageArea {
    values: HashMap<WebStorageString, WebStorageString>,
    size: usize,
}

#[derive(Debug, Default, Clone)]
struct MemoryWebStorageBackend {
    origins: HashMap<String, MemoryWebStorageArea>,
}

enum WebStorageBackend {
    Memory(Mutex<MemoryWebStorageBackend>),
    Sqlite(sqlite::SqliteBackend),
}

pub struct WebStorageStore {
    backend: WebStorageBackend,
    mutation_subscribers: Mutex<Vec<WebStorageMutationSubscriber>>,
}

pub type SharedWebStorageStore = Arc<WebStorageStore>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebStorageMutation {
    ItemAdded {
        area_key: String,
        key: WebStorageString,
        value: WebStorageString,
    },
    ItemUpdated {
        area_key: String,
        key: WebStorageString,
        old_value: WebStorageString,
        new_value: WebStorageString,
    },
    ItemRemoved {
        area_key: String,
        key: WebStorageString,
        old_value: WebStorageString,
    },
    ItemsCleared {
        area_key: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebStorageAreaKind {
    Local,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebStorageMutationRecord {
    pub area_kind: WebStorageAreaKind,
    pub mutation: WebStorageMutation,
}

struct WebStorageMutationSubscriber {
    area_kind: WebStorageAreaKind,
    queue: Weak<Mutex<VecDeque<WebStorageMutationRecord>>>,
}

#[derive(Clone)]
pub struct WebStorageMutationSubscription {
    queue: Arc<Mutex<VecDeque<WebStorageMutationRecord>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebStorageMutationError {
    QuotaExceeded,
    Persistence(String),
}

impl fmt::Display for WebStorageMutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QuotaExceeded => f.write_str("localStorage quota exceeded"),
            Self::Persistence(message) => write!(f, "localStorage persistence failed: {message}"),
        }
    }
}

impl std::error::Error for WebStorageMutationError {}

impl fmt::Debug for WebStorageMutationSubscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebStorageMutationSubscription")
            .field("pending", &self.queue.lock().len())
            .finish()
    }
}

impl PartialEq for WebStorageMutationSubscription {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.queue, &other.queue)
    }
}

impl Eq for WebStorageMutationSubscription {}

impl WebStorageMutationSubscription {
    pub fn drain(&self) -> Vec<WebStorageMutationRecord> {
        self.queue.lock().drain(..).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.lock().is_empty()
    }
}

impl WebStorageString {
    pub fn from_utf16_units(units: impl Into<Vec<u16>>) -> Self {
        Self {
            units: units.into().into(),
        }
    }

    pub fn from_utf8(value: &str) -> Self {
        Self::from_utf16_units(utf16_units(value))
    }

    pub fn as_units(&self) -> &[u16] {
        &self.units
    }

    pub fn to_string_lossy(&self) -> String {
        string_from_utf16_units_lossy(&self.units)
    }

    fn usage_bytes(&self) -> usize {
        let mut bytes = 0usize;
        for decoded in char::decode_utf16(self.units.iter().copied()) {
            let Ok(character) = decoded else {
                return self.units.len().saturating_mul(2);
            };
            bytes = bytes.saturating_add(character.len_utf8());
        }
        bytes
    }
}

impl std::borrow::Borrow<[u16]> for WebStorageString {
    fn borrow(&self) -> &[u16] {
        self.as_units()
    }
}

impl From<&str> for WebStorageString {
    fn from(value: &str) -> Self {
        Self::from_utf8(value)
    }
}

impl MemoryWebStorageArea {
    fn get_item_utf16(&self, key: &[u16]) -> Option<Vec<u16>> {
        self.values.get(key).map(|value| value.as_units().to_vec())
    }

    fn key_utf16(&self, index: usize) -> Option<Vec<u16>> {
        let mut keys = self.values.keys().collect::<Vec<_>>();
        keys.sort();
        keys.get(index).map(|key| key.as_units().to_vec())
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn contains_key_utf16(&self, key: &[u16]) -> bool {
        self.values.contains_key(key)
    }

    fn sorted_keys_utf16(&self) -> Vec<Vec<u16>> {
        let mut keys = self.values.keys().collect::<Vec<_>>();
        keys.sort();
        keys.into_iter()
            .map(|key| key.as_units().to_vec())
            .collect()
    }
}

fn updated_storage_size(
    current_size: usize,
    previous_value: Option<&WebStorageString>,
    new_value_size: usize,
) -> usize {
    current_size
        .saturating_sub(previous_value.map_or(0, WebStorageString::usage_bytes))
        .saturating_add(new_value_size)
}

impl MemoryWebStorageBackend {
    fn len(&self, origin: &str) -> usize {
        self.origins
            .get(origin)
            .map_or(0, MemoryWebStorageArea::len)
    }

    fn area_keys_for_origin(&self, origin: &str) -> Vec<String> {
        let area_prefix = web_storage_area_key_prefix_for_origin(origin);
        self.origins
            .keys()
            .filter(|area_key| area_key.starts_with(&area_prefix))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MemoryWebStorageArea, WEB_STORAGE_QUOTA_BYTES, WebStorageAreaKind, WebStorageBackend,
        WebStorageMutation, WebStorageMutationError, WebStorageMutationRecord, WebStorageStore,
        WebStorageString, deep_clone_shared_web_storage_store, new_shared_web_storage_store,
        web_storage_partitioned_area_key,
    };

    fn first_party_area_key(origin: &str) -> String {
        web_storage_partitioned_area_key(origin, origin)
    }

    #[test]
    fn memory_web_storage_preserves_sorted_key_and_quota_behavior() {
        let store = WebStorageStore::default();
        let area_key = first_party_area_key("https://a.test");
        assert!(store.set_item(&area_key, "b", "2"));
        assert!(store.set_item(&area_key, "a", "1"));
        assert_eq!(store.len(&area_key), 2);
        assert_eq!(store.key(&area_key, 0), Some("a".to_owned()));
        assert_eq!(store.sorted_keys(&area_key), vec!["a", "b"]);
        assert_eq!(store.get_item(&area_key, "b"), Some("2".to_owned()));
        assert!(store.contains_key(&area_key, "a"));
        assert!(store.remove_item(&area_key, "a"));
        assert!(!store.contains_key(&area_key, "a"));

        let large = "x".repeat(WEB_STORAGE_QUOTA_BYTES + 1);
        assert!(!store.set_item(&area_key, "large", &large));
        assert_eq!(
            store.try_set_item(&area_key, "large", &large),
            Err(WebStorageMutationError::QuotaExceeded)
        );
    }

    #[test]
    fn mutation_subscription_reports_only_successful_observable_changes() {
        let store = WebStorageStore::default();
        let area_key = first_party_area_key("https://a.test");
        let subscription = store.subscribe_mutations(WebStorageAreaKind::Local);

        assert!(store.set_item(&area_key, "a", "1"));
        assert!(store.set_item(&area_key, "a", "1"));
        assert!(store.set_item(&area_key, "a", "2"));
        assert!(!store.remove_item(&area_key, "missing"));
        assert!(store.remove_item(&area_key, "a"));
        assert!(!store.try_clear(&area_key).expect("empty clear should work"));
        assert!(store.set_item(&area_key, "b", "3"));
        assert!(
            store
                .try_clear(&area_key)
                .expect("nonempty clear should work")
        );

        assert_eq!(
            subscription.drain(),
            vec![
                WebStorageMutationRecord {
                    area_kind: WebStorageAreaKind::Local,
                    mutation: WebStorageMutation::ItemAdded {
                        area_key: area_key.clone(),
                        key: WebStorageString::from_utf8("a"),
                        value: WebStorageString::from_utf8("1"),
                    },
                },
                WebStorageMutationRecord {
                    area_kind: WebStorageAreaKind::Local,
                    mutation: WebStorageMutation::ItemUpdated {
                        area_key: area_key.clone(),
                        key: WebStorageString::from_utf8("a"),
                        old_value: WebStorageString::from_utf8("1"),
                        new_value: WebStorageString::from_utf8("2"),
                    },
                },
                WebStorageMutationRecord {
                    area_kind: WebStorageAreaKind::Local,
                    mutation: WebStorageMutation::ItemRemoved {
                        area_key: area_key.clone(),
                        key: WebStorageString::from_utf8("a"),
                        old_value: WebStorageString::from_utf8("2"),
                    },
                },
                WebStorageMutationRecord {
                    area_kind: WebStorageAreaKind::Local,
                    mutation: WebStorageMutation::ItemAdded {
                        area_key: area_key.clone(),
                        key: WebStorageString::from_utf8("b"),
                        value: WebStorageString::from_utf8("3"),
                    },
                },
                WebStorageMutationRecord {
                    area_kind: WebStorageAreaKind::Local,
                    mutation: WebStorageMutation::ItemsCleared {
                        area_key: area_key.clone(),
                    },
                },
            ]
        );
        assert!(subscription.is_empty());

        let empty_area_key = first_party_area_key("https://empty.test");
        match &store.backend {
            WebStorageBackend::Memory(memory) => {
                memory
                    .lock()
                    .origins
                    .insert(empty_area_key.clone(), MemoryWebStorageArea::default());
            }
            _ => panic!("default store should use the memory backend"),
        }
        assert!(
            store
                .try_clear(&empty_area_key)
                .expect("empty existing area should clear")
        );
        let WebStorageBackend::Memory(memory) = &store.backend else {
            panic!("default store should use the memory backend");
        };
        assert!(
            !memory.lock().origins.contains_key(&empty_area_key),
            "clear must preserve the existing area cleanup semantics"
        );
        assert!(
            subscription.is_empty(),
            "clearing an empty existing area must not publish a mutation"
        );

        drop(subscription);
        assert!(store.set_item(&area_key, "after-drop", "4"));
        assert!(store.mutation_subscribers.lock().is_empty());
    }

    #[test]
    fn one_mutation_subscription_preserves_cross_store_order_and_deduplicates_registration() {
        let area_key = first_party_area_key("https://a.test");
        let local = WebStorageStore::default();
        let session = WebStorageStore::default();
        let subscription = local.subscribe_mutations(WebStorageAreaKind::Local);
        local.add_mutation_subscription(WebStorageAreaKind::Local, &subscription);
        session.add_mutation_subscription(WebStorageAreaKind::Session, &subscription);

        assert!(local.set_item(&area_key, "local-first", "1"));
        assert!(session.set_item(&area_key, "session-second", "2"));
        assert!(local.set_item(&area_key, "local-third", "3"));

        let records = subscription.drain();
        assert_eq!(records.len(), 3);
        assert_eq!(
            records
                .iter()
                .map(|record| record.area_kind)
                .collect::<Vec<_>>(),
            vec![
                WebStorageAreaKind::Local,
                WebStorageAreaKind::Session,
                WebStorageAreaKind::Local,
            ]
        );
    }

    #[test]
    fn deep_clone_copies_all_areas_without_sharing_subsequent_mutations() {
        let source = new_shared_web_storage_store();
        let a_area_key = first_party_area_key("https://a.test");
        let b_area_key = first_party_area_key("https://b.test");
        {
            let source = source.as_ref();
            assert!(source.set_item(&a_area_key, "shared", "source"));
            assert!(source.set_item(&b_area_key, "other", "copied"));
        }
        let subscription = source
            .as_ref()
            .subscribe_mutations(WebStorageAreaKind::Session);

        let cloned = deep_clone_shared_web_storage_store(&source);
        {
            let cloned = cloned.as_ref();
            assert_eq!(
                cloned.get_item(&a_area_key, "shared"),
                Some("source".to_owned())
            );
            assert_eq!(
                cloned.get_item(&b_area_key, "other"),
                Some("copied".to_owned())
            );
            assert!(cloned.set_item(&a_area_key, "shared", "clone"));
            assert!(cloned.set_item(&a_area_key, "clone-only", "yes"));
        }
        assert!(
            subscription.is_empty(),
            "a deep-cloned sessionStorage namespace must not retain opener mutation subscribers"
        );
        {
            let source = source.as_ref();
            assert_eq!(
                source.get_item(&a_area_key, "shared"),
                Some("source".to_owned())
            );
            assert_eq!(source.get_item(&a_area_key, "clone-only"), None);
            assert!(source.set_item(&b_area_key, "source-only", "yes"));
        }
        assert_eq!(cloned.as_ref().get_item(&b_area_key, "source-only"), None);
    }

    #[test]
    fn web_storage_usage_tracks_value_bytes_by_storage_key() {
        let store = WebStorageStore::default();
        let a_area_key = first_party_area_key("https://a.test");
        let b_area_key = first_party_area_key("https://b.test");
        assert_eq!(store.usage_bytes(&a_area_key), 0);

        assert!(store.set_item(&a_area_key, "first", "abc"));
        assert!(store.set_item(&a_area_key, "second", "defg"));
        assert!(store.set_item(&b_area_key, "other", "xxxx"));
        assert_eq!(store.usage_bytes(&a_area_key), 7);
        assert_eq!(store.usage_bytes(&b_area_key), 4);

        assert!(store.set_item(&a_area_key, "first", "z"));
        assert_eq!(store.usage_bytes(&a_area_key), 5);

        assert!(store.remove_item(&a_area_key, "second"));
        assert_eq!(store.usage_bytes(&a_area_key), 1);

        store.clear_origin(&a_area_key);
        assert_eq!(store.usage_bytes(&a_area_key), 0);
        assert_eq!(store.usage_bytes(&b_area_key), 4);
    }

    #[test]
    fn origin_clear_includes_partitioned_area_keys() {
        let origin = "https://cdn.example.test";
        let first_party = first_party_area_key(origin);
        let top_a = web_storage_partitioned_area_key(origin, "https://top-a.example.test");
        let top_b = web_storage_partitioned_area_key(origin, "https://top-b.example.test");
        let sibling = web_storage_partitioned_area_key(
            "https://sibling.example.test",
            "https://top-a.example.test",
        );
        let store = WebStorageStore::default();
        assert!(store.set_item(&first_party, "fp", "a"));
        assert!(store.set_item(&top_a, "a", "bb"));
        assert!(store.set_item(&top_b, "b", "ccc"));
        assert!(store.set_item(&sibling, "s", "dddd"));

        assert_eq!(store.usage_bytes(&first_party), 1);

        store.clear_origin(&first_party);
        assert_eq!(store.get_item(&top_a, "a"), Some("bb".to_owned()));

        store.clear_origin_areas(origin);
        assert_eq!(store.get_item(&first_party, "fp"), None);
        assert_eq!(store.get_item(&top_a, "a"), None);
        assert_eq!(store.get_item(&top_b, "b"), None);
        assert_eq!(store.get_item(&sibling, "s"), Some("dddd".to_owned()));
    }
}
