use super::*;
use moli_indexeddb::sqlite::{MetadataReservation, service::DatabaseClient};
use std::collections::BTreeSet;

// Each immutable allocation owns its lease. Sharing a definition, retaining an
// old upgrade snapshot, or keeping a deleted index never copies or refunds it.
#[derive(Debug)]
pub(in crate::context_bootstrap::indexed_db) struct MetadataNode<T> {
    pub(super) value: T,
    reservation: MetadataReservation,
}

impl<T> std::ops::Deref for MetadataNode<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
impl<T: PartialEq> PartialEq for MetadataNode<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}
impl<T: Eq> Eq for MetadataNode<T> {}

pub(in crate::context_bootstrap::indexed_db) trait MetadataSize {
    fn heap_bytes(&self) -> usize;
}

impl<T: MetadataSize> MetadataNode<T> {
    fn bytes(heap: usize) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(2 * std::mem::size_of::<usize>())
            .saturating_add(heap)
    }

    pub(super) fn try_new(
        client: &DatabaseClient,
        heap: usize,
        build: impl FnOnce() -> T,
    ) -> Result<Rc<Self>, IndexedDbError> {
        // Charge before invoking a closure that copies names, paths or maps.
        let reservation = client.try_reserve_metadata(Self::bytes(heap))?;
        let mut node = Self {
            value: build(),
            reservation,
        };
        let bytes = Self::bytes(node.value.heap_bytes());
        debug_assert!(
            bytes <= node.reservation.reserved_bytes(),
            "schema copy was reserved before allocation"
        );
        node.reservation.try_resize(bytes)?;
        Ok(Rc::new(node))
    }

    pub(super) fn try_edit(
        &self,
        client: &DatabaseClient,
        growth: usize,
        edit: impl FnOnce(&mut T),
    ) -> Result<Rc<Self>, IndexedDbError>
    where
        T: Clone,
    {
        Self::try_new(client, self.heap_bytes().saturating_add(growth), || {
            let mut value = self.value.clone();
            edit(&mut value);
            value
        })
    }
}

// BTreeMap does not expose its allocation capacity. As in the native schema
// loader, charge an underfilled node and its edges for every live entry.
pub(super) fn map_entry_bytes<K, V>() -> usize {
    12 * (std::mem::size_of::<K>() + std::mem::size_of::<V>() + std::mem::size_of::<usize>()) + 64
}

pub(super) fn key_path_bytes(path: &KeyPath) -> usize {
    match path {
        KeyPath::String(value) => value.capacity(),
        KeyPath::Sequence(values) => {
            values.capacity() * std::mem::size_of::<String>()
                + values.iter().map(String::capacity).sum::<usize>()
        }
    }
}

impl MetadataSize for IndexInfo {
    fn heap_bytes(&self) -> usize {
        self.name.capacity() + key_path_bytes(&self.key_path)
    }
}

impl MetadataSize for IndexedDbObjectStoreMetadataData {
    fn heap_bytes(&self) -> usize {
        self.info.name.capacity()
            + self.info.key_path.as_ref().map_or(0, key_path_bytes)
            + self.info.index_names.capacity() * std::mem::size_of::<String>()
            + self
                .info
                .index_names
                .iter()
                .map(String::capacity)
                .sum::<usize>()
            + self
                .indexes
                .keys()
                .map(|name| {
                    map_entry_bytes::<String, Rc<MetadataNode<IndexInfo>>>() + name.capacity()
                })
                .sum::<usize>()
    }
}

impl MetadataSize for BTreeSet<String> {
    fn heap_bytes(&self) -> usize {
        self.iter()
            .map(|name| map_entry_bytes::<String, ()>() + name.capacity())
            .sum()
    }
}

pub(super) type StoreMap = BTreeMap<String, IndexedDbObjectStoreMetadata>;

