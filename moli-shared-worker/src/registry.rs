use std::collections::HashMap;

use parking_lot::Mutex;

use crate::{
    SharedWorkerClientId, SharedWorkerCompatibilityError, SharedWorkerDescriptor,
    SharedWorkerInstanceId, SharedWorkerKey,
};

#[derive(Debug)]
enum SharedWorkerEntryState<I> {
    Loading,
    Running { instance: I },
}

#[derive(Debug)]
struct SharedWorkerEntry<I> {
    instance_id: SharedWorkerInstanceId,
    descriptor: SharedWorkerDescriptor,
    clients: Vec<SharedWorkerClientId>,
    state: SharedWorkerEntryState<I>,
}

impl<I> SharedWorkerEntry<I> {
    fn ensure_compatible_with(
        &self,
        requested: &SharedWorkerDescriptor,
    ) -> Result<(), SharedWorkerCompatibilityError> {
        self.descriptor.ensure_compatible_with(requested)
    }

    fn remove_client(&mut self, client_id: SharedWorkerClientId) -> bool {
        self.clients.retain(|current| *current != client_id);
        self.clients.is_empty()
    }
}

#[derive(Debug, Default)]
struct SharedWorkerRegistryState<I> {
    next_client_id: u64,
    next_instance_id: u64,
    entries: HashMap<SharedWorkerKey, SharedWorkerEntry<I>>,
    client_keys: HashMap<SharedWorkerClientId, SharedWorkerKey>,
}

impl<I> SharedWorkerRegistryState<I> {
    fn next_client_id(&mut self) -> SharedWorkerClientId {
        self.next_client_id += 1;
        SharedWorkerClientId::new(self.next_client_id)
    }

    fn next_instance_id(&mut self) -> SharedWorkerInstanceId {
        self.next_instance_id += 1;
        SharedWorkerInstanceId::new(self.next_instance_id)
    }
}

/// Result of a renderer attempting to connect one SharedWorker client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SharedWorkerConnectAction<I> {
    /// No compatible slot existed. The embedder should start loading the script.
    StartLoading {
        instance_id: SharedWorkerInstanceId,
        client_id: SharedWorkerClientId,
    },
    /// A compatible slot is loading. The embedder should queue this client's
    /// MessagePort until the script load resolves.
    QueueWhileLoading {
        instance_id: SharedWorkerInstanceId,
        client_id: SharedWorkerClientId,
    },
    /// A compatible worker is already running. The embedder can dispatch the
    /// connect event immediately.
    ConnectToRunning {
        instance_id: SharedWorkerInstanceId,
        client_id: SharedWorkerClientId,
        instance: I,
    },
    /// A slot exists but constructor options are incompatible. The embedder may
    /// still surface this as an async client error after constructing the JS
    /// SharedWorker wrapper.
    RejectClient {
        client_id: SharedWorkerClientId,
        error: SharedWorkerCompatibilityError,
    },
}

/// Result of transitioning a loading slot into running.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SharedWorkerLoadReady<I> {
    Running {
        instance_id: SharedWorkerInstanceId,
        clients: Vec<SharedWorkerClientId>,
        instance: I,
    },
    Stale,
}

/// Result of failing a loading slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SharedWorkerLoadFailure {
    Failed {
        instance_id: SharedWorkerInstanceId,
        clients: Vec<SharedWorkerClientId>,
    },
    Stale,
}

/// Result of removing one client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SharedWorkerClientRemoval<I> {
    RemovedFromLoading {
        instance_id: SharedWorkerInstanceId,
    },
    RemovedFromRunning {
        instance_id: SharedWorkerInstanceId,
        instance: I,
    },
    CancelLoading {
        instance_id: SharedWorkerInstanceId,
        key: SharedWorkerKey,
    },
    Terminate {
        instance_id: SharedWorkerInstanceId,
        key: SharedWorkerKey,
        instance: I,
    },
    Missing,
}

/// Result of removing an instance because the worker closed or crashed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SharedWorkerInstanceRemoval<I> {
    Removed {
        key: SharedWorkerKey,
        instance_id: SharedWorkerInstanceId,
        clients: Vec<SharedWorkerClientId>,
        instance: Option<I>,
    },
    Missing,
}

