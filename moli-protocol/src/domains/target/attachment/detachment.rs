use anyhow::Result;

use super::*;
use crate::conn::{
    CdpSessionRoute, CommandDispatchContext, ProtocolWorkflowDriver, SessionDisposalPlan,
    SessionDisposalTarget, TargetSessionDetachCleanupPlan,
};
use crate::domains::target::session_disposal::{
    LiveSessionDisposal, SessionDisposalError, SessionDisposalIdentity,
};

pub(in crate::domains::target) struct TargetDetachment {
    sessions: Vec<SessionDetachment>,
}

struct SessionDetachment {
    identity: SessionDisposalIdentity,
    disposal: SessionDisposalWork,
    commit: SessionDetachmentCommit,
    child: bool,
    response_before_event: bool,
}

enum SessionDisposalWork {
    Own(LiveSessionDisposal),
    Join(crate::conn::SessionDisposalWait),
}

mod recovery;
pub(crate) use recovery::{TargetDetachmentFailureCleanup, TargetDetachmentRecovery};

#[derive(Clone)]
enum SessionDetachmentCommit {
    Browser { emit_event: bool },
    Target(TargetSessionDetachCleanupPlan),
}

pub(in crate::domains::target) struct TargetDetachmentOutput {
    sessions: Vec<SessionDetachmentOutput>,
}

struct SessionDetachmentOutput {
    identity: SessionDisposalIdentity,
    commit: SessionDetachmentCommit,
    child: bool,
    response_before_event: bool,
    background_events: Vec<BackgroundProtocolEvent>,
    protocol_events: Vec<BackgroundProtocolEvent>,
    result: Result<Option<moli_core::RendererOutputFence>>,
}

impl TargetDetachment {
    pub(in crate::domains::target) fn failure_cleanup(&self) -> TargetDetachmentFailureCleanup {
        TargetDetachmentFailureCleanup::new(&self.sessions)
    }

    pub(super) fn prepare(
        conn: &mut CdpConnection,
        command_session_id: Option<&str>,
        params: DetachParams,
    ) -> Result<Self> {
        if let Some(session_id) = params.session_id.as_deref() {
            let route = conn
                .session_route(Some(session_id))
                .ok_or(SessionDisposalError::InvalidSession)?;
            anyhow::ensure!(
                !conn.session_is_closing(session_id),
                SessionDisposalError::Closing
            );
            if let Some(requested) = params.target_id.as_deref() {
                anyhow::ensure!(
                    route_target_id(&route).is_none_or(|target| target == requested),
                    SessionDisposalError::UnknownTarget
                );
            }
        }
        if let Some(target_id) = params.target_id.as_deref() {
            select_browser_context_for_target(conn, target_id)?;
        }

        // Resolve the complete cascade before mutating any session. The active
        // context is restored by the caller before this owned work can wait.
        let mut declarations = Vec::new();
        if let Some(session_id) = params.session_id.as_deref() {
            for child in conn.attached_session_cascade_for_owner(Some(session_id)) {
                if let Some(route) = conn.session_route(Some(&child)) {
                    declarations.push((child, route, true, false));
                }
            }
            let route = conn
                .session_route(Some(session_id))
                .ok_or(SessionDisposalError::InvalidSession)?;
            declarations.push((session_id.to_owned(), route, false, true));
        } else if let Some(context) = conn.browser_context.as_ref() {
            let workers = params.target_id.as_deref().and_then(|target| {
                context
                    .shared_worker_target(target)
                    .map(|worker| worker.session_ids())
                    .or_else(|| {
                        context
                            .dedicated_worker_target(target)
                            .map(|worker| worker.session_ids())
                    })
                    .or_else(|| {
                        context
                            .service_worker_target(target)
                            .map(|worker| worker.session_ids())
                    })
            });
            if let Some(sessions) = workers {
                for session in sessions {
                    let route = conn
                        .session_route(Some(&session))
                        .ok_or(SessionDisposalError::InvalidSession)?;
                    declarations.push((session, route, false, false));
                }
            } else if let Some((target_id, Some(session_id))) = context.active_target_identity() {
                anyhow::ensure!(
                    params
                        .target_id
                        .as_deref()
                        .is_none_or(|target| target == target_id),
                    SessionDisposalError::UnknownTarget
                );
                let route = conn
                    .session_route(Some(&session_id))
                    .ok_or(SessionDisposalError::InvalidSession)?;
                declarations.push((session_id, route, false, true));
            }
        }
        let mut plans = Vec::with_capacity(declarations.len());
        for (session, route, child, response_before_event) in declarations {
            anyhow::ensure!(
                child || !conn.session_is_closing(&session),
                SessionDisposalError::Closing
            );
            let plan = SessionDisposalPlan::for_session_route(&session, &route)
                .ok_or(SessionDisposalError::InvalidSession)?;
            let response_before_event =
                response_before_event && !matches!(plan.target(), SessionDisposalTarget::Browser);
            let commit = if matches!(plan.target(), SessionDisposalTarget::Browser) {
                SessionDetachmentCommit::Browser { emit_event: !child }
            } else {
                SessionDetachmentCommit::Target(TargetSessionDetachCleanupPlan::new(
                    plan.target_id()
                        .ok_or(SessionDisposalError::InvalidSession)?,
                    &session,
                    None,
                    (!child && params.session_id.is_some())
                        .then_some(command_session_id)
                        .flatten(),
                ))
            };
            plans.push((plan, commit, child, response_before_event));
        }
        let sessions = plans
            .into_iter()
            .map(|(plan, commit, child, response_before_event)| {
                let (disposal, identity) = match conn.session_disposal_wait(plan.session_id()) {
                    Some(wait) => (
                        SessionDisposalWork::Join(wait),
                        SessionDisposalIdentity::capture(conn, plan),
                    ),
                    None => {
                        let disposal = LiveSessionDisposal::prepare(conn, plan);
                        let identity = disposal.identity();
                        (SessionDisposalWork::Own(disposal), identity)
                    }
                };
                SessionDetachment {
                    identity,
                    disposal,
                    commit,
                    child,
                    response_before_event,
                }
            })
            .collect();
        Ok(Self { sessions })
    }

