use super::*;
use crate::conn::{
    CapturedBody, CdpCommandTaskStep, CommandOwnerScope, DocumentNavigationToken,
    NavigationLoadOutcome, ProtocolStateRequestReceiver, ResponseCommitReady,
};
use crate::domains::page::BackgroundNavigationCompletion;
use serde_json::Value;
use tokio::sync::mpsc;

struct CommitFixture {
    ctx: TestContext,
    token: DocumentNavigationToken,
    navigation: NavigationDispatchState,
    prepared: ResponseCommitReady,
    requests: ProtocolStateRequestReceiver,
    completions: mpsc::UnboundedReceiver<BackgroundNavigationCompletion>,
}

async fn commit_fixture(replace_existing: bool) -> CommitFixture {
    let (mut ctx, mut token, mut navigation) = super::navigation_error::navigation_fixture();
    if replace_existing {
        ctx.conn
            .finish_renderer_document_navigation_for_owner(&navigation.owner, &token)
            .unwrap();
        ctx.conn
            .clear_pending_document_navigation_for_owner_if_loader_matches(
                &navigation.owner,
                &token.loader_id,
            );
        ctx.install_navigation_fixture_for_session_owner("about:blank", Some("SID-1"))
            .await;
        token = ctx
            .conn
            .start_document_navigation_for_owner(&navigation.owner, navigation.loader_id.clone())
            .unwrap();
    }
    navigation.requested_url = Url::parse("https://example.test/prepared").unwrap();
    ctx.conn
        .runtime_session_owner_slot_mut_for_owner(&navigation.owner)
        .unwrap()
        .claim_background_navigation_completion(&token, None)
        .unwrap();
    let prepared = ctx
        .conn
        .build_navigation_from_buffered_body_source_for_navigation_async(
            Some(&token),
            &navigation,
            navigation.requested_url.clone(),
            200,
            vec![("Content-Type".into(), b"text/html".to_vec())],
            CapturedBody::from_string("<title>prepared</title><p>first</p>".into()),
            None,
            Default::default(),
            Default::default(),
        )
        .await
        .unwrap();
    let NavigationLoadOutcome::ResponseCommitReady(prepared) = prepared else {
        panic!("fixture must stop before the renderer commit");
    };
    assert_eq!(prepared.stable_page_target().is_some(), replace_existing);
    let requests = ctx.conn.take_protocol_state_requests().unwrap();
    let (sender, completions) = mpsc::unbounded_channel();
    ctx.conn.set_background_navigation_completion_sender(sender);
    CommitFixture {
        ctx,
        token,
        navigation,
        prepared: *prepared,
        requests,
        completions,
    }
}

async fn independent_command(ctx: &mut TestContext, id: u64) {
    ctx.process_async(json!({"id": id, "method": "Browser.getVersion"}))
        .await;
    assert!(ctx.take_response_by_id(id)["result"]["product"].is_string());
}

async fn project_child_fact_during_handoff(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    source: moli_core::RendererDocumentLifecycleIdentity,
) -> Vec<crate::conn::BackgroundProtocolEvent> {
    use moli_core::page::{ChildFrameAttachmentSnapshot, ChildFrameTreeEventSnapshot};
    use moli_core::{
        RendererOutputCursor, RendererOutputItem, RendererOutputRecord, RendererOwnerAction,
    };
    let (page, binding) = conn
        .runtime_session_owner_slot_for_owner(owner)
        .unwrap()
        .page_slot()
        .prepared_document_output_binding()
        .unwrap();
    let page_owner = conn
        .pending_target_page_residence_identity_for_owner(owner)
        .or_else(|| conn.target_page_residence_identity_for_owner(owner))
        .unwrap();
    assert_eq!(page_owner.page_attachment_id(), binding.page_attachment_id);
    let publication_owner = crate::domains::activity::RendererPublicationOwner::PageTarget {
        browser_context_id: page_owner.browser_context_id().into(),
        target_id: page_owner.target_id().map(str::to_owned),
        renderer_page: page,
        page_owner,
    };
    let stream =
        moli_core::RendererOutputStreamIdentity::new_page_for_protocol_test(source.frame.page_id);
    let mut command = CommandDispatchContext::default();
    let (mut context, _scope) =
        crate::conn::CommandWorkflowContext::take_from(&mut command, None, None);
    let child = crate::domains::activity::RendererOutputProjection::prepare_record(
        conn,
        publication_owner,
        RendererOutputCursor::new_for_test(stream, 1),
        RendererOutputRecord::new_for_test(RendererOutputItem::OwnerAction(
            RendererOwnerAction::ChildFrameTree {
                source_document: source,
                event: ChildFrameTreeEventSnapshot::Attached(ChildFrameAttachmentSnapshot {
                    frame_id: "child-during-binding-handoff".into(),
                    parent_frame_id: Some(binding.frame_id),
                }),
            },
        )),
        &mut context,
    )
    .unwrap();
    child
        .run_using(
            &mut crate::conn::ProtocolWorkflowDriver::inline(conn),
            &mut context,
        )
        .await
        .unwrap();
    context.finish_projection_inline(conn, &mut command);
    command.take_protocol_events()
}

