use super::*;
use crate::conn::{
    CdpConnection, CdpSessionRoute, CommandOwnerScope, PageTargetHost, SharedWorkerTargetState,
    TargetRuntimeSlot, state::TargetFetchOwner,
};
use moli_core::{
    network::ResourceInterceptionPolicy,
    page::{PendingPageCommand, RendererPageRequestPolicyUpdate, SubresourceInterceptionClasses},
    runtime::NavigationEngine,
};
use moli_page_types::DevToolsSessionKey;

enum ResourceTargetRoute {
    Page {
        target_id: Option<String>,
        session_key: DevToolsSessionKey,
    },
    SharedWorker(String),
    DedicatedWorker(String),
}

pub(super) struct ResourceTargetStateRef<'a> {
    pub(super) browser_context_id: &'a str,
    pub(super) target_id: &'a str,
    pub(super) fetch_owner: &'a TargetFetchOwner,
    pub(super) runtime_slot: &'a TargetRuntimeSlot,
    pub(super) is_worker: bool,
}

enum ResourceTargetPolicy<'a> {
    Page {
        navigation_engine: Option<&'a NavigationEngine>,
        session_key: DevToolsSessionKey,
    },
    Worker(Option<&'a ResourceInterceptionPolicy>),
}

/// Borrowed resource state for the target resolved by the command route.
/// Page-only capabilities stay on the Page owner.
pub(super) struct ResourceTargetStateMut<'a> {
    pub(super) browser_context_id: &'a str,
    pub(super) target_id: &'a str,
    pub(super) command_session_id: Option<String>,
    pub(super) fetch_owner: &'a mut TargetFetchOwner,
    pub(super) runtime_slot: &'a mut TargetRuntimeSlot,
    policy: ResourceTargetPolicy<'a>,
}

impl<'a> ResourceTargetStateMut<'a> {
    pub(super) fn for_page(
        browser_context_id: &'a str,
        target: &'a mut PageTargetHost,
        session_key: DevToolsSessionKey,
    ) -> Self {
        let (target_id, fetch_owner, runtime_slot, navigation_engine) = target.resource_parts_mut();
        Self {
            browser_context_id,
            target_id,
            command_session_id: None,
            fetch_owner,
            runtime_slot,
            policy: ResourceTargetPolicy::Page {
                navigation_engine,
                session_key,
            },
        }
    }

    fn for_worker(browser_context_id: &'a str, target: &'a mut SharedWorkerTargetState) -> Self {
        Self {
            browser_context_id,
            target_id: &target.target_id,
            command_session_id: None,
            fetch_owner: &mut target.fetch_owner,
            runtime_slot: &mut target.runtime_slot,
            policy: ResourceTargetPolicy::Worker(target.interception_policy.as_ref()),
        }
    }

    pub(super) fn is_worker(&self) -> bool {
        matches!(self.policy, ResourceTargetPolicy::Worker(_))
    }

    pub(super) fn runtime_slot_for_command(&mut self) -> &mut TargetRuntimeSlot {
        if let ResourceTargetPolicy::Page { session_key, .. } = &self.policy
            && let Some(page) = self.runtime_slot.loaded_page_mut()
        {
            page.set_renderer_devtools_command_session_id(
                session_key.wire_session_id().map(str::to_owned),
            );
        }
        self.runtime_slot
    }

    pub(super) fn into_runtime_slot_for_command(mut self) -> &'a mut TargetRuntimeSlot {
        self.runtime_slot_for_command();
        self.runtime_slot
    }

    pub(super) fn start_fetch_policy_update(
        &self,
        classes: SubresourceInterceptionClasses,
    ) -> anyhow::Result<Option<PendingPageCommand>> {
        match self.policy {
            ResourceTargetPolicy::Page {
                navigation_engine, ..
            } => self.runtime_slot.start_page_policy_update(
                navigation_engine,
                RendererPageRequestPolicyUpdate::FetchInterception { classes },
                |page| page.start_set_fetch_subresource_interception(classes),
            ),
            ResourceTargetPolicy::Worker(policy) => {
                if let Some(policy) = policy {
                    policy.set_interception(classes);
                }
                Ok(None)
            }
        }
    }
}

impl BrowserContext {
    pub(crate) fn worker_resource_target(&self, id: &str) -> Option<&SharedWorkerTargetState> {
        self.shared_worker_target(id)
            .or_else(|| self.dedicated_worker_target(id).map(|target| &target.inner))
    }

    fn worker_resource_target_mut(&mut self, id: &str) -> Option<&mut SharedWorkerTargetState> {
        self.shared_worker_targets
            .values_mut()
            .find(|target| target.target_id == id)
            .or_else(|| {
                self.dedicated_worker_targets
                    .values_mut()
                    .find(|target| target.target_id == id)
                    .map(|target| &mut target.inner)
            })
    }
}

impl CdpConnection {
    pub(crate) fn native_network_request_id(
        &mut self,
        handle: moli_core::page::SubresourceNetworkRequestHandle,
    ) -> String {
        self.network_request_id_allocator
            .request_id_for_native(handle)
    }

    pub(crate) fn ingest_worker_network_output(
        &mut self,
        context: &str,
        target: &str,
        item: &moli_core::page::ScriptNetworkOutputItem,
    ) -> Option<crate::domains::network::TargetNetworkBacklogPreparedDelivery> {
        let mut allocator = std::mem::take(&mut self.network_request_id_allocator);
        let delivery = self
            .browser_context_by_id_mut(context)
            .and_then(|context| context.worker_resource_target_mut(target))
            .map(|target| {
                target
                    .runtime_slot
                    .ingest_worker_network_output(item, &mut allocator)
            });
        self.network_request_id_allocator = allocator;
        delivery
    }

