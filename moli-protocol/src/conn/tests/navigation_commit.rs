use super::*;
use crate::conn::{
    CapturedBody, CdpCommandTaskStep, CommandOwnerScope, DocumentNavigationToken,
    NavigationLoadOutcome, ResponseCommitReady,
};
use crate::domains::page::BackgroundNavigationCompletion;
use serde_json::Value;
use tokio::sync::mpsc;

struct CommitFixture {
    ctx: TestContext,
    token: DocumentNavigationToken,
    navigation: NavigationDispatchState,
    prepared: ResponseCommitReady,
    completions: mpsc::UnboundedReceiver<BackgroundNavigationCompletion>,
}

async fn commit_fixture(replace_existing: bool) -> CommitFixture {
    let (mut ctx, mut token, mut navigation) = super::navigation_error::navigation_fixture();
    if replace_existing {
        ctx.with_fixture(|conn| {
            conn.finish_renderer_document_navigation_for_owner(&navigation.owner, &token)
                .unwrap()
        });
        ctx.with_fixture(|conn| {
            conn.clear_pending_document_navigation_for_owner_if_loader_matches(
                &navigation.owner,
                &token.loader_id,
            )
        });
        ctx.install_navigation_fixture_for_session_owner("about:blank", Some("SID-1"))
            .await;
        token = ctx.with_fixture(|conn| {
            conn.start_document_navigation_for_owner(
                &navigation.owner,
                navigation.loader_id.clone(),
            )
            .unwrap()
        });
    }
    navigation.requested_url = Url::parse("https://example.test/prepared").unwrap();
    ctx.with_fixture(|conn| {
        conn.runtime_session_owner_slot_mut_for_owner(&navigation.owner)
            .unwrap()
            .claim_background_navigation_completion(&token, None)
            .unwrap()
    });
    let prepared = ctx
        .with_fixture(|conn| {
            conn.prepare_synthetic_navigation_response(
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
        })
        .unwrap()
        .run()
        .await
        .unwrap();
    let NavigationLoadOutcome::ResponseCommitReady(prepared) = prepared else {
        panic!("fixture must stop before the renderer commit");
    };
    assert_eq!(prepared.stable_page_target().is_some(), replace_existing);
    let (sender, completions) = mpsc::unbounded_channel();
    ctx.with_fixture(|conn| conn.set_background_navigation_completion_sender(sender));
    CommitFixture {
        ctx,
        token,
        navigation,
        prepared: *prepared,
        completions,
    }
}

async fn independent_command(ctx: &mut TestContext, id: u64) {
    ctx.process_async(json!({"id": id, "method": "Browser.getVersion"}))
        .await;
    assert!(ctx.take_response_by_id(id)["result"]["product"].is_string());
}

async fn project_child_fact_during_handoff(
    handle: &crate::conn::ProtocolStateHandle,
    owner: &CommandOwnerScope,
    source: moli_core::RendererDocumentLifecycleIdentity,
) -> Vec<crate::conn::BackgroundProtocolEvent> {
    use moli_core::page::{ChildFrameAttachmentSnapshot, ChildFrameTreeEventSnapshot};
    use moli_core::{
        RendererOutputCursor, RendererOutputItem, RendererOutputRecord, RendererOwnerAction,
    };
    let mut command = CommandDispatchContext::default();
    let (mut context, _scope) =
        crate::conn::CommandWorkflowContext::take_from(&mut command, None, None);
    let child = handle
        .with_state(|state| {
            state.fixture(|conn| {
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
                let publication_owner =
                    crate::domains::activity::RendererPublicationOwner::PageTarget {
                        browser_context_id: page_owner.browser_context_id().into(),
                        target_id: page_owner.target_id().map(str::to_owned),
                        renderer_page: page,
                        page_owner,
                    };
                let stream = moli_core::RendererOutputStreamIdentity::new_page_for_protocol_test(
                    source.frame.page_id,
                );
                crate::domains::activity::RendererOutputProjection::prepare_record(
                    conn,
                    publication_owner,
                    RendererOutputCursor::new_for_test(stream, 1),
                    RendererOutputRecord::new_for_test(RendererOutputItem::OwnerAction(
                        RendererOwnerAction::ChildFrameTree {
                            source_document: source,
                            event: ChildFrameTreeEventSnapshot::Attached(
                                ChildFrameAttachmentSnapshot {
                                    frame_id: "child-during-binding-handoff".into(),
                                    parent_frame_id: Some(binding.frame_id),
                                },
                            ),
                        },
                    )),
                    &mut context,
                )
                .unwrap()
            })
        })
        .unwrap();
    child
        .run_using(
            &mut crate::conn::ProtocolWorkflowDriver::new(handle.clone()),
            &mut context,
        )
        .await
        .unwrap();
    handle
        .with_state(|state| state.fixture(|conn| context.finish_projection(conn, &mut command)))
        .unwrap();
    command.take_protocol_events()
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
                    let CommitFixture { mut ctx, token, mut navigation, prepared,  mut completions } = commit_fixture(replace_existing).await;
                    if already_acked { navigation.navigate_id = None; }
                    ctx.enable_page_events_for_test(Some("SID-1"));
                    ctx.process_async(json!({"id":180,"sessionId":"SID-1","method":"Network.enable"})).await;
                    ctx.expect_result(180, json!({}), Some("SID-1"));
                    ctx.with_fixture(|conn| conn.start_background_prepared_navigation_commit(token, navigation.clone(), prepared));

                    let native = completions.recv().await.unwrap();
                    let started = ctx.with_fixture(|conn| conn.admit_background_navigation_completion(native)) ;
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
                    let pending = ctx.with_fixture(|conn| conn.try_start_owned_protocol_scheduler_work(work.unwrap())
                        .unwrap_or_else(|_| panic!("navigation uses the existing owner-work supervisor"))
                        .expect("the prepared tail has native work"));
                    let original = pending.clone();
                    let mut wait = Box::pin(pending.wait());
                    if cancel_phase == 1 {
                        drop(wait.as_mut().now_or_never());
                    }
                    if cancel_phase == 2 {
                        let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                            (&mut wait).await
                        }).await.unwrap();
                        drop(completed);
                    }
                    drop(wait);
                    for id in 210..218 { independent_command(&mut ctx, id).await; }
                    let mut wait = Box::pin(original.clone().wait());
                    let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                        (&mut wait).await
                    }).await.unwrap();
                    let handoff = ctx.with_fixture(|conn| conn.complete_owned_protocol_scheduler_work(completed));
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
                    ctx.with_fixture(|conn| conn.complete_owned_protocol_scheduler_work(duplicate));
                    assert!(completions.try_recv().is_err());
                    let outcome = ctx.with_fixture(|conn| conn.admit_background_navigation_completion(ready)) ;
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
    let access = conn.protocol_state_access();
    let workflow = crate::conn::PreparedCommandWorkflow::new(move |mut context| async move {
        let mut driver = crate::conn::ProtocolWorkflowDriver::new(access);
        let prepared = context
            .with_state(&mut driver, move |state, context| {
                state.prepare_navigation_protocol_completion(completion, context)
            })
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

                        mut completions,
                    } = commit_fixture(true).await;
                    ctx.process_async(
                        json!({"id": 189, "sessionId": "SID-1", "method": "Page.enable"}),
                    )
                    .await;
                    ctx.expect_result(189, json!({}), Some("SID-1"));
                    ctx.with_fixture(|conn| conn.start_background_prepared_navigation_commit(
                        token.clone(),
                        navigation.clone(),
                        prepared,
                    ));
                    let BackgroundNavigationCompletion::PreparedDocument(mut completion) =
                        completions.recv().await.unwrap()
                    else {
                        panic!("native commit expected")
                    };
                    if !settle_in_gap {
                        ctx.with_fixture(|conn| { assert!(conn.settle_background_navigation_completion(&token)) });
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
                        child_events = project_child_fact_during_handoff(&ctx.handle(), &navigation.owner, source).await;
                        assert_eq!(child_events.len(), 1, "Native child output precedes Page-handle adoption");
                        ctx.with_fixture(|conn| { assert!(child_events[0].route_is_current(conn)) });
                    }
                    let mut context = CommandDispatchContext::default();
                    let pending =
                        ctx.with_fixture(|conn| start_owned_protocol_tail(conn, *completion, &mut context));
                    let mut wait = Box::pin(pending.wait());
                    let completed = ctx.drive_owned_wait(&mut wait).await;
                    ctx.with_fixture(|conn| { assert!(
                        !conn.accepts_pending_document_navigation_for_owner(
                            &navigation.owner,
                            &token
                        )
                    ) });
                    if settle_in_gap {
                        ctx.with_fixture(|conn| { assert!(conn.settle_background_navigation_completion(&token)) });
                    }
                    if observe_commit_in_gap {
                        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
                            .await;
                    }
                    ctx.with_fixture(|conn| {
let slot = conn
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
});
                    ctx.with_fixture(|conn| { assert!(conn.target_root_document_protocol_attachment_identity_for_owner(&navigation.owner, source).is_none(), "retaining metadata does not grant the unbound Document new business authority") });
                    let late_child = project_child_fact_during_handoff(&ctx.handle(), &navigation.owner, source).await;
                    assert_eq!(late_child.len(), usize::from(observe_commit_in_gap), "adoption must preserve the original child fact without replaying it");
                    child_events.extend(late_child);
                    assert_eq!(child_events.len(), 1, "the exact Native child fact must survive protocol binding handoff");
                    ctx.with_fixture(|conn| { assert!(child_events[0].route_is_current(conn)) });
                    for title in ["first unbound title", "latest unbound title"] {
                        ctx.with_fixture(|conn| { assert_eq!(
                            conn.apply_renderer_document_title_for_owner(
                                &navigation.owner,
                                &moli_core::RendererDocumentTitleChanged {
                                    source_document: source,
                                    title: title.into()
                                },
                            ),
                            Some(false),
                            "title facts are retained without changing predecessor metadata"
                        ) });
                    }
                    let (_, visible) = ctx.with_fixture(|conn| conn
                        .ingest_renderer_document_lifecycle_events_for_owner(
                            &navigation.owner,
                            facts,
                        ));
                    assert!(visible.is_empty());
                    independent_command(&mut ctx, 191).await;
                    let CdpCommandTaskStep::Complete(outcome) = ctx.with_fixture(|conn| conn
                        .complete_command_dispatch_with_context(completed, &mut context)
                        )
                    else {
                        panic!("no wait remains")
                    };
                    let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
                    ctx.with_fixture(|conn| { assert!(child_events[0].route_is_current(conn), "adoption retains the original Native child event route") });
                    ctx.with_fixture(|conn| { assert_eq!(
                        conn
                            .devtools_target_info("TID-1")
                            .unwrap()
                            .title,
                        "latest unbound title",
                        "the original Document title joins its binding before load publication"
                    ) });
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
                    ctx.with_fixture(|conn| { assert!(
                        !conn
                            .runtime_session_owner_slot_for_owner(&navigation.owner)
                            .unwrap()
                            .page_slot()
                            .owns_prepared_document_handoff(&token)
                    ) });
                }
            }
        })
        .await;
}

