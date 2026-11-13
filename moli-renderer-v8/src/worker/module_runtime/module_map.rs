use std::collections::hash_map::Entry;

use super::{
    WorkerBootstrapError, WorkerModuleGraphFetchId, WorkerModuleGraphFetchOptions,
    WorkerModuleGraphFetchRequest, WorkerModuleKey, WorkerModuleMap, WorkerModuleRecord,
    module_tree,
};

pub(super) enum WorkerModuleMapEntry {
    Fetching {
        request: Box<WorkerModuleGraphFetchRequest>,
        clients: Vec<module_tree::SingleModuleClientToken>,
    },
    Loaded(usize),
}

pub(super) struct WorkerModuleScript {
    pub(super) key: WorkerModuleKey,
    pub(super) response_url: url::Url,
    pub(super) referrer_policy: Option<String>,
    pub(super) creation: Result<WorkerModuleRecord, Box<WorkerBootstrapError>>,
}

impl WorkerModuleMap {
    pub(super) fn loaded_entry(&self, key: &WorkerModuleKey) -> Option<usize> {
        match self.entries.get(key)? {
            WorkerModuleMapEntry::Loaded(entry) => Some(*entry),
            WorkerModuleMapEntry::Fetching { .. } => None,
        }
    }

    pub(super) fn start_or_join_fetch(
        &mut self,
        key: WorkerModuleKey,
        client: module_tree::SingleModuleClientToken,
        options: &WorkerModuleGraphFetchOptions,
        outbox: &mut Vec<WorkerModuleGraphFetchRequest>,
    ) -> module_tree::SingleModuleFetchDisposition {
        match self.entries.entry(key.clone()) {
            Entry::Occupied(mut entry) => match entry.get_mut() {
                WorkerModuleMapEntry::Loaded(entry) => {
                    let entry = *entry;
                    module_tree::SingleModuleFetchDisposition::Completed(super::tree::ready_module(
                        self, entry,
                    ))
                }
                WorkerModuleMapEntry::Fetching { clients, .. } => {
                    if !clients.contains(&client) {
                        clients.push(client);
                    }
                    module_tree::SingleModuleFetchDisposition::JoinedExistingFetch
                }
            },
            Entry::Vacant(entry) => {
                let fetch_id = self.next_fetch_id;
                self.next_fetch_id = fetch_id
                    .checked_add(1)
                    .expect("worker module fetch id space exhausted");
                let request = WorkerModuleGraphFetchRequest::new(
                    fetch_id,
                    key,
                    options.initiator_url.clone(),
                    options.csp_source,
                    self.credentials_mode,
                    options.referrer_policy.clone(),
                    options.browser_request_metadata,
                );
                entry.insert(WorkerModuleMapEntry::Fetching {
                    request: Box::new(request.clone()),
                    clients: vec![client],
                });
                self.pending_fetch_count += 1;
                // The runtime owns the action even if this tree fails before
                // returning NeedFetches. A fetch entry never depends on a job
                // retaining and forwarding its transport request.
                outbox.push(request);
                module_tree::SingleModuleFetchDisposition::StartedNetworkFetch {
                    fetch_id: module_tree::ModuleFetchId(fetch_id),
                }
            }
        }
    }

    pub(super) fn take_fetch(
        &mut self,
        key: &WorkerModuleKey,
        fetch_id: WorkerModuleGraphFetchId,
    ) -> Option<(
        WorkerModuleGraphFetchRequest,
        Vec<module_tree::SingleModuleClientToken>,
    )> {
        if !matches!(self.entries.get(key), Some(WorkerModuleMapEntry::Fetching { request, .. }) if request.fetch_id == fetch_id)
        {
            return None;
        }
        let Some(WorkerModuleMapEntry::Fetching { request, clients }) = self.entries.remove(key)
        else {
            unreachable!("checked the exact module fetch above");
        };
        self.pending_fetch_count -= 1;
        Some((*request, clients))
    }

