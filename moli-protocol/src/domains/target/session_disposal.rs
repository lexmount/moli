use anyhow::Result;
use futures_util::{
    FutureExt as _,
    future::{LocalBoxFuture, Shared},
};

use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandOwnerScope, ProtocolWorkflowDriver,
    RendererPageResidenceIdentity, SessionDisposalPlan, SessionDisposalTarget, TargetEventPlan,
    TargetSessionDetachCleanupPlan,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionDisposalError {
    #[error("InvalidSessionId")]
    InvalidSession,
    #[error("UnknownTargetId")]
    UnknownTarget,
    #[error("Session is closing")]
    Closing,
}

pub(in crate::domains::target) fn prepare_session_disposal(
    conn: &CdpConnection,
    detachment: TargetSessionDetachCleanupPlan,
) -> Result<(SessionDisposalPlan, TargetSessionDetachCleanupPlan)> {
    let session_id = detachment.session_id();
    let route = conn
        .session_route(Some(session_id))
        .ok_or(SessionDisposalError::InvalidSession)?;
    let plan = SessionDisposalPlan::for_session_route(session_id, &route)
        .ok_or(SessionDisposalError::InvalidSession)?;
    anyhow::ensure!(
        plan.target_id() == Some(detachment.target_id()),
        SessionDisposalError::UnknownTarget
    );
    Ok((plan, detachment))
}

pub(crate) enum PreparedSessionDisposal {
    Own(LiveSessionDisposal),
    Join {
        identity: SessionDisposalIdentity,
        wait: crate::conn::SessionDisposalWait,
    },
}

impl PreparedSessionDisposal {
    pub(crate) fn prepare(conn: &mut CdpConnection, plan: SessionDisposalPlan) -> Self {
        Self::for_identity(conn, SessionDisposalIdentity::capture(conn, plan))
            .expect("a finite disposal preparation retains its original binding")
    }

    pub(crate) fn for_identity(
        conn: &mut CdpConnection,
        identity: SessionDisposalIdentity,
    ) -> Option<Self> {
        if !identity.binding_is_current(conn) {
            return None;
        }
        Some(
            match conn.session_disposal_wait(identity.plan().session_id()) {
                Some(wait) => Self::Join { identity, wait },
                None => Self::Own(LiveSessionDisposal::prepare_for_identity(conn, identity)),
            },
        )
    }

    pub(crate) fn identity(&self) -> SessionDisposalIdentity {
        match self {
            Self::Own(disposal) => disposal.identity(),
            Self::Join { identity, .. } => identity.clone(),
        }
    }

    pub(crate) fn take_browser_events(&mut self) -> Vec<BackgroundProtocolEvent> {
        match self {
            Self::Own(disposal) => disposal.handlers.take_browser_events(),
            Self::Join { .. } => Vec::new(),
        }
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        background_events: &mut Vec<BackgroundProtocolEvent>,
        protocol_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> Result<Option<moli_core::RendererOutputFence>> {
        match self {
            Self::Own(disposal) => {
                disposal
                    .run(driver, background_events, protocol_events)
                    .await
            }
            Self::Join { wait, .. } => {
                wait.wait().await;
                Ok(None)
            }
        }
    }
}

/// Captures the original renderer before any handler can wait. A session can
/// remain attached while a successor Document replaces that renderer.
pub(crate) struct LiveSessionDisposal {
    handlers: crate::domains::session::PreparedSessionHandlers,
    native_retirements: Vec<moli_core::page::RendererRuntimeInspectorSessionRetirement>,
}

#[derive(Clone)]
pub(crate) struct SessionDisposalIdentity {
    plan: SessionDisposalPlan,
    cleanup_owner: CommandOwnerScope,
    binding: Option<crate::conn::SessionDisposalWait>,
    page: Option<SessionDisposalPage>,
}

#[derive(Clone)]
struct SessionDisposalPage {
    residence: RendererPageResidenceIdentity,
    attachment: Option<moli_core::page::RendererAgentAttachmentId>,
    document: Option<moli_core::RendererDocumentLifecycleIdentity>,
}

pub(crate) struct FailedSessionPageRetirement {
    page: Option<moli_core::page::Page>,
    events: Vec<BackgroundProtocolEvent>,
}

impl FailedSessionPageRetirement {
    pub(crate) fn take_page(&mut self) -> Option<moli_core::page::Page> {
        self.page.take()
    }
}

impl LiveSessionDisposal {
    pub(crate) fn identity(&self) -> SessionDisposalIdentity {
        self.handlers.identity().clone()
    }