#[tokio::test]
async fn owned_protocol_tail_does_not_finish_a_rebound_sessions_successor_navigation() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let CommitFixture {
                mut ctx,
                token,
                navigation,
                prepared,

                mut completions,
            } = commit_fixture(true).await;
            ctx.process_async(json!({"id": 188, "sessionId": "SID-1", "method": "Network.enable"}))
                .await;
            ctx.expect_result(188, json!({}), Some("SID-1"));
            ctx.with_fixture(|conn| {
                conn.start_background_prepared_navigation_commit(
                    token.clone(),
                    navigation.clone(),
                    prepared,
                )
            });
            let BackgroundNavigationCompletion::PreparedDocument(completion) =
                completions.recv().await.unwrap()
            else {
                panic!("native commit expected")
            };
            ctx.with_fixture(|conn| conn.settle_background_navigation_completion(&token));
            let mut context = CommandDispatchContext::default();
            let pending =
                ctx.with_fixture(|conn| start_owned_protocol_tail(conn, *completion, &mut context));
            let mut wait = Box::pin(pending.wait());
            let completed = ctx.drive_owned_wait(&mut wait).await;
            let route = ctx.with_fixture(|conn| conn.session_route(Some("SID-1")).unwrap());
            ctx.with_fixture(|conn| conn.register_session_route_for_test("SID-1", route));
            let successor = ctx.with_fixture(|conn| {
                conn.start_document_navigation_for_owner(&navigation.owner, "LID-successor".into())
                    .unwrap()
            });
            let CdpCommandTaskStep::Complete(outcome) = ctx.with_fixture(|conn| {
                conn.complete_command_dispatch_with_context(completed, &mut context)
            }) else {
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
            ctx.with_fixture(|conn| {
                assert!({
                    conn.accepts_pending_document_navigation_for_owner(
                        &navigation.owner,
                        &successor,
                    )
                })
            });
            ctx.with_fixture(|conn| {
                assert!({
                    conn.renderer_document_navigation_is_suspended_for_session_owner(Some("SID-1"))
                })
            });
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

                mut completions,
            } = commit_fixture(true).await;
            ctx.process_async(json!({"id": 60, "sessionId": "SID-1", "method": "Page.enable"}))
                .await;
            ctx.expect_result(60, json!({}), Some("SID-1"));
            ctx.with_fixture(|conn| {
                conn.start_background_prepared_navigation_commit(
                    token,
                    navigation.clone(),
                    prepared,
                )
            });
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
            let (_, visible) = ctx.with_fixture(|conn| {
                conn.ingest_renderer_document_lifecycle_events_for_owner(&navigation.owner, events)
            });
            assert!(visible.is_empty(), "unbound facts must not publish early");
            let outcome =
                ctx.with_fixture(|conn| conn.admit_background_navigation_completion(completion));
            let messages = ctx
                .route_fixture_background_turns(&mut completions, outcome)
                .await;
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

