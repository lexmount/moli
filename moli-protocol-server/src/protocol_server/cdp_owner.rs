use std::{
    collections::HashMap,
    sync::{Arc, Weak, atomic::AtomicU64},
};

use anyhow::{Context, Result, bail};
use moli_core::runtime::{NavigationRuntimeConfig, storage_partition::StoragePartitionState};
use moli_protocol::CdpInitialStoragePartition;
use parking_lot::Mutex;
use tokio::sync::{Notify, mpsc, oneshot};

use crate::{
    cdp_frontend::{CdpFrontendEndpoint, CdpFrontendReceivers, cdp_frontend_channel},
    cdp_frontend_router::CdpFrontendRouter,
    cdp_scheduler::{
        CdpCookieSnapshot, CdpOwnerActorLifecycle, CdpScheduler, CdpTargetHostIntegration,
        spawn_cdp_scheduler_actor,
    },
};

use super::{
    CookieProfileCommit, SharedCookieProfile, cdp_agent_host::SharedCdpAgentHostDirectory,
    protocol_local_executor::spawn_protocol_local_task,
};

use self::checkpoint::spawn_checkpoint_worker;

mod checkpoint;

#[derive(Clone)]
pub(super) struct SharedCdpOwnerRegistry {
    inner: Arc<CdpOwnerRegistryInner>,
}

struct CdpOwnerRegistryInner {
    config: CdpOwnerRuntimeConfig,
    state: Mutex<CdpOwnerRegistryState>,
    shared_owner: Mutex<Option<(u64, CdpFrontendEndpoint)>>,
    owner_finished: Notify,
}

#[derive(Default)]
struct CdpOwnerRegistryState {
    owners: HashMap<u64, CdpOwnerRecord>,
    shutting_down: bool,
}

struct CdpOwnerRecord {
    endpoint: CdpFrontendEndpoint,
}

#[derive(Clone)]
struct CdpOwnerRuntimeConfig {
    directory: SharedCdpAgentHostDirectory,
    target_id_allocator: Arc<AtomicU64>,
    tab_target_id_allocator: Arc<AtomicU64>,
    cookie_profile: SharedCookieProfile,
    storage_partition: Arc<StoragePartitionState>,
    navigation_runtime_config: NavigationRuntimeConfig,
}

impl SharedCdpOwnerRegistry {
    pub(super) fn new(
        directory: SharedCdpAgentHostDirectory,
        target_id_allocator: Arc<AtomicU64>,
        tab_target_id_allocator: Arc<AtomicU64>,
        cookie_profile: SharedCookieProfile,
        storage_partition: Arc<StoragePartitionState>,
        navigation_runtime_config: NavigationRuntimeConfig,
    ) -> Self {
        Self {
            inner: Arc::new(CdpOwnerRegistryInner {
                config: CdpOwnerRuntimeConfig {
                    directory,
                    target_id_allocator,
                    tab_target_id_allocator,
                    cookie_profile,
                    storage_partition,
                    navigation_runtime_config,
                },
                state: Mutex::new(CdpOwnerRegistryState::default()),
                shared_owner: Mutex::new(None),
                owner_finished: Notify::new(),
            }),
        }
    }

    pub(super) fn shared_owner(&self) -> Result<CdpFrontendEndpoint> {
        let mut shared_owner = self.inner.shared_owner.lock();
        if let Some((_, endpoint)) = shared_owner.as_ref() {
            return Ok(endpoint.clone());
        }
        let (owner_id, endpoint) = self
            .spawn_owner()
            .context("failed to spawn shared CDP owner")?;
        *shared_owner = Some((owner_id, endpoint.clone()));
        Ok(endpoint)
    }