/// Owner-scoped registry for SharedWorker instances and clients.
#[derive(Debug)]
pub struct SharedWorkerRegistry<I> {
    state: Mutex<SharedWorkerRegistryState<I>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SharedWorkerRegistryDiagnostics {
    pub entry_count: usize,
    pub loading_instance_count: usize,
    pub running_instance_count: usize,
    pub client_count: usize,
}

impl<I> Default for SharedWorkerRegistry<I> {
    fn default() -> Self {
        Self {
            state: Mutex::new(SharedWorkerRegistryState {
                next_client_id: 0,
                next_instance_id: 0,
                entries: HashMap::new(),
                client_keys: HashMap::new(),
            }),
        }
    }
}

impl<I> SharedWorkerRegistry<I> {
    pub fn diagnostics(&self) -> SharedWorkerRegistryDiagnostics {
        let state = self.state.lock();
        let mut diagnostics = SharedWorkerRegistryDiagnostics {
            entry_count: state.entries.len(),
            ..Default::default()
        };
        for entry in state.entries.values() {
            diagnostics.client_count += entry.clients.len();
            match entry.state {
                SharedWorkerEntryState::Loading => {
                    diagnostics.loading_instance_count += 1;
                }
                SharedWorkerEntryState::Running { .. } => {
                    diagnostics.running_instance_count += 1;
                }
            }
        }
        diagnostics
    }
}

impl<I> SharedWorkerRegistry<I>
where
    I: Clone,
{
    /// Connect a new client to a SharedWorker key and return the embedder action.
    pub fn connect(
        &self,
        key: SharedWorkerKey,
        descriptor: SharedWorkerDescriptor,
    ) -> SharedWorkerConnectAction<I> {
        let mut state = self.state.lock();
        let client_id = state.next_client_id();
        let instance_id = state.next_instance_id();
        if let Some(entry) = state.entries.get_mut(&key) {
            if let Err(error) = entry.ensure_compatible_with(&descriptor) {
                return SharedWorkerConnectAction::RejectClient { client_id, error };
            }
            let instance_id = entry.instance_id;
            entry.clients.push(client_id);
            let action = match &entry.state {
                SharedWorkerEntryState::Loading => SharedWorkerConnectAction::QueueWhileLoading {
                    instance_id,
                    client_id,
                },
                SharedWorkerEntryState::Running { instance } => {
                    SharedWorkerConnectAction::ConnectToRunning {
                        instance_id,
                        client_id,
                        instance: instance.clone(),
                    }
                }
            };
            state.client_keys.insert(client_id, key);
            return action;
        }
        state.client_keys.insert(client_id, key.clone());
        state.entries.insert(
            key,
            SharedWorkerEntry {
                instance_id,
                descriptor,
                clients: vec![client_id],
                state: SharedWorkerEntryState::Loading,
            },
        );
        SharedWorkerConnectAction::StartLoading {
            instance_id,
            client_id,
        }
    }

    /// Mark a loading slot as running and return clients that should receive
    /// connect events.
    pub fn finish_loading(
        &self,
        key: &SharedWorkerKey,
        instance_id: SharedWorkerInstanceId,
        instance: I,
    ) -> SharedWorkerLoadReady<I> {
        let mut state = self.state.lock();
        let Some(entry) = state.entries.get_mut(key) else {
            return SharedWorkerLoadReady::Stale;
        };
        if entry.instance_id != instance_id {
            return SharedWorkerLoadReady::Stale;
        }
        if !matches!(entry.state, SharedWorkerEntryState::Loading) || entry.clients.is_empty() {
            return SharedWorkerLoadReady::Stale;
        }
        entry.state = SharedWorkerEntryState::Running {
            instance: instance.clone(),
        };
        SharedWorkerLoadReady::Running {
            instance_id,
            clients: entry.clients.clone(),
            instance,
        }
    }

    /// Fail a loading slot and remove all pending clients.
    pub fn fail_loading(
        &self,
        key: &SharedWorkerKey,
        instance_id: SharedWorkerInstanceId,
    ) -> SharedWorkerLoadFailure {
        let mut state = self.state.lock();
        let Some(entry) = state.entries.get(key) else {
            return SharedWorkerLoadFailure::Stale;
        };
        if entry.instance_id != instance_id
            || !matches!(entry.state, SharedWorkerEntryState::Loading)
        {
            return SharedWorkerLoadFailure::Stale;
        }
        let entry = state.entries.remove(key).expect("entry checked above");
        let clients = entry.clients.clone();
        for client_id in &clients {
            state.client_keys.remove(client_id);
        }
        SharedWorkerLoadFailure::Failed {
            instance_id,
            clients,
        }
    }

    /// Remove one client and return whether the embedder should cancel/terminate.
    pub fn remove_client(&self, client_id: SharedWorkerClientId) -> SharedWorkerClientRemoval<I> {
        let mut state = self.state.lock();
        let Some(key) = state.client_keys.remove(&client_id) else {
            return SharedWorkerClientRemoval::Missing;
        };
        let Some(entry) = state.entries.get_mut(&key) else {
            return SharedWorkerClientRemoval::Missing;
        };
        let instance_id = entry.instance_id;
        if !entry.remove_client(client_id) {
            return match &entry.state {
                SharedWorkerEntryState::Loading => {
                    SharedWorkerClientRemoval::RemovedFromLoading { instance_id }
                }
                SharedWorkerEntryState::Running { instance } => {
                    SharedWorkerClientRemoval::RemovedFromRunning {
                        instance_id,
                        instance: instance.clone(),
                    }
                }
            };
        }
        let entry = state.entries.remove(&key).expect("entry checked above");
        match entry.state {
            SharedWorkerEntryState::Loading => {
                SharedWorkerClientRemoval::CancelLoading { instance_id, key }
            }
            SharedWorkerEntryState::Running { instance } => SharedWorkerClientRemoval::Terminate {
                instance_id,
                key,
                instance,
            },
        }
    }

    /// Return all clients currently attached to an instance.
    pub fn clients_for_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Vec<SharedWorkerClientId> {
        let state = self.state.lock();
        state
            .entries
            .values()
            .find(|entry| entry.instance_id == instance_id)
            .map(|entry| entry.clients.clone())
            .unwrap_or_default()
    }

    /// Return clients still waiting on a loading instance.
    pub fn loading_clients_for_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Vec<SharedWorkerClientId> {
        let state = self.state.lock();
        state
            .entries
            .values()
            .find(|entry| {
                entry.instance_id == instance_id
                    && matches!(entry.state, SharedWorkerEntryState::Loading)
            })
            .map(|entry| entry.clients.clone())
            .unwrap_or_default()
    }

    /// Return the running embedder instance for an instance id.
    pub fn running_instance(&self, instance_id: SharedWorkerInstanceId) -> Option<I> {
        let state = self.state.lock();
        state
            .entries
            .values()
            .find(|entry| entry.instance_id == instance_id)
            .and_then(|entry| match &entry.state {
                SharedWorkerEntryState::Loading => None,
                SharedWorkerEntryState::Running { instance } => Some(instance.clone()),
            })
    }

    /// Remove one instance, usually because its worker thread closed.
    pub fn remove_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> SharedWorkerInstanceRemoval<I> {
        let mut state = self.state.lock();
        let Some(key) = state
            .entries
            .iter()
            .find_map(|(key, entry)| (entry.instance_id == instance_id).then(|| key.clone()))
        else {
            return SharedWorkerInstanceRemoval::Missing;
        };
        let entry = state.entries.remove(&key).expect("entry checked above");
        let clients = entry.clients.clone();
        for client_id in &clients {
            state.client_keys.remove(client_id);
        }
        let instance = match entry.state {
            SharedWorkerEntryState::Loading => None,
            SharedWorkerEntryState::Running { instance } => Some(instance),
        };
        SharedWorkerInstanceRemoval::Removed {
            key,
            instance_id,
            clients,
            instance,
        }
    }

    /// Remove every loading or running instance when its Context shuts down.
    pub fn remove_all_instances(&self) -> Vec<SharedWorkerInstanceRemoval<I>> {
        let mut state = self.state.lock();
        let entries = std::mem::take(&mut state.entries);
        state.client_keys.clear();
        entries
            .into_iter()
            .map(|(key, entry)| {
                let clients = entry.clients.clone();
                let instance_id = entry.instance_id;
                let instance = match entry.state {
                    SharedWorkerEntryState::Loading => None,
                    SharedWorkerEntryState::Running { instance } => Some(instance),
                };
                SharedWorkerInstanceRemoval::Removed {
                    key,
                    instance_id,
                    clients,
                    instance,
                }
            })
            .collect()
    }

    /// Return whether the registry has no live entries.
    pub fn is_empty(&self) -> bool {
        self.state.lock().entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use moli_storage_key::MoliStorageKey;

    use super::*;
    use crate::{
        SharedWorkerCreationContextType, SharedWorkerCredentialsMode, SharedWorkerSameSiteCookies,
        SharedWorkerScriptType,
    };

    fn key(name: &str) -> SharedWorkerKey {
        SharedWorkerKey::new(
            first_party_storage_key("https://example.test", "https://example.test"),
            "https://example.test/worker.js".to_owned(),
            name.to_owned(),
            SharedWorkerSameSiteCookies::All,
        )
    }

    fn first_party_storage_key(origin: &str, top_level_site: &str) -> MoliStorageKey {
        MoliStorageKey::new(
            origin.to_owned(),
            top_level_site.to_owned(),
            None,
            moli_storage_key::StoragePartitionRelation::FirstParty,
        )
    }

    fn partitioned_storage_key(origin: &str, top_level_site: &str) -> MoliStorageKey {
        MoliStorageKey::new(
            origin.trim_end_matches('/').to_owned(),
            top_level_site.to_owned(),
            None,
            moli_storage_key::StoragePartitionRelation::ThirdParty,
        )
    }

    #[test]
    fn same_key_queues_while_loading_then_connects_to_running_instance() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let descriptor = SharedWorkerDescriptor::default();
        let key = key("a");

        let first = registry.connect(key.clone(), descriptor.clone());
        let (instance_id, first_client) = match first {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected StartLoading, got {other:?}"),
        };
        let second = registry.connect(key.clone(), descriptor.clone());
        let second_client = match second {
            SharedWorkerConnectAction::QueueWhileLoading {
                instance_id: queued_id,
                client_id,
            } => {
                assert_eq!(queued_id, instance_id);
                client_id
            }
            other => panic!("expected QueueWhileLoading, got {other:?}"),
        };

        let loaded = registry.finish_loading(&key, instance_id, 7);
        assert!(matches!(
            loaded,
            SharedWorkerLoadReady::Running {
                instance: 7,
                clients,
                ..
            } if clients == vec![first_client, second_client]
        ));

        let third = registry.connect(key, descriptor);
        assert!(matches!(
            third,
            SharedWorkerConnectAction::ConnectToRunning { instance: 7, .. }
        ));
    }

    #[test]
    fn type_mismatch_rejects_client_not_another_instance() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("a");
        let first = registry.connect(key.clone(), SharedWorkerDescriptor::default());
        let instance_id = match first {
            SharedWorkerConnectAction::StartLoading { instance_id, .. } => instance_id,
            other => panic!("expected StartLoading, got {other:?}"),
        };
        registry.finish_loading(&key, instance_id, 1);

        let rejected = registry.connect(
            key,
            SharedWorkerDescriptor::new(
                SharedWorkerScriptType::Module,
                SharedWorkerCredentialsMode::SameOrigin,
                false,
                SharedWorkerCreationContextType::Secure,
            ),
        );

        assert!(matches!(
            rejected,
            SharedWorkerConnectAction::RejectClient {
                error: SharedWorkerCompatibilityError::ScriptType { .. },
                ..
            }
        ));
    }