async fn drain_owned_background_turns(
    ctx: &mut TestContext,
    requests: &mut ProtocolStateRequestReceiver,
    completions: &mut mpsc::UnboundedReceiver<BackgroundNavigationCompletion>,
    initial: crate::conn::CdpRendererOwnerTurnOutcome,
) -> Vec<Value> {
    use crate::domains::activity::ProtocolSchedulerWorkKind;
    let mut turns = std::collections::VecDeque::from([initial]);
    let mut messages = Vec::new();
    loop {
        if let Some(turn) = turns.pop_front() {
            let (events, scheduler_events) =
                ctx.route_completed_command_outcome_for_test(turn).await;
            messages.extend(events);
            let mut other_events = Vec::new();
            for event in scheduler_events {
                match event {
                    crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }
                        if work.kind() == ProtocolSchedulerWorkKind::NavigationProtocolCompletionOwnerAction => {
                        let pending = ctx.conn.try_start_owned_protocol_scheduler_work(work)
                            .unwrap_or_else(|_| panic!("navigation uses its existing supervisor")).unwrap();
                        let mut wait = Box::pin(pending.wait());
                        let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                            loop { tokio::select! {
                                completed = &mut wait => break completed,
                                request = requests.recv() => request.unwrap().apply(&mut ctx.conn),
                            }}
                        }).await.unwrap();
                        let handoff = ctx.conn.complete_owned_protocol_scheduler_work(completed);
                        turns.push_back(handoff.with_renderer_output_predecessor(None));
                    }
                    event => other_events.push(event),
                }
            }
            ctx.route_direct_command_output_for_test(Vec::new(), other_events)
                .await;
        } else if let Ok(completion) = completions.try_recv() {
            let Ok(turn) = ctx
                .conn
                .try_start_background_navigation_completion_turn(completion)
            else {
                panic!("background workflow completion retains its finite owner boundary");
            };
            turns.push_back(turn);
        } else {
            return messages;
        }
    }
}

#[tokio::test]
async fn failed_background_protocol_handoff_retires_only_its_original_unbound_request() {
    use futures_util::FutureExt as _;
    tokio::task::LocalSet::new().run_until(async {
        for start_successor in [false, true] {
            let CommitFixture { mut ctx, token, navigation, prepared, mut requests, mut completions } = commit_fixture(true).await;
            ctx.conn.start_background_prepared_navigation_commit(token.clone(), navigation.clone(), prepared);
            requests.recv().await.unwrap().apply(&mut ctx.conn);
            requests.recv().await.unwrap().apply(&mut ctx.conn);
            let native = completions.recv().await.unwrap();
            let Ok(started) = ctx.conn.try_start_background_navigation_completion_turn(native) else { panic!("owned native completion") };
            let (_, events, _) = started.into_protocol_event_parts();
            let work = events.into_iter().find_map(|event| match event {
                crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } => Some(work),
                _ => None,
            }).unwrap();
            let pending = ctx.conn.try_start_owned_protocol_scheduler_work(work)
                .unwrap_or_else(|_| panic!("owned protocol tail")).unwrap();
            let mut wait = Box::pin(pending.wait());
            for _ in 0..3 {
                tokio::select! {
                    _ = &mut wait => panic!("installation and protocol commit precede final publication"),
                    request = requests.recv() => request.unwrap().apply(&mut ctx.conn),
                }
            }
            assert!(!ctx.conn.accepts_pending_document_navigation_for_owner(&navigation.owner, &token));
            // Reject the real queued state operation after commit, before
            // lifecycle binding. No test-only production fault hook is needed.
            assert!(wait.as_mut().now_or_never().is_none());
            drop(requests.recv().await.unwrap());
            let completed = wait.await;
            let handoff = ctx.conn.complete_owned_protocol_scheduler_work(completed);
            let (events, _) = handoff.into_protocol_event_parts();
            assert!(events.is_empty());
            let ready = completions.recv().await.unwrap();
            let successor = start_successor.then(|| ctx.conn.start_document_navigation_for_owner(
                &navigation.owner, "LID-after-failed-handoff".into(),
            ).unwrap());
            let Ok(recovery) = ctx.conn.try_start_background_navigation_completion_turn(ready) else { panic!("finite failure recovery") };
            let messages = drain_owned_background_turns(&mut ctx, &mut requests, &mut completions, recovery).await;
            let source = messages.iter().filter(|message| message["id"] == 1).collect::<Vec<_>>();
            assert_eq!(source.len(), 1, "{messages:?}");
            assert_eq!(source[0]["result"]["errorText"], moli_fetch::NET_ERR_ABORTED_ERROR_TEXT);
            assert!(!ctx.conn.abandon_unbound_prepared_document_navigation_for_owner(&navigation.owner, &token), "the failed source has already released its unbound request");
            if let Some(successor) = successor {
                assert!(ctx.conn.accepts_pending_document_navigation_for_owner(&navigation.owner, &successor));
                assert!(ctx.conn.renderer_document_navigation_is_suspended_for_session_owner(Some("SID-1")));
            } else {
                ctx.process_async(json!({"id":330,"sessionId":"SID-1","method":"Runtime.evaluate","params":{"expression":"document.title","returnByValue":true}})).await;
                assert_eq!(ctx.take_response_by_id(330)["result"]["result"]["value"], "prepared");
            }
            assert!(completions.try_recv().is_err());
        }
    }).await;
}

