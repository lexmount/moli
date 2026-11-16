use super::*;
use crate::conn::{CdpConnection, CdpSessionRoute, CommandOwnerScope, SharedWorkerTargetState};

impl BrowserContext {
    pub(super) fn start_resource_interception_policy_update(
        &mut self,
        target_id: &str,
        classes: moli_core::page::SubresourceInterceptionClasses,
    ) -> anyhow::Result<Option<moli_core::page::PendingPageCommand>> {
        if let Some(worker) = self.worker_resource_target_mut(target_id) {
            if let Some(policy) = &worker.interception_policy {
                policy.set_interception(classes);
            }
            return Ok(None);
        }
        self.start_page_policy_update(
            target_id,
            moli_core::page::RendererPageRequestPolicyUpdate::FetchInterception { classes },
            |page| page.start_set_fetch_subresource_interception(classes),
        )
    }

    pub(crate) fn worker_resource_target(&self, id: &str) -> Option<&SharedWorkerTargetState> {
        self.shared_worker_target(id)
            .or_else(|| self.dedicated_worker_target(id).map(|target| &target.inner))
    }
    pub(crate) fn worker_resource_target_mut(
        &mut self,
        id: &str,
    ) -> Option<&mut SharedWorkerTargetState> {
        let Self {
            shared_worker_targets,
            dedicated_worker_targets,
            ..
        } = self;
        shared_worker_targets
            .values_mut()
            .find(|target| target.target_id == id)
            .or_else(|| {
                dedicated_worker_targets
                    .values_mut()
                    .map(|target| &mut target.inner)
                    .find(|target| target.target_id == id)
            })
    }

    pub(super) fn resource_target(&self, id: &str) -> Option<ResourceTargetStateRef<'_>> {
        let (fetch_owner, runtime_slot) = if let Some(target) = self.worker_resource_target(id) {
            (&target.fetch_owner, &target.runtime_slot)
        } else {
            let target = self.page_targets.get(id)?;
            (&target.fetch_owner, &target.runtime_slot)
        };
        Some(ResourceTargetStateRef {
            fetch_owner,
            runtime_slot,
        })
    }

    pub(super) fn resource_target_mut(
        &mut self,
        id: &str,
        session_id: Option<&str>,
    ) -> Option<ResourceTargetStateMut<'_>> {
        let owner_key = format!("{}:{id}", self.id);
        let (fetch_owner, runtime_slot) = if let Some(target) = self
            .shared_worker_targets
            .values_mut()
            .find(|target| target.target_id == id)
        {
            (&mut target.fetch_owner, &mut target.runtime_slot)
        } else if let Some(target) = self
            .dedicated_worker_targets
            .values_mut()
            .find(|target| target.target_id == id)
        {
            (
                &mut target.inner.fetch_owner,
                &mut target.inner.runtime_slot,
            )
        } else {
            let target = self.page_targets.get_mut(id)?;
            (&mut target.fetch_owner, &mut target.runtime_slot)
        };
        Some(ResourceTargetStateMut {
            fetch_owner,
            runtime_slot,
            command_session_id: session_id.map(str::to_owned),
            owner_key,
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
    pub(super) fn resource_target_identity_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<(String, String)> {
        match owner.resolve_route(self)? {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            }
            | CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => Some((browser_context_id, target_id)),
            _ => {
                let resolved = self.target_session_owner_for_owner(owner)?;
                Some((resolved.browser_context_id, resolved.target_id))
            }
        }
    }
    pub(super) fn resource_session_owner_ref_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<ResourceTargetStateRef<'_>> {
        let (context, target) = self.resource_target_identity_for_owner(owner)?;
        self.browser_context_by_id(&context)?
            .resource_target(&target)
    }
    pub(super) fn resource_session_owner_mut_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<ResourceTargetStateMut<'_>> {
        let (context, target) = self.resource_target_identity_for_owner(owner)?;
        self.browser_context_by_id_mut(&context)?
            .resource_target_mut(&target, owner.session_id())
    }
    pub(super) fn resource_session_owner_mut(
        &mut self,
        session: Option<&str>,
    ) -> Option<ResourceTargetStateMut<'_>> {
        let owner = CommandOwnerScope::capture(self, session);
        self.resource_session_owner_mut_for_owner(&owner)
    }
}

