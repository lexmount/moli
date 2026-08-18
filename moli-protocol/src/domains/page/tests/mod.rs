use super::{LOADER_ID, build_mhtml_snapshot, child_frame_security_identity};
use crate::conn::DocumentStartScript;
use crate::conn::{
    BackgroundTarget, BrowserContext, CdpCommandTaskStep, CdpSchedulerEvent, EmulatedDeviceMetrics,
    FetchInterceptionPattern, FetchRequestStage, NETWORK_ERROR_PAGE_URL, PendingCdpCommandDispatch,
    ServiceWorkerTargetState, URL_BASE,
};
use crate::devtools_runtime::{
    AutomationEvent, DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult,
    DevToolsGetFrameTreeCommand, DevToolsGetFrameTreesCommand, DevToolsProtocol, DevToolsTargetId,
    NavigationFrameEventKind,
};
use crate::testing::{
    TestContext, wait_until_frame_stopped_loading, wait_until_message, wait_until_messages,
    wait_until_renderer_document_load, wait_until_scheduler_message,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use moli_core::page::{
    RendererDocumentLifecycleEventKind, RendererDocumentLifecycleIdentity,
    RendererDocumentLifecycleMilestone, RendererDocumentToken, RendererFrameToken,
    RendererJavaScriptDialogCompletion, RendererJavaScriptDialogId, RendererJavaScriptDialogSource,
    RendererLifecycleEpoch, RendererPendingJavaScriptDialog, RendererServiceWorkerVersionStatus,
};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;

fn take_response_by_id(ctx: &mut TestContext, id: u64) -> serde_json::Value {
    let pos = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(id))
        .expect("expected a response with the requested id");
    ctx.sent.remove(pos)
}

fn renderer_dialog_source_document_for_test() -> RendererDocumentLifecycleIdentity {
    let page_id = moli_core::PageId::new_for_testing(77);
    RendererDocumentLifecycleIdentity {
        frame: RendererFrameToken { page_id },
        document: RendererDocumentToken::new_for_testing(page_id, 1),
        epoch: RendererLifecycleEpoch(1),
    }
}

fn renderer_dialog_for_test(
    frame_id: Option<&str>,
    dialog_type: &str,
    message: &str,
    default_prompt: &str,
    completion: Option<RendererJavaScriptDialogCompletion>,
) -> RendererPendingJavaScriptDialog {
    let source = frame_id.map_or(RendererJavaScriptDialogSource::RootFrame, |frame_id| {
        RendererJavaScriptDialogSource::ChildFrame {
            frame_id: frame_id.to_owned(),
            local_window_id: 1,
            document_id: 1,
        }
    });
    RendererPendingJavaScriptDialog::new(
        RendererJavaScriptDialogId::new(1),
        renderer_dialog_source_document_for_test(),
        source,
        "about:blank".to_owned(),
        dialog_type.to_owned(),
        message.to_owned(),
        default_prompt.to_owned(),
        completion,
    )
}

fn target_dialog_for_test(
    page_owner: crate::conn::TargetPageResidenceIdentity,
    frame_id: &str,
    dialog_type: &str,
    message: &str,
    default_prompt: &str,
    completion: Option<RendererJavaScriptDialogCompletion>,
) -> crate::conn::TargetJavaScriptDialog {
    crate::conn::TargetJavaScriptDialog::new(
        page_owner,
        frame_id.to_owned(),
        renderer_dialog_for_test(
            Some(frame_id),
            dialog_type,
            message,
            default_prompt,
            completion,
        ),
    )
}
async fn complete_pending_command_task_for_test(
    ctx: &mut TestContext,
    mut pending: PendingCdpCommandDispatch,
) -> (Vec<serde_json::Value>, Vec<CdpSchedulerEvent>) {
    loop {
        let completed = pending.wait().await;
        match ctx.conn.complete_pending_command_dispatch(completed).await {
            CdpCommandTaskStep::Pending(next) => pending = *next,
            CdpCommandTaskStep::Complete(outcome) => {
                return ctx.route_completed_command_outcome_for_test(outcome).await;
            }
        }
    }
}

