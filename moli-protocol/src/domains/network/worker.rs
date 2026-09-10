use moli_core::page::RendererWorkerNetworkSource;

use crate::conn::{CdpConnection, CdpSessionRoute, CommandOwnerScope};

use super::TargetNetworkAgentState;

impl CdpConnection {
    pub(crate) fn network_owner_identity_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<(String, Option<String>)> {
        match owner.resolve_route(self)? {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            }
            | CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id,
                target_id,
            } => Some((browser_context_id, Some(target_id))),
            _ => self.target_owner_identity_for_owner(owner),
        }
    }

    /// Resolve the physical Worker, never the currently selected Page or a
    /// replacement execution of the same Service Worker version.
    pub(crate) fn native_worker_network_owner(
        &self,
        context_id: &str,
        source: &RendererWorkerNetworkSource,
    ) -> Option<CommandOwnerScope> {
        let context = self.browser_context_by_id(context_id)?;
        let route = match source {
            RendererWorkerNetworkSource::Shared(instance) => CdpSessionRoute::SharedWorkerTarget {
                browser_context_id: context_id.to_owned(),
                target_id: context
                    .shared_worker_targets
                    .get(instance)?
                    .target_id
                    .clone(),
            },
            RendererWorkerNetworkSource::Service { version, run } => {
                let target = context.service_worker_targets.get(version)?;
                if target.active_renderer_run() != Some(run) {
                    return None;
                }
                CdpSessionRoute::ServiceWorkerTarget {
                    browser_context_id: context_id.to_owned(),
                    target_id: target.target_id.clone(),
                }
            }
        };
        Some(CommandOwnerScope::for_route(route))
    }

    pub(crate) fn network_agent_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<&TargetNetworkAgentState> {
        match owner.resolve_route(self)? {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => Some(
                &self
                    .browser_context_by_id(&browser_context_id)?
                    .shared_worker_target(&target_id)?
                    .network,
            ),
            CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id,
                target_id,
            } => Some(
                &self
                    .browser_context_by_id(&browser_context_id)?
                    .service_worker_target(&target_id)?
                    .network,
            ),
            _ => Some(
                &self
                    .runtime_session_owner_slot_for_owner(owner)
                    .ok()?
                    .network_agent,
            ),
        }
    }

    pub(crate) fn network_agent_for_owner_mut(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<&mut TargetNetworkAgentState> {
        match owner.resolve_route(self)? {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => Some(
                &mut self
                    .browser_context_by_id_mut(&browser_context_id)?
                    .shared_worker_target_mut(&target_id)?
                    .network,
            ),
            CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id,
                target_id,
            } => Some(
                &mut self
                    .browser_context_by_id_mut(&browser_context_id)?
                    .service_worker_target_mut(&target_id)?
                    .network,
            ),
            _ => Some(
                &mut self
                    .runtime_session_owner_slot_mut_for_owner(owner)
                    .ok()?
                    .network_agent,
            ),
        }
    }
}