async fn process_preload_command(ctx: &mut TestContext, command: Value) {
    let command = crate::conn::ParsedCdpCommand::parse_value(command).unwrap();
    let mut context = CommandDispatchContext::default();
    let mut step = ctx.with_fixture(|conn| {
        conn.start_parsed_command_dispatch_with_context(&command, &mut context)
    });
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
                    (&mut wait).await
                })
                .await
                .unwrap();
                step = ctx.with_fixture(|conn| {
                    conn.complete_command_dispatch_with_context(completed, &mut context)
                });
            }
        }
    }
}

#[tokio::test]
async fn commit_captures_preload_changes_accepted_before_native_admission() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for replace_existing in [false, true] {
                for remove_preload in [false, true] {
                    let CommitFixture {
                        mut ctx,
                        token,
                        navigation,
                        prepared,

                        mut completions,
                    } = commit_fixture(replace_existing).await;
                    let mut preload_id = None;
                    if remove_preload {
                        process_preload_command(&mut ctx, json!({
                            "id": 70, "sessionId": "SID-1",
                            "method": "Page.addScriptToEvaluateOnNewDocument",
                            "params": {"source": "globalThis.__commitConfiguration = 'removed'"}
                        })).await;
                        preload_id = Some(ctx.take_response_by_id(70)["result"]["identifier"].clone());
                    }
                    if let Some(identifier) = preload_id {
                        assert!(identifier.is_string());
                        process_preload_command(&mut ctx, json!({
                            "id": 71, "sessionId": "SID-1",
                            "method": "Page.removeScriptToEvaluateOnNewDocument",
                            "params": {"identifier": identifier}
                        })).await;
                        ctx.expect_result(71, json!({}), Some("SID-1"));
                    } else {
                        process_preload_command(&mut ctx, json!({
                            "id": 71, "sessionId": "SID-1",
                            "method": "Page.addScriptToEvaluateOnNewDocument",
                            "params": {"source": "globalThis.__commitConfiguration = 'latest'"}
                        })).await;
                        assert!(ctx.take_response_by_id(71)["result"]["identifier"].is_string());
                    }
                    independent_command(&mut ctx, 72).await;
                    assert!(completions.try_recv().is_err());
                    ctx.with_fixture(|conn| conn.start_background_prepared_navigation_commit(
                        token,
                        navigation,
                        prepared,
                    ));

                    let completion = completions.recv().await.unwrap();
                    assert!(matches!(&completion, BackgroundNavigationCompletion::PreparedDocument(result) if result.navigation.is_ok()));
                    let outcome = ctx.with_fixture(|conn| conn.admit_background_navigation_completion(completion));
            let messages = ctx.route_fixture_background_turns(&mut completions, outcome).await;
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

                mut completions,
            } = commit_fixture(true).await;
            ctx.process_async(json!({"id": 89, "method": "Network.enable", "sessionId": "SID-1"}))
                .await;
            let previous = ctx.with_fixture(|conn| {
                conn.renderer_page_residence_identity_for_owner(&navigation.owner)
            });
            ctx.with_fixture(|conn| {
                conn.start_background_prepared_navigation_commit(
                    token,
                    navigation.clone(),
                    prepared,
                )
            });
            let tasks = ctx.with_fixture(|conn| conn.take_background_navigation_tasks());
            assert_eq!(tasks.len(), 1);
            for task in tasks {
                task.cancel_and_join().await;
            }
            let completion = completions.recv().await.unwrap();
            let outcome =
                ctx.with_fixture(|conn| conn.admit_background_navigation_completion(completion));
            let messages = ctx
                .route_fixture_background_turns(&mut completions, outcome)
                .await;
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
            ctx.with_fixture(|conn| {
                assert_eq!(
                    conn.renderer_page_residence_identity_for_owner(&navigation.owner),
                    previous
                )
            });
            ctx.with_fixture(|conn| assert!(!conn.has_inflight_background_navigation()));
            assert!(completions.try_recv().is_err());
        })
        .await;
}

