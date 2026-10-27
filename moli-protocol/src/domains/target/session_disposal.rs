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

pub(crate) async fn close_page_target_using(
    driver: &mut ProtocolWorkflowDriver<'_>,
    target_id: String,
    events: &mut Vec<BackgroundProtocolEvent>,
    reason: &'static str,
) -> Result<Option<crate::conn::ClosedPageTarget>> {
    let (pending, claimed_events) = driver
        .with_state(move |state| {
            let mut events = Vec::new();
            let pending = state.start_page_target_close(&target_id, &mut events, reason);
            (pending, events)
        })
        .await?;
    events.extend(claimed_events);
    let Some(pending) = pending else {
        return Ok(None);
    };
    let completed = pending.wait().await;
    driver
        .with_state(move |state| state.finish_page_target_close(completed))
        .await
        .map(Some)
}

pub(crate) async fn dispose_closed_target_sessions_using(
    driver: &mut ProtocolWorkflowDriver<'_>,
    plan: crate::conn::TargetClosureCleanupPlan,
    parent_session_id: Option<String>,
) -> Result<TargetEventPlan> {
    let pending = driver
        .with_state(move |state| state.prepare_closed_target_sessions(plan, parent_session_id))
        .await?;
    let completed = pending.wait().await;
    driver
        .with_state(move |state| state.finish_closed_target_sessions(completed))
        .await
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

fn prepare_browser_session_disposal(
    conn: &CdpConnection,
    session_id: &str,
) -> Result<SessionDisposalPlan> {
    let route = conn
        .session_route(Some(session_id))
        .ok_or(SessionDisposalError::InvalidSession)?;
    let plan = SessionDisposalPlan::for_session_route(session_id, &route)
        .ok_or(SessionDisposalError::InvalidSession)?;
    anyhow::ensure!(
        matches!(plan.target(), SessionDisposalTarget::Browser),
        SessionDisposalError::InvalidSession
    );
    Ok(plan)
}

pub(super) struct TargetSessionDisposalOutcome {
    event_plan: TargetEventPlan,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
}

impl TargetSessionDisposalOutcome {
    pub(super) fn into_parts(self) -> (TargetEventPlan, Option<moli_core::RendererOutputFence>) {
        (self.event_plan, self.renderer_output_predecessor)
    }
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

    pub(crate) fn owns_cleanup(&self) -> bool {
        matches!(self, Self::Own(_))
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
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
}

#[derive(Clone)]
pub(crate) struct SessionDisposalIdentity {
    plan: SessionDisposalPlan,
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
        conn.mark_session_closing(handlers.identity().plan());
        Self { handlers }
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
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
        let completion = identity.clone();
        if let Err(error) = driver
            .with_state(move |state| state.detach_disposed_session_inspector(&completion))
            .await?
        {
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
        Self {
            plan,
            binding,
            page,
        }
    }

    pub(crate) fn plan(&self) -> &SessionDisposalPlan {
        &self.plan
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
        driver: &mut ProtocolWorkflowDriver<'_>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
        error: anyhow::Error,
    ) -> Result<()> {
        if !matches!(self.plan.target(), SessionDisposalTarget::PageTarget { .. }) {
            return Err(error);
        }
        let preparation = self.clone();
        let mut retired = driver
            .with_state(move |state| state.prepare_failed_session_page_retirement(&preparation))
            .await?;
        if let Some(page) = retired.take_page() {
            let _ = page.close_async().await;
        }
        let completion = self.clone();
        background_events.extend(
            driver
                .with_state(move |state| {
                    state.finish_failed_session_page_retirement(&completion, retired)
                })
                .await?,
        );
        tracing::warn!(session_id = self.plan.session_id(), %error,
            "finished failed session cleanup after exact Page retirement");
        Ok(())
    }

    fn owns_current_page(&self, conn: &CdpConnection) -> bool {
        let owner = CommandOwnerScope::for_session(self.plan.session_id());
        self.binding_is_current(conn)
            && self.page.as_ref().is_some_and(|expected| {
                conn.renderer_page_residence_identity_for_owner(&owner) == Some(expected.residence)
                    && conn.current_renderer_agent_attachment_id_for_owner(&owner)
                        == expected.attachment
                    && conn.target_root_document_lifecycle_identity_for_owner(&owner)
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
        let owner = CommandOwnerScope::for_session(self.plan.session_id());
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
        if !self.binding_is_current(conn) {
            return Vec::new();
        }
        if let SessionDisposalTarget::PageTarget { session_key, .. } = self.plan.target() {
            // Retire only this session's metadata. Native removal belongs to
            // the original Page, never to a replacement installed during close.
            let _ = conn
                .with_target_owner_state_for_session_mut(Some(self.plan.session_id()), |state| {
                    state.remove_document_start_scripts_for_session(session_key)
                });
        }
        let owner = CommandOwnerScope::for_session(self.plan.session_id());
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

async fn dispose_live_session_domains_async(
    conn: &mut CdpConnection,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    plan: &SessionDisposalPlan,
) -> Result<Option<moli_core::RendererOutputFence>> {
    LiveSessionDisposal::prepare(conn, plan.clone())
        .run(
            &mut ProtocolWorkflowDriver::inline(conn),
            background_events,
            protocol_events,
        )
        .await
}

pub(super) async fn dispose_browser_session_without_event_async(
    conn: &mut CdpConnection,
    session_id: &str,
) -> Result<TargetEventPlan> {
    let plan = prepare_browser_session_disposal(conn, session_id)?;
    let mut discarded_background_events = Vec::new();
    let mut discarded_protocol_events = Vec::new();
    dispose_live_session_domains_async(
        conn,
        &mut discarded_background_events,
        &mut discarded_protocol_events,
        &plan,
    )
    .await?;
    conn.commit_browser_session_disposal_without_event(&plan)
}

/// Rolls back a prepared attachment after it has become capable of owning
/// renderer/domain state but before its attached event is published. No
/// protocol event is emitted because the frontend never observed the session.
pub(crate) async fn dispose_uncommitted_session_async(
    conn: &mut CdpConnection,
    plan: &SessionDisposalPlan,
) -> Result<()> {
    let mut discarded_background_events = Vec::new();
    let mut discarded_protocol_events = Vec::new();
    dispose_live_session_domains_async(
        conn,
        &mut discarded_background_events,
        &mut discarded_protocol_events,
        plan,
    )
    .await?;
    conn.commit_session_disposal(plan)
}

pub(super) async fn dispose_target_session_async(
    conn: &mut CdpConnection,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    cleanup_plan: TargetSessionDetachCleanupPlan,
) -> Result<TargetSessionDisposalOutcome> {
    let (plan, detachment) = prepare_session_disposal(conn, cleanup_plan)?;
    let renderer_output_predecessor =
        dispose_live_session_domains_async(conn, background_events, protocol_events, &plan).await?;

    conn.commit_session_disposal(&plan)?;
    let event_plan = conn.commit_target_session_detachment_event_plan(detachment);
    Ok(TargetSessionDisposalOutcome {
        event_plan,
        renderer_output_predecessor,
    })
}

pub(super) async fn dispose_primary_page_session_preserving_frontend_async(
    conn: &mut CdpConnection,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    session_id: &str,
) -> Result<Option<moli_core::RendererOutputFence>> {
    let route = conn
        .session_route(Some(session_id))
        .ok_or(SessionDisposalError::InvalidSession)?;
    let plan = SessionDisposalPlan::for_session_route(session_id, &route)
        .ok_or(SessionDisposalError::InvalidSession)?;
    if !matches!(
        plan.target(),
        SessionDisposalTarget::PageTarget {
            session_key: moli_page_types::DevToolsSessionKey::Primary,
            ..
        }
    ) {
        anyhow::bail!(SessionDisposalError::InvalidSession);
    }

    let predecessor =
        dispose_live_session_domains_async(conn, background_events, protocol_events, &plan).await?;
    if !conn.release_primary_target_session_binding_without_event(session_id) {
        anyhow::bail!(SessionDisposalError::InvalidSession);
    }
    Ok(predecessor)
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
    use crate::conn::{
        BrowserContext, CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand,
    };
    use crate::testing::{TestContext, wait_until_renderer_document_load};
    use base64::Engine as _;
    use serde_json::json;

    #[tokio::test]
    async fn cancelled_old_session_cleanup_cannot_retire_or_reconfigure_its_successor() {
        for retire_before_successor in [false, true] {
            let mut context = TestContext::new();
            let mut browser = BrowserContext::new("BID-retired-cleanup".into());
            browser.set_active_target_id("TID-retired-cleanup");
            browser.attach_active_session("SID-old");
            assert!(
                browser.assign_attached_session_to_target("TID-retired-cleanup", "SID-peer".into())
            );
            context
                .conn
                .install_browser_context_fixture_for_test(browser);
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
            let command = ParsedCdpCommand::parse_str(json!({"id": 10, "method": "Target.detachFromTarget", "params": {"sessionId": "SID-old"}}).to_string()).unwrap();
            let mut dispatch_context = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(pending) = context
                .conn
                .start_parsed_command_dispatch_with_context(&command, &mut dispatch_context)
            else {
                panic!("cleanup must retain its native work");
            };
            let mut pending = Some(pending);
            assert!(
                !context
                    .conn
                    .navigation_load_inputs_for_owner(&CommandOwnerScope::for_session("SID-peer"))
                    .script_execution_disabled,
                "Closing must revoke the old session's script policy"
            );
            let mut recovery = if retire_before_successor {
                drop(pending.take());
                Some(context.conn.complete_failed_command_wait_with_context(
                    Some(10),
                    None,
                    "controlled cancellation",
                    &mut dispatch_context,
                ))
            } else {
                None
            };
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
            let successor = context
                .conn
                .renderer_page_residence_identity_for_owner(&owner)
                .unwrap();
            let document = context
                .conn
                .target_root_document_lifecycle_identity_for_owner(&owner);
            context.process_async(json!({"id": 21, "sessionId": "SID-peer", "method": "Runtime.evaluate", "params": {"expression": "globalThis.successor", "returnByValue": true}})).await;
            let before_recovery = context.take_response_by_id(21);
            assert_eq!(
                before_recovery["result"]["result"]["value"],
                json!([true, "undefined"]),
                "before recovery retirement={retire_before_successor}: {before_recovery:?}"
            );
            // Exercise both sides of the retirement boundary: either A was
            // already taken before B's installation, or cancellation occurs
            // after B replaced A and may no longer retire the current Page.
            drop(pending.take());
            let recovery = recovery.take().unwrap_or_else(|| {
                context.conn.complete_failed_command_wait_with_context(
                    Some(10),
                    None,
                    "controlled cancellation",
                    &mut dispatch_context,
                )
            });
            let CdpCommandTaskStep::Pending(recovery) = recovery else {
                panic!("cancellation retains the original cleanup scope");
            };
            let completed = recovery.wait().await;
            let CdpCommandTaskStep::Complete(outcome) = context
                .conn
                .complete_pending_command_dispatch_with_context(completed, &mut dispatch_context)
                .await
            else {
                panic!("recovery has no further wait");
            };
            assert!(
                !outcome
                    .into_parts()
                    .0
                    .iter()
                    .any(|event| event["method"] == "Inspector.targetCrashed"
                        || event["method"] == "Target.targetCrashed")
            );
            assert_eq!(
                context
                    .conn
                    .renderer_page_residence_identity_for_owner(&owner),
                Some(successor)
            );
            assert_eq!(
                context
                    .conn
                    .target_root_document_lifecycle_identity_for_owner(&owner),
                document
            );
            assert!(!context.conn.has_attached_devtools_session("SID-old"));
            assert!(context.conn.has_attached_devtools_session("SID-peer"));
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