    pub(crate) fn prepare(conn: &mut CdpConnection, plan: SessionDisposalPlan) -> Self {
        Self::prepare_for_identity(conn, SessionDisposalIdentity::capture(conn, plan))
    }

    fn prepare_for_identity(conn: &mut CdpConnection, identity: SessionDisposalIdentity) -> Self {
        let handlers =
            crate::domains::session::PreparedSessionHandlers::prepare_for_identity(conn, identity);
        let owner = CommandOwnerScope::for_session(handlers.identity().plan().session_id());
        let native_retirements = if matches!(
            handlers.identity().plan().target(),
            SessionDisposalTarget::PageTarget { .. }
        ) {
            conn.prepare_page_inspector_session_retirements(&owner)
        } else {
            Vec::new()
        };
        conn.mark_session_closing(handlers.identity().plan());
        Self {
            handlers,
            native_retirements,
        }
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        background_events: &mut Vec<BackgroundProtocolEvent>,
        protocol_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> Result<Option<moli_core::RendererOutputFence>> {
        let identity = self.identity();
        let disposal = self
            .handlers
            .run(driver, background_events, protocol_events)
            .await;
        let (error, predecessor) = disposal.into_parts();
        if let Some(error) = error {
            identity
                .recover_failure(driver, background_events, error)
                .await?;
        }
        let detached = if self.native_retirements.is_empty() {
            let completion = identity.clone();
            driver.with_state(move |state| state.detach_disposed_session_inspector(&completion))?
        } else {
            self.native_retirements
                .into_iter()
                .try_for_each(|retirement| retirement.finish())
        };
        if let Err(error) = detached {
            identity
                .recover_failure(driver, background_events, error)
                .await?;
        }
        Ok(predecessor)
    }
}

impl SessionDisposalIdentity {
    pub(crate) fn capture(conn: &CdpConnection, plan: SessionDisposalPlan) -> Self {
        let owner = CommandOwnerScope::for_session(plan.session_id());
        let page = conn
            .runtime_session_owner_slot_for_owner(&owner)
            .and_then(|slot| slot.loaded_page())
            .map(|page| SessionDisposalPage {
                residence: RendererPageResidenceIdentity::from_page(page),
                attachment: page.renderer_agent_attachment_id(),
                document: conn.target_root_document_lifecycle_identity_for_owner(&owner),
            });
        let binding = conn.session_binding_lifetime(plan.session_id());
        let cleanup_owner = owner
            .resolve_route(conn)
            .map(CommandOwnerScope::for_route)
            .unwrap_or(owner);
        Self {
            plan,
            cleanup_owner,
            binding,
            page,
        }
    }

    pub(crate) fn plan(&self) -> &SessionDisposalPlan {
        &self.plan
    }

    pub(crate) fn cleanup_owner(&self) -> &CommandOwnerScope {
        &self.cleanup_owner
    }

    pub(crate) fn cleanup_binding_was_not_replaced(&self, conn: &CdpConnection) -> bool {
        let current = conn.session_binding_lifetime(self.plan.session_id());
        current.is_none() || current.as_ref() == self.binding.as_ref()
    }

    /// Closing retains cleanup authority; only this original binding may
    /// accept its state updates. Literal session ids and routes can be reused.
    pub(crate) fn binding_is_current(&self, conn: &CdpConnection) -> bool {
        let current = conn.session_binding_lifetime(self.plan.session_id());
        match self.binding.as_ref() {
            Some(binding) => !binding.is_closed() && current.as_ref() == Some(binding),
            // Prepared attachment rollback may have no committed CDP binding.
            // It must not claim a binding registered after preparation.
            None => current.is_none(),
        }
    }

