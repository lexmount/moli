// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;
use hashbrown::Equivalent;

/// An entry in the cache.
pub(crate) struct Entry<ID, T> {
    pub epoch: u64,
    pub id: ID,
    pub data: T,
}

/// A least-recently-used cache. This cache uses a linear scan of its entries
/// to find a given entry - it is optimised for a low number of entries. Preferably
/// keep `max_entries` low - in the order of tens.
pub(crate) struct LruCache<ID, T> {
    entries: Vec<Entry<ID, T>>,
    epoch: u64,
    max_entries: usize,
}

impl<ID, T> LruCache<ID, T> {
    pub(crate) fn new(max_entries: usize) -> Self {
        Self {
            entries: Vec::default(),
            epoch: 0,
            max_entries,
        }
    }

    /// Returns a reference to the entry with the given ID. If the entry is not
    /// found, it is created and returned using `make_data`.
    ///
    /// The lookup key must be `Equivalent` to ID for lookups and convertible `Into<ID>`
    /// for creating new entries.
    pub(crate) fn entry<K>(&mut self, id: K, make_data: impl FnOnce() -> T) -> &T
    where
        K: Equivalent<ID> + Into<ID>,
    {
        let index = self.find_entry(id, make_data);
        self.epoch += 1;
        let entry = &mut self.entries[index];
        entry.epoch = self.epoch;
        &entry.data
    }

    fn find_entry<K>(&mut self, id: K, make_data: impl FnOnce() -> T) -> usize
    where
        K: Equivalent<ID> + Into<ID>,
    {
        let epoch = self.epoch;
        let mut lowest_serial = epoch;
        let mut lowest_index = 0;
        for (i, entry) in self.entries.iter().enumerate() {
            if id.equivalent(&entry.id) {
                return i;
            }
            if entry.epoch < lowest_serial {
                lowest_serial = entry.epoch;
                lowest_index = i;
            }
        }
        if self.entries.len() < self.max_entries {
            lowest_index = self.entries.len();
            self.entries.push(Entry {
                epoch,
                id: id.into(),
                data: make_data(),
            });
        } else {
            let entry = &mut self.entries[lowest_index];
            entry.epoch = epoch;
            entry.id = id.into();
            entry.data = make_data();
        }
        lowest_index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::{String, ToString};

    #[derive(Debug, Clone, PartialEq)]
    struct TestId(String);
    struct TestLookupKey<'a>(&'a str);

    impl<'a> Equivalent<TestId> for TestLookupKey<'a> {
        fn equivalent(&self, key: &TestId) -> bool {
            self.0 == key.0.as_str()
        }
    }

    impl<'a> From<TestLookupKey<'a>> for TestId {
        fn from(key: TestLookupKey<'a>) -> Self {
            Self(key.0.to_string())
        }
    }

    impl Equivalent<Self> for TestId {
        fn equivalent(&self, key: &Self) -> bool {
            self.0 == key.0
        }
    }

    #[test]
    fn test_retrieve_existing_entry() {
        let mut cache = LruCache::new(3);

        // Insert an entry
        let value1 = cache.entry(TestLookupKey("key1"), || 42);
        assert_eq!(*value1, 42);

        // Retrieve the same entry - make_data should not be called
        let value2 = cache.entry(TestLookupKey("key1"), || {
            panic!("Should not create new data")
        });
        assert_eq!(*value2, 42);
        assert_eq!(cache.entries.len(), 1);
    }

    #[test]
    fn test_multiple_entries() {
        let mut cache = LruCache::new(3);

        let value1 = cache.entry(TestLookupKey("key1"), || 1);
        assert_eq!(*value1, 1);

        let value2 = cache.entry(TestLookupKey("key2"), || 2);
        assert_eq!(*value2, 2);

        let value3 = cache.entry(TestLookupKey("key3"), || 3);
        assert_eq!(*value3, 3);

        assert_eq!(cache.entries.len(), 3);
        assert_eq!(cache.epoch, 3);
    }

    #[test]
    fn test_lru_eviction() {
        let mut cache = LruCache::new(3);

        // Add three entries
        cache.entry(TestLookupKey("key1"), || 1);
        cache.entry(TestLookupKey("key2"), || 2);
        cache.entry(TestLookupKey("key3"), || 3);

        // Access key1 to update its epoch
        cache.entry(TestLookupKey("key1"), || panic!("Should not create"));

        // Add key4 - should evict key2 (oldest untouched)
        cache.entry(TestLookupKey("key4"), || 4);

        // Verify key1 is still present
        let value1 = cache.entry(TestLookupKey("key1"), || {
            panic!("key1 should still be present")
        });
        assert_eq!(*value1, 1);

        // Verify key2 was evicted
        let mut was_created = false;
        cache.entry(TestLookupKey("key2"), || {
            was_created = true;
            20
        });
        assert!(was_created, "key2 should have been evicted");
    }

    #[test]
    fn test_lru_eviction_after_multiple_hits() {
        let mut cache = LruCache::new(3);

        cache.entry(TestLookupKey("key1"), || 1);
        cache.entry(TestLookupKey("key2"), || 2);
        cache.entry(TestLookupKey("key3"), || 3);

        // Hit all three in order: key3 first (making it LRU among hits),
        // then key1, then key2 (most recently used).
        cache.entry(TestLookupKey("key3"), || panic!("Should not create"));
        cache.entry(TestLookupKey("key1"), || panic!("Should not create"));
        cache.entry(TestLookupKey("key2"), || panic!("Should not create"));

        // Insert key4 — should evict key3 (least recently accessed)
        cache.entry(TestLookupKey("key4"), || 4);

        // key1 and key2 should still be present with original values
        // (check these first since verifying key3 eviction will trigger another eviction)
        let v1 = cache.entry(TestLookupKey("key1"), || {
            panic!("key1 should still be present")
        });
        assert_eq!(*v1, 1);

        let v2 = cache.entry(TestLookupKey("key2"), || {
            panic!("key2 should still be present")
        });
        assert_eq!(*v2, 2);

        // key3 should have been evicted
        let mut key3_recreated = false;
        cache.entry(TestLookupKey("key3"), || {
            key3_recreated = true;
            30
        });
        assert!(key3_recreated, "key3 should have been evicted as the LRU");
    }
}