#[tokio::test]
async fn background_protocol_completion_rejoins_waits_and_keeps_its_original_publication_boundary()
{
    use crate::domains::activity::{ProtocolNavigationDependency, ProtocolSchedulerWorkKind};
    use futures_util::FutureExt as _;
    tokio::task::LocalSet::new().run_until(async {
        for replace_existing in [false, true] {
            for already_acked in [false, true] {
                for cancel_phase in 0..3 {
                    let CommitFixture { mut ctx, token, mut navigation, prepared, mut requests, mut completions } = commit_fixture(replace_existing).await;
                    if already_acked { navigation.navigate_id = None; }
                    ctx.enable_page_events_for_test(Some("SID-1"));
                    ctx.process_async(json!({"id":180,"sessionId":"SID-1","method":"Network.enable"})).await;
                    ctx.expect_result(180, json!({}), Some("SID-1"));
                    ctx.conn.start_background_prepared_navigation_commit(token, navigation.clone(), prepared);
                    for _ in 0..if replace_existing { 2 } else { 1 } {
                        requests.recv().await.unwrap().apply(&mut ctx.conn);
                    }
                    let native = completions.recv().await.unwrap();
                    let Ok(started) = ctx.conn.try_start_background_navigation_completion_turn(native) else {
                        panic!("production admission owns the prepared protocol tail");
                    };
                    let (events, scheduler_events, predecessor) = started.into_protocol_event_parts();
                    assert!(predecessor.is_none());
                    assert!(events.is_empty(), "the finite admission does not publish a source result");
                    let mut work = None;
                    let mut other_events = Vec::new();
                    for event in scheduler_events {
                        match event {
                            crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work: candidate }
                                if candidate.kind() == ProtocolSchedulerWorkKind::NavigationProtocolCompletionOwnerAction => {
                                assert_eq!(candidate.navigation_dependency(), ProtocolNavigationDependency::ReplacesPendingLoad);
                                assert_eq!(candidate.navigation_gate_target_id(), Some("TID-1"));
                                assert!(work.replace(candidate).is_none());
                            }
                            event => other_events.push(event),
                        }
                    }
                    ctx.route_direct_command_output_for_test(Vec::new(), other_events).await;
                    let pending = ctx.conn.try_start_owned_protocol_scheduler_work(work.unwrap())
                        .unwrap_or_else(|_| panic!("navigation uses the existing owner-work supervisor"))
                        .expect("the prepared tail has native work");
                    let original = pending.clone();
                    let mut wait = Box::pin(pending.wait());
                    if cancel_phase == 1 {
                        assert!(wait.as_mut().now_or_never().is_none());
                        requests.recv().await.unwrap().apply(&mut ctx.conn);
                        assert!(wait.as_mut().now_or_never().is_none());
                    }
                    if cancel_phase == 2 {
                        let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                            loop { tokio::select! {
                                completed = &mut wait => break completed,
                                request = requests.recv() => request.unwrap().apply(&mut ctx.conn),
                            }}
                        }).await.unwrap();
                        drop(completed);
                    }
                    drop(wait);
                    for id in 210..218 { independent_command(&mut ctx, id).await; }
                    let mut wait = Box::pin(original.clone().wait());
                    let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                        loop { tokio::select! {
                            completed = &mut wait => break completed,
                            request = requests.recv() => request.unwrap().apply(&mut ctx.conn),
                        }}
                    }).await.unwrap();
                    let handoff = ctx.conn.complete_owned_protocol_scheduler_work(completed);
                    let (events, _) = handoff.into_protocol_event_parts();
                    assert!(events.is_empty(), "the generic supervisor does not flatten a navigation cursor");
                    let ready = completions.recv().await.unwrap();
                    assert_eq!(ready.kind(), "protocol_workflow_ready");
                    drop(ready);
                    let ready = completions.recv().await.unwrap();
                    assert_eq!(ready.kind(), "protocol_workflow_ready", "delivery loss retains actual success and the same finalizer");
                    // A second ready notification can only observe the same
                    // already-claimed result; it cannot enqueue another source.
                    let duplicate = original.wait().await;
                    ctx.conn.complete_owned_protocol_scheduler_work(duplicate);
                    assert!(completions.try_recv().is_err());
                    let Ok(outcome) = ctx.conn.try_start_background_navigation_completion_turn(ready) else {
                        panic!("final source publication is a finite background owner turn");
                    };
                    let (mut messages, scheduler_events) = ctx.route_completed_command_outcome_for_test(outcome).await;
                    let sent_start = ctx.sent.len();
                    ctx.route_direct_command_output_for_test(Vec::new(), scheduler_events).await;
                    if !messages.iter().any(|message| message["method"] == "Page.frameStoppedLoading") {
                        crate::testing::wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
                    }
                    messages.extend(ctx.sent.drain(sent_start..));
                    assert_eq!(messages.iter().filter(|message| message["id"] == 1).count(), usize::from(!already_acked));
                    for method in ["Network.responseReceived", "Page.frameNavigated", "Page.domContentEventFired", "Page.loadEventFired"] {
                        assert_eq!(messages.iter().filter(|message| message["method"] == method).count(), 1, "{method}: {messages:?}");
                    }
                    let metadata = messages.iter().position(|message| message["method"] == "Network.responseReceived").unwrap();
                    let commit = messages.iter().position(|message| message["method"] == "Page.frameNavigated").unwrap();
                    let load = messages.iter().position(|message| message["method"] == "Page.loadEventFired").unwrap();
                    assert!(metadata < commit && commit < load);
                    assert!(completions.try_recv().is_err(), "the completed source obligation is disarmed");
                }
            }
        }
    }).await;
}

