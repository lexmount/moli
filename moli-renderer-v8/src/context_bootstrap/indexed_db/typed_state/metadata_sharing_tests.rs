use super::*;

#[derive(Default)]
pub(crate) struct IndexedDbMetadataSharingProbe {
    maps: Vec<std::rc::Weak<MetadataNode<StoreMap>>>,
    stores: Vec<std::rc::Weak<MetadataNode<IndexedDbObjectStoreMetadataData>>>,
    indexes: Vec<std::rc::Weak<MetadataNode<IndexInfo>>>,
    store_states: Vec<std::rc::Weak<RefCell<IndexedDbObjectStoreLifecycleState>>>,
    index_states: Vec<std::rc::Weak<RefCell<IndexedDbIndexLifecycleState>>>,
    pub(crate) store_handles: usize,
    pub(crate) index_handles: usize,
}

impl IndexedDbMetadataSharingProbe {
    pub(crate) fn live_nodes(&self) -> [usize; 3] {
        [
            self.maps
                .iter()
                .filter(|node| node.strong_count() != 0)
                .count(),
            self.stores
                .iter()
                .filter(|node| node.strong_count() != 0)
                .count(),
            self.indexes
                .iter()
                .filter(|node| node.strong_count() != 0)
                .count(),
        ]
    }

    pub(crate) fn live_handles(&self) -> [usize; 2] {
        [
            self.store_states
                .iter()
                .filter(|state| state.strong_count() != 0)
                .count(),
            self.index_states
                .iter()
                .filter(|state| state.strong_count() != 0)
                .count(),
        ]
    }

    fn index(&mut self, info: &Rc<MetadataNode<IndexInfo>>) {
        if !self
            .indexes
            .iter()
            .any(|node| node.as_ptr() == Rc::as_ptr(info))
        {
            self.indexes.push(Rc::downgrade(info));
        }
    }

    fn store(&mut self, metadata: &IndexedDbObjectStoreMetadata) {
        if self
            .stores
            .iter()
            .any(|node| node.as_ptr() == Rc::as_ptr(&metadata.data))
        {
            return;
        }
        self.stores.push(Rc::downgrade(&metadata.data));
        for info in metadata.indexes.values() {
            self.index(info);
        }
    }

    fn map(&mut self, map: &Rc<MetadataNode<StoreMap>>) {
        if self
            .maps
            .iter()
            .any(|node| node.as_ptr() == Rc::as_ptr(map))
        {
            return;
        }
        self.maps.push(Rc::downgrade(map));
        for store in map.values() {
            self.store(store);
        }
    }
}

pub(crate) fn indexed_db_metadata_sharing_probe_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    database: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbMetadataSharingProbe> {
    let id = indexed_db_typed_state_id(scope, database)?;
    let owner = indexed_db_runtime_state_table_for_object(scope, database);
    let mut probe = IndexedDbMetadataSharingProbe::default();
    {
        let table = owner.borrow();
        let state = table.databases.get(&id)?;
        let mirror = state.mirror.upgrade()?;
        let mirror = mirror.borrow();
        probe.map(mirror.metadata.as_ref()?);
        if let Some(snapshot) = &mirror.upgrade_metadata {
            probe.map(&snapshot.stores);
        }
    }
    for table in indexed_db_handle_tables(scope, database) {
        let indexes = {
            let table = table.borrow();
            for state in table
                .object_stores
                .values()
                .filter_map(std::rc::Weak::upgrade)
            {
                let store = state.borrow();
                if store.database.to_local(scope) == Some(database) {
                    probe.store_handles += 1;
                    probe.store_states.push(Rc::downgrade(&state));
                    probe.store(&store.metadata);
                }
            }
            table
                .indexes
                .values()
                .filter_map(|index| {
                    let state = index.upgrade()?;
                    let index = state.borrow();
                    Some((
                        index.object_store.to_local(scope)?,
                        index.info.clone(),
                        Rc::downgrade(&state),
                    ))
                })
                .collect::<Vec<_>>()
        };
        for (store, info, state) in indexes {
            if indexed_db_object_store_database(scope, store) == Some(database) {
                probe.index_handles += 1;
                probe.index_states.push(state);
                probe.index(&info);
            }
        }
    }
    Some(probe)
}
