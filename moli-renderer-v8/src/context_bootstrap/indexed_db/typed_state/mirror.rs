use super::*;
use moli_indexeddb::sqlite::{IndexId, StoreId};
use std::collections::BTreeSet;

pub(super) type StoreMap = BTreeMap<moli_indexeddb::IndexedDbName, IndexedDbObjectStoreMetadata>;

pub(super) fn store_by_id(stores: &StoreMap, id: StoreId) -> Option<&IndexedDbObjectStoreMetadata> {
    stores.values().find(|store| store.id == id)
}

// Editing a candidate keeps every previously shared snapshot immutable.
pub(super) fn edit_shared<T: Clone>(value: &Rc<T>, edit: impl FnOnce(&mut T)) -> Rc<T> {
    let mut candidate = (**value).clone();
    edit(&mut candidate);
    Rc::new(candidate)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::context_bootstrap::indexed_db) struct IndexedDbObjectStoreMetadata {
    pub(super) data: Rc<IndexedDbObjectStoreMetadataData>,
    pub(super) id: StoreId,
    pub(super) created_in_upgrade: std::cell::Cell<bool>,
    pub(super) created_indexes: RefCell<Option<Rc<BTreeSet<IndexId>>>>,
    pub(super) visible_indexes: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::context_bootstrap::indexed_db) struct IndexedDbObjectStoreMetadataData {
    pub(super) info: ObjectStoreInfo,
    pub(super) indexes: BTreeMap<moli_indexeddb::IndexedDbName, IndexedDbIndexMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IndexedDbIndexMetadata {
    pub(super) info: Rc<IndexInfo>,
    pub(super) id: IndexId,
}

impl std::ops::Deref for IndexedDbObjectStoreMetadata {
    type Target = IndexedDbObjectStoreMetadataData;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl IndexedDbObjectStoreMetadata {
    fn from_data(id: StoreId, data: Rc<IndexedDbObjectStoreMetadataData>) -> Self {
        Self {
            data,
            id,
            created_in_upgrade: std::cell::Cell::new(false),
            created_indexes: RefCell::new(None),
            visible_indexes: true,
        }
    }

    pub(in crate::context_bootstrap::indexed_db) fn from_info(
        id: StoreId,
        info: &ObjectStoreInfo,
    ) -> Self {
        debug_assert!(info.index_names.is_empty());
        Self::from_data(
            id,
            Rc::new(IndexedDbObjectStoreMetadataData {
                info: info.clone(),
                indexes: BTreeMap::new(),
            }),
        )
    }

    pub(in crate::context_bootstrap::indexed_db) fn from_sql(
        name: &moli_indexeddb::IndexedDbName,
        store: &moli_indexeddb::sqlite::StoreMetadata,
    ) -> Self {
        let indexes = store
            .indexes
            .iter()
            .map(|(name, index)| {
                (
                    name.clone(),
                    IndexedDbIndexMetadata {
                        info: Rc::new(IndexInfo {
                            name: name.clone(),
                            key_path: index.options.key_path.clone(),
                            unique: index.options.unique,
                            multi_entry: index.options.multi_entry,
                        }),
                        id: index.id,
                    },
                )
            })
            .collect();
        let mut names = store.indexes.keys().cloned().collect::<Vec<_>>();
        names.sort_unstable();
        Self::from_data(
            store.id,
            Rc::new(IndexedDbObjectStoreMetadataData {
                info: ObjectStoreInfo {
                    name: name.into(),
                    key_path: store.options.key_path.clone(),
                    auto_increment: store.options.auto_increment,
                    index_names: names,
                },
                indexes,
            }),
        )
    }

    pub(in crate::context_bootstrap::indexed_db) fn id(&self) -> StoreId {
        self.id
    }

    pub(in crate::context_bootstrap::indexed_db) fn index_id(
        &self,
        name: &moli_indexeddb::IndexedDbName,
    ) -> Option<IndexId> {
        self.visible_indexes
            .then(|| self.indexes.get(name).map(|index| index.id))
            .flatten()
    }

    pub(super) fn index_by_id(&self, id: IndexId) -> Option<&IndexedDbIndexMetadata> {
        self.visible_indexes
            .then(|| self.indexes.values().find(|index| index.id == id))
            .flatten()
    }

    pub(in crate::context_bootstrap::indexed_db) fn info(&self) -> &ObjectStoreInfo {
        &self.info
    }
    pub(in crate::context_bootstrap::indexed_db) fn index_names(
        &self,
    ) -> &[moli_indexeddb::IndexedDbName] {
        if self.visible_indexes {
            &self.info.index_names
        } else {
            &[]
        }
    }
    pub(in crate::context_bootstrap::indexed_db) fn index(
        &self,
        name: &moli_indexeddb::IndexedDbName,
    ) -> Option<&IndexInfo> {
        self.visible_indexes
            .then(|| self.indexes.get(name).map(|index| &*index.info))
            .flatten()
    }
    pub(in crate::context_bootstrap::indexed_db) fn shared_index(
        &self,
        name: &moli_indexeddb::IndexedDbName,
    ) -> Option<Rc<IndexInfo>> {
        self.visible_indexes
            .then(|| self.indexes.get(name).map(|index| Rc::clone(&index.info)))
            .flatten()
    }
    #[cfg(test)]
    pub(in crate::context_bootstrap::indexed_db) fn indexes_in_name_order(&self) -> Vec<IndexInfo> {
        self.index_names()
            .iter()
            .filter_map(|name| self.index(name).cloned())
            .collect()
    }
    pub(super) fn index_created_in_upgrade(&self, name: &moli_indexeddb::IndexedDbName) -> bool {
        self.created_in_upgrade.get()
            || self.created_indexes.borrow().as_ref().is_some_and(|names| {
                self.indexes
                    .get(name)
                    .is_some_and(|index| names.contains(&index.id))
            })
    }
    pub(super) fn with_index(&self, id: IndexId, info: &IndexInfo) -> Self {
        let mut markers = self
            .created_indexes
            .borrow()
            .as_ref()
            .map(|names| (**names).clone())
            .unwrap_or_default();
        markers.insert(id);
        let data = edit_shared(&self.data, |data| {
            if !data.info.index_names.iter().any(|name| name == &info.name) {
                data.info.index_names.push(info.name.clone());
            }
            data.indexes.insert(
                info.name.clone(),
                IndexedDbIndexMetadata {
                    info: Rc::new(info.clone()),
                    id,
                },
            );
        });
        Self {
            data,
            created_indexes: RefCell::new(Some(Rc::new(markers))),
            ..self.clone()
        }
    }
    pub(super) fn without_index(&self, name: &moli_indexeddb::IndexedDbName) -> Self {
        let data = edit_shared(&self.data, |data| {
            data.info.index_names.retain(|candidate| candidate != name);
            data.indexes.remove(name);
        });
        let created_indexes = self.created_indexes.borrow().as_ref().map(|markers| {
            edit_shared(markers, |ids| {
                if let Some(index) = self.indexes.get(name) {
                    ids.remove(&index.id);
                }
            })
        });
        Self {
            data,
            created_indexes: RefCell::new(created_indexes),
            ..self.clone()
        }
    }
    pub(super) fn with_name(&self, name: &moli_indexeddb::IndexedDbName) -> Self {
        Self {
            data: edit_shared(&self.data, |data| data.info.name = name.clone()),
            ..self.clone()
        }
    }
    pub(super) fn with_renamed_index(
        &self,
        name: &moli_indexeddb::IndexedDbName,
        new_name: &moli_indexeddb::IndexedDbName,
    ) -> Self {
        let data = edit_shared(&self.data, |data| {
            let mut index = data.indexes.remove(name).expect("renamed index exists");
            index.info = edit_shared(&index.info, |info| info.name = new_name.clone());
            data.indexes.insert(new_name.clone(), index);
            for candidate in &mut data.info.index_names {
                if candidate == name {
                    *candidate = new_name.clone();
                }
            }
        });
        Self {
            data,
            ..self.clone()
        }
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

pub(super) fn missing_mirror() -> IndexedDbError {
    IndexedDbError::InvalidState("IndexedDB schema mirror is unavailable".into())
}