impl MetadataSize for StoreMap {
    fn heap_bytes(&self) -> usize {
        self.keys()
            .map(|name| map_entry_bytes::<String, IndexedDbObjectStoreMetadata>() + name.capacity())
            .sum()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::context_bootstrap::indexed_db) struct IndexedDbObjectStoreMetadata {
    pub(super) data: Rc<MetadataNode<IndexedDbObjectStoreMetadataData>>,
    pub(super) created_in_upgrade: std::cell::Cell<bool>,
    pub(super) created_indexes: RefCell<Option<Rc<MetadataNode<BTreeSet<String>>>>>,
    pub(super) visible_indexes: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::context_bootstrap::indexed_db) struct IndexedDbObjectStoreMetadataData {
    pub(super) info: ObjectStoreInfo,
    pub(super) indexes: BTreeMap<String, Rc<MetadataNode<IndexInfo>>>,
}

impl std::ops::Deref for IndexedDbObjectStoreMetadata {
    type Target = IndexedDbObjectStoreMetadataData;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl IndexedDbObjectStoreMetadata {
    fn from_data(data: Rc<MetadataNode<IndexedDbObjectStoreMetadataData>>) -> Self {
        Self {
            data,
            created_in_upgrade: std::cell::Cell::new(false),
            created_indexes: RefCell::new(None),
            visible_indexes: true,
        }
    }

    pub(in crate::context_bootstrap::indexed_db) fn try_from_info(
        client: &DatabaseClient,
        info: &ObjectStoreInfo,
    ) -> Result<Self, IndexedDbError> {
        debug_assert!(info.index_names.is_empty());
        let heap = info.name.capacity() + info.key_path.as_ref().map_or(0, key_path_bytes);
        Ok(Self::from_data(MetadataNode::try_new(
            client,
            heap,
            || IndexedDbObjectStoreMetadataData {
                info: info.clone(),
                indexes: BTreeMap::new(),
            },
        )?))
    }

    pub(in crate::context_bootstrap::indexed_db) fn try_from_sql(
        client: &DatabaseClient,
        name: &str,
        store: &moli_indexeddb::sqlite::StoreMetadata,
    ) -> Result<Self, IndexedDbError> {
        let heap = name.len()
            + store.options.key_path.as_ref().map_or(0, key_path_bytes)
            + store.indexes.len() * std::mem::size_of::<String>()
            + store
                .indexes
                .keys()
                .map(|name| {
                    map_entry_bytes::<String, Rc<MetadataNode<IndexInfo>>>() + 2 * name.len()
                })
                .sum::<usize>();
        // Reserve the container before cloning even its first map key.
        let mut reservation = client.try_reserve_metadata(MetadataNode::<
            IndexedDbObjectStoreMetadataData,
        >::bytes(heap))?;
        let mut indexes = BTreeMap::new();
        for (name, index) in &store.indexes {
            let info = MetadataNode::try_new(
                client,
                name.len() + key_path_bytes(&index.options.key_path),
                || IndexInfo {
                    name: name.clone(),
                    key_path: index.options.key_path.clone(),
                    unique: index.options.unique,
                    multi_entry: index.options.multi_entry,
                },
            )?;
            indexes.insert(name.clone(), info);
        }
        let mut names = Vec::with_capacity(store.indexes.len());
        names.extend(store.indexes.keys().cloned());
        names.sort_unstable_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        let value = IndexedDbObjectStoreMetadataData {
            info: ObjectStoreInfo {
                name: name.into(),
                key_path: store.options.key_path.clone(),
                auto_increment: store.options.auto_increment,
                index_names: names,
            },
            indexes,
        };
        reservation.try_resize(MetadataNode::<IndexedDbObjectStoreMetadataData>::bytes(
            value.heap_bytes(),
        ))?;
        Ok(Self::from_data(Rc::new(MetadataNode {
            value,
            reservation,
        })))
    }

    pub(in crate::context_bootstrap::indexed_db) fn info(&self) -> &ObjectStoreInfo {
        &self.info
    }
    pub(in crate::context_bootstrap::indexed_db) fn index_names(&self) -> &[String] {
        if self.visible_indexes {
            &self.info.index_names
        } else {
            &[]
        }
    }
    pub(in crate::context_bootstrap::indexed_db) fn index(&self, name: &str) -> Option<&IndexInfo> {
        self.visible_indexes
            .then(|| self.indexes.get(name).map(|info| &***info))
            .flatten()
    }
    pub(in crate::context_bootstrap::indexed_db) fn shared_index(
        &self,
        name: &str,
    ) -> Option<Rc<MetadataNode<IndexInfo>>> {
        self.visible_indexes
            .then(|| self.indexes.get(name).cloned())
            .flatten()
    }
    #[cfg(test)]
    pub(in crate::context_bootstrap::indexed_db) fn indexes_in_name_order(&self) -> Vec<IndexInfo> {
        self.index_names()
            .iter()
            .filter_map(|name| self.index(name).cloned())
            .collect()
    }
    pub(super) fn index_created_in_upgrade(&self, name: &str) -> bool {
        self.created_in_upgrade.get()
            || self
                .created_indexes
                .borrow()
                .as_ref()
                .is_some_and(|names| names.contains(name))
    }
    pub(super) fn try_set_index(
        &self,
        client: &DatabaseClient,
        info: &IndexInfo,
    ) -> Result<Self, IndexedDbError> {
        let index = MetadataNode::try_new(client, info.heap_bytes(), || info.clone())?;
        let marker_growth = map_entry_bytes::<String, ()>() + info.name.len();
        let markers = match &*self.created_indexes.borrow() {
            Some(names) => names.try_edit(client, marker_growth, |names| {
                names.insert(info.name.clone());
            })?,
            None => MetadataNode::try_new(client, marker_growth, || {
                BTreeSet::from([info.name.clone()])
            })?,
        };
        let growth = map_entry_bytes::<String, Rc<MetadataNode<IndexInfo>>>()
            + 2 * info.name.len()
            + (2 * self.info.index_names.len()).max(4) * std::mem::size_of::<String>();
        let data = self.data.try_edit(client, growth, |data| {
            if !data.info.index_names.iter().any(|name| name == &info.name) {
                data.info.index_names.push(info.name.clone());
            }
            data.indexes.insert(info.name.clone(), index);
        })?;
        Ok(Self {
            data,
            created_indexes: RefCell::new(Some(markers)),
            ..self.clone()
        })
    }
    pub(super) fn try_remove_index(
        &self,
        client: &DatabaseClient,
        name: &str,
    ) -> Result<Self, IndexedDbError> {
        let data = self.data.try_edit(client, 0, |data| {
            data.info.index_names.retain(|candidate| candidate != name);
            data.indexes.remove(name);
            if data.indexes.is_empty() {
                data.indexes = BTreeMap::new();
            }
        })?;
        Ok(Self {
            data,
            ..self.clone()
        })
    }
    pub(super) fn mark_created_in_upgrade(&mut self, created: bool) {
        self.created_in_upgrade.set(created);
    }
    pub(super) fn without_indexes(&self) -> Self {
        Self {
            visible_indexes: false,
            ..self.clone()
        }
    }
}

pub(super) fn store_map_growth(name: &str) -> usize {
    map_entry_bytes::<String, IndexedDbObjectStoreMetadata>() + name.len()
}

pub(super) fn missing_mirror() -> IndexedDbError {
    IndexedDbError::InvalidState("IndexedDB schema mirror is unavailable".into())
}

pub(super) fn store_map(
    value: StoreMap,
    reservation: MetadataReservation,
) -> Rc<MetadataNode<StoreMap>> {
    Rc::new(MetadataNode { value, reservation })
}
