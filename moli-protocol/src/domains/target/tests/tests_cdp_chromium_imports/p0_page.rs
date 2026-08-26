use super::super::tests_cdp_smoke_fixture::SmokeFixtureServer;
use super::super::*;
use super::support::CdpPageHarness;
use crate::testing::wait_until_messages;
use serde_json::json;

// P0 browser contract source:
// docs/cdp-test-migration-roadmap-2026-05-14.md Page navigation / lifecycle.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_page_reload_emits_load_events() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 130_000).await;

    page.enable_page(&mut ctx, 130_005).await;
    page.navigate(&mut ctx, 130_006, fixture.url("/plain?reload-before"))
        .await;
    ctx.sent.clear();

    let reload = page
        .command(&mut ctx, 130_007, "Page.reload", json!({}))
        .await;
    assert_eq!(reload["result"], json!({}));
    wait_until_messages(
        &mut ctx,
        Some(page.session_id.as_str()),
        "Page.reload should emit load events",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Page.loadEventFired")
                    && message["sessionId"] == json!(page.session_id)
            }) && messages.iter().any(|message| {
                message["method"] == json!("Page.frameStoppedLoading")
                    && message["params"]["frameId"] == json!(page.target_id)
            })
        },
    )
    .await;
}

// P0 browser contract source:
// Chromium same-document navigation tests plus Playwright `page.goto(...#hash)` behavior.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_page_same_document_navigation_emits_navigated_within_document() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 131_000).await;

    page.enable_page(&mut ctx, 131_005).await;
    page.navigate(&mut ctx, 131_006, fixture.url("/plain?same-doc"))
        .await;
    ctx.sent.clear();

    let url = fixture.url("/plain?same-doc#hash");
    let response = page
        .evaluate_value(&mut ctx, 131_007, "location.hash = 'hash'; location.href")
        .await;
    assert_eq!(response["result"]["result"]["value"], json!(url));
    wait_until_messages(
        &mut ctx,
        Some(page.session_id.as_str()),
        "same-document navigation event",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Page.navigatedWithinDocument")
                    && message["params"]["frameId"] == json!(page.target_id)
                    && message["params"]["url"] == json!(url)
                    && message["params"]["navigationType"] == json!("fragment")
            })
        },
    )
    .await;
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Page.frameNavigated")),
        "{:?}",
        ctx.sent
    );
}

// P0 browser contract source:
// Chromium target/page termination contracts.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_page_close_destroys_target() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 131_999,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;
    ctx.expect_result(131_999, json!({}), None);

    let page = CdpPageHarness::attach(&mut ctx, 132_000).await;
    let tab_target_id = tab_id_for_page(&ctx, &page.target_id);
    ctx.process_async(json!({
        "id": 132_005,
        "method": "Target.setDiscoverTargets",
        "params": {
            "discover": true,
            "filter": [{}]
        }
    }))
    .await;
    ctx.expect_result(132_005, json!({}), None);
    ctx.take_first_matching("reported tab target", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
    });
    ctx.sent.clear();

    page.expect_empty_command(&mut ctx, 132_007, "Page.close", json!({}))
        .await;

    let messages = ctx.take_all();
    let inspector_detached = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Inspector.detached")
                && message["sessionId"] == json!(page.session_id)
        })
        .unwrap_or_else(|| panic!("missing Inspector.detached: {messages:?}"));
    let target_info_changed = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!(page.target_id)
                && message["params"]["targetInfo"]["attached"] == json!(false)
        })
        .unwrap_or_else(|| panic!("missing detached Target.targetInfoChanged: {messages:?}"));
    let target_detached = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Target.detachedFromTarget")
                && message["params"]["targetId"] == json!(page.target_id)
                && message["params"]["sessionId"] == json!(page.session_id)
                && message["params"].get("reason").is_none()
        })
        .unwrap_or_else(|| panic!("missing Target.detachedFromTarget: {messages:?}"));
    let target_destroyed = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Target.targetDestroyed")
                && message["params"]["targetId"] == json!(page.target_id)
        })
        .unwrap_or_else(|| panic!("missing Target.targetDestroyed: {messages:?}"));
    let tab_destroyed = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Target.targetDestroyed")
                && message["params"]["targetId"] == json!(tab_target_id)
        })
        .unwrap_or_else(|| panic!("missing tab Target.targetDestroyed: {messages:?}"));
    assert!(
        inspector_detached < target_info_changed
            && target_info_changed < target_detached
            && target_detached < target_destroyed
            && target_destroyed < tab_destroyed,
        "Page.close terminal event order should match Chromium: {messages:?}"
    );

    ctx.process_async(json!({
        "id": 132_008,
        "method": "Target.getTargets"
    }))
    .await;
    let response = ctx.take_response_by_id(132_008);
    let targets = response["result"]["targetInfos"]
        .as_array()
        .expect("targetInfos");
    assert!(
        targets
            .iter()
            .all(|target| target["targetId"] != json!(page.target_id)),
        "closed target must be removed from the target host registry: {targets:?}"
    );
}

