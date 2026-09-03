use std::collections::HashMap;

use parking_lot::Mutex;

use crate::{
    SharedWorkerClientId, SharedWorkerClientOwnerId, SharedWorkerCompatibilityError,
    SharedWorkerDescriptor, SharedWorkerInstanceId, SharedWorkerKey,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SharedWorkerClientRecord {
    client_id: SharedWorkerClientId,
    owner_id: SharedWorkerClientOwnerId,
}

#[derive(Debug)]
enum SharedWorkerEntryState<I> {
    Loading,
    Running { instance: I },
}

#[derive(Debug)]
struct SharedWorkerEntry<I> {
    instance_id: SharedWorkerInstanceId,
    descriptor: SharedWorkerDescriptor,
    clients: Vec<SharedWorkerClientRecord>,
    state: SharedWorkerEntryState<I>,
}

impl<I> SharedWorkerEntry<I> {
    fn ensure_compatible_with(
        &self,
        requested: &SharedWorkerDescriptor,
    ) -> Result<(), SharedWorkerCompatibilityError> {
        self.descriptor.ensure_compatible_with(requested)
    }

    fn add_client(&mut self, client: SharedWorkerClientRecord) {
        self.clients.push(client);
    }

    fn remove_client(&mut self, client_id: SharedWorkerClientId) -> bool {
        self.clients
            .retain(|current| current.client_id != client_id);
        self.clients.is_empty()
    }

    fn client_owner_id(
        &self,
        client_id: SharedWorkerClientId,
    ) -> Option<SharedWorkerClientOwnerId> {
        self.clients
            .iter()
            .find_map(|client| (client.client_id == client_id).then_some(client.owner_id))
    }

    fn client_ids(&self) -> Vec<SharedWorkerClientId> {
        self.clients.iter().map(|client| client.client_id).collect()
    }

    fn client_owner_ids(&self) -> Vec<SharedWorkerClientOwnerId> {
        let mut owners = Vec::new();
        for client in &self.clients {
            if !owners.contains(&client.owner_id) {
                owners.push(client.owner_id);
            }
        }
        owners
    }

    fn client_count_for_owner(&self, owner_id: SharedWorkerClientOwnerId) -> usize {
        self.clients
            .iter()
            .filter(|client| client.owner_id == owner_id)
            .count()
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

fn last_client_removed_event(
    instance_id: SharedWorkerInstanceId,
    owner_id: Option<SharedWorkerClientOwnerId>,
    emit: bool,
) -> Vec<SharedWorkerClientOwnerEvent> {
    match (owner_id, emit) {
        (Some(owner_id), true) => vec![SharedWorkerClientOwnerEvent::LastClientRemoved {
            instance_id,
            owner_id,
        }],
        _ => Vec::new(),
    }
}

fn last_client_removed_events<I>(
    instance_id: SharedWorkerInstanceId,
    entry: &SharedWorkerEntry<I>,
) -> Vec<SharedWorkerClientOwnerEvent> {
    entry
        .client_owner_ids()
        .into_iter()
        .map(|owner_id| SharedWorkerClientOwnerEvent::LastClientRemoved {
            instance_id,
            owner_id,
        })
        .collect()
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

/// Owner-level lifecycle event derived from port-level client changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedWorkerClientOwnerEvent {
    FirstClientAdded {
        instance_id: SharedWorkerInstanceId,
        owner_id: SharedWorkerClientOwnerId,
    },
    LastClientRemoved {
        instance_id: SharedWorkerInstanceId,
        owner_id: SharedWorkerClientOwnerId,
    },
}

/// Registry action plus owner-level lifecycle events produced atomically under
/// the registry lock.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedWorkerObservedAction<T> {
    pub action: T,
    pub owner_events: Vec<SharedWorkerClientOwnerEvent>,
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
        self.connect_observed(key, descriptor).action
    }

    /// Connect a new client and return owner-level lifecycle events.
    pub fn connect_observed(
        &self,
        key: SharedWorkerKey,
        descriptor: SharedWorkerDescriptor,
    ) -> SharedWorkerObservedAction<SharedWorkerConnectAction<I>> {
        let mut state = self.state.lock();
        let client_id = state.next_client_id();
        let client_owner_id = SharedWorkerClientOwnerId::unique_for_client(client_id);
        Self::connect_locked(&mut state, key, descriptor, client_id, client_owner_id)
    }

    /// Connect a new client owned by an embedder browsing context/frame.
    pub fn connect_with_owner(
        &self,
        key: SharedWorkerKey,
        descriptor: SharedWorkerDescriptor,
        client_owner_id: SharedWorkerClientOwnerId,
    ) -> SharedWorkerConnectAction<I> {
        self.connect_with_owner_observed(key, descriptor, client_owner_id)
            .action
    }

    /// Connect a new client owned by an embedder context and return owner-level
    /// lifecycle events.
    pub fn connect_with_owner_observed(
        &self,
        key: SharedWorkerKey,
        descriptor: SharedWorkerDescriptor,
        client_owner_id: SharedWorkerClientOwnerId,
    ) -> SharedWorkerObservedAction<SharedWorkerConnectAction<I>> {
        let mut state = self.state.lock();
        let client_id = state.next_client_id();
        Self::connect_locked(&mut state, key, descriptor, client_id, client_owner_id)
    }

    fn connect_locked(
        state: &mut SharedWorkerRegistryState<I>,
        key: SharedWorkerKey,
        descriptor: SharedWorkerDescriptor,
        client_id: SharedWorkerClientId,
        client_owner_id: SharedWorkerClientOwnerId,
    ) -> SharedWorkerObservedAction<SharedWorkerConnectAction<I>> {
        let instance_id = state.next_instance_id();
        let client = SharedWorkerClientRecord {
            client_id,
            owner_id: client_owner_id,
        };
        if let Some(entry) = state.entries.get_mut(&key) {
            if let Err(error) = entry.ensure_compatible_with(&descriptor) {
                return SharedWorkerObservedAction {
                    action: SharedWorkerConnectAction::RejectClient { client_id, error },
                    owner_events: Vec::new(),
                };
            }
            let instance_id = entry.instance_id;
            let first_client_for_owner = entry.client_count_for_owner(client_owner_id) == 0;
            entry.add_client(client);
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
            return SharedWorkerObservedAction {
                action,
                owner_events: first_client_for_owner
                    .then_some(SharedWorkerClientOwnerEvent::FirstClientAdded {
                        instance_id,
                        owner_id: client_owner_id,
                    })
                    .into_iter()
                    .collect(),
            };
        }

        let clients = vec![client];
        state.client_keys.insert(client_id, key.clone());
        state.entries.insert(
            key,
            SharedWorkerEntry {
                instance_id,
                descriptor,
                clients,
                state: SharedWorkerEntryState::Loading,
            },
        );
        SharedWorkerObservedAction {
            action: SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            },
            owner_events: vec![SharedWorkerClientOwnerEvent::FirstClientAdded {
                instance_id,
                owner_id: client_owner_id,
            }],
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
            clients: entry.client_ids(),
            instance,
        }
    }

    /// Fail a loading slot and remove all pending clients.
    pub fn fail_loading(
        &self,
        key: &SharedWorkerKey,
        instance_id: SharedWorkerInstanceId,
    ) -> SharedWorkerLoadFailure {
        self.fail_loading_observed(key, instance_id).action
    }

    /// Fail a loading slot and return owner-level removal events.
    pub fn fail_loading_observed(
        &self,
        key: &SharedWorkerKey,
        instance_id: SharedWorkerInstanceId,
    ) -> SharedWorkerObservedAction<SharedWorkerLoadFailure> {
        let mut state = self.state.lock();
        let Some(entry) = state.entries.get(key) else {
            return SharedWorkerObservedAction {
                action: SharedWorkerLoadFailure::Stale,
                owner_events: Vec::new(),
            };
        };
        if entry.instance_id != instance_id
            || !matches!(entry.state, SharedWorkerEntryState::Loading)
        {
            return SharedWorkerObservedAction {
                action: SharedWorkerLoadFailure::Stale,
                owner_events: Vec::new(),
            };
        }
        let entry = state.entries.remove(key).expect("entry checked above");
        let owner_events = last_client_removed_events(instance_id, &entry);
        let clients = entry.client_ids();
        for client_id in &clients {
            state.client_keys.remove(client_id);
        }
        SharedWorkerObservedAction {
            action: SharedWorkerLoadFailure::Failed {
                instance_id,
                clients,
            },
            owner_events,
        }
    }

    /// Remove one client and return whether the embedder should cancel/terminate.
    pub fn remove_client(&self, client_id: SharedWorkerClientId) -> SharedWorkerClientRemoval<I> {
        self.remove_client_observed(client_id).action
    }

    /// Remove one client and return owner-level lifecycle events.
    pub fn remove_client_observed(
        &self,
        client_id: SharedWorkerClientId,
    ) -> SharedWorkerObservedAction<SharedWorkerClientRemoval<I>> {
        let mut state = self.state.lock();
        let Some(key) = state.client_keys.remove(&client_id) else {
            return SharedWorkerObservedAction {
                action: SharedWorkerClientRemoval::Missing,
                owner_events: Vec::new(),
            };
        };
        let Some(entry) = state.entries.get_mut(&key) else {
            return SharedWorkerObservedAction {
                action: SharedWorkerClientRemoval::Missing,
                owner_events: Vec::new(),
            };
        };
        let instance_id = entry.instance_id;
        let owner_id = entry.client_owner_id(client_id);
        let last_client_for_owner =
            owner_id.is_some_and(|owner_id| entry.client_count_for_owner(owner_id) == 1);
        if !entry.remove_client(client_id) {
            let action = match &entry.state {
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
            return SharedWorkerObservedAction {
                action,
                owner_events: last_client_removed_event(
                    instance_id,
                    owner_id,
                    last_client_for_owner,
                ),
            };
        }
        let entry = state.entries.remove(&key).expect("entry checked above");
        let action = match entry.state {
            SharedWorkerEntryState::Loading => {
                SharedWorkerClientRemoval::CancelLoading { instance_id, key }
            }
            SharedWorkerEntryState::Running { instance } => SharedWorkerClientRemoval::Terminate {
                instance_id,
                key,
                instance,
            },
        };
        SharedWorkerObservedAction {
            action,
            owner_events: last_client_removed_event(instance_id, owner_id, last_client_for_owner),
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
            .map(SharedWorkerEntry::client_ids)
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
            .map(SharedWorkerEntry::client_ids)
            .unwrap_or_default()
    }

    /// Return distinct owner ids currently attached to an instance.
    pub fn client_owner_ids_for_instance(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Vec<SharedWorkerClientOwnerId> {
        let state = self.state.lock();
        state
            .entries
            .values()
            .find(|entry| entry.instance_id == instance_id)
            .map(SharedWorkerEntry::client_owner_ids)
            .unwrap_or_default()
    }

    /// Return the number of port-level clients for one owner on an instance.
    pub fn client_count_for_owner(
        &self,
        instance_id: SharedWorkerInstanceId,
        owner_id: SharedWorkerClientOwnerId,
    ) -> usize {
        let state = self.state.lock();
        state
            .entries
            .values()
            .find(|entry| entry.instance_id == instance_id)
            .map(|entry| entry.client_count_for_owner(owner_id))
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
        self.remove_instance_observed(instance_id).action
    }

    /// Remove one instance and return owner-level removal events.
    pub fn remove_instance_observed(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> SharedWorkerObservedAction<SharedWorkerInstanceRemoval<I>> {
        let mut state = self.state.lock();
        let Some(key) = state
            .entries
            .iter()
            .find_map(|(key, entry)| (entry.instance_id == instance_id).then(|| key.clone()))
        else {
            return SharedWorkerObservedAction {
                action: SharedWorkerInstanceRemoval::Missing,
                owner_events: Vec::new(),
            };
        };
        let entry = state.entries.remove(&key).expect("entry checked above");
        let owner_events = last_client_removed_events(instance_id, &entry);
        let clients = entry.client_ids();
        for client_id in &clients {
            state.client_keys.remove(client_id);
        }
        let instance = match entry.state {
            SharedWorkerEntryState::Loading => None,
            SharedWorkerEntryState::Running { instance } => Some(instance),
        };
        SharedWorkerObservedAction {
            action: SharedWorkerInstanceRemoval::Removed {
                key,
                instance_id,
                clients,
                instance,
            },
            owner_events,
        }
    }

    /// Remove every loading or running instance, usually because the owning
    /// browser context / storage partition is shutting down.
    pub fn remove_all_instances(&self) -> Vec<SharedWorkerInstanceRemoval<I>> {
        self.remove_all_instances_observed()
            .into_iter()
            .map(|observed| observed.action)
            .collect()
    }

    /// Remove every instance and return owner-level removal events.
    pub fn remove_all_instances_observed(
        &self,
    ) -> Vec<SharedWorkerObservedAction<SharedWorkerInstanceRemoval<I>>> {
        let mut state = self.state.lock();
        let entries = std::mem::take(&mut state.entries);
        state.client_keys.clear();
        entries
            .into_iter()
            .map(|(key, entry)| {
                let clients = entry.client_ids();
                let instance_id = entry.instance_id;
                let owner_events = last_client_removed_events(instance_id, &entry);
                let instance = match entry.state {
                    SharedWorkerEntryState::Loading => None,
                    SharedWorkerEntryState::Running { instance } => Some(instance),
                };
                SharedWorkerObservedAction {
                    action: SharedWorkerInstanceRemoval::Removed {
                        key,
                        instance_id,
                        clients,
                        instance,
                    },
                    owner_events,
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

    fn owner(id: u64) -> SharedWorkerClientOwnerId {
        SharedWorkerClientOwnerId::from_u64(id)
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
    fn same_owner_clients_remain_separate_but_owner_count_is_aggregated() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("same-owner");
        let descriptor = SharedWorkerDescriptor::default();
        let owner_id = owner(10);
        let first = registry.connect_with_owner(key.clone(), descriptor.clone(), owner_id);
        let (instance_id, first_client) = match first {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected StartLoading, got {other:?}"),
        };
        let second_client =
            match registry.connect_with_owner(key.clone(), descriptor.clone(), owner_id) {
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
            registry.client_owner_ids_for_instance(instance_id),
            vec![owner_id]
        );
        assert_eq!(registry.client_count_for_owner(instance_id, owner_id), 2);

        assert_eq!(
            registry.remove_client(first_client),
            SharedWorkerClientRemoval::RemovedFromLoading { instance_id }
        );
        assert_eq!(
            registry.clients_for_instance(instance_id),
            vec![second_client]
        );
        assert_eq!(
            registry.client_owner_ids_for_instance(instance_id),
            vec![owner_id]
        );
        assert_eq!(registry.client_count_for_owner(instance_id, owner_id), 1);
    }

    #[test]
    fn removing_last_client_for_one_owner_keeps_other_owner_attached() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("two-owners");
        let descriptor = SharedWorkerDescriptor::default();
        let owner_a = owner(11);
        let owner_b = owner(12);
        let first = registry.connect_with_owner(key.clone(), descriptor.clone(), owner_a);
        let (instance_id, owner_a_first) = match first {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected StartLoading, got {other:?}"),
        };
        let owner_a_second =
            match registry.connect_with_owner(key.clone(), descriptor.clone(), owner_a) {
                SharedWorkerConnectAction::QueueWhileLoading { client_id, .. } => client_id,
                other => panic!("expected QueueWhileLoading, got {other:?}"),
            };
        let owner_b_client = match registry.connect_with_owner(key.clone(), descriptor, owner_b) {
            SharedWorkerConnectAction::QueueWhileLoading { client_id, .. } => client_id,
            other => panic!("expected QueueWhileLoading, got {other:?}"),
        };

        assert_eq!(
            registry.client_owner_ids_for_instance(instance_id),
            vec![owner_a, owner_b]
        );

        assert_eq!(
            registry.remove_client(owner_a_first),
            SharedWorkerClientRemoval::RemovedFromLoading { instance_id }
        );
        assert_eq!(registry.client_count_for_owner(instance_id, owner_a), 1);

        assert_eq!(
            registry.remove_client(owner_a_second),
            SharedWorkerClientRemoval::RemovedFromLoading { instance_id }
        );
        assert_eq!(
            registry.client_owner_ids_for_instance(instance_id),
            vec![owner_b]
        );
        assert_eq!(
            registry.clients_for_instance(instance_id),
            vec![owner_b_client]
        );
    }

    #[test]
    fn observed_connect_and_remove_report_owner_refcount_edges() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("owner-observed");
        let descriptor = SharedWorkerDescriptor::default();
        let owner_id = owner(30);

        let first = registry.connect_with_owner_observed(key.clone(), descriptor.clone(), owner_id);
        let (instance_id, first_client) = match first.action {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            other => panic!("expected StartLoading, got {other:?}"),
        };
        assert_eq!(
            first.owner_events,
            vec![SharedWorkerClientOwnerEvent::FirstClientAdded {
                instance_id,
                owner_id,
            }]
        );

        let second = registry.connect_with_owner_observed(key, descriptor, owner_id);
        let second_client = match second.action {
            SharedWorkerConnectAction::QueueWhileLoading { client_id, .. } => client_id,
            other => panic!("expected QueueWhileLoading, got {other:?}"),
        };
        assert!(second.owner_events.is_empty());

        let first_removed = registry.remove_client_observed(first_client);
        assert_eq!(
            first_removed.action,
            SharedWorkerClientRemoval::RemovedFromLoading { instance_id }
        );
        assert!(first_removed.owner_events.is_empty());

        let second_removed = registry.remove_client_observed(second_client);
        assert!(matches!(
            second_removed.action,
            SharedWorkerClientRemoval::CancelLoading { .. }
        ));
        assert_eq!(
            second_removed.owner_events,
            vec![SharedWorkerClientOwnerEvent::LastClientRemoved {
                instance_id,
                owner_id,
            }]
        );
    }

    #[test]
    fn observed_instance_removal_reports_each_owner_once() {
        let registry = SharedWorkerRegistry::<u64>::default();
        let key = key("owner-remove-instance");
        let descriptor = SharedWorkerDescriptor::default();
        let owner_a = owner(40);
        let owner_b = owner(41);
        let first = registry.connect_with_owner(key.clone(), descriptor.clone(), owner_a);
        let instance_id = match first {
            SharedWorkerConnectAction::StartLoading { instance_id, .. } => instance_id,
            other => panic!("expected StartLoading, got {other:?}"),
        };
        assert!(matches!(
            registry.connect_with_owner(key.clone(), descriptor.clone(), owner_a),
            SharedWorkerConnectAction::QueueWhileLoading { .. }
        ));
        assert!(matches!(
            registry.connect_with_owner(key, descriptor, owner_b),
            SharedWorkerConnectAction::QueueWhileLoading { .. }
        ));

        let removed = registry.remove_instance_observed(instance_id);

        assert!(matches!(
            removed.action,
            SharedWorkerInstanceRemoval::Removed { .. }
        ));
        assert_eq!(
            removed.owner_events,
            vec![
                SharedWorkerClientOwnerEvent::LastClientRemoved {
                    instance_id,
                    owner_id: owner_a,
                },
                SharedWorkerClientOwnerEvent::LastClientRemoved {
                    instance_id,
                    owner_id: owner_b,
                },
            ]
        );
        assert!(registry.is_empty());
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
