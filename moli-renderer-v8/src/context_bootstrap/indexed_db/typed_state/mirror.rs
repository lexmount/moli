use super::*;
use moli_indexeddb::IndexedDbName;

pub(super) type StoreMap = BTreeMap<IndexedDbName, IndexedDbObjectStoreMetadata>;

// Editing a candidate keeps all previously shared snapshots immutable.
pub(super) fn edit_shared<T: Clone>(value: &Rc<T>, edit: impl FnOnce(&mut T)) -> Rc<T> {
    let mut candidate = (**value).clone();
    edit(&mut candidate);
    Rc::new(candidate)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IndexedDbIndexMetadata {
    pub(super) info: Rc<IndexInfo>,
    // The original immutable definition is the identity and rollback source.
    pub(super) origin: Rc<IndexInfo>,
    pub(super) created_in_upgrade: std::cell::Cell<bool>,
}

impl IndexedDbIndexMetadata {
    fn new(info: IndexInfo, created_in_upgrade: bool) -> Self {
        let info = Rc::new(info);
        Self {
            origin: Rc::clone(&info),
            info,
            created_in_upgrade: std::cell::Cell::new(created_in_upgrade),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::context_bootstrap::indexed_db) struct IndexedDbObjectStoreMetadata {
    pub(super) data: Rc<IndexedDbObjectStoreMetadataData>,
    pub(super) created_in_upgrade: std::cell::Cell<bool>,
    pub(super) visible_indexes: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::context_bootstrap::indexed_db) struct IndexedDbObjectStoreMetadataData {
    pub(super) info: Rc<ObjectStoreInfo>,
    pub(super) origin: Rc<ObjectStoreInfo>,
    pub(super) indexes: BTreeMap<IndexedDbName, IndexedDbIndexMetadata>,
}

impl std::ops::Deref for IndexedDbObjectStoreMetadata {
    type Target = IndexedDbObjectStoreMetadataData;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl IndexedDbObjectStoreMetadata {
    fn from_data(data: Rc<IndexedDbObjectStoreMetadataData>) -> Self {
        Self {
            data,
            created_in_upgrade: std::cell::Cell::new(false),
            visible_indexes: true,
        }
    }
    pub(in crate::context_bootstrap::indexed_db) fn from_info(info: &ObjectStoreInfo) -> Self {
        debug_assert!(info.index_names.is_empty());
        let info = Rc::new(info.clone());
        Self::from_data(Rc::new(IndexedDbObjectStoreMetadataData {
            origin: Rc::clone(&info),
            info,
            indexes: BTreeMap::new(),
        }))
    }
    pub(in crate::context_bootstrap::indexed_db) fn from_sql(
        name: &IndexedDbName,
        store: &moli_indexeddb::sqlite::StoreMetadata,
    ) -> Self {
        let indexes = store
            .indexes
            .iter()
            .map(|(name, index)| {
                (
                    name.clone(),
                    IndexedDbIndexMetadata::new(
                        IndexInfo {
                            name: name.clone(),
                            key_path: index.options.key_path.clone(),
                            unique: index.options.unique,
                            multi_entry: index.options.multi_entry,
                        },
                        false,
                    ),
                )
            })
            .collect();
        let info = Rc::new(ObjectStoreInfo {
            name: name.clone(),
            key_path: store.options.key_path.clone(),
            auto_increment: store.options.auto_increment,
            index_names: store.indexes.keys().cloned().collect(),
        });
        Self::from_data(Rc::new(IndexedDbObjectStoreMetadataData {
            origin: Rc::clone(&info),
            info,
            indexes,
        }))
    }
    pub(in crate::context_bootstrap::indexed_db) fn info(&self) -> &ObjectStoreInfo {
        &self.info
    }
    pub(in crate::context_bootstrap::indexed_db) fn index_names(&self) -> &[IndexedDbName] {
        if self.visible_indexes {
            &self.info.index_names
        } else {
            &[]
        }
    }
    pub(in crate::context_bootstrap::indexed_db) fn index(
        &self,
        name: &IndexedDbName,
    ) -> Option<&IndexInfo> {
        self.visible_indexes
            .then(|| self.indexes.get(name).map(|index| index.info.as_ref()))
            .flatten()
    }
    pub(in crate::context_bootstrap::indexed_db) fn shared_index(
        &self,
        name: &IndexedDbName,
    ) -> Option<Rc<IndexInfo>> {
        self.index_metadata(name).map(|index| index.info)
    }
    pub(super) fn index_metadata(&self, name: &IndexedDbName) -> Option<IndexedDbIndexMetadata> {
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
    pub(super) fn with_index(&self, info: &IndexInfo) -> Self {
        let data = edit_shared(&self.data, |data| {
            let store = Rc::make_mut(&mut data.info);
            if !store.index_names.contains(&info.name) {
                store.index_names.push(info.name.clone());
                store.index_names.sort_unstable();
            }
            data.indexes.insert(
                info.name.clone(),
                IndexedDbIndexMetadata::new(info.clone(), true),
            );
        });
        Self {
            data,
            ..self.clone()
        }
    }
    pub(super) fn without_index(&self, name: &IndexedDbName) -> Self {
        let data = edit_shared(&self.data, |data| {
            Rc::make_mut(&mut data.info)
                .index_names
                .retain(|candidate| candidate != name);
            data.indexes.remove(name);
        });
        Self {
            data,
            ..self.clone()
        }
    }
    pub(super) fn with_renamed_index(
        &self,
        old_name: &IndexedDbName,
        name: &IndexedDbName,
    ) -> Result<Self, IndexedDbError> {
        let mut index = self.index_metadata(old_name).ok_or_else(missing_mirror)?;
        index.info = Rc::new(IndexInfo {
            name: name.clone(),
            ..(*index.info).clone()
        });
        let data = edit_shared(&self.data, |data| {
            data.indexes.remove(old_name);
            data.indexes.insert(name.clone(), index);
            let store = Rc::make_mut(&mut data.info);
            store.index_names.retain(|candidate| candidate != old_name);
            store.index_names.push(name.clone());
            store.index_names.sort_unstable();
        });
        Ok(Self {
            data,
            ..self.clone()
        })
    }
    pub(super) fn with_name(&self, name: &IndexedDbName) -> Self {
        let data = edit_shared(&self.data, |data| {
            Rc::make_mut(&mut data.info).name = name.clone();
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