#[tokio::test]
async fn prepared_navigation_commit_releases_state_during_native_wait_and_before_publication() {
    use futures_util::FutureExt as _;
    tokio::task::LocalSet::new().run_until(async {
        for replace_existing in [false, true] {
            let CommitFixture { mut ctx, token, navigation, prepared, mut completions } = commit_fixture(replace_existing).await;
            let gate = if replace_existing { Some(ctx.hold_fixture_renderer(Some("SID-1")).await) } else { None };
            let commit = crate::conn::PreparedDocumentCommitState::new();
            let mut waiting = Box::pin(commit.commit(ctx.handle(), token.clone(), &navigation, prepared));
            if gate.is_some() {
                assert!(waiting.as_mut().now_or_never().is_none(), "the real renderer commit is held");
            }
            independent_command(&mut ctx, 90).await;
            drop(gate);
            let result = ctx.drive_owned_wait(waiting).await;
            assert!(result.is_ok());
            // The Native result is retained outside state until its original finite publication.
            independent_command(&mut ctx, 91).await;
            let completion = BackgroundNavigationCompletion::PreparedDocument(Box::new(commit.complete(token, navigation, result)));
            let outcome = ctx.with_fixture(|conn| conn.admit_background_navigation_completion(completion));
            let messages = ctx.route_fixture_background_turns(&mut completions, outcome).await;
            assert_eq!(messages.iter().filter(|message| message["id"] == 1).count(), 1);
            ctx.with_fixture(|conn| assert!(!conn.has_inflight_background_navigation()));
            ctx.process_async(json!({"id":92,"sessionId":"SID-1","method":"Runtime.evaluate","params":{"expression":"document.title","returnByValue":true}})).await;
            assert_eq!(ctx.take_response_by_id(92)["result"]["result"]["value"], "prepared");
        }
    }).await;
}