    pub(in crate::domains::target) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
    ) -> TargetDetachmentOutput {
        let mut sessions = Vec::with_capacity(self.sessions.len());
        for session in self.sessions {
            let mut background_events = Vec::new();
            let mut protocol_events = Vec::new();
            let result = match session.disposal {
                SessionDisposalWork::Own(disposal) => {
                    disposal
                        .run(driver, &mut background_events, &mut protocol_events)
                        .await
                }
                SessionDisposalWork::Join(wait) => {
                    wait.wait().await;
                    Ok(None)
                }
            };
            sessions.push(SessionDetachmentOutput {
                identity: session.identity,
                commit: session.commit,
                child: session.child,
                response_before_event: session.response_before_event,
                background_events,
                protocol_events,
                result,
            });
        }
        TargetDetachmentOutput { sessions }
    }
}

impl TargetDetachmentOutput {
    pub(in crate::domains::target) fn finish(
        self,
        conn: &mut CdpConnection,
        command_context: &mut CommandDispatchContext,
    ) -> CommandOutputPlan {
        let mut output = TargetCommandOutput::default();
        let mut replied = false;
        for session in self.sessions {
            command_context
                .protocol_events_mut()
                .extend(session.protocol_events);
            output
                .background_events_mut()
                .extend(session.background_events);
            let cleanup_error = match session.result {
                Ok(predecessor) => {
                    if let Some(predecessor) = predecessor {
                        command_context.set_renderer_output_predecessor(predecessor);
                    }
                    None
                }
                Err(error) => Some(error),
            };
            let (events, commit_error) =
                commit_session_detachment(conn, &session.identity, session.commit);
            let result = match cleanup_error.or(commit_error) {
                Some(error) => {
                    output.target_events_mut().extend_background_events(events);
                    Err(error)
                }
                None => Ok(events),
            };
            match result {
                Ok(events) => {
                    if !session.child && session.response_before_event && !replied {
                        output.push_success();
                        replied = true;
                    }
                    output.target_events_mut().extend_background_events(events);
                }
                Err(error) if session.child => {
                    tracing::warn!(session_id = session.identity.plan().session_id(), %error,
                        "failed to dispose child session");
                }
                Err(error) => {
                    if !replied {
                        let code = if matches!(
                            session.identity.plan().target(),
                            SessionDisposalTarget::Browser
                        ) {
                            -31998
                        } else {
                            -32000
                        };
                        output.push_error(code, error);
                        replied = true;
                    }
                }
            }
        }
        if !replied {
            output.push_success();
        }
        output.into_plan()
    }
}

fn commit_session_detachment(
    conn: &mut CdpConnection,
    identity: &SessionDisposalIdentity,
    commit: SessionDetachmentCommit,
) -> (crate::conn::TargetEventPlan, Option<anyhow::Error>) {
    let plan = identity.plan();
    if !identity.is_current(conn) {
        return (Default::default(), None);
    }
    let result = match commit {
        SessionDetachmentCommit::Browser { emit_event: true } => {
            conn.commit_browser_session_disposal_event_plan(plan)
        }
        SessionDetachmentCommit::Browser { emit_event: false } => {
            conn.commit_browser_session_disposal_without_event(plan)
        }
        SessionDetachmentCommit::Target(detachment) => {
            let error = conn.commit_session_disposal(plan).err();
            let events = conn.commit_target_session_detachment_event_plan(detachment);
            return (events, error);
        }
    };
    match result {
        Ok(events) => (events, None),
        Err(error) => (Default::default(), Some(error)),
    }
}

