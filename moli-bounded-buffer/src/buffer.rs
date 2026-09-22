use std::{
    borrow::Borrow,
    collections::{HashMap, VecDeque},
    hash::Hash,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Total and per-entry byte limits for a [`BoundedByteBuffer`].
pub struct ByteLimits {
    /// Maximum sum of the retained entry charges.
    pub max_total_bytes: usize,
    /// Maximum charge accepted for any one entry.
    pub max_entry_bytes: usize,
}

impl ByteLimits {
    /// Constructs a pair of byte limits.
    pub const fn new(max_total_bytes: usize, max_entry_bytes: usize) -> Self {
        Self {
            max_total_bytes,
            max_entry_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BufferedValue<V> {
    value: V,
    byte_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A byte-budgeted buffer that evicts entries in insertion order.
pub struct BoundedByteBuffer<K: Eq + Hash, V> {
    limits: ByteLimits,
    used_bytes: usize,
    insertion_order: VecDeque<K>,
    entries: HashMap<K, BufferedValue<V>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// The ownership result of inserting a value into a [`BoundedByteBuffer`].
pub enum InsertOutcome<K, V> {
    /// The value was retained and older entries may have been evicted.
    Stored { evicted: Vec<(K, V)> },
    /// The value exceeded an entry or total limit and was not retained.
    Rejected { key: K, value: V },
}

impl<K, V> BoundedByteBuffer<K, V>
where
    K: Clone + Eq + Hash,
{
    /// Constructs an empty buffer with the provided limits.
    pub fn new(limits: ByteLimits) -> Self {
        Self {
            limits,
            used_bytes: 0,
            insertion_order: VecDeque::new(),
            entries: HashMap::new(),
        }
    }

    /// Returns the configured limits.
    pub fn limits(&self) -> ByteLimits {
        self.limits
    }

    /// Returns the sum of the logical byte charges for retained entries.
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    /// Returns the number of retained entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the buffer contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns whether the buffer contains `key`.
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.entries.contains_key(key)
    }

    /// Returns the value retained for `key`.
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.entries.get(key).map(|entry| &entry.value)
    }

    /// Returns the mutable value retained for `key`.
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.entries.get_mut(key).map(|entry| &mut entry.value)
    }

    /// Iterates over retained entries in unspecified order.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter().map(|(key, entry)| (key, &entry.value))
    }

    /// Inserts an entry with its logical byte charge.
    ///
    /// Replacing an existing key first removes its previous charge. A stored
    /// replacement becomes the newest entry for future eviction.
    pub fn insert(&mut self, key: K, value: V, byte_len: usize) -> InsertOutcome<K, V> {
        self.remove(&key);
        if byte_len > self.limits.max_entry_bytes || byte_len > self.limits.max_total_bytes {
            return InsertOutcome::Rejected { key, value };
        }

        let mut evicted = Vec::new();
        while self.used_bytes > self.limits.max_total_bytes - byte_len {
            let Some((evicted_key, evicted_value)) = self.pop_oldest() else {
                break;
            };
            evicted.push((evicted_key, evicted_value));
        }

        self.used_bytes += byte_len;
        self.insertion_order.push_back(key.clone());
        self.entries.insert(key, BufferedValue { value, byte_len });
        InsertOutcome::Stored { evicted }
    }

    /// Removes an entry and returns its value, releasing its byte charge.
    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        let entry = self.entries.remove(key)?;
        self.used_bytes -= entry.byte_len;
        self.insertion_order
            .retain(|candidate| <K as Borrow<Q>>::borrow(candidate) != key);
        Some(entry.value)
    }

    /// Removes all entries and releases all byte charges.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.insertion_order.clear();
        self.used_bytes = 0;
    }

    fn pop_oldest(&mut self) -> Option<(K, V)> {
        while let Some(key) = self.insertion_order.pop_front() {
            let Some(entry) = self.entries.remove(&key) else {
                continue;
            };
            self.used_bytes -= entry.byte_len;
            return Some((key, entry.value));
        }
        None
    }
}