async fn loaded_page_html_for_test(ctx: &mut TestContext) -> String {
    let page = ctx
        .conn
        .browser_context
        .as_mut()
        .and_then(|bc| bc.active_target.runtime_slot.loaded_page_mut())
        .expect("loaded page");
    page.serialize_html_async()
        .await
        .expect("loaded page should serialize HTML")
}

fn consume_main_document_navigation_start(ctx: &mut TestContext) {
    assert_eq!(ctx.take_one()["method"], "Page.frameStartedNavigating");
    assert_eq!(ctx.take_one()["method"], "Page.frameStartedLoading");
}
fn take_main_document_request_pause(ctx: &mut TestContext) -> serde_json::Value {
    consume_main_document_navigation_start(ctx);
    loop {
        let message = ctx.take_one();
        match message["method"].as_str() {
            Some("Fetch.requestPaused") => return message,
            Some("Network.requestWillBeSent") | Some("Network.requestWillBeSentExtraInfo") => {}
            other => {
                panic!("expected main-document Fetch.requestPaused, got {other:?}: {message:?}")
            }
        }
    }
}

fn take_main_document_response_pause_after_extra_info(
    ctx: &mut TestContext,
    network_id: &serde_json::Value,
    expected_status: u16,
) -> serde_json::Value {
    let request_extra_info = ctx.take_one();
    assert_eq!(
        request_extra_info["method"],
        "Network.requestWillBeSentExtraInfo"
    );
    assert_eq!(request_extra_info["params"]["requestId"], *network_id);

    let response_extra_info = ctx.take_one();
    assert_eq!(
        response_extra_info["method"],
        "Network.responseReceivedExtraInfo"
    );
    assert_eq!(response_extra_info["params"]["requestId"], *network_id);
    assert_eq!(response_extra_info["params"]["statusCode"], expected_status);

    let paused = ctx.take_one();
    assert_eq!(paused["method"], "Fetch.requestPaused");
    paused
}