/// The shared Fetch/Network capabilities of a Page or Worker target. This view
/// deliberately has no Page host, navigation owner or document methods.
pub(super) struct ResourceTargetStateMut<'a> {
    pub(super) fetch_owner: &'a mut crate::conn::state::TargetFetchOwner,
    pub(super) runtime_slot: &'a mut crate::conn::TargetRuntimeSlot,
    pub(super) command_session_id: Option<String>,
    pub(super) owner_key: String,
}

impl ResourceTargetStateMut<'_> {
    pub(super) fn runtime_slot_mut(&mut self) -> &mut crate::conn::TargetRuntimeSlot {
        self.runtime_slot
    }
}

pub(super) struct ResourceTargetStateRef<'a> {
    pub(super) fetch_owner: &'a crate::conn::state::TargetFetchOwner,
    pub(super) runtime_slot: &'a crate::conn::TargetRuntimeSlot,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{DedicatedWorkerTargetState, TargetPageResidenceIdentity};
    use moli_shared_worker::SharedWorkerInstanceId;

    fn context() -> BrowserContext {
        let mut context = BrowserContext::new_with_page_for_test("BID-resource", "PAGE");
        let local = moli_core::RendererOwnerLocalHostId::new_for_testing(1);
        context.insert_shared_worker_target(SharedWorkerTargetState::new(
            local,
            SharedWorkerInstanceId::from_u64(11),
            "SHARED".into(),
            None,
            "https://resource.test/shared.js".into(),
            String::new(),
        ));
        context.insert_dedicated_worker_target(DedicatedWorkerTargetState::new(
            TargetPageResidenceIdentity::new_for_test(
                "BID-resource".into(),
                Some("PAGE".into()),
                1,
            ),
            local,
            12,
            "DEDICATED".into(),
            String::new(),
            Vec::new(),
        ));
        context
    }

    #[test]
    fn page_and_worker_resource_views_isolate_fetch_configuration_and_io() {
        let mut context = context();
        for (index, id) in ["PAGE", "SHARED", "DEDICATED"].into_iter().enumerate() {
            let mut owner = context
                .resource_target_mut(id, Some("SID-resource"))
                .unwrap();
            assert_eq!(owner.command_session_id.as_deref(), Some("SID-resource"));
            owner.configure_fetch(
                Some("SID-resource".into()),
                false,
                vec![crate::conn::FetchInterceptionPattern {
                    url_pattern: format!("*/probe-{index}"),
                    resource_type_filter: None,
                    request_stage: crate::conn::FetchRequestStage::Request,
                }],
            );
            assert!(owner.fetch_owner.is_enabled());
            let handle = owner.runtime_slot.allocate_io_stream_handle();
            owner.runtime_slot.insert_io_stream_body_source(
                handle.clone(),
                crate::conn::CapturedBody::from_bytes_spooled(vec![index as u8]),
                0,
            );
            assert!(owner.runtime_slot.io_stream(&handle).is_some());
        }
        for id in ["PAGE", "SHARED", "DEDICATED"] {
            let owner = context.resource_target_mut(id, None).unwrap();
            assert!(owner.fetch_owner.config_snapshot().is_enabled());
            owner.fetch_owner.reset_config();
        }
        assert!(
            !context
                .resource_target("PAGE")
                .unwrap()
                .fetch_owner
                .is_enabled()
        );
    }

    #[test]
    fn retired_worker_owner_never_falls_back_to_the_active_page() {
        let mut connection = CdpConnection::default();
        connection.install_browser_context_fixture_for_test(context());
        for route in [
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id: "BID-resource".into(),
                target_id: "closed-worker".into(),
            },
            CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id: "BID-resource".into(),
                target_id: "closed-worker".into(),
            },
        ] {
            let owner = CommandOwnerScope::for_route(route);
            assert!(
                connection
                    .resource_session_owner_mut_for_owner(&owner)
                    .is_none()
            );
            assert!(
                connection
                    .resource_session_owner_ref_for_owner(&owner)
                    .is_none()
            );
        }
        assert!(
            !connection
                .browser_context_by_id("BID-resource")
                .unwrap()
                .resource_target("PAGE")
                .unwrap()
                .fetch_owner
                .is_enabled()
        );
    }
}