#[tokio::test]
async fn replaced_navigation_cancels_queued_commit_admission_and_settles_once() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let CommitFixture { mut ctx, token, navigation, prepared,  mut completions } = commit_fixture(true).await;
            ctx.with_fixture(|conn| conn.start_background_prepared_navigation_commit(token.clone(), navigation.clone(), prepared));
            let current = ctx.with_fixture(|conn| conn.start_document_navigation_for_owner(&navigation.owner, "replacement".into()).unwrap());
            let completion = completions.recv().await.unwrap();
            assert!(matches!(&completion, BackgroundNavigationCompletion::PreparedDocument(result) if result.transaction.is_none() && result.navigation.is_err()));
            let page_before = ctx.with_fixture(|conn| conn.renderer_page_residence_identity_for_owner(&navigation.owner));
            let outcome = ctx.with_fixture(|conn| conn.admit_background_navigation_completion(completion));
            let messages = ctx.route_fixture_background_turns(&mut completions, outcome).await;
            assert_eq!(messages.iter().filter(|message| message["id"] == json!(1)).count(), 1);
            assert!(messages.iter().any(|message| message["result"]["errorText"] == json!("net::ERR_ABORTED")));
            ctx.with_fixture(|conn| { assert_eq!(conn.renderer_page_residence_identity_for_owner(&navigation.owner), page_before) });
            ctx.with_fixture(|conn| { assert!(conn.accepts_pending_document_navigation_for_owner(&navigation.owner, &current)) });
            assert!(completions.try_recv().is_err());
        })
        .await;
}