fn load_bc_with_target(ctx: &mut TestContext, bc_id: &str, target_id: &str, url: &str) {
    let mut bc = BrowserContext::new(bc_id.into());
    bc.set_active_target_id(target_id);
    bc.set_target_url(url.into());
    ctx.conn.browser_context = Some(bc);
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
    bc.stage_active_target_demoting_current(
        target_id.to_owned(),
        Some(session_id.to_owned()),
        url.to_owned(),
        Some(url.to_owned()),
    );
    ctx.conn.browser_context = Some(bc);
}
fn load_bc_with_service_worker_target(ctx: &mut TestContext) {
    let mut bc = BrowserContext::new("BID-service-worker-frame-tree".to_owned());
    let target = ServiceWorkerTargetState::new(
        3,
        7,
        "TID-service-worker".to_owned(),
        "https://example.test/service-worker.js".to_owned(),
        "https://example.test/".to_owned(),
        RendererServiceWorkerVersionStatus::Activated,
        None,
    );
    bc.insert_service_worker_target(target);
    ctx.conn.browser_context = Some(bc);
}
async fn ensure_initial_document_for_session(ctx: &mut TestContext, session_id: Option<&str>) {
    let pending = ctx
        .conn
        .start_initial_document_page_ensure_for_session_owner(session_id)
        .expect("target lifecycle initial document ensure should start")
        .expect("metadata-only initial target should need an initial document page build");
    let completed = pending
        .wait()
        .await
        .expect("initial document page build should complete");
    let diagnostics = ctx
        .conn
        .complete_initial_document_page_build_for_owner_with_creation_diagnostics(completed)
        .await
        .expect("initial document should install on captured owner");
    if let Some(predecessor) = diagnostics.renderer_output_predecessor {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }
}
async fn child_frame_id_for_single_iframe(ctx: &mut TestContext, id: u64) -> String {
    ctx.process_async(json!({"id": id, "method": "Page.getFrameTree"}))
        .await;
    take_response_by_id(ctx, id)["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned()
}
async fn assert_child_frame_navigation_completion(
    ctx: &mut TestContext,
    child_frame_id: &str,
    expected_name: Option<&str>,
    expected_url: Option<&str>,
) {
    let child_frame_id_owned = child_frame_id.to_owned();
    wait_until_message(
        ctx,
        "SID-1",
        "child frame Page.frameNavigated",
        move |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id_owned)
                && expected_url.is_none_or(|expected_url| {
                    message["params"]["frame"]["url"] == json!(expected_url)
                })
        },
    )
    .await;
    let child_frame_id_owned = child_frame_id.to_owned();
    wait_until_message(
        ctx,
        "SID-1",
        "child frame Page.frameStoppedLoading",
        move |message| {
            message["method"] == json!("Page.frameStoppedLoading")
                && message["params"]["frameId"] == json!(child_frame_id_owned)
        },
    )
    .await;
    let child_navigated = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
                && expected_url.is_none_or(|expected_url| {
                    message["params"]["frame"]["url"] == json!(expected_url)
                })
        })
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "child frame should emit Page.frameNavigated; sent={:?}",
                ctx.sent
            )
        });
    if let Some(expected_name) = expected_name {
        assert_eq!(
            child_navigated["params"]["frame"]["name"],
            json!(expected_name)
        );
    }
    if let Some(expected_url) = expected_url {
        assert_eq!(
            child_navigated["params"]["frame"]["url"],
            json!(expected_url)
        );
    }
    let child_loader_id = child_navigated["params"]["frame"]["loaderId"]
        .as_str()
        .expect("child navigation loader id")
        .to_owned();

    let lifecycle_names = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!(child_frame_id)
                && message["params"]["loaderId"] == json!(child_loader_id)
        })
        .map(|message| {
            message["params"]["name"]
                .as_str()
                .expect("lifecycle name")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        lifecycle_names,
        vec![
            "init".to_owned(),
            "DOMContentLoaded".to_owned(),
            "load".to_owned(),
            "networkAlmostIdle".to_owned(),
            "networkIdle".to_owned(),
        ],
        "child lifecycle messages: {:?}",
        ctx.sent
            .iter()
            .filter(|message| {
                message["method"] == json!("Page.lifecycleEvent")
                    && message["params"]["frameId"] == json!(child_frame_id)
                    && message["params"]["loaderId"] == json!(child_loader_id)
            })
            .collect::<Vec<_>>()
    );
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Page.frameStoppedLoading")
                && message["params"]["frameId"] == json!(child_frame_id)
        }),
        "child frame should emit Page.frameStoppedLoading"
    );
}
async fn wait_for_child_frame_navigated_url(
    ctx: &mut TestContext,
    child_frame_id: &str,
    expected_url: &str,
) {
    let child_frame_id_owned = child_frame_id.to_owned();
    let expected_url_owned = expected_url.to_owned();
    wait_until_message(
        ctx,
        "SID-1",
        "child frame Page.frameNavigated for URL",
        move |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id_owned)
                && message["params"]["frame"]["url"] == json!(expected_url_owned)
        },
    )
    .await;
}
fn assert_child_frame_attached(
    ctx: &TestContext,
    child_frame_id: &str,
    expected_parent_frame_id: &str,
) {
    let attached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .unwrap_or_else(|| {
            panic!(
                "child frame should emit Page.frameAttached; sent={:?}",
                ctx.sent
            )
        });
    let navigated_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
        })
        .expect("child frame should emit Page.frameNavigated");
    assert_eq!(
        ctx.sent[attached_index]["params"]["parentFrameId"],
        json!(expected_parent_frame_id)
    );
    assert!(
        attached_index < navigated_index,
        "Page.frameAttached should precede Page.frameNavigated; sent={:?}",
        ctx.sent
    );
}

mod capture;
mod dialog;
mod document_content;
mod frame_tree;
mod lifecycle;
mod navigation;
mod resources;
mod runtime;
mod scripts;