    pub(crate) fn is_current(&self, conn: &CdpConnection) -> bool {
        self.binding_is_current(conn)
            && conn
                .session_route(Some(self.plan.session_id()))
                .and_then(|route| {
                    SessionDisposalPlan::for_session_route(self.plan.session_id(), &route)
                })
                .as_ref()
                == Some(&self.plan)
    }

    async fn recover_failure(
        &self,
        driver: &mut ProtocolWorkflowDriver,
        background_events: &mut Vec<BackgroundProtocolEvent>,
        error: anyhow::Error,
    ) -> Result<()> {
        if !matches!(self.plan.target(), SessionDisposalTarget::PageTarget { .. }) {
            return Err(error);
        }
        let preparation = self.clone();
        let mut retired = driver
            .with_state(move |state| state.prepare_failed_session_page_retirement(&preparation))?;
        if let Some(page) = retired.take_page() {
            let _ = page.close_async().await;
        }
        let completion = self.clone();
        background_events.extend(driver.with_state(move |state| {
            state.finish_failed_session_page_retirement(&completion, retired)
        })?);
        tracing::warn!(session_id = self.plan.session_id(), %error,
            "finished failed session cleanup after exact Page retirement");
        Ok(())
    }

    fn owns_current_page(&self, conn: &CdpConnection) -> bool {
        let owner = self.cleanup_owner();
        self.cleanup_binding_was_not_replaced(conn)
            && self.page.as_ref().is_some_and(|expected| {
                conn.renderer_page_residence_identity_for_owner(owner) == Some(expected.residence)
                    && conn.current_renderer_agent_attachment_id_for_owner(owner)
                        == expected.attachment
                    && conn.target_root_document_lifecycle_identity_for_owner(owner)
                        == expected.document
            })
    }

    pub(crate) fn detach_inspector(&self, conn: &mut CdpConnection) -> Result<()> {
        if !self.binding_is_current(conn) {
            return Ok(());
        }
        if matches!(self.plan.target(), SessionDisposalTarget::PageTarget { .. })
            && !self.owns_current_page(conn)
        {
            return Ok(());
        }
        crate::domains::runtime::detach_session_inspector(conn, &self.plan)
    }

    pub(crate) fn prepare_failed_page_retirement(
        &self,
        conn: &mut CdpConnection,
    ) -> FailedSessionPageRetirement {
        let owner = self.cleanup_owner().clone();
        if !self.owns_current_page(conn) {
            return FailedSessionPageRetirement {
                page: None,
                events: Vec::new(),
            };
        }
        let inspector_sessions = conn.page_event_session_ids_for_owner(&owner);
        for session in &inspector_sessions {
            let inspector_owner = session
                .as_deref()
                .map(CommandOwnerScope::for_session)
                .unwrap_or_else(|| owner.clone());
            let _ =
                conn.with_target_devtools_session_state_for_owner_mut(&inspector_owner, |state| {
                    state
                        .runtime_session_state
                        .record_inspector_target_crashed()
                });
        }
        let page = conn.take_crashed_page_for_owner(&owner);
        let mut events: Vec<_> = inspector_sessions
            .into_iter()
            .map(|session| BackgroundProtocolEvent::inspector_target_crashed(session.as_deref()))
            .collect();
        if let Some(target_id) = self.plan.target_id() {
            events.extend(
                conn.target_crashed_events_for_all_discovery_owners(target_id, "crashed", 1),
            );
        }
        FailedSessionPageRetirement { page, events }
    }

