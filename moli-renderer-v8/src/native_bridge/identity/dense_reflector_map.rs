use std::collections::HashMap;

use super::ReflectorId;

/// Stores realm-local wrapper values keyed by store-wide reflector IDs.
///
/// Reflector IDs are allocated monotonically by `BridgeIdentityStore`, while a
/// V8 context only caches the wrappers materialized in that context. Most
/// contexts therefore observe contiguous ID runs, but a late-created or
/// isolated context can start at a large ID or skip a wide range. The first
/// observed ID becomes `dense_base`, so it never creates empty slots below
/// that ID. Small later gaps stay in the dense vector; large gaps use the
/// sparse map instead of expanding the vector with mostly empty slots.
///
/// An ID is present in at most one backing store. Dense slot `index` always
/// represents `dense_base + index`.
#[derive(Debug)]
pub(super) struct DenseReflectorMap<V> {
    dense_base: Option<u64>,
    dense: Vec<Option<V>>,
    sparse: HashMap<ReflectorId, V>,
}

/// Maximum number of empty dense slots accepted to avoid a hash-map entry.
const MAX_DENSE_REFLECTOR_ID_GAP: u64 = 64;

impl<V> Default for DenseReflectorMap<V> {
    fn default() -> Self {
        Self {
            dense_base: None,
            dense: Vec::new(),
            sparse: HashMap::new(),
        }
    }
}

impl<V> DenseReflectorMap<V> {
    fn dense_index(&self, id: ReflectorId) -> Option<usize> {
        let offset = id.raw().checked_sub(self.dense_base?)?;
        let index = usize::try_from(offset).ok()?;
        (index < self.dense.len()).then_some(index)
    }

    pub(super) fn get(&self, id: &ReflectorId) -> Option<&V> {
        self.dense_index(*id)
            .and_then(|index| self.dense[index].as_ref())
            .or_else(|| self.sparse.get(id))
    }

    pub(super) fn remove(&mut self, id: &ReflectorId) -> Option<V> {
        self.dense_index(*id)
            .and_then(|index| self.dense[index].take())
            .or_else(|| self.sparse.remove(id))
    }

    pub(super) fn insert(&mut self, id: ReflectorId, value: V) -> Option<V> {
        let raw = id.raw();
        let base = self.dense_base.get_or_insert(raw);
        let dense_end = base.saturating_add(self.dense.len() as u64);
        if raw >= *base && raw <= dense_end.saturating_add(MAX_DENSE_REFLECTOR_ID_GAP) {
            let index = usize::try_from(raw - *base).expect("dense reflector index overflow");
            if index >= self.dense.len() {
                self.dense.resize_with(index + 1, || None);
            }
            let sparse = self.sparse.remove(&id);
            return self.dense[index].replace(value).or(sparse);
        }
        self.sparse.insert(id, value)
    }

    #[cfg(test)]
    pub(super) fn clear(&mut self) {
        self.dense_base = None;
        self.dense.clear();
        self.sparse.clear();
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(ReflectorId, &mut V) -> bool) {
        if let Some(base) = self.dense_base {
            for (index, slot) in self.dense.iter_mut().enumerate() {
                let Some(value) = slot.as_mut() else {
                    continue;
                };
                let raw = base
                    .checked_add(index as u64)
                    .expect("dense reflector id overflow");
                if !keep(ReflectorId::from_raw(raw), value) {
                    *slot = None;
                }
            }
            if let Some(first_live) = self.dense.iter().position(Option::is_some) {
                if first_live != 0 {
                    self.dense.drain(..first_live);
                    self.dense_base = Some(
                        base.checked_add(first_live as u64)
                            .expect("dense reflector id overflow"),
                    );
                }
                let retained_len = self
                    .dense
                    .iter()
                    .rposition(Option::is_some)
                    .map_or(0, |index| index + 1);
                self.dense.truncate(retained_len);
            } else {
                self.dense.clear();
                self.dense_base = None;
            }
        }
        self.sparse.retain(|id, value| keep(*id, value));
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.dense.iter().flatten().count() + self.sparse.len()
    }

    #[cfg(test)]
    pub(super) fn values(&self) -> impl Iterator<Item = &V> {
        self.dense.iter().flatten().chain(self.sparse.values())
    }
}

#[cfg(test)]
mod tests {
    use super::{DenseReflectorMap, ReflectorId};

    #[test]
    fn keeps_sequential_ids_dense_and_large_gaps_sparse() {
        let mut entries = DenseReflectorMap::default();
        assert_eq!(entries.insert(ReflectorId::from_raw(50_000), 1_u32), None);
        assert_eq!(entries.insert(ReflectorId::from_raw(50_001), 2_u32), None);
        assert_eq!(entries.insert(ReflectorId::from_raw(100_000), 3_u32), None);

        assert_eq!(entries.dense_base, Some(50_000));
        assert_eq!(entries.dense.len(), 2);
        assert_eq!(entries.sparse.len(), 1);
        assert_eq!(entries.get(&ReflectorId::from_raw(50_001)), Some(&2));
        assert_eq!(entries.get(&ReflectorId::from_raw(100_000)), Some(&3));
        assert_eq!(entries.len(), 3);

        for (raw, value) in [(50_001, 2), (100_000, 3)] {
            let id = ReflectorId::from_raw(raw);
            assert_eq!(entries.remove(&id), Some(value));
            assert_eq!(entries.get(&id), None);
            assert_eq!(entries.remove(&id), None);
            assert_eq!(entries.insert(id, value), None);
        }

        entries.retain(|id, _| id.raw() != 50_000);
        assert_eq!(entries.dense_base, Some(50_001));
        assert_eq!(entries.values().copied().collect::<Vec<_>>(), vec![2, 3]);

        entries.clear();
        assert_eq!(entries.len(), 0);
        assert_eq!(entries.dense_base, None);
    }
}