    #[test]
    fn extended_lifetime_mismatch_rejects_client_not_another_instance() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("a");
        let first = registry.connect(key.clone(), SharedWorkerDescriptor::default());
        let instance_id = match first {
            SharedWorkerConnectAction::StartLoading { instance_id, .. } => instance_id,
            other => panic!("expected StartLoading, got {other:?}"),
        };
        registry.finish_loading(&key, instance_id, 1);

        let rejected = registry.connect(
            key,
            SharedWorkerDescriptor::new(
                SharedWorkerScriptType::Classic,
                SharedWorkerCredentialsMode::SameOrigin,
                true,
                SharedWorkerCreationContextType::Secure,
            ),
        );

        assert!(matches!(
            rejected,
            SharedWorkerConnectAction::RejectClient {
                error: SharedWorkerCompatibilityError::ExtendedLifetime {
                    existing: false,
                    requested: true,
                },
                ..
            }
        ));
        assert_eq!(registry.diagnostics().entry_count, 1);
    }

    #[test]
    fn creation_context_mismatch_rejects_client_not_another_instance() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("a");
        let first = registry.connect(key.clone(), SharedWorkerDescriptor::default());
        let instance_id = match first {
            SharedWorkerConnectAction::StartLoading { instance_id, .. } => instance_id,
            other => panic!("expected StartLoading, got {other:?}"),
        };
        registry.finish_loading(&key, instance_id, 1);

        let rejected = registry.connect(
            key,
            SharedWorkerDescriptor::new(
                SharedWorkerScriptType::Classic,
                SharedWorkerCredentialsMode::SameOrigin,
                false,
                SharedWorkerCreationContextType::Nonsecure,
            ),
        );

        assert!(matches!(
            rejected,
            SharedWorkerConnectAction::RejectClient {
                error: SharedWorkerCompatibilityError::CreationContextType { .. },
                ..
            }
        ));
    }

    #[test]
    fn same_site_cookie_mode_is_part_of_matching_key() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let storage_key = first_party_storage_key("https://example.test", "https://example.test");
        let all_key = SharedWorkerKey::new(
            storage_key.clone(),
            "https://example.test/worker.js".to_owned(),
            "cookies".to_owned(),
            SharedWorkerSameSiteCookies::All,
        );
        let none_key = SharedWorkerKey::new(
            storage_key,
            "https://example.test/worker.js".to_owned(),
            "cookies".to_owned(),
            SharedWorkerSameSiteCookies::None,
        );
        let first = registry.connect(all_key, SharedWorkerDescriptor::default());
        let first_instance_id = match first {
            SharedWorkerConnectAction::StartLoading { instance_id, .. } => instance_id,
            other => panic!("expected StartLoading for All key, got {other:?}"),
        };

        let second = registry.connect(none_key, SharedWorkerDescriptor::default());
        assert!(matches!(
            second,
            SharedWorkerConnectAction::StartLoading { instance_id, .. }
                if instance_id != first_instance_id
        ));
    }

    #[test]
    fn storage_key_is_part_of_matching_key() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let first_party_key =
            first_party_storage_key("https://cdn.example.test", "https://example.test");
        let third_party_key =
            partitioned_storage_key("https://cdn.example.test", "https://other.test");
        let first_key = SharedWorkerKey::new(
            first_party_key,
            "https://cdn.example.test/worker.js".to_owned(),
            "partitioned-worker".to_owned(),
            SharedWorkerSameSiteCookies::None,
        );
        let second_key = SharedWorkerKey::new(
            third_party_key,
            "https://cdn.example.test/worker.js".to_owned(),
            "partitioned-worker".to_owned(),
            SharedWorkerSameSiteCookies::None,
        );
        let first = registry.connect(first_key, SharedWorkerDescriptor::default());
        let first_instance_id = match first {
            SharedWorkerConnectAction::StartLoading { instance_id, .. } => instance_id,
            other => panic!("expected StartLoading for first storage key, got {other:?}"),
        };

        let second = registry.connect(second_key, SharedWorkerDescriptor::default());
        assert!(matches!(
            second,
            SharedWorkerConnectAction::StartLoading { instance_id, .. }
                if instance_id != first_instance_id
        ));
    }

    #[test]
    fn same_site_cookie_mode_defaults_from_storage_key_relation() {
        let first_party = first_party_storage_key("https://example.test", "https://example.test");
        let third_party = partitioned_storage_key("https://cdn.example.test", "https://other.test");

        assert_eq!(
            SharedWorkerSameSiteCookies::default_for_storage_key(&first_party),
            SharedWorkerSameSiteCookies::All
        );
        assert_eq!(
            SharedWorkerSameSiteCookies::default_for_storage_key(&third_party),
            SharedWorkerSameSiteCookies::None
        );
        assert!(SharedWorkerSameSiteCookies::All.is_allowed_for_storage_key(&first_party));
        assert!(!SharedWorkerSameSiteCookies::All.is_allowed_for_storage_key(&third_party));
    }

    #[test]
    fn failing_load_removes_all_pending_clients() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("a");
        let descriptor = SharedWorkerDescriptor::default();
        let first = registry.connect(key.clone(), descriptor.clone());
        let instance_id = match first {
            SharedWorkerConnectAction::StartLoading { instance_id, .. } => instance_id,
            other => panic!("expected StartLoading, got {other:?}"),
        };
        let _ = registry.connect(key.clone(), descriptor);

        let failure = registry.fail_loading(&key, instance_id);
        assert!(matches!(
            failure,
            SharedWorkerLoadFailure::Failed { clients, .. } if clients.len() == 2
        ));
        assert!(registry.is_empty());
    }

    #[test]
    fn removing_last_running_client_requests_termination() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("a");
        let first = registry.connect(key.clone(), SharedWorkerDescriptor::default());
        let (instance_id, client_id) = match first {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected StartLoading, got {other:?}"),
        };
        registry.finish_loading(&key, instance_id, 9);

        let removal = registry.remove_client(client_id);
        assert!(matches!(
            removal,
            SharedWorkerClientRemoval::Terminate { instance: 9, .. }
        ));
        assert!(registry.is_empty());
    }

    #[test]
    fn removing_last_loading_client_cancels_loading_slot() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("a");
        let first = registry.connect(key, SharedWorkerDescriptor::default());
        let client_id = match first {
            SharedWorkerConnectAction::StartLoading { client_id, .. } => client_id,
            other => panic!("expected StartLoading, got {other:?}"),
        };

        let removal = registry.remove_client(client_id);
        assert!(matches!(
            removal,
            SharedWorkerClientRemoval::CancelLoading { .. }
        ));
        assert!(registry.is_empty());
    }

    #[test]
    fn removing_non_last_loading_client_reports_loading_state() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("a");
        let descriptor = SharedWorkerDescriptor::default();
        let first = registry.connect(key.clone(), descriptor.clone());
        let (instance_id, first_client) = match first {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected StartLoading, got {other:?}"),
        };
        let second_client = match registry.connect(key, descriptor) {
            SharedWorkerConnectAction::QueueWhileLoading {
                instance_id: queued_instance_id,
                client_id,
            } => {
                assert_eq!(queued_instance_id, instance_id);
                client_id
            }
            other => panic!("expected QueueWhileLoading, got {other:?}"),
        };

        assert_eq!(
            registry.remove_client(first_client),
            SharedWorkerClientRemoval::RemovedFromLoading { instance_id }
        );
        assert_eq!(
            registry.clients_for_instance(instance_id),
            vec![second_client]
        );
        assert_eq!(registry.running_instance(instance_id), None);
    }

    #[test]
    fn removing_non_last_running_client_returns_running_instance() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("a");
        let descriptor = SharedWorkerDescriptor::default();
        let first = registry.connect(key.clone(), descriptor.clone());
        let (instance_id, first_client) = match first {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected StartLoading, got {other:?}"),
        };
        let second_client = match registry.connect(key.clone(), descriptor) {
            SharedWorkerConnectAction::QueueWhileLoading { client_id, .. } => client_id,
            other => panic!("expected QueueWhileLoading, got {other:?}"),
        };
        registry.finish_loading(&key, instance_id, 42);

        assert_eq!(
            registry.remove_client(first_client),
            SharedWorkerClientRemoval::RemovedFromRunning {
                instance_id,
                instance: 42,
            }
        );
        assert_eq!(
            registry.clients_for_instance(instance_id),
            vec![second_client]
        );
        assert_eq!(registry.running_instance(instance_id), Some(42));
    }

    #[test]
    fn removing_worker_instance_removes_clients_and_allows_fresh_slot() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("a");
        let descriptor = SharedWorkerDescriptor::default();
        let first = registry.connect(key.clone(), descriptor.clone());
        let (instance_id, first_client) = match first {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected StartLoading, got {other:?}"),
        };
        registry.finish_loading(&key, instance_id, 3);
        let second_client = match registry.connect(key.clone(), descriptor.clone()) {
            SharedWorkerConnectAction::ConnectToRunning { client_id, .. } => client_id,
            other => panic!("expected ConnectToRunning, got {other:?}"),
        };

        let removal = registry.remove_instance(instance_id);
        assert!(matches!(
            removal,
            SharedWorkerInstanceRemoval::Removed {
                instance: Some(3),
                clients,
                ..
            } if clients.len() == 2
                && clients.contains(&first_client)
                && clients.contains(&second_client)
        ));
        assert!(registry.is_empty());
        assert!(matches!(
            registry.connect(key, descriptor),
            SharedWorkerConnectAction::StartLoading { .. }
        ));
    }

    #[test]
    fn removing_all_instances_drains_loading_and_running_state() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let loading_key = key("loading");
        let running_key = key("running");
        let descriptor = SharedWorkerDescriptor::default();
        let loading = registry.connect(loading_key.clone(), descriptor.clone());
        let running = registry.connect(running_key.clone(), descriptor);
        let (loading_instance_id, loading_client_id) = match loading {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected loading StartLoading, got {other:?}"),
        };
        let (running_instance_id, running_client_id) = match running {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected running StartLoading, got {other:?}"),
        };
        registry.finish_loading(&running_key, running_instance_id, 77);

        let removals = registry.remove_all_instances();

        assert_eq!(removals.len(), 2);
        assert!(registry.is_empty());
        assert!(matches!(
            registry.remove_client(loading_client_id),
            SharedWorkerClientRemoval::Missing
        ));
        assert!(matches!(
            registry.remove_client(running_client_id),
            SharedWorkerClientRemoval::Missing
        ));
        assert!(removals.iter().any(|removal| matches!(
            removal,
            SharedWorkerInstanceRemoval::Removed {
                key,
                clients,
                instance_id,
                instance: None,
            } if key == &loading_key
                && clients == &vec![loading_client_id]
                && instance_id == &loading_instance_id
        )));
        assert!(removals.iter().any(|removal| matches!(
            removal,
            SharedWorkerInstanceRemoval::Removed {
                key,
                clients,
                instance_id,
                instance: Some(77),
            } if key == &running_key
                && clients == &vec![running_client_id]
                && instance_id == &running_instance_id
        )));
    }

    #[test]
    fn running_instance_and_clients_are_lookupable_by_instance_id() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("a");
        let descriptor = SharedWorkerDescriptor::default();
        let first = registry.connect(key.clone(), descriptor.clone());
        let (instance_id, first_client) = match first {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected StartLoading, got {other:?}"),
        };
        let second_client = match registry.connect(key.clone(), descriptor) {
            SharedWorkerConnectAction::QueueWhileLoading { client_id, .. } => client_id,
            other => panic!("expected QueueWhileLoading, got {other:?}"),
        };

        assert_eq!(registry.running_instance(instance_id), None);
        registry.finish_loading(&key, instance_id, 42);

        assert_eq!(registry.running_instance(instance_id), Some(42));
        let clients = registry.clients_for_instance(instance_id);
        assert_eq!(clients, vec![first_client, second_client]);
    }

    #[test]
    fn loading_clients_only_reports_loading_instances() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("a");
        let descriptor = SharedWorkerDescriptor::default();
        let first = registry.connect(key.clone(), descriptor.clone());
        let (instance_id, first_client) = match first {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected StartLoading, got {other:?}"),
        };
        let second_client = match registry.connect(key.clone(), descriptor) {
            SharedWorkerConnectAction::QueueWhileLoading { client_id, .. } => client_id,
            other => panic!("expected QueueWhileLoading, got {other:?}"),
        };

        assert_eq!(
            registry.loading_clients_for_instance(instance_id),
            vec![first_client, second_client]
        );

        registry.finish_loading(&key, instance_id, 42);

        assert!(
            registry
                .loading_clients_for_instance(instance_id)
                .is_empty()
        );
        assert_eq!(
            registry.clients_for_instance(instance_id),
            vec![first_client, second_client]
        );
    }

    #[test]
    fn clients_remain_separate_while_sharing_an_instance() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("separate-clients");
        let descriptor = SharedWorkerDescriptor::default();
        let first = registry.connect(key.clone(), descriptor.clone());
        let (instance_id, first_client) = match first {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected StartLoading, got {other:?}"),
        };
        let second_client = match registry.connect(key.clone(), descriptor.clone()) {
            SharedWorkerConnectAction::QueueWhileLoading {
                instance_id: queued_id,
                client_id,
            } => {
                assert_eq!(queued_id, instance_id);
                client_id
            }
            other => panic!("expected QueueWhileLoading, got {other:?}"),
        };

        assert_eq!(
            registry.clients_for_instance(instance_id),
            vec![first_client, second_client]
        );

        assert_eq!(
            registry.remove_client(first_client),
            SharedWorkerClientRemoval::RemovedFromLoading { instance_id }
        );
        assert_eq!(
            registry.clients_for_instance(instance_id),
            vec![second_client]
        );
    }

    #[test]
    fn removing_clients_keeps_the_surviving_connection_attached() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("three-clients");
        let descriptor = SharedWorkerDescriptor::default();
        let first = registry.connect(key.clone(), descriptor.clone());
        let (instance_id, first_client) = match first {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected StartLoading, got {other:?}"),
        };
        let second_client = match registry.connect(key.clone(), descriptor.clone()) {
            SharedWorkerConnectAction::QueueWhileLoading { client_id, .. } => client_id,
            other => panic!("expected QueueWhileLoading, got {other:?}"),
        };
        let third_client = match registry.connect(key.clone(), descriptor) {
            SharedWorkerConnectAction::QueueWhileLoading { client_id, .. } => client_id,
            other => panic!("expected QueueWhileLoading, got {other:?}"),
        };
        assert_eq!(
            registry.clients_for_instance(instance_id),
            vec![first_client, second_client, third_client]
        );

        assert_eq!(
            registry.remove_client(first_client),
            SharedWorkerClientRemoval::RemovedFromLoading { instance_id }
        );
        assert_eq!(
            registry.clients_for_instance(instance_id),
            vec![second_client, third_client]
        );

        assert_eq!(
            registry.remove_client(second_client),
            SharedWorkerClientRemoval::RemovedFromLoading { instance_id }
        );

        assert_eq!(
            registry.clients_for_instance(instance_id),
            vec![third_client]
        );
    }

    #[test]
    fn loading_instance_survives_until_its_last_client_is_removed() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("client-membership");
        let descriptor = SharedWorkerDescriptor::default();
        let first = registry.connect(key.clone(), descriptor.clone());
        let (instance_id, first_client) = match first {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected StartLoading, got {other:?}"),
        };
        assert_eq!(
            registry.clients_for_instance(instance_id),
            vec![first_client]
        );
        let second_client = match registry.connect(key, descriptor) {
            SharedWorkerConnectAction::QueueWhileLoading { client_id, .. } => client_id,
            other => panic!("expected QueueWhileLoading, got {other:?}"),
        };
        assert_eq!(
            registry.clients_for_instance(instance_id),
            vec![first_client, second_client]
        );
        assert_eq!(
            registry.remove_client(first_client),
            SharedWorkerClientRemoval::RemovedFromLoading { instance_id }
        );
        assert_eq!(
            registry.clients_for_instance(instance_id),
            vec![second_client]
        );
        assert!(matches!(
            registry.remove_client(second_client),
            SharedWorkerClientRemoval::CancelLoading { .. }
        ));
        assert!(registry.clients_for_instance(instance_id).is_empty());
        assert!(registry.is_empty());
    }

    #[test]
    fn instance_removal_releases_every_client() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("remove-instance-clients");
        let descriptor = SharedWorkerDescriptor::default();
        let instance_id = match registry.connect(key.clone(), descriptor.clone()) {
            SharedWorkerConnectAction::StartLoading { instance_id, .. } => instance_id,
            other => panic!("expected StartLoading, got {other:?}"),
        };
        for _ in 0..2 {
            assert!(matches!(
                registry.connect(key.clone(), descriptor.clone()),
                SharedWorkerConnectAction::QueueWhileLoading { .. }
            ));
        }

        let clients = registry.clients_for_instance(instance_id);
        assert_eq!(clients.len(), 3);
        let SharedWorkerInstanceRemoval::Removed {
            clients: removed, ..
        } = registry.remove_instance(instance_id)
        else {
            panic!("instance must be removed")
        };
        assert_eq!(removed, clients);
        assert!(registry.clients_for_instance(instance_id).is_empty());
        assert!(registry.is_empty());
        for client in clients {
            assert_eq!(
                registry.remove_client(client),
                SharedWorkerClientRemoval::Missing
            );
        }
    }

    #[test]
    fn diagnostics_count_loading_running_and_clients() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let loading_key = key("diagnostics-loading");
        let running_key = key("diagnostics-running");
        let descriptor = SharedWorkerDescriptor::default();

        let loading = registry.connect(loading_key, descriptor.clone());
        let loading_instance_id = match loading {
            SharedWorkerConnectAction::StartLoading { instance_id, .. } => instance_id,
            other => panic!("expected StartLoading, got {other:?}"),
        };
        assert!(matches!(
            registry.connect(running_key.clone(), descriptor.clone()),
            SharedWorkerConnectAction::StartLoading { .. }
        ));
        let running_instance_id = match registry.connect(running_key.clone(), descriptor.clone()) {
            SharedWorkerConnectAction::QueueWhileLoading { instance_id, .. } => instance_id,
            other => panic!("expected QueueWhileLoading, got {other:?}"),
        };
        registry.finish_loading(&running_key, running_instance_id, 42);

        assert_eq!(
            registry.diagnostics(),
            SharedWorkerRegistryDiagnostics {
                entry_count: 2,
                loading_instance_count: 1,
                running_instance_count: 1,
                client_count: 3,
            }
        );
        assert_eq!(
            registry
                .loading_clients_for_instance(loading_instance_id)
                .len(),
            1
        );
    }
}
