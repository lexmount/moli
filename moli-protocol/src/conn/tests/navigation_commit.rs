use super::*;
use crate::conn::{
    CapturedBody, CdpCommandTaskStep, DocumentNavigationToken, NavigationLoadOutcome,
    ProtocolStateRequestReceiver, ResponseCommitReady,
};
use crate::domains::page::BackgroundNavigationCompletion;
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
                let facts =
                    std::iter::once((dcl, RendererDocumentLifecycleMilestone::DomContentLoaded))
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
                ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
                    .await;
                let mut context = CommandDispatchContext::default();
                let pending = start_owned_protocol_tail(&mut ctx.conn, *completion, &mut context);
                let mut wait = Box::pin(pending.wait());
                // The first three operations install the Page, claim original workers,
                // and commit protocol identity. Hold the next state operation so live
                // facts arrive after commit but before lifecycle publication.
                for _ in 0..3 {
                    assert!(wait.as_mut().now_or_never().is_none());
                    requests.recv().await.unwrap().apply(&mut ctx.conn);
                }
                assert!(
                    !ctx.conn
                        .accepts_pending_document_navigation_for_owner(&navigation.owner, &token)
                );
                if settle_in_gap {
                    assert!(ctx.conn.settle_background_navigation_completion(&token));
                }
                let (_, visible) = ctx
                    .conn
                    .ingest_renderer_document_lifecycle_events_for_owner(&navigation.owner, facts);
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
                        ctx.process_async(json!({
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
                        ctx.process_async(json!({
                            "id": 71, "sessionId": "SID-1",
                            "method": "Page.removeScriptToEvaluateOnNewDocument",
                            "params": {"identifier": identifier}
                        })).await;
                        ctx.expect_result(71, json!({}), Some("SID-1"));
                    } else {
                        ctx.process_async(json!({
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