    pub(crate) fn finish_failed_page_retirement(
        &self,
        conn: &mut CdpConnection,
        retired: FailedSessionPageRetirement,
    ) -> Vec<BackgroundProtocolEvent> {
        if !self.cleanup_binding_was_not_replaced(conn) {
            return Vec::new();
        }
        if let SessionDisposalTarget::PageTarget { session_key, .. } = self.plan.target() {
            // Retire only this session's metadata. Native removal belongs to
            // the original Page, never to a replacement installed during close.
            let _ = conn.with_target_owner_state_for_owner_mut(self.cleanup_owner(), |state| {
                state.remove_document_start_scripts_for_session(session_key)
            });
        }
        let owner = self.cleanup_owner().clone();
        if conn
            .renderer_page_residence_identity_for_owner(&owner)
            .is_none()
        {
            retired.events
        } else {
            // Do not deliver A's late crash notification to a replacement B.
            Vec::new()
        }
    }
}

/// Completes disposal after a worker target has transferred and dropped its
/// per-session state. Renderer-owned resources have already gone away with
/// the target, but connection-owned handlers still retire before the control
/// plane binding is removed.
#[derive(Clone)]
pub(crate) struct PreparedRemovedWorkerSession {
    identity: SessionDisposalIdentity,
    detachment: TargetSessionDetachCleanupPlan,
    pending: Shared<LocalBoxFuture<'static, ()>>,
}

impl PreparedRemovedWorkerSession {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        cleanup_plan: TargetSessionDetachCleanupPlan,
    ) -> Result<Self> {
        let (plan, detachment) = prepare_session_disposal(conn, cleanup_plan)?;
        anyhow::ensure!(
            matches!(
                plan.target(),
                SessionDisposalTarget::SharedWorkerTarget { .. }
                    | SessionDisposalTarget::DedicatedWorkerTarget { .. }
                    | SessionDisposalTarget::ServiceWorkerTarget { .. }
            ),
            SessionDisposalError::InvalidSession
        );
        let identity = SessionDisposalIdentity::capture(conn, plan);
        let pending = match conn.session_disposal_wait(identity.plan().session_id()) {
            Some(wait) => async move { wait.wait().await }.boxed_local().shared(),
            None => {
                conn.mark_session_closing(identity.plan());
                let pending = crate::domains::session::prepare_closed_handlers(
                    conn,
                    identity.plan().session_id(),
                );
                async move {
                    if let Some(pending) = pending {
                        pending.wait().await;
                    }
                }
                .boxed_local()
                .shared()
            }
        };
        Ok(Self {
            identity,
            detachment,
            pending,
        })
    }

    pub(crate) async fn wait(&self) {
        self.pending.clone().await;
    }