    pub(super) fn detach_tree(&mut self, tree_id: module_tree::ModuleTreeId) {
        for entry in self.entries.values_mut() {
            if let WorkerModuleMapEntry::Fetching { clients, .. } = entry {
                clients.retain(|client| client.tree_id != tree_id);
            }
        }
    }

    pub(super) fn has_pending_fetches(&self) -> bool {
        self.pending_fetch_count != 0
    }

    pub(super) fn insert_script(&mut self, script: WorkerModuleScript) -> usize {
        let entry = self.scripts.len();
        self.entries
            .insert(script.key.clone(), WorkerModuleMapEntry::Loaded(entry));
        self.scripts.push(script);
        entry
    }

    pub(super) fn script(&self, entry: usize) -> &WorkerModuleScript {
        &self.scripts[entry]
    }

    pub(super) fn record(&self, entry: usize) -> &WorkerModuleRecord {
        self.scripts[entry].creation.as_ref().unwrap_or_else(|_| {
            unreachable!("only successfully created module scripts have V8 records")
        })
    }

    pub(super) fn record_mut(&mut self, entry: usize) -> &mut WorkerModuleRecord {
        self.scripts[entry].creation.as_mut().unwrap_or_else(|_| {
            unreachable!("only successfully created module scripts have V8 records")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::module_runtime::WorkerModuleGraphFetchCspSource;
    use module_tree::{ModuleTreeId, SingleModuleClientToken, SingleModuleFetchDisposition};
    use moli_fetch::{BrowserRequestMetadata, RequestCredentialsMode};

    #[test]
    fn late_attempt_completion_cannot_settle_a_retry_or_retire_its_clients() {
        let url = url::Url::parse("https://example.test/module.mjs").unwrap();
        let key = WorkerModuleKey::javascript_or_wasm(url.clone());
        let options = WorkerModuleGraphFetchOptions {
            initiator_url: url,
            csp_source: WorkerModuleGraphFetchCspSource::DynamicImportGraph,
            referrer_policy: None,
            browser_request_metadata: BrowserRequestMetadata::Fetch,
        };
        let client = |id| SingleModuleClientToken {
            tree_id: ModuleTreeId(id),
            sequence: 0,
        };
        let mut modules = WorkerModuleMap::new(RequestCredentialsMode::SameOrigin, None);
        let mut outbox = Vec::new();
        let SingleModuleFetchDisposition::StartedNetworkFetch { fetch_id: first } =
            modules.start_or_join_fetch(key.clone(), client(1), &options, &mut outbox)
        else {
            panic!("first attempt should start a request");
        };
        let (_, original_clients) = modules.take_fetch(&key, first.0).unwrap();
        assert_eq!(original_clients, [client(1)]);
        assert!(!modules.has_pending_fetches());
        let SingleModuleFetchDisposition::StartedNetworkFetch { fetch_id: retry } =
            modules.start_or_join_fetch(key.clone(), client(2), &options, &mut outbox)
        else {
            panic!("a later explicit import should retry the failed load");
        };
        assert!(modules.take_fetch(&key, first.0).is_none());
        assert!(modules.has_pending_fetches());
        assert_eq!(
            modules.start_or_join_fetch(key.clone(), client(3), &options, &mut outbox),
            SingleModuleFetchDisposition::JoinedExistingFetch
        );
        modules.detach_tree(ModuleTreeId(2));
        modules.detach_tree(ModuleTreeId(3));
        assert!(
            modules.has_pending_fetches(),
            "retiring trees does not orphan their physical request"
        );
        assert_eq!(
            modules.start_or_join_fetch(key.clone(), client(4), &options, &mut outbox),
            SingleModuleFetchDisposition::JoinedExistingFetch
        );
        let (_, retry_clients) = modules.take_fetch(&key, retry.0).unwrap();
        assert_eq!(retry_clients, [client(4)]);
        assert_eq!(
            original_clients,
            [client(1)],
            "old failure recipients remain immutable"
        );
        assert_eq!(outbox.len(), 2, "one network action per attempt");
        assert!(modules.take_fetch(&key, retry.0).is_none());
        assert!(!modules.has_pending_fetches());
    }
}
