use anyhow::Result;

use super::*;
use crate::conn::{
    CdpSessionRoute, CommandDispatchContext, ProtocolWorkflowDriver, SessionDisposalPlan,
    SessionDisposalTarget, TargetSessionDetachCleanupPlan,
};
use crate::domains::target::session_disposal::{
    PreparedSessionDisposal, SessionDisposalError, SessionDisposalIdentity,
};

pub(in crate::domains::target) struct TargetDetachment {
    sessions: Vec<SessionDetachment>,
}

struct SessionDetachment {
    identity: SessionDisposalIdentity,
    disposal: PreparedSessionDisposal,
    commit: SessionDetachmentCommit,
    child: bool,
    response_before_event: bool,
}

mod recovery;
pub(crate) use recovery::{TargetDetachmentFailureCleanup, TargetDetachmentRecovery};

#[derive(Clone)]
enum SessionDetachmentCommit {
    Browser { emit_event: bool },
    Target(TargetSessionDetachCleanupPlan),
}

pub(crate) struct TargetDetachmentOutput {
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
    pub(in crate::domains::target) fn prepare_auto_attached_sessions(
        conn: &mut CdpConnection,
        owner_session_id: Option<&str>,
    ) -> Self {
        let declarations = conn.auto_attached_session_cascade_for_owner(owner_session_id);
        let sessions = declarations
            .into_iter()
            .filter_map(|session| {
                let route = conn.session_route(Some(&session))?;
                let plan = SessionDisposalPlan::for_session_route(&session, &route)?;
                let commit = match plan.target() {
                    SessionDisposalTarget::Browser => {
                        SessionDetachmentCommit::Browser { emit_event: false }
                    }
                    _ => SessionDetachmentCommit::Target(TargetSessionDetachCleanupPlan::new(
                        plan.target_id()?,
                        &session,
                        None,
                        None,
                    )),
                };
                let disposal = PreparedSessionDisposal::prepare(conn, plan);
                Some(SessionDetachment {
                    identity: disposal.identity(),
                    disposal,
                    commit,
                    child: true,
                    response_before_event: false,
                })
            })
            .collect();
        Self { sessions }
    }

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
                let disposal = PreparedSessionDisposal::prepare(conn, plan);
                let identity = disposal.identity();
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
        driver: &mut ProtocolWorkflowDriver,
    ) -> TargetDetachmentOutput {
        let mut sessions = Vec::with_capacity(self.sessions.len());
        for session in self.sessions {
            let mut background_events = Vec::new();
            let mut protocol_events = Vec::new();
            let result = session
                .disposal
                .run(driver, &mut background_events, &mut protocol_events)
                .await;
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
    pub(crate) fn finish(
        self,
        conn: &mut CdpConnection,
        command_context: &mut CommandDispatchContext,
        command_id: Option<u64>,
        response_session_id: Option<&str>,
    ) -> CommandOutputPlan {
        let mut output = TargetCommandOutput::default();
        let mut predecessors = Vec::new();
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
                    predecessors.extend(predecessor);
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
        let output = output.into_plan();
        predecessors.extend(command_context.take_renderer_output_predecessor());
        let Some(first) = predecessors.first() else {
            return output;
        };
        if predecessors
            .iter()
            .all(|fence| fence.cursor().stream() == first.cursor().stream())
        {
            for predecessor in predecessors {
                command_context.set_renderer_output_predecessor(predecessor);
            }
            return output;
        }

        // A browser/tab detach cascade can disable Fetch on multiple Pages.
        // Keep their exact FIFO dependencies independent: one Page's tail
        // cannot stand in for another's. Publish the terminals, detach events
        // and original response together only after every prefix is projected.
        let mut events = command_context.take_protocol_events();
        events.extend(output.into_background_events(command_id, response_session_id));
        conn.publish_renderer_fenced_protocol_output_after(events, predecessors, None);
        CommandOutputPlan::default()
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
    use crate::conn::{BrowserContext, RendererPageResidenceIdentity};
    use crate::testing::TestContext;

    #[tokio::test]
    async fn multi_page_detachment_waits_for_every_original_output_prefix() {
        use moli_core::{
            PageId, RendererOutputCursor, RendererOutputFence, RendererOutputItem,
            RendererOutputPublication, RendererOutputRecord, RendererOutputStreamControl,
            RendererOutputStreamIdentity, RendererOutputTransportMessage,
            RendererProtocolObservation,
        };

        let mut browser = BrowserContext::new("BID-detach-prefixes".into());
        browser.set_active_target_id("TID-first");
        browser.attach_active_session("SID-first");
        browser.stage_background_target(
            "TID-second".into(),
            Some("SID-second".into()),
            "about:blank".into(),
            None,
            None,
        );
        let mut context = TestContext::new();
        context.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        let prefixes = [
            ("TID-first", "SID-first", 71, 3),
            ("TID-second", "SID-second", 72, 1),
        ]
        .map(|(target, session, page, sequence)| {
            let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(
                PageId::new_for_testing(page),
            );
            context.with_fixture(|conn| {
                let residence =
                    RendererPageResidenceIdentity::from_residence(stream.residence()).unwrap();
                conn.bind_renderer_page_output_owner(
                    residence,
                    crate::conn::TargetPageResidenceIdentity::new_for_test(
                        "BID-detach-prefixes".into(),
                        Some(target.into()),
                        page,
                    ),
                );
                conn.apply_renderer_output_stream_control(RendererOutputStreamControl::Opened {
                    stream,
                });
            });
            (target, session, stream, sequence)
        });
        let work = context.with_fixture(|conn| {
            let sessions = prefixes
                .iter()
                .map(|&(target, session, stream, sequence)| {
                    let route = conn.session_route(Some(session)).unwrap();
                    let plan = SessionDisposalPlan::for_session_route(session, &route).unwrap();
                    let identity = SessionDisposalIdentity::capture(conn, plan.clone());
                    conn.mark_session_closing(&plan);
                    SessionDetachmentOutput {
                        identity,
                        commit: SessionDetachmentCommit::Target(
                            TargetSessionDetachCleanupPlan::new(target, session, None, None),
                        ),
                        child: true,
                        response_before_event: false,
                        background_events: Vec::new(),
                        protocol_events: Vec::new(),
                        result: Ok(Some(RendererOutputFence::new_for_test(
                            RendererOutputCursor::new_for_test(stream, sequence),
                        ))),
                    }
                })
                .collect();
            let mut command_context = CommandDispatchContext::default();
            let output = TargetDetachmentOutput { sessions }.finish(
                conn,
                &mut command_context,
                Some(91),
                Some("SID-browser-command"),
            );
            assert!(
                output.is_empty(),
                "the response must retain both dependencies"
            );
            assert!(command_context.take_protocol_events().is_empty());
            assert!(command_context.take_renderer_output_predecessor().is_none());
            conn.take_scheduler_events()
                .into_iter()
                .find_map(|event| match event {
                    crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } => Some(work),
                    _ => None,
                })
                .expect("one durable output owns the original command response")
        });
        assert_eq!(work.renderer_output_predecessors().len(), 2);
        for (index, &(_, _, stream, sequence)) in prefixes.iter().enumerate() {
            context
                .with_fixture(|conn| assert!(!work.renderer_output_predecessor_is_projected(conn)));
            for sequence in 1..=sequence {
                let outcome = context
                    .handle()
                    .with_state(|state| {
                        state.admit_renderer_output(RendererOutputTransportMessage::Publication(
                            RendererOutputPublication::new_for_test(
                                RendererOutputCursor::new_for_test(stream, sequence),
                                vec![RendererOutputRecord::new_for_test(
                                    RendererOutputItem::Observation(
                                        RendererProtocolObservation::RuntimeLifecycleError {
                                            text: "historical renderer output".into(),
                                            execution_context_id: None,
                                        },
                                    ),
                                )],
                            ),
                        ))
                    })
                    .unwrap();
                let (events, scheduler) = outcome.into_protocol_event_parts();
                assert!(events.is_empty());
                for event in scheduler {
                    let crate::conn::CdpSchedulerEvent::RendererOutputProjectionPrepared {
                        pending,
                    } = event
                    else {
                        panic!("expected the exact renderer projection");
                    };
                    let completed = context.drive_owned_wait(pending.wait()).await;
                    let outcome = context
                        .handle()
                        .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
                        .unwrap();
                    assert!(outcome.into_protocol_event_parts().0.is_empty());
                }
            }
            context.with_fixture(|conn| {
                assert_eq!(
                    work.renderer_output_predecessor_is_projected(conn),
                    index == 1
                );
            });
        }
        let events = context
            .run_fixture_scheduler_work(work)
            .await
            .into_protocol_event_parts()
            .0
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        assert_eq!(
            events.len(),
            3,
            "each detach and the response appear once: {events:?}"
        );
        assert!(
            events[..2]
                .iter()
                .all(|event| event["method"] == "Target.detachedFromTarget")
        );
        assert_eq!(
            events[2],
            json!({"id": 91, "sessionId": "SID-browser-command", "result": {}})
        );
    }

    #[tokio::test]
    async fn rebound_session_keeps_new_scripts_and_binding_after_old_cleanup_or_recovery() {
        for cancelled in [false, true] {
            let mut context = TestContext::new();
            let mut browser = BrowserContext::new("BID-rebound-disposal".into());
            browser.set_active_target_id("TID-rebound-disposal");
            browser.attach_active_session("SID-rebound-disposal");
            context.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
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
            let state_owner = context.handle();
            let (prepared, recovery, route, page, successor) = state_owner
                .with_state(|state| {
                    state.fixture(|connection| {
                        let prepared = TargetDetachment::prepare(
                            connection,
                            None,
                            DetachParams {
                                session_id: Some("SID-rebound-disposal".into()),
                                target_id: None,
                            },
                        )
                        .unwrap();
                        let original = prepared.sessions[0].identity.clone();
                        let recovery = cancelled.then(|| prepared.failure_cleanup());
                        let route = connection
                            .session_route(Some("SID-rebound-disposal"))
                            .unwrap();
                        let page = connection.renderer_page_residence_identity_for_owner(
                            &CommandOwnerScope::for_session("SID-rebound-disposal"),
                        );
                        // Reuse the route and Page while replacing only the binding lifetime.
                        connection
                            .with_target_devtools_session_state_for_owner_mut(
                                &CommandOwnerScope::for_session("SID-rebound-disposal"),
                                |state| state.lifecycle = Default::default(),
                            )
                            .unwrap();
                        connection
                            .register_session_route_for_test("SID-rebound-disposal", route.clone());
                        let successor = connection
                            .session_binding_lifetime("SID-rebound-disposal")
                            .unwrap();
                        assert!(!original.binding_is_current(connection));
                        (prepared, recovery, route, page, successor)
                    })
                })
                .unwrap();
            context
                .process_and_wait_for_response_async(json!({
                    "id":2,"sessionId":"SID-rebound-disposal",
                    "method":"Page.addScriptToEvaluateOnNewDocument",
                    "params":{"source":"globalThis.successorBinding = true;"}
                }))
                .await;
            let added = context.take_response_by_id(2);
            let identifier = added["result"]["identifier"].as_str().unwrap().to_owned();
            if let Some(recovery) = recovery {
                drop(prepared);
                let pending = state_owner
                    .with_state(|state| {
                        state.fixture(|connection| {
                            recovery.prepare(
                                connection,
                                &mut CommandDispatchContext::default(),
                                CommandOutputPlan::default(),
                            )
                        })
                    })
                    .unwrap();
                let completed = pending.wait().await;
                state_owner
                    .with_state(|state| state.fixture(|connection| completed.finish(connection)))
                    .unwrap();
            } else {
                let output = prepared
                    .run(&mut ProtocolWorkflowDriver::new(state_owner.clone()))
                    .await;
                state_owner
                    .with_state(|state| {
                        state.fixture(|connection| {
                            output.finish(
                                connection,
                                &mut CommandDispatchContext::default(),
                                None,
                                None,
                            )
                        })
                    })
                    .unwrap();
            }
            state_owner
                .with_state(|state| {
                    state.fixture(|connection| {
            assert_eq!(
                connection.session_route(Some("SID-rebound-disposal")),
                Some(route)
            );
            assert_eq!(
                connection
                    .session_binding_lifetime("SID-rebound-disposal"),
                Some(successor)
            );
            assert_eq!(
                connection.renderer_page_residence_identity_for_owner(
                    &CommandOwnerScope::for_session("SID-rebound-disposal")
                ),
                page
            );
            assert!(
                connection
                    .target_owner_state_for_session(Some("SID-rebound-disposal"))
                    .unwrap()
                    .document_start_scripts
                    .iter()
                    .any(|(id, _)| id == &identifier),
                "old cleanup cannot remove the successor's script (cancelled: {cancelled})"
            );
            })
                })
                .unwrap();
            context
                .process_and_wait_for_response_async(json!({
                    "id":3,"sessionId":"SID-rebound-disposal",
                    "method":"Runtime.evaluate","params":{"expression":"6 * 7","returnByValue":true}
                }))
                .await;
            let evaluated = context.take_response_by_id(3);
            assert_eq!(
                evaluated["result"]["result"]["value"], 42,
                "old cleanup cannot detach the successor Inspector: {evaluated:?}"
            );
        }
    }
}