// P0 browser contract source:
// Chromium target/page termination contracts.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_page_crash_emits_target_crashed() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 132_999,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;
    ctx.expect_result(132_999, json!({}), None);
    let page = CdpPageHarness::attach(&mut ctx, 133_000).await;

    page.enable_inspector(&mut ctx, 133_005).await;
    ctx.sent.clear();

    page.expect_empty_command(&mut ctx, 133_006, "Page.crash", json!({}))
        .await;

    let messages = ctx.take_all();
    assert!(
        messages.iter().any(|message| {
            message["method"] == json!("Target.targetCrashed")
                && message["params"]["targetId"] == json!(page.target_id)
                && message["params"]["status"] == json!("crashed")
                && message["params"]["errorCode"] == json!(1)
        }),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|message| {
            message["method"] == json!("Inspector.targetCrashed")
                && message["sessionId"] == json!(page.session_id)
        }),
        "{messages:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_page_crash_without_discovery_omits_target_crashed() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 133_100).await;
    ctx.process_async(json!({
        "id": 133_105,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": false }
    }))
    .await;
    ctx.expect_result(133_105, json!({}), None);
    ctx.sent.clear();

    page.expect_empty_command(&mut ctx, 133_106, "Page.crash", json!({}))
        .await;

    let messages = ctx.take_all();
    assert!(
        messages.iter().any(|message| {
            message["method"] == json!("Inspector.targetCrashed")
                && message["sessionId"] == json!(page.session_id)
        }),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .all(|message| message["method"] != json!("Target.targetCrashed")),
        "Target.targetCrashed requires a TargetHandler that reported the host: {messages:?}"
    );
}

// P0 browser contract source:
// Chromium Page.getFrameTree nested iframe visibility.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_page_get_frame_tree_includes_child_iframe() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 140_000).await;

    page.enable_page(&mut ctx, 140_005).await;
    page.navigate(&mut ctx, 140_006, fixture.url("/iframe"))
        .await;

    crate::testing::wait_until_message(
        &mut ctx,
        Some(page.session_id.as_str()),
        "child frame attachment after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(page.target_id)
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(page.target_id)
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("child frameAttached event: {:?}", ctx.sent));
    crate::testing::wait_until_message(
        &mut ctx,
        Some(page.session_id.as_str()),
        "child frame navigation commit before frame tree URL assertion",
        |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
                && message["params"]["frame"]["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/child"))
        },
    )
    .await;

    let frame_tree = page
        .command(&mut ctx, 140_007, "Page.getFrameTree", json!({}))
        .await;
    let child_frames = frame_tree["result"]["frameTree"]["childFrames"]
        .as_array()
        .unwrap_or_else(|| panic!("childFrames array: {frame_tree}"));
    assert_eq!(child_frames.len(), 1, "{frame_tree}");
    assert_eq!(
        frame_tree["result"]["frameTree"]["frame"]["id"],
        page.target_id
    );
    assert_eq!(child_frames[0]["frame"]["id"], json!(child_frame_id));
    assert!(
        child_frames[0]["frame"]["url"]
            .as_str()
            .is_some_and(|url| url.ends_with("/child")),
        "{frame_tree}"
    );
}

// P0 browser contract source:
// Chromium `InspectorPageAgent::FrameAttachedToParent` flushes Page.frameAttached
// before later lifecycle observations for the attached child frame.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_page_frame_attached_precedes_root_dcl_for_parser_iframe() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 145_000).await;

    page.enable_page(&mut ctx, 145_005).await;
    page.navigate(&mut ctx, 145_006, fixture.url("/iframe"))
        .await;
    wait_until_messages(
        &mut ctx,
        Some(page.session_id.as_str()),
        "parser iframe root DOMContentLoaded",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Page.domContentEventFired")
                    && message["sessionId"] == json!(page.session_id)
            })
        },
    )
    .await;

    let child_attached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(page.target_id)
        })
        .unwrap_or_else(|| panic!("child frameAttached event: {:?}", ctx.sent));
    let root_dcl_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.domContentEventFired")
                && message["sessionId"] == json!(page.session_id)
        })
        .unwrap_or_else(|| panic!("root domContentEventFired event: {:?}", ctx.sent));

    assert!(
        child_attached_index < root_dcl_index,
        "Page.frameAttached should be visible before root DCL; sent={:?}",
        ctx.sent
    );
}

// P0 browser contract source:
// Chromium child-frame lifecycle ordering before main load completion.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_page_child_frame_lifecycle_precedes_main_load() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 146_000).await;

    page.enable_page(&mut ctx, 146_005).await;
    page.navigate(&mut ctx, 146_006, fixture.url("/iframe"))
        .await;

    wait_until_messages(
        &mut ctx,
        Some(page.session_id.as_str()),
        "child frame attached before lifecycle assertions",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Page.frameAttached")
                    && message["params"]["parentFrameId"] == json!(page.target_id)
            })
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(page.target_id)
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("child frameAttached event: {:?}", ctx.sent));
    wait_until_messages(
        &mut ctx,
        Some(page.session_id.as_str()),
        "child frame lifecycle before main load assertions",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["id"] == json!(child_frame_id)
            }) && messages.iter().any(|message| {
                message["method"] == json!("Page.frameStoppedLoading")
                    && message["params"]["frameId"] == json!(child_frame_id)
            }) && messages
                .iter()
                .any(|message| message["method"] == json!("Page.loadEventFired"))
        },
    )
    .await;
    let child_attached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .unwrap_or_else(|| panic!("child frameAttached index: {:?}", ctx.sent));
    let child_navigated_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
        })
        .unwrap_or_else(|| panic!("child frameNavigated index: {:?}", ctx.sent));
    let child_stopped_loading_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameStoppedLoading")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .unwrap_or_else(|| panic!("child frameStoppedLoading index: {:?}", ctx.sent));
    let main_load_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.loadEventFired"))
        .unwrap_or_else(|| panic!("main loadEventFired index: {:?}", ctx.sent));

    assert!(
        child_attached_index < child_navigated_index,
        "child frameAttached should precede frameNavigated: {:?}",
        ctx.sent
    );
    assert!(
        child_navigated_index < main_load_index,
        "child frameNavigated should precede main loadEventFired: {:?}",
        ctx.sent
    );
    assert!(
        child_stopped_loading_index < main_load_index,
        "child frameStoppedLoading should precede main loadEventFired: {:?}",
        ctx.sent
    );
}