fn start_owned_protocol_tail(
    conn: &mut CdpConnection,
    completion: crate::conn::BackgroundPreparedNavigationCommit,
    context: &mut CommandDispatchContext,
) -> crate::conn::PendingCdpCommandDispatch {
    let access = conn.protocol_state_access().unwrap();
    let workflow = crate::conn::PreparedCommandWorkflow::new(move |mut context| async move {
        let mut driver = crate::conn::ProtocolWorkflowDriver::owned(access);
        let prepared = context
            .with_state(&mut driver, move |state, context| {
                state.prepare_navigation_protocol_completion(completion, context)
            })
            .await
            .unwrap();
        prepared
            .run(
                &mut driver,
                context,
                crate::domains::page::NavigationProtocolCleanup::default(),
                || {},
            )
            .await
    });
    let CdpCommandTaskStep::Pending(pending) =
        conn.start_command_workflow(workflow, Some(190), Some("SID-1"), context)
    else {
        panic!("the navigation protocol tail owns its waits");
    };
    *pending
}

#[tokio::test]
async fn owned_protocol_tail_stages_lifecycle_after_commit_until_its_finite_publication() {
    use futures_util::FutureExt as _;
    use moli_core::page::{
        RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
        RendererDocumentLifecycleMilestone,
    };
    tokio::task::LocalSet::new()
        .run_until(async {
            for settle_in_gap in [false, true] {
                for observe_commit_in_gap in [false, true] {
                    let CommitFixture {
                        mut ctx,
                        token,
                        navigation,
                        prepared,
                        mut requests,
                        mut completions,
                    } = commit_fixture(true).await;
                    ctx.process_async(
                        json!({"id": 189, "sessionId": "SID-1", "method": "Page.enable"}),
                    )
                    .await;
                    ctx.expect_result(189, json!({}), Some("SID-1"));
                    ctx.conn.start_background_prepared_navigation_commit(
                        token.clone(),
                        navigation.clone(),
                        prepared,
                    );
                    requests.recv().await.unwrap().apply(&mut ctx.conn);
                    requests.recv().await.unwrap().apply(&mut ctx.conn);
                    let BackgroundNavigationCompletion::PreparedDocument(mut completion) =
                        completions.recv().await.unwrap()
                    else {
                        panic!("native commit expected")
                    };
                    if !settle_in_gap {
                        assert!(ctx.conn.settle_background_navigation_completion(&token));
                    }
                    let loaded = completion.navigation.as_mut().unwrap();
                    let snapshot = loaded.page_creation_artifacts.lifecycle_snapshot;
                    let dcl = snapshot.dom_content_loaded.unwrap();
                    let facts = std::iter::once((
                        dcl,
                        RendererDocumentLifecycleMilestone::DomContentLoaded,
                    ))
                    .chain(
                        snapshot
                            .load
                            .map(|stamp| (stamp, RendererDocumentLifecycleMilestone::Load)),
                    )
                    .map(|(stamp, milestone)| RendererDocumentLifecycleEvent {
                        frame: snapshot.frame,
                        document: snapshot.document,
                        epoch: snapshot.epoch,
                        sequence: stamp.sequence,
                        timestamp_micros: stamp.timestamp_micros,
                        kind: RendererDocumentLifecycleEventKind::Milestone(milestone),
                    })
                    .collect();
                    loaded
                        .page_creation_artifacts
                        .initial_lifecycle_events
                        .retain(|event| {
                            !matches!(event.kind, RendererDocumentLifecycleEventKind::Milestone(_))
                        });
                    loaded
                        .page_creation_artifacts
                        .lifecycle_snapshot
                        .dom_content_loaded = None;
                    loaded.page_creation_artifacts.lifecycle_snapshot.load = None;
                    let predecessor = loaded.renderer_output_predecessor.clone().unwrap();
                    let source = moli_core::RendererDocumentLifecycleIdentity {
                        frame: snapshot.frame,
                        document: snapshot.document,
                        epoch: snapshot.epoch,
                    };
                    let mut child_events = Vec::new();
                    if !observe_commit_in_gap {
                        ctx.route_direct_command_renderer_predecessor_for_test(predecessor.clone())
                            .await;
                        child_events = project_child_fact_during_handoff(&mut ctx.conn, &navigation.owner, source).await;
                        assert_eq!(child_events.len(), 1, "Native child output precedes Page-handle adoption");
                        assert!(child_events[0].route_is_current(&ctx.conn));
                    }
                    let mut context = CommandDispatchContext::default();
                    let pending =
                        start_owned_protocol_tail(&mut ctx.conn, *completion, &mut context);
                    let mut wait = Box::pin(pending.wait());
                    // The first three operations install the Page, claim original workers,
                    // and commit protocol identity. Hold the next state operation so live
                    // facts arrive after commit but before lifecycle publication.
                    for _ in 0..3 {
                        assert!(wait.as_mut().now_or_never().is_none());
                        requests.recv().await.unwrap().apply(&mut ctx.conn);
                    }
                    assert!(
                        !ctx.conn.accepts_pending_document_navigation_for_owner(
                            &navigation.owner,
                            &token
                        )
                    );
                    if settle_in_gap {
                        assert!(ctx.conn.settle_background_navigation_completion(&token));
                    }
                    if observe_commit_in_gap {
                        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
                            .await;
                    }
                    let slot = ctx
                        .conn
                        .runtime_session_owner_slot_for_owner(&navigation.owner)
                        .unwrap();
                    assert!(
                        slot.accepts_parser_script_fetch_source(source),
                        "the original parser retains progress authority until binding handoff"
                    );
                    let (page, binding) = slot
                        .page_slot()
                        .prepared_document_output_binding()
                        .expect("committed but unbound Document keeps its temporary output route");
                    assert_eq!(page.page_id(), snapshot.frame.page_id);
                    assert_eq!(binding.renderer_document_identity(), source);
                    assert!(ctx.conn.target_root_document_protocol_attachment_identity_for_owner(&navigation.owner, source).is_none(), "retaining metadata does not grant the unbound Document new business authority");
                    let late_child = project_child_fact_during_handoff(&mut ctx.conn, &navigation.owner, source).await;
                    assert_eq!(late_child.len(), usize::from(observe_commit_in_gap), "adoption must preserve the original child fact without replaying it");
                    child_events.extend(late_child);
                    assert_eq!(child_events.len(), 1, "the exact Native child fact must survive protocol binding handoff");
                    assert!(child_events[0].route_is_current(&ctx.conn));
                    for title in ["first unbound title", "latest unbound title"] {
                        assert_eq!(
                            ctx.conn.apply_renderer_document_title_for_owner(
                                &navigation.owner,
                                &moli_core::RendererDocumentTitleChanged {
                                    source_document: source,
                                    title: title.into()
                                },
                            ),
                            Some(false),
                            "title facts are retained without changing predecessor metadata"
                        );
                    }
                    let (_, visible) = ctx
                        .conn
                        .ingest_renderer_document_lifecycle_events_for_owner(
                            &navigation.owner,
                            facts,
                        );
                    assert!(visible.is_empty());
                    independent_command(&mut ctx, 191).await;
                    let completed = loop {
                        tokio::select! {
                            completed = &mut wait => break completed,
                            request = requests.recv() => request.unwrap().apply(&mut ctx.conn),
                        }
                    };
                    let CdpCommandTaskStep::Complete(outcome) = ctx
                        .conn
                        .try_complete_ready_command_dispatch_with_context(completed, &mut context)
                        .unwrap_or_else(|_| panic!("final publication is finite"))
                    else {
                        panic!("no wait remains")
                    };
                    let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
                    assert!(child_events[0].route_is_current(&ctx.conn), "adoption retains the original Native child event route");
                    assert_eq!(
                        ctx.conn
                            .devtools_target_info("TID-1")
                            .unwrap()
                            .title,
                        "latest unbound title",
                        "the original Document title joins its binding before load publication"
                    );
                    for method in std::iter::once("Page.domContentEventFired")
                        .chain(snapshot.load.map(|_| "Page.loadEventFired"))
                    {
                        assert_eq!(
                            messages
                                .iter()
                                .filter(|message| message["method"] == method)
                                .count(),
                            1,
                            "{method}"
                        );
                    }
                    for id in [1, 190] {
                        assert_eq!(
                            messages
                                .iter()
                                .filter(|message| message["id"] == id)
                                .count(),
                            1
                        );
                    }
                    let response = messages
                        .iter()
                        .position(|message| message["id"] == 190)
                        .unwrap();
                    let dcl = messages
                        .iter()
                        .position(|message| message["method"] == "Page.domContentEventFired")
                        .unwrap();
                    assert!(response < dcl);
                    assert!(
                        !ctx.conn
                            .runtime_session_owner_slot_for_owner(&navigation.owner)
                            .unwrap()
                            .page_slot()
                            .owns_prepared_document_handoff(&token)
                    );
                }
            }
        })
        .await;
}

