use super::target_session_owner::TargetSessionOwnerMut;
use super::*;
use crate::conn::{CdpConnection, CdpSessionRoute, CommandOwnerScope, SharedWorkerTargetState};
use moli_page_types::DevToolsSessionKey;

impl BrowserContext {
    pub(crate) fn worker_resource_target(&self, id: &str) -> Option<&SharedWorkerTargetState> {
        self.shared_worker_target(id)
            .or_else(|| self.dedicated_worker_target(id).map(|target| &target.inner))
    }
    pub(crate) fn worker_resource_target_mut(
        &mut self,
        id: &str,
    ) -> Option<&mut SharedWorkerTargetState> {
        if self.shared_worker_target(id).is_some() {
            return self.shared_worker_target_mut(id);
        }
        self.dedicated_worker_target_mut(id)
            .map(|target| &mut target.inner)
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

    pub(crate) fn worker_resource_identity_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<(String, String)> {
        let (context, target) = match owner.resolve_route(self)? {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            }
            | CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => (browser_context_id, target_id),
            _ => return None,
        };
        self.browser_context_by_id(&context)?
            .worker_resource_target(&target)?;
        Some((context, target))
    }
    pub(super) fn resource_session_owner_mut_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<TargetSessionOwnerMut<'_>> {
        if let Some((context, target_id)) = self.worker_resource_identity_for_owner(owner) {
            return Some(TargetSessionOwnerMut {
                browser_context: self.browser_context_by_id_mut(&context)?,
                target_id,
                command_session_id: owner.session_id().map(str::to_owned),
                session_key: DevToolsSessionKey::Primary,
            });
        }
        self.target_session_owner_mut_for_owner(owner)
    }
    pub(super) fn resource_session_owner_mut(
        &mut self,
        session: Option<&str>,
    ) -> Option<TargetSessionOwnerMut<'_>> {
        let owner = CommandOwnerScope::capture(self, session);
        self.resource_session_owner_mut_for_owner(&owner)
    }
}