    pub(crate) fn finish(self, conn: &mut CdpConnection) -> TargetEventPlan {
        if !self.identity.is_current(conn) {
            return TargetEventPlan::default();
        }
        // Only this binding may release its connection-owned trace and detach.
        // A later binding can reuse every literal id, including its route.
        conn.cancel_tracing_for_session_owner(Some(self.identity.plan().session_id()));
        conn.commit_target_session_detachment_event_plan(self.detachment)
    }
}

/// Emergency completion for a DedicatedWorker retirement whose renderer
/// output failed after the target state was already removed. The renderer is
/// unavailable, so only connection and control-plane ownership remain.
pub(super) fn dispose_removed_dedicated_worker_session_after_failed_retirement(
    conn: &mut CdpConnection,
    cleanup_plan: TargetSessionDetachCleanupPlan,
) -> TargetEventPlan {
    crate::domains::session::dispose_closed_handlers_sync(conn, cleanup_plan.session_id());
    conn.commit_target_session_detachment_after_prepared_state_delta_event_plan(cleanup_plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::BrowserContext;
    use crate::testing::{TestContext, wait_until_renderer_document_load};
    use base64::Engine as _;
    use serde_json::json;

    #[tokio::test]
    async fn late_old_session_cleanup_cannot_retire_or_reconfigure_its_successor() {
        for retire_before_successor in [false, true] {
            let mut context = TestContext::new();
            let mut browser = BrowserContext::new("BID-retired-cleanup".into());
            browser.set_active_target_id("TID-retired-cleanup");
            browser.attach_active_session("SID-old");
            assert!(
                browser.assign_attached_session_to_target("TID-retired-cleanup", "SID-peer".into())
            );
            context.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
            context
                .install_navigation_fixture_for_session_owner(
                    "about:blank#original",
                    Some("SID-old"),
                )
                .await;
            for (id, method, params) in [
                (
                    1,
                    "Page.addScriptToEvaluateOnNewDocument",
                    json!({"source": "globalThis.oldSessionPreload = true;"}),
                ),
                (
                    2,
                    "Emulation.setScriptExecutionDisabled",
                    json!({"value": true}),
                ),
            ] {
                context.process_async(json!({"id": id, "sessionId": "SID-old", "method": method, "params": params})).await;
                assert!(context.take_response_by_id(id).get("result").is_some());
            }
            let work = context.process_command_only_async(json!({
                "id": 10, "method": "Target.detachFromTarget", "params": {"sessionId": "SID-old"}
            })).await;
            assert!(
                context
                    .sent
                    .iter()
                    .any(|message| message["id"] == 10 && message["result"] == json!({}))
            );
            assert!(context.with_fixture(|conn| !conn.has_attached_devtools_session("SID-old")));
            let mut cleanup = Some(work);
            context.with_fixture(|conn| {
                assert!(
                    !conn
                        .navigation_load_inputs_for_owner(&CommandOwnerScope::for_session(
                            "SID-peer"
                        ))
                        .script_execution_disabled,
                    "Closing must revoke the old session's script policy"
                )
            });
            if retire_before_successor {
                context
                    .route_direct_command_output_for_test(Vec::new(), cleanup.take().unwrap())
                    .await;
                context
                    .wait_for_direct_command_work_completion_for_test(
                        "original cleanup before successor",
                    )
                    .await;
            }
            let successor_url = format!("data:text/html;base64,{}", base64::engine::general_purpose::STANDARD.encode(
                "<script>globalThis.successor=[true,typeof oldSessionPreload]</script><p>successor</p>"));
            context
                .process_async(json!({
                    "id": 20, "sessionId": "SID-peer", "method": "Page.navigate",
                    "params": {"url": successor_url},
                }))
                .await;
            let navigation = context.take_response_by_id(20);
            assert!(
                navigation.get("result").is_some(),
                "retirement before successor={retire_before_successor}: {navigation:?}"
            );
            assert!(
                navigation["result"].get("errorText").is_none(),
                "retirement before successor={retire_before_successor}: {navigation:?}"
            );
            let loader_id = navigation["result"]["loaderId"].as_str().unwrap();
            wait_until_renderer_document_load(
                &mut context,
                Some("SID-peer"),
                "TID-retired-cleanup",
                loader_id,
            )
            .await;
            let owner = CommandOwnerScope::for_session("SID-peer");
            let successor = context.with_fixture(|conn| {
                conn.renderer_page_residence_identity_for_owner(&owner)
                    .unwrap()
            });
            let document = context.with_fixture(|conn| {
                conn.target_root_document_lifecycle_identity_for_owner(&owner)
            });
            context.process_async(json!({"id": 21, "sessionId": "SID-peer", "method": "Runtime.evaluate", "params": {"expression": "globalThis.successor", "returnByValue": true}})).await;
            let before_recovery = context.take_response_by_id(21);
            assert_eq!(
                before_recovery["result"]["result"]["value"],
                json!([true, "undefined"]),
                "before recovery retirement={retire_before_successor}: {before_recovery:?}"
            );
            // Browser withdrawal is complete; native cleanup may settle before
            // or after a successor replaces the original renderer.
            if let Some(work) = cleanup.take() {
                context
                    .route_direct_command_output_for_test(Vec::new(), work)
                    .await;
                context
                    .wait_for_direct_command_work_completion_for_test(
                        "original cleanup after successor",
                    )
                    .await;
            }
            assert!(
                !context
                    .sent
                    .iter()
                    .any(|event| event["method"] == "Inspector.targetCrashed"
                        || event["method"] == "Target.targetCrashed")
            );
            context.with_fixture(|conn| {
                assert_eq!(
                    conn.renderer_page_residence_identity_for_owner(&owner),
                    Some(successor)
                )
            });
            context.with_fixture(|conn| {
                assert_eq!(
                    conn.target_root_document_lifecycle_identity_for_owner(&owner),
                    document
                )
            });
            context.with_fixture(|conn| assert!(!conn.has_attached_devtools_session("SID-old")));
            context.with_fixture(|conn| assert!(conn.has_attached_devtools_session("SID-peer")));
            context.process_async(json!({"id": 11, "sessionId": "SID-peer", "method": "Runtime.evaluate", "params": {"expression": "globalThis.successor", "returnByValue": true}})).await;
            let evaluation = context.take_response_by_id(11);
            assert_eq!(
                evaluation["result"]["result"]["value"],
                json!([true, "undefined"]),
                "retirement before successor={retire_before_successor}: {evaluation:?}"
            );
        }
    }
}