#[tokio::test]
async fn owned_protocol_tail_does_not_finish_a_rebound_sessions_successor_navigation() {
    use futures_util::FutureExt as _;
    tokio::task::LocalSet::new()
        .run_until(async {
            let CommitFixture {
                mut ctx,
                token,
                navigation,
                prepared,
                mut requests,
                mut completions,
            } = commit_fixture(true).await;
            ctx.process_async(json!({"id": 188, "sessionId": "SID-1", "method": "Network.enable"}))
                .await;
            ctx.expect_result(188, json!({}), Some("SID-1"));
            ctx.conn.start_background_prepared_navigation_commit(
                token.clone(),
                navigation.clone(),
                prepared,
            );
            requests.recv().await.unwrap().apply(&mut ctx.conn);
            requests.recv().await.unwrap().apply(&mut ctx.conn);
            let BackgroundNavigationCompletion::PreparedDocument(completion) =
                completions.recv().await.unwrap()
            else {
                panic!("native commit expected")
            };
            ctx.conn.settle_background_navigation_completion(&token);
            let mut context = CommandDispatchContext::default();
            let pending = start_owned_protocol_tail(&mut ctx.conn, *completion, &mut context);
            let mut wait = Box::pin(pending.wait());
            assert!(wait.as_mut().now_or_never().is_none());
            requests.recv().await.unwrap().apply(&mut ctx.conn);
            let route = ctx.conn.session_route(Some("SID-1")).unwrap();
            ctx.conn.register_session_route_for_test("SID-1", route);
            let successor = ctx
                .conn
                .start_document_navigation_for_owner(&navigation.owner, "LID-successor".into())
                .unwrap();
            let completed = loop {
                tokio::select! {
                    completed = &mut wait => break completed,
                    request = requests.recv() => request.unwrap().apply(&mut ctx.conn),
                }
            };
            let CdpCommandTaskStep::Complete(outcome) = ctx
                .conn
                .try_complete_ready_command_dispatch_with_context(completed, &mut context)
                .unwrap_or_else(|_| panic!("final publication is finite"))
            else {
                panic!("no wait remains")
            };
            let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["id"] == 190 && message.get("result").is_some())
                    .count(),
                1
            );
            assert_eq!(
                messages.iter().filter(|message| message["id"] == 1).count(),
                1
            );
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["method"] == "Network.responseReceived"
                        && message["params"]["loaderId"] == navigation.loader_id)
                    .count(),
                1,
                "the original frozen response metadata survives replacement"
            );
            assert!(
                ctx.conn
                    .accepts_pending_document_navigation_for_owner(&navigation.owner, &successor)
            );
            assert!(
                ctx.conn
                    .renderer_document_navigation_is_suspended_for_session_owner(Some("SID-1"))
            );
        })
        .await;
}

