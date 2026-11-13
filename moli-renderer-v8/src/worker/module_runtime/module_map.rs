use std::collections::{HashMap, HashSet, hash_map::Entry};

use super::{
    WorkerBootstrapError, WorkerModuleGraphFetchBatch, WorkerModuleGraphFetchId,
    WorkerModuleGraphFetchRequest, WorkerModuleKey, WorkerModuleRecord,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum WorkerModuleWaiter {
    Bootstrap,
    DynamicImport(u64),
}

enum WorkerModuleMapEntry {
    Fetching {
        request: Box<WorkerModuleGraphFetchRequest>,
        waiters: HashSet<WorkerModuleWaiter>,
    },
    Compiled {
        entry: usize,
        module: Box<WorkerModuleRecord>,
    },
    ParseError(Box<WorkerBootstrapError>),
}

#[derive(Default)]
pub(super) struct WorkerModuleMap {
    entries: HashMap<WorkerModuleKey, WorkerModuleMapEntry>,
    fetch_keys: HashMap<WorkerModuleGraphFetchId, WorkerModuleKey>,
    // Stable graph indices survive transitions of unrelated module-map entries.
    compiled_keys: Vec<WorkerModuleKey>,
}

impl WorkerModuleMap {
    pub(super) fn compiled_entry(&self, key: &WorkerModuleKey) -> Option<usize> {
        match self.entries.get(key)? {
            WorkerModuleMapEntry::Compiled { entry, .. } => Some(*entry),
            _ => None,
        }
    }

    pub(super) fn compile_error(&self, key: &WorkerModuleKey) -> Option<Box<WorkerBootstrapError>> {
        match self.entries.get(key)? {
            WorkerModuleMapEntry::ParseError(error) => Some(error.clone()),
            _ => None,
        }
    }

    pub(super) fn fetch_id(&self, key: &WorkerModuleKey) -> Option<WorkerModuleGraphFetchId> {
        match self.entries.get(key)? {
            WorkerModuleMapEntry::Fetching { request, .. } => Some(request.fetch_id),
            _ => None,
        }
    }

    // Subscribe only after graph discovery succeeds. Each physical fetch is
    // admitted once; importing jobs keep keys, never copies of these requests.
    pub(super) fn subscribe(
        &mut self,
        waiter: WorkerModuleWaiter,
        requests: WorkerModuleGraphFetchBatch,
    ) -> Option<WorkerModuleGraphFetchBatch> {
        let mut new_requests = Vec::new();
        for request in requests.requests {
            match self.entries.entry(request.key.clone()) {
                Entry::Vacant(entry) => {
                    self.fetch_keys
                        .insert(request.fetch_id, request.key.clone());
                    entry.insert(WorkerModuleMapEntry::Fetching {
                        request: Box::new(request.clone()),
                        waiters: HashSet::from([waiter]),
                    });
                    new_requests.push(request);
                }
                Entry::Occupied(mut entry) => {
                    let WorkerModuleMapEntry::Fetching { waiters, .. } = entry.get_mut() else {
                        unreachable!("only unresolved module keys can subscribe to a fetch");
                    };
                    waiters.insert(waiter);
                }
            }
        }
        (!new_requests.is_empty()).then(|| WorkerModuleGraphFetchBatch::new(new_requests))
    }

    pub(super) fn unsubscribe(
        &mut self,
        waiter: WorkerModuleWaiter,
        keys: impl IntoIterator<Item = WorkerModuleKey>,
    ) {
        for key in keys {
            if let Some(WorkerModuleMapEntry::Fetching { waiters, .. }) = self.entries.get_mut(&key)
            {
                waiters.remove(&waiter);
            }
        }
    }

    pub(super) fn take_fetch(
        &mut self,
        fetch_id: WorkerModuleGraphFetchId,
    ) -> Option<(WorkerModuleGraphFetchRequest, HashSet<WorkerModuleWaiter>)> {
        let key = self.fetch_keys.remove(&fetch_id)?;
        let Some(WorkerModuleMapEntry::Fetching { request, waiters }) = self.entries.remove(&key)
        else {
            unreachable!("fetch index must identify a fetching module-map entry");
        };
        // Load failures leave this key absent, permitting a later explicit
        // import to retry. Successful fetches replace it with Compiled or
        // ParseError before any of these waiters are woken.
        Some((*request, waiters))
    }

    pub(super) fn has_pending_fetches(&self) -> bool {
        !self.fetch_keys.is_empty()
    }

    pub(super) fn insert_compiled(&mut self, module: WorkerModuleRecord) -> usize {
        let entry = self.compiled_keys.len();
        self.compiled_keys.push(module.key.clone());
        self.entries.insert(
            module.key.clone(),
            WorkerModuleMapEntry::Compiled {
                entry,
                module: Box::new(module),
            },
        );
        entry
    }

    pub(super) fn insert_parse_error(
        &mut self,
        key: WorkerModuleKey,
        error: Box<WorkerBootstrapError>,
    ) {
        self.entries
            .insert(key, WorkerModuleMapEntry::ParseError(error));
    }

    pub(super) fn record(&self, entry: usize) -> &WorkerModuleRecord {
        let WorkerModuleMapEntry::Compiled { module, .. } =
            &self.entries[&self.compiled_keys[entry]]
        else {
            unreachable!("compiled graph index must identify a compiled module");
        };
        module
    }

    pub(super) fn record_mut(&mut self, entry: usize) -> &mut WorkerModuleRecord {
        let Some(WorkerModuleMapEntry::Compiled { module, .. }) =
            self.entries.get_mut(&self.compiled_keys[entry])
        else {
            unreachable!("compiled graph index must identify a compiled module");
        };
        module
    }

    pub(super) fn records(&self) -> impl Iterator<Item = &WorkerModuleRecord> {
        self.compiled_keys
            .iter()
            .map(|key| match &self.entries[key] {
                WorkerModuleMapEntry::Compiled { module, .. } => module.as_ref(),
                _ => unreachable!("compiled graph index must identify a compiled module"),
            })
    }
}
