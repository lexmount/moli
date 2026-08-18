use std::{collections::HashMap, hash::Hash};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReflectorId(u64);

impl ReflectorId {
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Reflector<K> {
    id: ReflectorId,
    key: K,
}

impl<K: Clone> Reflector<K> {
    pub fn id(&self) -> ReflectorId {
        self.id
    }

    pub fn key(&self) -> K {
        self.key.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DomPtr<K>(K);

impl<K: Clone> DomPtr<K> {
    pub fn new(key: K) -> Self {
        Self(key)
    }

    pub fn key(&self) -> K {
        self.0.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DomRoot<K> {
    reflector: Reflector<K>,
}

impl<K: Clone> DomRoot<K> {
    pub fn reflector_id(&self) -> ReflectorId {
        self.reflector.id()
    }
}

#[derive(Debug, Clone)]
pub struct ReflectorRegistry<K> {
    next_id: u64,
    ids_by_key: HashMap<K, ReflectorId>,
    keys_by_id: HashMap<ReflectorId, K>,
}

impl<K> Default for ReflectorRegistry<K> {
    fn default() -> Self {
        Self {
            next_id: 0,
            ids_by_key: HashMap::new(),
            keys_by_id: HashMap::new(),
        }
    }
}

impl<K> ReflectorRegistry<K>
where
    K: Clone + Eq + Hash,
{
    pub fn intern(&mut self, key: K) -> Reflector<K> {
        if let Some(existing) = self.ids_by_key.get(&key).copied() {
            return Reflector { id: existing, key };
        }

        self.next_id = self.next_id.checked_add(1).expect("reflector id overflow");
        let id = ReflectorId(self.next_id);
        self.ids_by_key.insert(key.clone(), id);
        self.keys_by_id.insert(id, key.clone());
        Reflector { id, key }
    }

    pub fn existing(&self, key: K) -> Option<Reflector<K>> {
        self.ids_by_key
            .get(&key)
            .copied()
            .map(|id| Reflector { id, key })
    }

    pub fn root(&mut self, ptr: DomPtr<K>) -> DomRoot<K> {
        DomRoot {
            reflector: self.intern(ptr.key()),
        }
    }

    pub fn key_for_id(&self, id: ReflectorId) -> Option<K> {
        self.keys_by_id.get(&id).cloned()
    }

    pub fn len(&self) -> usize {
        self.ids_by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids_by_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::ReflectorRegistry;

    #[test]
    fn reflector_registry_reuses_identity_for_same_key() {
        let mut registry = ReflectorRegistry::default();

        let first = registry.intern(7_u32);
        let second = registry.intern(7_u32);

        assert_eq!(first.id(), second.id());
        assert_eq!(first.key(), second.key());
        assert_eq!(first.id().raw(), 1);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.key_for_id(first.id()), Some(7_u32));
    }

    #[test]
    fn reflector_registry_supports_non_copy_keys() {
        let mut registry = ReflectorRegistry::default();

        let first = registry.intern("highlight(name)".to_owned());
        let second = registry.intern("highlight(name)".to_owned());
        let other = registry.intern("highlight(other)".to_owned());

        assert_eq!(first.id(), second.id());
        assert_ne!(first.id(), other.id());
        assert_eq!(
            registry.key_for_id(first.id()),
            Some("highlight(name)".to_owned())
        );
    }

    #[test]
    fn reflector_registry_allocates_distinct_identity_for_distinct_keys() {
        let mut registry = ReflectorRegistry::default();

        let first = registry.intern(1_u32);
        let second = registry.intern(2_u32);

        assert_ne!(first.id(), second.id());
        assert_eq!(registry.existing(1_u32), Some(first));
        assert_eq!(registry.existing(2_u32), Some(second));
        assert!(registry.existing(99_u32).is_none());
        assert_eq!(registry.key_for_id(first.id()), Some(1_u32));
        assert_eq!(registry.key_for_id(second.id()), Some(2_u32));
        assert_eq!(registry.len(), 2);
        assert!(!registry.is_empty());
    }
}