#[tokio::test]
async fn prepared_document_lifecycle_arriving_before_protocol_binding_is_replayed() {
    use moli_core::page::{
        RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
        RendererDocumentLifecycleMilestone,
    };
    tokio::task::LocalSet::new()
        .run_until(async {
            let CommitFixture {
                mut ctx,
                token,
                navigation,
                prepared,
                mut requests,
                mut completions,
            } = commit_fixture(true).await;
            ctx.process_async(json!({"id": 60, "sessionId": "SID-1", "method": "Page.enable"}))
                .await;
            ctx.expect_result(60, json!({}), Some("SID-1"));
            ctx.conn.start_background_prepared_navigation_commit(
                token,
                navigation.clone(),
                prepared,
            );
            requests.recv().await.unwrap().apply(&mut ctx.conn);
            requests.recv().await.unwrap().apply(&mut ctx.conn);
            let mut completion = completions.recv().await.unwrap();
            let BackgroundNavigationCompletion::PreparedDocument(result) = &mut completion else {
                panic!("prepared commit expected")
            };
            let artifacts = &mut result.navigation.as_mut().unwrap().page_creation_artifacts;
            let snapshot = artifacts.lifecycle_snapshot;
            let dcl = snapshot
                .dom_content_loaded
                .expect("buffered fixture reaches DCL");
            let milestones =
                std::iter::once((dcl, RendererDocumentLifecycleMilestone::DomContentLoaded)).chain(
                    snapshot
                        .load
                        .map(|stamp| (stamp, RendererDocumentLifecycleMilestone::Load)),
                );
            let events = milestones
                .map(|(stamp, milestone)| RendererDocumentLifecycleEvent {
                    frame: snapshot.frame,
                    document: snapshot.document,
                    epoch: snapshot.epoch,
                    sequence: stamp.sequence,
                    timestamp_micros: stamp.timestamp_micros,
                    kind: RendererDocumentLifecycleEventKind::Milestone(milestone),
                })
                .collect::<Vec<_>>();
            // Model the production DocumentCommit reply boundary: the frozen
            // prefix predates DCL/load, whose exact native facts arrive while
            // the actor has not yet installed the protocol lifecycle binding.
            artifacts.initial_lifecycle_events.retain(|event| {
                !matches!(event.kind, RendererDocumentLifecycleEventKind::Milestone(_))
            });
            artifacts.lifecycle_snapshot.dom_content_loaded = None;
            artifacts.lifecycle_snapshot.load = None;
            let predecessor = result
                .navigation
                .as_ref()
                .unwrap()
                .renderer_output_predecessor
                .clone()
                .unwrap();
            // Consume the concrete prefix now as well. Leaving a duplicate
            // native fact queued until after binding would conceal the loss.
            ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
                .await;
            let (_, visible) = ctx
                .conn
                .ingest_renderer_document_lifecycle_events_for_owner(&navigation.owner, events);
            assert!(visible.is_empty(), "unbound facts must not publish early");
            let outcome = ctx
                .conn
                .drain_background_navigation_completion_turn_async(completion)
                .await;
            let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
            let methods = std::iter::once("Page.domContentEventFired")
                .chain(snapshot.load.map(|_| "Page.loadEventFired"));
            for method in methods {
                assert_eq!(
                    messages
                        .iter()
                        .filter(|message| message["method"] == method)
                        .count(),
                    1,
                    "{method} must survive binding: {messages:#?}"
                );
            }
        })
        .await;
}

async fn process_preload_with_state_requests(
    ctx: &mut TestContext,
    requests: &mut ProtocolStateRequestReceiver,
    command: Value,
) {
    let command = crate::conn::ParsedCdpCommand::parse_value(command).unwrap();
    let mut context = CommandDispatchContext::default();
    let mut step = ctx
        .conn
        .start_parsed_command_dispatch_with_context(&command, &mut context);
    loop {
        match step {
            CdpCommandTaskStep::Complete(outcome) => {
                let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
                ctx.sent.extend(messages);
                return;
            }
            CdpCommandTaskStep::Pending(pending) => {
                let mut wait = Box::pin(pending.wait());
                let completed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        tokio::select! {
                            completed = &mut wait => break completed,
                            request = requests.recv() => request.unwrap().apply(&mut ctx.conn),
                        }
                    }
                })
                .await
                .unwrap();
                step = ctx
                    .conn
                    .try_complete_ready_command_dispatch_with_context(completed, &mut context)
                    .unwrap_or_else(|_| {
                        panic!("preload completion is finite or owns its next wait")
                    });
            }
        }
    }
}