    fn resource_target_route_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<(String, ResourceTargetRoute)> {
        let page = || ResourceTargetRoute::Page {
            target_id: None,
            session_key: DevToolsSessionKey::Primary,
        };
        Some(match owner.resolve_route(self)? {
            CdpSessionRoute::Browser => (self.browser_context.as_ref()?.id.clone(), page()),
            CdpSessionRoute::BrowserContext { browser_context_id } => (browser_context_id, page()),
            CdpSessionRoute::PageTarget {
                browser_context_id,
                target_id,
                session_key,
            } => (
                browser_context_id,
                ResourceTargetRoute::Page {
                    target_id: Some(target_id),
                    session_key,
                },
            ),
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => (
                browser_context_id,
                ResourceTargetRoute::SharedWorker(target_id),
            ),
            CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => (
                browser_context_id,
                ResourceTargetRoute::DedicatedWorker(target_id),
            ),
            CdpSessionRoute::TabTarget { .. } | CdpSessionRoute::ServiceWorkerTarget { .. } => {
                return None;
            }
        })
    }

    pub(super) fn resource_target_state_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<ResourceTargetStateRef<'_>> {
        let (context_id, route) = self.resource_target_route_for_owner(owner)?;
        let context = self.browser_context_by_id(&context_id)?;
        let worker = match route {
            ResourceTargetRoute::Page { target_id, .. } => {
                let id = target_id
                    .as_deref()
                    .or_else(|| context.active_target_id())?;
                let target = context.page_target(id)?;
                return Some(ResourceTargetStateRef {
                    browser_context_id: &context.id,
                    target_id: target.target_id(),
                    fetch_owner: &target.fetch_owner,
                    runtime_slot: &target.runtime_slot,
                    is_worker: false,
                });
            }
            ResourceTargetRoute::SharedWorker(target_id) => {
                context.shared_worker_target(&target_id)?
            }
            ResourceTargetRoute::DedicatedWorker(target_id) => {
                &context.dedicated_worker_target(&target_id)?.inner
            }
        };
        Some(ResourceTargetStateRef {
            browser_context_id: &context.id,
            target_id: &worker.target_id,
            fetch_owner: &worker.fetch_owner,
            runtime_slot: &worker.runtime_slot,
            is_worker: true,
        })
    }

    pub(crate) fn owner_has_worker_resource_target(&self, owner: &CommandOwnerScope) -> bool {
        self.resource_target_state_for_owner(owner)
            .is_some_and(|target| target.is_worker)
    }

    pub(super) fn resource_target_state_mut_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<ResourceTargetStateMut<'_>> {
        let (context_id, route) = self.resource_target_route_for_owner(owner)?;
        let context = self.browser_context_by_id_mut(&context_id)?;
        let mut target = match route {
            ResourceTargetRoute::Page {
                target_id,
                session_key,
            } => {
                let page = match target_id {
                    Some(id) => context.page_targets.get_mut(&id)?,
                    None => context.page_targets.active_mut()?,
                };
                ResourceTargetStateMut::for_page(&context.id, page, session_key)
            }
            ResourceTargetRoute::SharedWorker(target_id) => {
                let target = context
                    .shared_worker_targets
                    .values_mut()
                    .find(|target| target.target_id == target_id)?;
                ResourceTargetStateMut::for_worker(&context.id, target)
            }
            ResourceTargetRoute::DedicatedWorker(target_id) => {
                let target = context
                    .dedicated_worker_targets
                    .values_mut()
                    .find(|target| target.target_id == target_id)?;
                ResourceTargetStateMut::for_worker(&context.id, &mut target.inner)
            }
        };
        target.command_session_id = owner.session_id().map(str::to_owned);
        Some(target)
    }

    pub(super) fn resource_target_state_mut(
        &mut self,
        session: Option<&str>,
    ) -> Option<ResourceTargetStateMut<'_>> {
        let owner = CommandOwnerScope::capture(self, session);
        self.resource_target_state_mut_for_owner(&owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::RendererOwnerLocalHostId;
    use moli_shared_worker::SharedWorkerInstanceId;

    #[test]
    fn scoped_worker_stream_resolves_only_for_its_target() {
        let mut context = BrowserContext::new_with_page_for_test("BID-stream", "TID-page");
        let mut worker = SharedWorkerTargetState::new(
            RendererOwnerLocalHostId::new_for_testing(1),
            SharedWorkerInstanceId::from_u64(1),
            "TID-worker".to_owned(),
            None,
            "https://example.test/worker.js".to_owned(),
            String::new(),
        );
        worker.attach_session("SID-worker".to_owned());
        context.insert_shared_worker_target(worker);
        let mut conn = CdpConnection::default();
        conn.install_browser_context_fixture_for_test(context);

        let handle = conn
            .open_io_stream_for_session_owner(Some("SID-worker"), vec![1, 2, 3])
            .unwrap();
        assert!(conn.io_stream_for_stream_owner(None, &handle).is_none());
        assert!(!conn.close_io_stream_for_stream_owner(None, &handle));
        assert!(
            conn.io_stream_for_stream_owner(Some("SID-worker"), &handle)
                .is_some()
        );
        assert!(conn.close_io_stream_for_stream_owner(Some("SID-worker"), &handle));
        assert!(
            conn.io_stream_for_stream_owner(Some("SID-worker"), &handle)
                .is_none()
        );
    }
}