#[tokio::test]
async fn cancelled_adoption_cannot_clean_up_a_replacement_page() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let CommitFixture { mut ctx, token, navigation, prepared,  mut completions } = commit_fixture(true).await;
            use futures_util::FutureExt as _;
            let gate = ctx.hold_fixture_renderer(Some("SID-1")).await;
            let commit = crate::conn::PreparedDocumentCommitState::new();
            let mut waiting = Box::pin(commit.commit(ctx.handle(), token.clone(), &navigation, prepared));
            assert!(waiting.as_mut().now_or_never().is_none(), "hold the real admitted Native replacement");
            let current = ctx.with_fixture(|conn| conn.start_document_navigation_for_owner(&navigation.owner, "replacement".into()).unwrap());
            drop(waiting);
            let completion = BackgroundNavigationCompletion::PreparedDocument(Box::new(commit.cancel(token.clone(), navigation.clone())));
            drop(gate);
            assert!(matches!(&completion, BackgroundNavigationCompletion::PreparedDocument(result) if result.transaction.is_some() && result.navigation.is_err()));
            ctx.with_fixture(|conn| conn.finish_renderer_document_navigation_for_owner(&navigation.owner, &current).unwrap());
            ctx.with_fixture(|conn| conn.clear_pending_document_navigation_for_owner_if_loader_matches(&navigation.owner, &current.loader_id));
            ctx.install_navigation_fixture_for_session_owner("data:text/html,<title>replacement</title>", Some("SID-1")).await;
            let replacement = ctx.with_fixture(|conn| conn.renderer_page_residence_identity_for_owner(&navigation.owner));
            let attachment = ctx.with_fixture(|conn| conn.current_renderer_agent_attachment_id_for_owner(&navigation.owner));
            let outcome = ctx.with_fixture(|conn| conn.admit_background_navigation_completion(completion));
            let messages = ctx.route_fixture_background_turns(&mut completions, outcome).await;
            assert_eq!(messages.iter().filter(|message| message["id"] == json!(1)).count(), 1);
            ctx.with_fixture(|conn| { assert_eq!(conn.renderer_page_residence_identity_for_owner(&navigation.owner), replacement) });
            ctx.with_fixture(|conn| { assert_eq!(conn.current_renderer_agent_attachment_id_for_owner(&navigation.owner), attachment) });
            ctx.process_async(json!({
                "id": 95, "sessionId": "SID-1", "method": "Runtime.evaluate",
                "params": {"expression": "document.title", "returnByValue": true}
            })).await;
            assert_eq!(ctx.take_response_by_id(95)["result"]["result"]["value"], json!("replacement"));
            assert!(completions.try_recv().is_err());
        })
        .await;
}

