use anyhow::{Context as _, Result};

use super::*;
use crate::conn::{
    CdpSessionRoute, CommandDispatchContext, ProtocolWorkflowDriver, SessionDisposalPlan,
    SessionDisposalTarget, TargetSessionDetachCleanupPlan,
};
use crate::domains::target::session_disposal::LiveSessionDisposal;

pub(in crate::domains::target) struct TargetDetachment {
    sessions: Vec<SessionDetachment>,
}

struct SessionDetachment {
    plan: SessionDisposalPlan,
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
    plan: SessionDisposalPlan,
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
                .context("InvalidSessionId")?;
            anyhow::ensure!(!conn.session_is_closing(session_id), "Session is closing");
            if let Some(requested) = params.target_id.as_deref() {
                anyhow::ensure!(
                    route_target_id(&route).is_none_or(|target| target == requested),
                    "UnknownTargetId"
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
                .context("InvalidSessionId")?;
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
                        .context("InvalidSessionId")?;
                    declarations.push((session, route, false, false));
                }
            } else if let Some((target_id, Some(session_id))) = context.active_target_identity() {
                anyhow::ensure!(
                    params
                        .target_id
                        .as_deref()
                        .is_none_or(|target| target == target_id),
                    "UnknownTargetId"
                );
                let route = conn
                    .session_route(Some(&session_id))
                    .context("InvalidSessionId")?;
                declarations.push((session_id, route, false, true));
            }
        }
        let mut plans = Vec::with_capacity(declarations.len());
        for (session, route, child, response_before_event) in declarations {
            anyhow::ensure!(
                child || !conn.session_is_closing(&session),
                "Session is closing"
            );
            let plan = SessionDisposalPlan::for_session_route(&session, &route)
                .context("InvalidSessionId")?;
            let response_before_event =
                response_before_event && !matches!(plan.target(), SessionDisposalTarget::Browser);
            let commit = if matches!(plan.target(), SessionDisposalTarget::Browser) {
                SessionDetachmentCommit::Browser { emit_event: !child }
            } else {
                SessionDetachmentCommit::Target(TargetSessionDetachCleanupPlan::new(
                    plan.target_id().context("InvalidSessionId")?,
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
                let disposal = match conn.session_disposal_wait(plan.session_id()) {
                    Some(wait) => SessionDisposalWork::Join(wait),
                    None => {
                        SessionDisposalWork::Own(LiveSessionDisposal::prepare(conn, plan.clone()))
                    }
                };
                SessionDetachment {
                    plan,
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
                plan: session.plan,
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
                commit_session_detachment(conn, &session.plan, session.commit);
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
                    tracing::warn!(session_id = session.plan.session_id(), %error,
                        "failed to dispose child session");
                }
                Err(error) => {
                    if !replied {
                        let code =
                            if matches!(session.plan.target(), SessionDisposalTarget::Browser) {
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
    plan: &SessionDisposalPlan,
    commit: SessionDetachmentCommit,
) -> (crate::conn::TargetEventPlan, Option<anyhow::Error>) {
    let current = conn
        .session_route(Some(plan.session_id()))
        .and_then(|route| SessionDisposalPlan::for_session_route(plan.session_id(), &route));
    if current.as_ref() != Some(plan) {
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
