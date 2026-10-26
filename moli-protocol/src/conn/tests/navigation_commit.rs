use super::*;
use crate::conn::{
    CapturedBody, DocumentNavigationToken, NavigationLoadOutcome, ProtocolStateRequestReceiver,
    ResponseCommitReady,
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
                // Configuration has completed, but no commit is admitted until
                // the actor handles this synchronous operation.
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
