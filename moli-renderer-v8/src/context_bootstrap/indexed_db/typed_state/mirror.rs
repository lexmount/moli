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
    pub(super) info: ObjectStoreInfo,
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
        Self::from_data(Rc::new(IndexedDbObjectStoreMetadataData {
            info: info.clone(),
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
        Self::from_data(Rc::new(IndexedDbObjectStoreMetadataData {
            info: ObjectStoreInfo {
                name: name.clone(),
                key_path: store.options.key_path.clone(),
                auto_increment: store.options.auto_increment,
                index_names: store.indexes.keys().cloned().collect(),
            },
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
            if !data.info.index_names.contains(&info.name) {
                data.info.index_names.push(info.name.clone());
                data.info.index_names.sort_unstable();
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
            data.info.index_names.retain(|candidate| candidate != name);
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
            data.info
                .index_names
                .retain(|candidate| candidate != old_name);
            data.info.index_names.push(name.clone());
            data.info.index_names.sort_unstable();
        });
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

pub(super) fn missing_mirror() -> IndexedDbError {
    IndexedDbError::InvalidState("IndexedDB schema mirror is unavailable".into())
}