    fn spawn_owner(&self) -> Result<(u64, CdpFrontendEndpoint)> {
        let owner_id = self.inner.config.directory.allocate_owner_id();
        let (endpoint, receivers) = cdp_frontend_channel();
        let lifecycle_observer = self
            .inner
            .config
            .directory
            .lifecycle_observer(owner_id, endpoint.clone());
        {
            let mut state = self.inner.state.lock();
            if state.shutting_down {
                bail!("CDP owner registry is shutting down");
            }
            state.owners.insert(
                owner_id,
                CdpOwnerRecord {
                    endpoint: endpoint.clone(),
                },
            );
        }

        let initial_cookies = self.inner.config.cookie_profile.snapshot();
        let initial_storage_partition = CdpInitialStoragePartition::from_storage_partition(
            initial_cookies.clone(),
            self.inner.config.storage_partition.as_ref(),
        );
        let (checkpoint_tx, checkpoint_rx) = mpsc::unbounded_channel();
        let checkpoint_worker = spawn_checkpoint_worker(
            self.inner.config.cookie_profile.clone(),
            initial_cookies,
            checkpoint_rx,
        );
        let owner_finished_rx = spawn_owner_task(
            receivers,
            initial_storage_partition,
            self.inner.config.navigation_runtime_config.clone(),
            Some(CdpTargetHostIntegration::new(
                self.inner.config.target_id_allocator.clone(),
                self.inner.config.tab_target_id_allocator.clone(),
                lifecycle_observer,
            )),
            Some(CdpOwnerActorLifecycle {
                checkpoint_tx: checkpoint_tx.clone(),
            }),
            checkpoint_tx,
        );
        let weak_registry = Arc::downgrade(&self.inner);
        let directory = self.inner.config.directory.clone();
        tokio::spawn(async move {
            if owner_finished_rx.await.is_err() {
                tracing::warn!(owner_id, "CDP owner task stopped without completion");
            }
            if let Err(error) = checkpoint_worker.await {
                tracing::warn!(owner_id, ?error, "CDP owner checkpoint worker failed");
            }
            directory.remove_owner(owner_id);
            finish_owner(&weak_registry, owner_id);
        });
        Ok((owner_id, endpoint))
    }

    pub(super) async fn shutdown(&self) {
        let endpoints = {
            let mut state = self.inner.state.lock();
            state.shutting_down = true;
            state
                .owners
                .values()
                .map(|owner| owner.endpoint.clone())
                .collect::<Vec<_>>()
        };
        for endpoint in endpoints {
            endpoint.shutdown();
        }
        loop {
            let owner_finished = self.inner.owner_finished.notified();
            if self.inner.state.lock().owners.is_empty() {
                return;
            }
            owner_finished.await;
        }
    }

    #[cfg(test)]
    pub(super) fn owner_count(&self) -> usize {
        self.inner.state.lock().owners.len()
    }
}

impl Drop for CdpOwnerRegistryInner {
    fn drop(&mut self) {
        let state = self.state.get_mut();
        for owner in state.owners.values() {
            owner.endpoint.shutdown();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_owner_task(
    receivers: CdpFrontendReceivers,
    initial_storage_partition: CdpInitialStoragePartition,
    navigation_runtime_config: NavigationRuntimeConfig,
    target_host_integration: Option<CdpTargetHostIntegration>,
    owner_lifecycle: Option<CdpOwnerActorLifecycle>,
    checkpoint_tx: mpsc::UnboundedSender<CdpCookieSnapshot>,
) -> oneshot::Receiver<()> {
    spawn_protocol_local_task("cdp-owner", move || async move {
        let (scheduler, scheduler_receivers) =
            CdpScheduler::new_with_deferred_default_target_runtime(
                initial_storage_partition,
                navigation_runtime_config,
                target_host_integration,
            );
        let actor = spawn_cdp_scheduler_actor(
            scheduler,
            scheduler_receivers,
            CdpFrontendRouter::new(),
            receivers,
            owner_lifecycle,
        );
        let snapshot = actor.await.unwrap_or_default();
        let _ = checkpoint_tx.send(snapshot);
    })
}

fn finish_owner(registry: &Weak<CdpOwnerRegistryInner>, owner_id: u64) {
    let Some(registry) = registry.upgrade() else {
        return;
    };
    if let Some(owner) = registry.state.lock().owners.remove(&owner_id) {
        owner.endpoint.shutdown();
    }
    let mut shared_owner = registry.shared_owner.lock();
    if shared_owner
        .as_ref()
        .is_some_and(|(shared_owner_id, _)| *shared_owner_id == owner_id)
    {
        *shared_owner = None;
    }
    registry.owner_finished.notify_waiters();
}