#[tokio::test]
async fn non_document_background_completion_keeps_one_terminal_and_preserves_successor() {
    use crate::domains::network::{
        CompletedDownloadProgressTransfer, CompletedMainDocumentNetworkEvents,
    };
    for download in [false, true] {
        for successor in [false, true] {
            let CommitFixture {
                mut ctx,
                token,
                navigation,
                prepared,

                mut completions,
            } = commit_fixture(true).await;
            drop(prepared);
            let current_page = ctx.with_fixture(|conn| {
                conn.renderer_page_residence_identity_for_owner(&navigation.owner)
            });
            let replacement = successor.then(|| {
                ctx.with_fixture(|conn| {
                    conn.start_document_navigation_for_owner(
                        &navigation.owner,
                        navigation.loader_id.clone(),
                    )
                    .unwrap()
                })
            });
            let command_id = navigation.navigate_id.unwrap();
            let owner = navigation.owner.clone();
            let response = if download {
                Ok(NavigationLoadOutcome::download(
                    crate::conn::DownloadNavigation {
                        final_url: navigation.requested_url.clone(),
                        progress_transfer: CompletedDownloadProgressTransfer::new(
                            vec![1, 2, 3],
                            CompletedMainDocumentNetworkEvents::new(
                                "GET".into(),
                                Vec::new(),
                                None,
                                200,
                                vec![("Content-Type".into(), b"application/octet-stream".to_vec())],
                                Vec::new(),
                                Vec::new(),
                                false,
                                false,
                            ),
                        ),
                    },
                ))
            } else {
                Err(moli_fetch::FetchCancelled.into())
            };
            let completion = BackgroundNavigationCompletion::new(token, navigation, response);
            let first =
                ctx.with_fixture(|conn| conn.admit_background_navigation_completion(completion));
            let messages = ctx
                .route_fixture_background_turns(&mut completions, first)
                .await;
            let replies: Vec<_> = messages
                .iter()
                .filter(|message| message["id"] == command_id)
                .collect();
            assert_eq!(replies.len(), 1, "source terminates once: {messages:?}");
            if download && !successor {
                assert_eq!(replies[0]["result"]["isDownload"], true, "{messages:?}");
            } else {
                assert!(
                    replies[0].get("error").is_some()
                        || replies[0]["result"]["errorText"].is_string(),
                    "{replies:?}"
                );
            }
            ctx.with_fixture(|conn| {
                assert_eq!(
                    conn.renderer_page_residence_identity_for_owner(&owner),
                    current_page
                )
            });
            if let Some(replacement) = replacement {
                ctx.with_fixture(|conn| {
                    assert!({
                        conn.accepts_pending_document_navigation_for_owner(&owner, &replacement)
                    })
                });
            } else {
                ctx.with_fixture(|conn| {
                    assert!(!conn.has_pending_document_navigation_for_owner(&owner))
                });
            }
            assert!(
                completions.try_recv().is_err(),
                "accepted source output cannot synthesize a second completion"
            );
        }
    }
}