fn route_target_id(route: &CdpSessionRoute) -> Option<&str> {
    match route {
        CdpSessionRoute::Browser | CdpSessionRoute::BrowserContext { .. } => None,
        CdpSessionRoute::TabTarget { tab_target_id, .. } => Some(tab_target_id),
        CdpSessionRoute::PageTarget { target_id, .. }
        | CdpSessionRoute::SharedWorkerTarget { target_id, .. }
        | CdpSessionRoute::DedicatedWorkerTarget { target_id, .. }
        | CdpSessionRoute::ServiceWorkerTarget { target_id, .. } => Some(target_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::BrowserContext;
    use crate::testing::TestContext;

    #[tokio::test]
    async fn rebound_session_keeps_new_scripts_and_binding_after_old_cleanup_or_recovery() {
        for cancelled in [false, true] {
            let mut context = TestContext::new();
            let mut browser = BrowserContext::new("BID-rebound-disposal".into());
            browser.set_active_target_id("TID-rebound-disposal");
            browser.attach_active_session("SID-rebound-disposal");
            context
                .conn
                .install_browser_context_fixture_for_test(browser);
            context
                .install_navigation_fixture_for_session_owner(
                    "about:blank",
                    Some("SID-rebound-disposal"),
                )
                .await;
            context
                .process_async(json!({"id":1,"sessionId":"SID-rebound-disposal",
                "method":"Page.addScriptToEvaluateOnNewDocument",
                "params":{"source":"globalThis.originalBinding = true;"}}))
                .await;
            assert!(context.take_response_by_id(1).get("result").is_some());
            let prepared = TargetDetachment::prepare(
                &mut context.conn,
                None,
                DetachParams {
                    session_id: Some("SID-rebound-disposal".into()),
                    target_id: None,
                },
            )
            .unwrap();
            let original = prepared.sessions[0].identity.clone();
            let recovery = cancelled.then(|| prepared.failure_cleanup());
            let route = context
                .conn
                .session_route(Some("SID-rebound-disposal"))
                .unwrap();
            let page = context.conn.renderer_page_residence_identity_for_owner(
                &CommandOwnerScope::for_session("SID-rebound-disposal"),
            );
            // Preserve every literal id, route and Page identity while replacing
            // only the committed binding lifetime. Model its fresh agent state.
            context
                .conn
                .register_session_route_for_test("SID-rebound-disposal", route.clone());
            context
                .conn
                .with_target_devtools_session_state_for_owner_mut(
                    &CommandOwnerScope::for_session("SID-rebound-disposal"),
                    |state| state.closing = false,
                )
                .unwrap();
            let successor = context
                .conn
                .session_binding_lifetime("SID-rebound-disposal")
                .unwrap();
            assert!(!original.binding_is_current(&context.conn));
            context
                .process_async(json!({"id":2,"sessionId":"SID-rebound-disposal",
                "method":"Page.addScriptToEvaluateOnNewDocument",
                "params":{"source":"globalThis.successorBinding = true;"}}))
                .await;
            let added = context.take_response_by_id(2);
            let identifier = added["result"]["identifier"].as_str().unwrap().to_owned();
            if let Some(recovery) = recovery {
                drop(prepared);
                recovery
                    .prepare(
                        &mut context.conn,
                        &mut CommandDispatchContext::default(),
                        CommandOutputPlan::default(),
                    )
                    .wait()
                    .await
                    .finish(&mut context.conn);
            } else {
                let output = prepared
                    .run(&mut ProtocolWorkflowDriver::inline(&mut context.conn))
                    .await;
                output.finish(&mut context.conn, &mut CommandDispatchContext::default());
            }
            assert_eq!(
                context.conn.session_route(Some("SID-rebound-disposal")),
                Some(route)
            );
            assert_eq!(
                context
                    .conn
                    .session_binding_lifetime("SID-rebound-disposal"),
                Some(successor)
            );
            assert_eq!(
                context.conn.renderer_page_residence_identity_for_owner(
                    &CommandOwnerScope::for_session("SID-rebound-disposal")
                ),
                page
            );
            assert!(
                context
                    .conn
                    .target_owner_state_for_session(Some("SID-rebound-disposal"))
                    .unwrap()
                    .document_start_scripts
                    .iter()
                    .any(|(id, _)| id == &identifier),
                "old cleanup cannot remove the successor's script (cancelled: {cancelled})"
            );
            context
                .process_async(json!({"id":3,"sessionId":"SID-rebound-disposal",
                "method":"Runtime.evaluate","params":{"expression":"6 * 7","returnByValue":true}}))
                .await;
            let evaluated = context.take_response_by_id(3);
            assert_eq!(
                evaluated["result"]["result"]["value"], 42,
                "old cleanup cannot detach the successor Inspector: {evaluated:?}"
            );
        }
    }
}
