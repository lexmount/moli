use super::*;
use crate::conn::BrowserContext;
use crate::testing::TestContext;
use moli_core::ace_page_task::{AcePageTask, AcePageTaskOutput, PageExtensionHost};
use serde_json::json;

#[derive(Debug)]
struct Probe {
    input: bool,
    fail: bool,
}
impl AcePageTask for Probe {
    fn is_input(&self) -> bool {
        self.input
    }
    fn run(self: Box<Self>, host: &mut dyn PageExtensionHost) -> anyhow::Result<AcePageTaskOutput> {
        anyhow::ensure!(!self.fail, "probe failure");
        Ok(AcePageTaskOutput {
            value: json!({"url":host.url()}),
            input_outcome: None,
        })
    }
}

async fn fixture() -> TestContext {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-task", "TID-task", "SID-task", "about:blank");
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url::Url::parse("https://example.test/task").unwrap(),
        "<!doctype html><p>fixture</p>".into(),
        Some("SID-task"),
    )
    .await;
    ctx.take_all();
    ctx
}

fn start(
    ctx: &TestContext,
    task: impl AcePageTask + 'static,
    context: &mut crate::conn::CommandDispatchContext,
) -> PendingAcePageTaskDispatch {
    let (pending, events) = ctx
        .state
        .with_state(|state| state.start_ace_page_task("TID-task", Box::new(task), context))
        .unwrap();
    assert!(events.is_empty(), "loaded fixture has no admission events");
    pending
}
async fn finish(
    ctx: &mut TestContext,
    completed: CompletedAcePageTaskDispatch,
    context: &mut crate::conn::CommandDispatchContext,
) -> Result<AcePageTaskResult, AcePageTaskError> {
    let (result, outcome) = ctx
        .state
        .with_state(|state| state.complete_ace_page_task(completed, context))
        .unwrap();
    let events = ctx
        .route_completed_command_outcome_for_test(outcome)
        .await
        .0;
    assert!(
        events.iter().all(|event| event.get("id").is_none()),
        "page tasks never publish a protocol response: {events:?}"
    );
    result
}

#[tokio::test(flavor = "current_thread")]
async fn ace_page_tasks_recover_after_failure_and_reject_stale_observations() {
    let mut ctx = fixture().await;
    let mut context = crate::conn::CommandDispatchContext::default();
    let failed = start(
        &ctx,
        Probe {
            input: false,
            fail: true,
        },
        &mut context,
    )
    .wait()
    .await;
    assert_eq!(
        finish(&mut ctx, failed, &mut context)
            .await
            .unwrap_err()
            .code,
        409
    );
    let ready = start(
        &ctx,
        Probe {
            input: false,
            fail: false,
        },
        &mut context,
    )
    .wait()
    .await;
    assert_eq!(
        finish(&mut ctx, ready, &mut context).await.unwrap().value["url"],
        "https://example.test/task"
    );
    for input in [false, true] {
        let mut ctx = fixture().await;
        let mut context = crate::conn::CommandDispatchContext::default();
        let completed = start(&ctx, Probe { input, fail: false }, &mut context)
            .wait()
            .await;
        let replacement = ctx
            .build_fixture_page("data:text/html,<p>Replacement</p>")
            .await
            .unwrap();
        ctx.with_fixture(|conn| {
            conn.browser_context
                .as_mut()
                .unwrap()
                .active_page_target_mut()
                .runtime_slot
                .replace_loaded_page(Some(replacement))
        });
        let result = finish(&mut ctx, completed, &mut context).await;
        if input {
            assert!(
                result.is_ok(),
                "input may complete after navigation: {result:?}"
            );
        } else {
            assert_eq!(result.unwrap_err().code, 409);
        }
    }
    let mut ctx = fixture().await;
    let mut context = crate::conn::CommandDispatchContext::default();
    let completed = start(
        &ctx,
        Probe {
            input: true,
            fail: false,
        },
        &mut context,
    )
    .wait()
    .await;
    drop(ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .take_page_target_for_close("TID-task")
    }));
    assert_eq!(
        finish(&mut ctx, completed, &mut context)
            .await
            .unwrap_err()
            .code,
        404,
        "a closed target cannot accept a late input completion"
    );
}

#[derive(Debug)]
struct CountRuns(std::sync::Arc<std::sync::atomic::AtomicUsize>);
impl AcePageTask for CountRuns {
    fn run(self: Box<Self>, _: &mut dyn PageExtensionHost) -> anyhow::Result<AcePageTaskOutput> {
        let runs = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        Ok(AcePageTaskOutput {
            value: json!({"runs":runs, "id":0, "result":null, "error":"payload"}),
            input_outcome: None,
        })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn ace_page_task_ready_receipt_loss_preserves_one_typed_result_and_one_execution() {
    let mut ctx = fixture().await;
    let mut context = crate::conn::CommandDispatchContext::default();
    let runs = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let pending = start(&ctx, CountRuns(runs.clone()), &mut context);
    drop(pending.wait().await);
    let completed = pending.wait().await;
    let result = finish(&mut ctx, completed, &mut context).await.unwrap();
    assert_eq!(
        result.value,
        json!({"runs":1, "id":0, "result":null, "error":"payload"})
    );
    assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
}

fn load_bc_with_session(
    ctx: &mut TestContext,
    bc_id: &str,
    target_id: &str,
    session_id: &str,
    url: &str,
) {
    let mut bc = BrowserContext::new(bc_id.into());
    // Use the same target-staging boundary as production. In particular,
    // about:blank must own an initial-empty-document record before its Page is
    // materialized; setting only target/session/url metadata creates a Page
    // with no exact renderer Document attachment and makes typed child-frame
    // output correctly fail authorization.
    bc.stage_foreground_target(
        target_id.to_owned(),
        Some(session_id.to_owned()),
        url.to_owned(),
        Some(url.to_owned()),
    );
    ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(bc));
    ctx.with_fixture(|conn| {
        conn.register_session_route_for_test(
            session_id,
            crate::conn::CdpSessionRoute::PageTarget {
                browser_context_id: bc_id.to_owned(),
                target_id: target_id.to_owned(),
                session_key: moli_page_types::DevToolsSessionKey::Primary,
            },
        )
    });
}
