use anyhow::Result;

use super::*;
use crate::conn::{
    CdpSessionRoute, ProtocolWorkflowDriver, SessionDisposalPlan, SessionDisposalTarget,
    TargetSessionDetachCleanupPlan,
};
use crate::domains::target::session_disposal::{
    PreparedSessionDisposal, SessionDisposalError, SessionDisposalIdentity,
};

pub(crate) struct TargetDetachment {
    sessions: Vec<SessionDetachment>,
}

struct SessionDetachment {
    identity: SessionDisposalIdentity,
    disposal: PreparedSessionDisposal,
    commit: SessionDetachmentCommit,
    child: bool,
    response_before_event: bool,
}

enum SessionDetachmentCommit {
    Browser { emit_event: bool },
    Target(TargetSessionDetachCleanupPlan),
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

    pub(crate) fn commit_in_browser(self, conn: &mut CdpConnection) -> CommandOutputPlan {
        let mut native = Vec::with_capacity(self.sessions.len());
        let mut prefix = CommandOutputPlan::default();
        let mut output = CommandOutputPlan::default();
        let mut replied = false;
        for mut session in self.sessions {
            prefix.extend_background_events(session.disposal.take_browser_events());
            let (events, error) =
                commit_session_detachment(conn, &session.identity, session.commit);
            if let Some(error) = error {
                output.extend_background_events(events);
                if session.child {
                    tracing::warn!(session_id = session.identity.plan().session_id(), %error,
                        "failed to dispose child session");
                } else if !replied {
                    let code = if matches!(
                        session.identity.plan().target(),
                        SessionDisposalTarget::Browser
                    ) {
                        -31998
                    } else {
                        -32000
                    };
                    output.push_error_without_session(code, format!("{error:#}"));
                    replied = true;
                }
            } else {
                if !session.child && session.response_before_event && !replied {
                    output.push_success();
                    replied = true;
                }
                output.extend_background_events(events);
            }
            native.push((session.identity, session.disposal));
        }
        if !replied {
            output.push_success();
        }
        prefix.extend(output);
        // Each retirement owns its native barrier. A busy target must not
        // delay another target's finalizer or its successor session.
        for (identity, disposal) in native {
            let access = conn.protocol_state_access();
            let workflow = crate::conn::PreparedCommandWorkflow::new(move |context| async move {
                let mut driver = ProtocolWorkflowDriver::new(access);
                let mut output = CommandOutputPlan::default();
                let mut background = Vec::new();
                let mut protocol = Vec::new();
                if let Err(error) = disposal
                    .run(&mut driver, &mut background, &mut protocol)
                    .await
                {
                    tracing::warn!(session_id = identity.plan().session_id(), %error, "retired session native cleanup failed");
                }
                output.extend_background_events(background);
                output.extend_background_events(protocol);
                context.complete(output)
            });
            crate::conn::PendingProtocolSchedulerWork::publish_workflow(conn, workflow, None);
        }
        prefix
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
    async fn browser_commit_preserves_cascade_order_and_supervises_native_cleanup() {
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
        let (output, work) = context.with_fixture(|conn| {
            let sessions = [("TID-first", "SID-first"), ("TID-second", "SID-second")]
                .into_iter()
                .map(|(target, session)| {
                    let route = conn.session_route(Some(session)).unwrap();
                    let plan = SessionDisposalPlan::for_session_route(session, &route).unwrap();
                    let disposal = PreparedSessionDisposal::prepare(conn, plan);
                    SessionDetachment {
                        identity: disposal.identity(),
                        disposal,
                        commit: SessionDetachmentCommit::Target(
                            TargetSessionDetachCleanupPlan::new(target, session, None, None),
                        ),
                        child: true,
                        response_before_event: false,
                    }
                })
                .collect();
            let output = TargetDetachment { sessions }.commit_in_browser(conn);
            for session in ["SID-first", "SID-second"] {
                assert!(conn.session_route(Some(session)).is_none());
            }
            let work = conn.take_scheduler_events();
            assert_eq!(
                work.iter()
                    .filter(|event| matches!(
                        event,
                        crate::conn::CdpSchedulerEvent::ProtocolTaskPrepared { .. }
                    ))
                    .count(),
                2,
                "each session has independent supervised native work"
            );
            (output, work)
        });
        let messages = output
            .into_background_events(Some(91), Some("SID-browser-command"))
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 3, "{messages:?}");
        for (message, session) in messages.iter().zip(["SID-first", "SID-second"]) {
            assert_eq!(message["method"], "Target.detachedFromTarget");
            assert_eq!(message["params"]["sessionId"], session);
        }
        assert_eq!(
            messages[2],
            json!({"id":91,"sessionId":"SID-browser-command","result":{}})
        );

        context
            .route_direct_command_output_for_test(Vec::new(), work)
            .await;
        context
            .wait_for_direct_command_work_completion_for_test("cascade native cleanup")
            .await;
        assert!(
            context.take_all().is_empty(),
            "native cleanup cannot publish a second Browser response"
        );
    }

    #[tokio::test]
    async fn rebound_session_keeps_new_scripts_and_binding_after_old_cleanup() {
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
        let (prepared, route, page, successor) = state_owner
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
                    (prepared, route, page, successor)
                })
            })
            .unwrap();
        let (events, work) = context.with_fixture(|connection| {
            let output = prepared.commit_in_browser(connection);
            (
                output.into_background_events(None, None),
                connection.take_scheduler_events(),
            )
        });
        context
            .route_direct_command_output_for_test(events, work)
            .await;
        context
            .process_and_wait_for_response_async(json!({
                "id":2,"sessionId":"SID-rebound-disposal",
                "method":"Page.addScriptToEvaluateOnNewDocument",
                "params":{"source":"globalThis.successorBinding = true;"}
            }))
            .await;
        context
            .wait_for_direct_command_work_completion_for_test("old binding native cleanup")
            .await;
        let added = context.take_response_by_id(2);
        let identifier = added["result"]["identifier"].as_str().unwrap().to_owned();
        state_owner
            .with_state(|state| {
                state.fixture(|connection| {
                    assert_eq!(
                        connection.session_route(Some("SID-rebound-disposal")),
                        Some(route)
                    );
                    assert_eq!(
                        connection.session_binding_lifetime("SID-rebound-disposal"),
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
                        "old cleanup cannot remove the successor's script"
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
