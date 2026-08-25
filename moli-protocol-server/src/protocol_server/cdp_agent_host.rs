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
    page_hosts: HashMap<String, CdpPageAgentHost>,
}

#[derive(Clone)]
struct CdpPageAgentHost {
    owner_id: u64,
    target_info: DevToolsTargetInfo,
    endpoint: CdpFrontendEndpoint,
    is_active: bool,
}

#[derive(Clone)]
pub(super) struct CdpPageAgentHostRoute {
    pub(super) endpoint: CdpFrontendEndpoint,
    pub(super) target_info: DevToolsTargetInfo,
}

impl CdpPageAgentHost {
    fn browser_context_id(&self) -> Option<&str> {
        self.target_info
            .browser_context_id
            .as_ref()
            .map(|id| id.as_str())
    }
}

impl CdpAgentHostDirectoryState {
    fn activate_page_host(&mut self, owner_id: u64, target_id: &str) {
        let Some(browser_context_id) = self
            .page_hosts
            .get(target_id)
            .filter(|host| host.owner_id == owner_id)
            .map(|host| host.browser_context_id().map(str::to_owned))
        else {
            return;
        };

        for host in self.page_hosts.values_mut().filter(|host| {
            host.owner_id == owner_id && host.browser_context_id() == browser_context_id.as_deref()
        }) {
            host.is_active = false;
        }
        if let Some(host) = self.page_hosts.get_mut(target_id) {
            host.is_active = true;
        }
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

    pub(super) fn lookup_page(&self, target_id: &str) -> Option<CdpPageAgentHostRoute> {
        let state = self.inner.lock();
        let host = state.page_hosts.get(target_id)?;
        Some(CdpPageAgentHostRoute {
            endpoint: host.endpoint.clone(),
            target_info: host.target_info.clone(),
        })
    }

    pub(super) fn page_target_infos(&self) -> Vec<DevToolsTargetInfo> {
        let state = self.inner.lock();
        let mut hosts = state.page_hosts.iter().collect::<Vec<_>>();
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
            .page_hosts
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
                let Some(target_id) = page_target_id(&target_info) else {
                    return;
                };
                let mut state = self.inner.lock();
                if let Some(existing) = state.page_hosts.get(&target_id)
                    && existing.owner_id != owner_id
                {
                    tracing::warn!(
                        target_id,
                        existing_owner_id = existing.owner_id,
                        owner_id,
                        "refusing to replace a live CDP page agent host"
                    );
                    return;
                }
                state.page_hosts.insert(
                    target_id,
                    CdpPageAgentHost {
                        owner_id,
                        target_info,
                        endpoint: endpoint.clone(),
                        is_active: false,
                    },
                );
            }
            CdpTargetHostLifecycleDelta::InfoChanged(target_info) => {
                let Some(target_id) = page_target_id(&target_info) else {
                    return;
                };
                let mut state = self.inner.lock();
                if let Some(host) = state.page_hosts.get_mut(&target_id)
                    && host.owner_id == owner_id
                {
                    host.target_info = target_info;
                }
            }
            CdpTargetHostLifecycleDelta::Activated { target_id } => {
                self.inner.lock().activate_page_host(owner_id, &target_id);
            }
            CdpTargetHostLifecycleDelta::Destroyed { target_id } => {
                let endpoint = {
                    let mut state = self.inner.lock();
                    if state
                        .page_hosts
                        .get(&target_id)
                        .is_some_and(|host| host.owner_id == owner_id)
                    {
                        state
                            .page_hosts
                            .remove(&target_id)
                            .map(|host| host.endpoint)
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

fn page_target_id(target_info: &DevToolsTargetInfo) -> Option<String> {
    if target_info.kind != DevToolsTargetKind::Page {
        return None;
    }
    target_info
        .target_id
        .as_ref()
        .map(|target_id| target_id.as_str().to_owned())
}