#[tokio::test]
async fn queued_commit_captures_preload_changes_at_native_admission() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for replace_existing in [false, true] {
                for remove_preload in [false, true] {
                    let CommitFixture {
                        mut ctx,
                        token,
                        navigation,
                        prepared,
                        mut requests,
                        mut completions,
                    } = commit_fixture(replace_existing).await;
                    let mut preload_id = None;
                    if remove_preload {
                        process_preload_with_state_requests(&mut ctx, &mut requests, json!({
                            "id": 70, "sessionId": "SID-1",
                            "method": "Page.addScriptToEvaluateOnNewDocument",
                            "params": {"source": "globalThis.__commitConfiguration = 'removed'"}
                        })).await;
                        preload_id = Some(ctx.take_response_by_id(70)["result"]["identifier"].clone());
                    }
                    ctx.conn.start_background_prepared_navigation_commit(
                        token,
                        navigation,
                        prepared,
                    );
                    // Hold the real state request, not a timing assumption:
                    // the workflow is alive, but native admission is queued.
                    let admission = requests.recv().await.unwrap();
                    if let Some(identifier) = preload_id {
                        assert!(identifier.is_string());
                        process_preload_with_state_requests(&mut ctx, &mut requests, json!({
                            "id": 71, "sessionId": "SID-1",
                            "method": "Page.removeScriptToEvaluateOnNewDocument",
                            "params": {"identifier": identifier}
                        })).await;
                        ctx.expect_result(71, json!({}), Some("SID-1"));
                    } else {
                        process_preload_with_state_requests(&mut ctx, &mut requests, json!({
                            "id": 71, "sessionId": "SID-1",
                            "method": "Page.addScriptToEvaluateOnNewDocument",
                            "params": {"source": "globalThis.__commitConfiguration = 'latest'"}
                        })).await;
                        assert!(ctx.take_response_by_id(71)["result"]["identifier"].is_string());
                    }
                    independent_command(&mut ctx, 72).await;
                    assert!(completions.try_recv().is_err());
                    admission.apply(&mut ctx.conn);
                    if replace_existing {
                        requests.recv().await.unwrap().apply(&mut ctx.conn);
                    }
                    let completion = completions.recv().await.unwrap();
                    assert!(matches!(&completion, BackgroundNavigationCompletion::PreparedDocument(result) if result.navigation.is_ok()));
                    let outcome = ctx.conn.drain_background_navigation_completion_turn_async(completion).await;
                    let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
                    assert_eq!(messages.iter().filter(|message| message["id"] == json!(1)).count(), 1);
                    ctx.process_async(json!({
                        "id": 73, "sessionId": "SID-1", "method": "Runtime.evaluate",
                        "params": {"expression": "String(globalThis.__commitConfiguration)", "returnByValue": true}
                    })).await;
                    assert_eq!(
                        ctx.take_response_by_id(73)["result"]["result"]["value"],
                        json!(if remove_preload { "undefined" } else { "latest" }),
                        "replace_existing={replace_existing}, remove_preload={remove_preload}"
                    );
                }
            }
        })
        .await;
}

#[tokio::test]
async fn aborting_owned_commit_before_admission_preserves_page_and_reports_failure() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let CommitFixture {
                mut ctx,
                token,
                navigation,
                prepared,
                mut requests,
                mut completions,
            } = commit_fixture(true).await;
            ctx.process_async(json!({"id": 89, "method": "Network.enable", "sessionId": "SID-1"}))
                .await;
            let previous = ctx
                .conn
                .renderer_page_residence_identity_for_owner(&navigation.owner);
            ctx.conn.start_background_prepared_navigation_commit(
                token,
                navigation.clone(),
                prepared,
            );
            let request = requests.recv().await.unwrap();
            let tasks = ctx.conn.take_background_navigation_tasks();
            assert_eq!(tasks.len(), 1);
            for task in tasks {
                task.cancel_and_join().await;
            }
            request.apply(&mut ctx.conn);
            let completion = completions.recv().await.unwrap();
            let outcome = ctx
                .conn
                .drain_background_navigation_completion_turn_async(completion)
                .await;
            let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["id"] == json!(1))
                    .count(),
                1
            );
            assert!(
                messages
                    .iter()
                    .any(|message| message["id"] == json!(1) && message.get("error").is_some())
            );
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["method"] == "Network.loadingFailed")
                    .count(),
                1,
                "commit failure must settle the original network request: {messages:#?}"
            );
            assert_eq!(
                ctx.conn
                    .renderer_page_residence_identity_for_owner(&navigation.owner),
                previous
            );
            assert!(!ctx.conn.has_inflight_background_navigation());
            assert!(completions.try_recv().is_err());
        })
        .await;
}

