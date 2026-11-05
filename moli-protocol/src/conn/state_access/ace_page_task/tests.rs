use super::*;
use crate::conn::BrowserContext;
use crate::testing::TestContext;
use moli_core::ace_page_task::{
    AcePageTask, AcePageTaskKind, AcePageTaskOutput, PageExtensionHost,
};
use serde_json::json;

#[derive(Debug)]
struct Probe {
    input: bool,
    fail: bool,
}
impl AcePageTask for Probe {
    fn kind(&self) -> AcePageTaskKind {
        if self.input {
            AcePageTaskKind::Input
        } else {
            AcePageTaskKind::ReadOnly
        }
    }
    fn run(self: Box<Self>, host: &mut dyn PageExtensionHost) -> anyhow::Result<AcePageTaskOutput> {
        anyhow::ensure!(!self.fail, "probe failure");
        Ok(AcePageTaskOutput {
            value: json!({"url":host.url()}),
        })
    }
}

async fn fixture() -> TestContext {
    fixture_with_markup("<!doctype html><p>fixture</p>").await
}

async fn fixture_with_markup(markup: &str) -> TestContext {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-task", "TID-task", "SID-task", "about:blank");
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url::Url::parse("https://example.test/task").unwrap(),
        markup.into(),
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
    finish_with_events(ctx, completed, context).await.0
}

async fn finish_with_events(
    ctx: &mut TestContext,
    completed: CompletedAcePageTaskDispatch,
    context: &mut crate::conn::CommandDispatchContext,
) -> (
    Result<AcePageTaskResult, AcePageTaskError>,
    Vec<serde_json::Value>,
) {
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
    (result, events)
}

#[derive(Debug)]
struct MultipleInputs {
    fail: bool,
}

impl AcePageTask for MultipleInputs {
    fn kind(&self) -> AcePageTaskKind {
        AcePageTaskKind::Input
    }

    fn run(self: Box<Self>, host: &mut dyn PageExtensionHost) -> anyhow::Result<AcePageTaskOutput> {
        for id in ["first", "file", "second"] {
            let handle = host.dom().element_handle_by_id(id).unwrap();
            host.activate(handle, 0.0, 0.0, 0)?;
        }
        host.key("keyup", "Shift", "ShiftLeft", "", 0, false, false)?;
        anyhow::ensure!(!self.fail, "task failed after inputs");
        Ok(AcePageTaskOutput {
            value: json!("done"),
        })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn ace_page_task_publishes_each_input_effect_even_after_task_error() {
    for fail in [false, true] {
        let mut ctx = fixture_with_markup(
            r#"<!doctype html><a id="first" download="first.txt" href="/first.txt">first</a>
            <input id="file" type="file" multiple>
            <a id="second" download="second.txt" href="/second.txt">second</a>"#,
        )
        .await;
        // Denied downloads still publish their lifecycle, without network or disk I/O.
        ctx.process_async(json!({
            "id": 1, "method": "Browser.setDownloadBehavior",
            "params": { "behavior": "deny", "eventsEnabled": true }
        }))
        .await;
        ctx.expect_result(1, json!({}), None);
        ctx.process_async(json!({
            "id": 2, "method": "Page.setInterceptFileChooserDialog", "sessionId": "SID-task",
            "params": { "enabled": true }
        }))
        .await;
        ctx.expect_result(2, json!({}), Some("SID-task"));
        ctx.take_all();
        let mut context = crate::conn::CommandDispatchContext::default();
        let completed = start(&ctx, MultipleInputs { fail }, &mut context)
            .wait()
            .await;
        let (result, events) = finish_with_events(&mut ctx, completed, &mut context).await;
        if fail {
            assert!(
                result
                    .unwrap_err()
                    .message
                    .contains("task failed after inputs")
            );
        } else {
            assert_eq!(result.unwrap().value, "done");
        }
        let effects = events
            .iter()
            .filter(|event| {
                event["method"] == "Browser.downloadWillBegin"
                    || event["method"] == "Page.fileChooserOpened"
            })
            .collect::<Vec<_>>();
        assert_eq!(
            effects.len(),
            3,
            "effects must survive task completion: {events:?}"
        );
        assert_eq!(effects[0]["params"]["suggestedFilename"], "first.txt");
        assert_eq!(effects[1]["method"], "Page.fileChooserOpened");
        assert_eq!(effects[1]["params"]["mode"], "selectMultiple");
        assert!(effects[1]["params"]["backendNodeId"].as_u64().unwrap() > 0);
        assert_eq!(effects[2]["params"]["suggestedFilename"], "second.txt");
    }
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
    fn kind(&self) -> AcePageTaskKind {
        AcePageTaskKind::ReadOnly
    }
    fn run(self: Box<Self>, _: &mut dyn PageExtensionHost) -> anyhow::Result<AcePageTaskOutput> {
        let runs = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        Ok(AcePageTaskOutput {
            value: json!({"runs":runs, "id":0, "result":null, "error":"payload"}),
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
