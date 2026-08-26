use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use moli_protocol::{
    CdpTargetHostLifecycleDelta, CdpTargetHostLifecycleObserver, DevToolsTargetInfo,
    DevToolsTargetKind,
};
use parking_lot::Mutex;

use crate::cdp_frontend::CdpFrontendEndpoint;

#[derive(Clone, Default)]
pub(super) struct SharedCdpAgentHostDirectory {
    inner: Arc<Mutex<CdpAgentHostDirectoryState>>,
    next_owner_id: Arc<AtomicU64>,
}

#[derive(Default)]
struct CdpAgentHostDirectoryState {
    hosts: HashMap<String, CdpAgentHost>,
}

#[derive(Clone)]
struct CdpAgentHost {
    owner_id: u64,
    target_info: DevToolsTargetInfo,
    endpoint: CdpFrontendEndpoint,
    is_active: bool,
}

#[derive(Clone)]
pub(super) struct CdpAgentHostRoute {
    pub(super) endpoint: CdpFrontendEndpoint,
    pub(super) target_info: DevToolsTargetInfo,
}

impl CdpAgentHost {
    fn browser_context_id(&self) -> Option<&str> {
        self.target_info
            .browser_context_id
            .as_ref()
            .map(|id| id.as_str())
    }
}

impl CdpAgentHostDirectoryState {
    fn activate_host(&mut self, owner_id: u64, target_id: &str) -> bool {
        let identity = self
            .hosts
            .get(target_id)
            .filter(|host| host.owner_id == owner_id)
            .map(|host| {
                (
                    host.browser_context_id().map(str::to_owned),
                    host.target_info.kind,
                )
            });
        let Some((browser_context_id, kind)) = identity else {
            return false;
        };

        for host in self.hosts.values_mut().filter(|host| {
            host.owner_id == owner_id
                && host.target_info.kind == kind
                && host.browser_context_id() == browser_context_id.as_deref()
        }) {
            host.is_active = false;
        }
        if let Some(host) = self.hosts.get_mut(target_id) {
            host.is_active = true;
            return true;
        }
        false
    }
}

impl SharedCdpAgentHostDirectory {
    pub(super) fn allocate_owner_id(&self) -> u64 {
        self.next_owner_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub(super) fn lifecycle_observer(
        &self,
        owner_id: u64,
        endpoint: CdpFrontendEndpoint,
    ) -> CdpTargetHostLifecycleObserver {
        let directory = self.clone();
        CdpTargetHostLifecycleObserver::new(move |delta| {
            directory.apply_delta(owner_id, &endpoint, delta);
        })
    }

    pub(super) fn lookup_target(&self, target_id: &str) -> Option<CdpAgentHostRoute> {
        let state = self.inner.lock();
        let host = state.hosts.get(target_id)?;
        Some(CdpAgentHostRoute {
            endpoint: host.endpoint.clone(),
            target_info: host.target_info.clone(),
        })
    }

    pub(super) fn remote_debugging_target_infos(
        &self,
        include_tab_targets: bool,
    ) -> Vec<DevToolsTargetInfo> {
        let state = self.inner.lock();
        let mut hosts = state
            .hosts
            .iter()
            .filter(|(_, host)| {
                host.target_info.kind == DevToolsTargetKind::Page
                    || (include_tab_targets && host.target_info.kind == DevToolsTargetKind::Tab)
            })
            .collect::<Vec<_>>();
        hosts.sort_by(|(left_id, left), (right_id, right)| {
            right
                .is_active
                .cmp(&left.is_active)
                .then_with(|| left_id.cmp(right_id))
        });
        hosts
            .into_iter()
            .map(|(_, host)| host.target_info.clone())
            .collect()
    }

    pub(super) fn remove_owner(&self, owner_id: u64) {
        self.inner
            .lock()
            .hosts
            .retain(|_, host| host.owner_id != owner_id);
    }

    fn apply_delta(
        &self,
        owner_id: u64,
        endpoint: &CdpFrontendEndpoint,
        delta: CdpTargetHostLifecycleDelta,
    ) {
        match delta {
            CdpTargetHostLifecycleDelta::Created(target_info) => {
                let Some(target_id) = remote_debugging_target_id(&target_info) else {
                    return;
                };
                let mut state = self.inner.lock();
                if let Some(existing) = state.hosts.get(&target_id)
                    && existing.owner_id != owner_id
                {
                    tracing::warn!(
                        target_id,
                        existing_owner_id = existing.owner_id,
                        owner_id,
                        "refusing to replace a live CDP agent host"
                    );
                    return;
                }
                state.hosts.insert(
                    target_id,
                    CdpAgentHost {
                        owner_id,
                        target_info,
                        endpoint: endpoint.clone(),
                        is_active: false,
                    },
                );
            }
            CdpTargetHostLifecycleDelta::InfoChanged(target_info) => {
                let Some(target_id) = remote_debugging_target_id(&target_info) else {
                    return;
                };
                let mut state = self.inner.lock();
                if let Some(host) = state.hosts.get_mut(&target_id)
                    && host.owner_id == owner_id
                {
                    host.target_info = target_info;
                }
            }
            CdpTargetHostLifecycleDelta::Activated { target_id } => {
                self.inner.lock().activate_host(owner_id, &target_id);
            }
            CdpTargetHostLifecycleDelta::Destroyed { target_id } => {
                let endpoint = {
                    let mut state = self.inner.lock();
                    if state
                        .hosts
                        .get(&target_id)
                        .is_some_and(|host| host.owner_id == owner_id)
                    {
                        state.hosts.remove(&target_id).map(|host| host.endpoint)
                    } else {
                        None
                    }
                };
                if let Some(endpoint) = endpoint {
                    endpoint.target_destroyed(target_id);
                }
            }
        }
    }
}

fn remote_debugging_target_id(target_info: &DevToolsTargetInfo) -> Option<String> {
    if !matches!(
        target_info.kind,
        DevToolsTargetKind::Page | DevToolsTargetKind::Tab
    ) {
        return None;
    }
    target_info
        .target_id
        .as_ref()
        .map(|target_id| target_id.as_str().to_owned())
}