#[tokio::test]
async fn prepared_navigation_commit_yields_between_configuration_commit_and_adoption() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for replace_existing in [false, true] {
                let CommitFixture {
                    mut ctx,
                    token,
                    navigation,
                    prepared,
                    mut requests,
                    mut completions,
                } = commit_fixture(replace_existing).await;
                ctx.conn.start_background_prepared_navigation_commit(
                    token.clone(),
                    navigation.clone(),
                    prepared,
                );
                // Configuration is captured together with native admission
                // when the actor handles this synchronous operation.
                let admission = requests.recv().await.unwrap();
                independent_command(&mut ctx, 90).await;
                assert!(completions.try_recv().is_err());
                admission.apply(&mut ctx.conn);
                if replace_existing {
                    // The native commit reply is ready, but the existing Page
                    // has not adopted it. Other commands remain admissible.
                    let adoption = requests.recv().await.unwrap();
                    independent_command(&mut ctx, 91).await;
                    assert!(completions.try_recv().is_err());
                    adoption.apply(&mut ctx.conn);
                }
                let completion = completions.recv().await.unwrap();
                assert!(matches!(&completion, BackgroundNavigationCompletion::PreparedDocument(result) if result.navigation.is_ok()));
                let outcome = ctx.conn.drain_background_navigation_completion_turn_async(completion).await;
                let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
                assert_eq!(messages.iter().filter(|message| message["id"] == json!(1)).count(), 1);
                assert!(!ctx.conn.has_inflight_background_navigation());
                ctx.process_async(json!({
                    "id": 92, "sessionId": "SID-1", "method": "Runtime.evaluate",
                    "params": {"expression": "document.title", "returnByValue": true}
                })).await;
                assert_eq!(ctx.take_response_by_id(92)["result"]["result"]["value"], json!("prepared"));
            }
        })
        .await;
}

#[tokio::test]
async fn replaced_navigation_cancels_queued_commit_admission_and_settles_once() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let CommitFixture { mut ctx, token, navigation, prepared, mut requests, mut completions } = commit_fixture(true).await;
            ctx.conn.start_background_prepared_navigation_commit(token.clone(), navigation.clone(), prepared);
            let old_admission = requests.recv().await.unwrap();
            let current = ctx.conn.start_document_navigation_for_owner(&navigation.owner, "replacement".into()).unwrap();
            let completion = completions.recv().await.unwrap();
            assert!(matches!(&completion, BackgroundNavigationCompletion::PreparedDocument(result) if result.transaction.is_none() && result.navigation.is_err()));
            old_admission.apply(&mut ctx.conn);
            let page_before = ctx.conn.renderer_page_residence_identity_for_owner(&navigation.owner);
            let outcome = ctx.conn.drain_background_navigation_completion_turn_async(completion).await;
            let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
            assert_eq!(messages.iter().filter(|message| message["id"] == json!(1)).count(), 1);
            assert!(messages.iter().any(|message| message["result"]["errorText"] == json!("net::ERR_ABORTED")));
            assert_eq!(ctx.conn.renderer_page_residence_identity_for_owner(&navigation.owner), page_before);
            assert!(ctx.conn.accepts_pending_document_navigation_for_owner(&navigation.owner, &current));
            assert!(completions.try_recv().is_err());
        })
        .await;
}

#[tokio::test]
async fn cancelled_adoption_cannot_clean_up_a_replacement_page() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let CommitFixture { mut ctx, token, navigation, prepared, mut requests, mut completions } = commit_fixture(true).await;
            ctx.conn.start_background_prepared_navigation_commit(token.clone(), navigation.clone(), prepared);
            requests.recv().await.unwrap().apply(&mut ctx.conn);
            // A's renderer has committed. Hold its successful adoption request
            // while B starts, which aborts A's owned task.
            let old_adoption = requests.recv().await.unwrap();
            let current = ctx.conn.start_document_navigation_for_owner(&navigation.owner, "replacement".into()).unwrap();
            let completion = completions.recv().await.unwrap();
            assert!(matches!(&completion, BackgroundNavigationCompletion::PreparedDocument(result) if result.transaction.is_some() && result.navigation.is_err()));
            ctx.conn.finish_renderer_document_navigation_for_owner(&navigation.owner, &current).unwrap();
            ctx.conn.clear_pending_document_navigation_for_owner_if_loader_matches(&navigation.owner, &current.loader_id);
            ctx.install_navigation_fixture_for_session_owner("data:text/html,<title>replacement</title>", Some("SID-1")).await;
            let replacement = ctx.conn.renderer_page_residence_identity_for_owner(&navigation.owner);
            let attachment = ctx.conn.current_renderer_agent_attachment_id_for_owner(&navigation.owner);
            old_adoption.apply(&mut ctx.conn);
            let outcome = ctx.conn.drain_background_navigation_completion_turn_async(completion).await;
            let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
            assert_eq!(messages.iter().filter(|message| message["id"] == json!(1)).count(), 1);
            assert_eq!(ctx.conn.renderer_page_residence_identity_for_owner(&navigation.owner), replacement);
            assert_eq!(ctx.conn.current_renderer_agent_attachment_id_for_owner(&navigation.owner), attachment);
            ctx.process_async(json!({
                "id": 95, "sessionId": "SID-1", "method": "Runtime.evaluate",
                "params": {"expression": "document.title", "returnByValue": true}
            })).await;
            assert_eq!(ctx.take_response_by_id(95)["result"]["result"]["value"], json!("replacement"));
            assert!(completions.try_recv().is_err());
        })
        .await;
}
