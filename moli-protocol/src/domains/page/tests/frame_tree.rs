use super::*;

async fn assert_initial_document_frame_tree_waits_for_navigation(popup: bool) {
    let mut ctx = TestContext::new();
    ctx.enable_background_navigation_scheduler_for_test();
    load_bc_with_target(&mut ctx, "BID-INITIAL-TREE", "TID-OPENER", "about:blank");
    ctx.install_navigation_fixture_for_session_owner("about:blank", None)
        .await;
    ctx.process_async(json!({
        "id": 2000,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true }
    }))
    .await;
    ctx.take_all();
    let url = "data:text/html,<title>initial-tree-committed</title>";
    let create = if popup {
        json!({
            "id": 2001,
            "method": "Runtime.evaluate",
            "params": { "expression": format!("void window.open({})", json!(url)) }
        })
    } else {
        json!({ "id": 2001, "method": "Target.createTarget", "params": { "url": url } })
    };
    ctx.process_async(create).await;
    let created = ctx.take_all();
    let attached = created
        .iter()
        .find(|message| {
            message["method"] == "Target.attachedToTarget"
                && message["params"]["targetInfo"]["url"] == url
        })
        .expect("initial target attachment");
    assert_eq!(attached["params"]["waitingForDebugger"], true);
    let session = attached["params"]["sessionId"].as_str().unwrap();
    let target = attached["params"]["targetInfo"]["targetId"]
        .as_str()
        .unwrap();
    ctx.process_async(json!({ "id": 2002, "method": "Page.enable", "sessionId": session }))
        .await;
    let enabled = ctx.take_all();

    if !popup {
        // Chromium reports waitingForDebugger for browser-created targets, but
        // their initial navigation is not gated on runIfWaitingForDebugger.
        let committed = if let Some(committed) = created.iter().chain(&enabled).find(|message| {
            message["method"] == "Page.frameNavigated"
                && message["sessionId"] == session
                && message["params"]["frame"]["url"] == url
        }) {
            committed.clone()
        } else {
            ctx.wait_for_scheduler_message("browser-created document commit", |message| {
                message["method"] == "Page.frameNavigated"
                    && message["sessionId"] == session
                    && message["params"]["frame"]["url"] == url
            })
            .await
        };
        ctx.process_async(json!({
            "id": 2005, "method": "Runtime.evaluate", "sessionId": session,
            "params": { "expression": "location.href", "returnByValue": true }
        }))
        .await;
        let evaluated = take_response_by_id(&mut ctx, 2005);
        assert_eq!(evaluated["result"]["result"]["value"], url, "{evaluated}");
        for (id, method) in [(2003, "Page.getFrameTree"), (2004, "Page.getResourceTree")] {
            ctx.process_async(json!({ "id": id, "method": method, "sessionId": session }))
                .await;
            let reply = take_response_by_id(&mut ctx, id);
            let frame = &reply["result"]["frameTree"]["frame"];
            assert_eq!(frame["id"], target);
            assert_eq!(frame["url"], url);
            assert_eq!(frame["loaderId"], committed["params"]["frame"]["loaderId"]);
        }
        return;
    }

    // Chromium's initial DocumentLoader URL serializes as ":", although the
    // initial DOM document has location.href == "about:blank". Playwright
    // uses this distinction to wait for the popup's first real commit.
    let mut initial_loader = None;
    for (id, method) in [(2003, "Page.getFrameTree"), (2004, "Page.getResourceTree")] {
        let raw = json!({ "id": id, "method": method, "sessionId": session }).to_string();
        let pending = ctx.conn.try_start_pending_command_dispatch(&raw).unwrap();
        assert_eq!(pending.kind_name(), "Native");
        let (messages, _) = ctx
            .complete_command_task_step_for_test(crate::conn::CdpCommandTaskStep::Pending(
                Box::new(pending),
            ))
            .await;
        let reply = messages.iter().find(|message| message["id"] == id).unwrap();
        let frame = &reply["result"]["frameTree"]["frame"];
        assert_eq!(frame["id"], target);
        assert_eq!(frame["url"], ":", "{method}: {reply}");
        if let Some(loader) = &initial_loader {
            assert_eq!(&frame["loaderId"], loader);
        } else {
            assert!(!frame["loaderId"].as_str().unwrap().is_empty());
            initial_loader = Some(frame["loaderId"].clone());
        }
    }
    ctx.process_async(json!({
        "id": 2005, "method": "Runtime.evaluate", "sessionId": session,
        "params": { "expression": "location.href", "returnByValue": true }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 2005)["result"]["result"]["value"],
        "about:blank"
    );
    ctx.process_async(json!({
        "id": 2006, "method": "Runtime.runIfWaitingForDebugger", "sessionId": session
    }))
    .await;
    let committed = ctx
        .wait_for_scheduler_message("first target document commit", |message| {
            message["method"] == "Page.frameNavigated"
                && message["sessionId"] == session
                && message["params"]["frame"]["url"] == url
        })
        .await;
    let committed_loader = committed["params"]["frame"]["loaderId"].clone();
    assert_ne!(Some(&committed_loader), initial_loader.as_ref());
    for (id, method) in [(2007, "Page.getFrameTree"), (2008, "Page.getResourceTree")] {
        ctx.process_async(json!({ "id": id, "method": method, "sessionId": session }))
            .await;
        let reply = take_response_by_id(&mut ctx, id);
        let frame = &reply["result"]["frameTree"]["frame"];
        assert_eq!(frame["url"], url);
        assert_eq!(frame["loaderId"], committed_loader);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_frame_tree_distinguishes_initial_document_from_first_commit() {
    tokio::task::LocalSet::new()
        .run_until(assert_initial_document_frame_tree_waits_for_navigation(
            true,
        ))
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_created_target_frame_tree_commits_without_startup_resume() {
    tokio::task::LocalSet::new()
        .run_until(assert_initial_document_frame_tree_waits_for_navigation(
            false,
        ))
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn initial_document_frame_tree_without_renderer_does_not_claim_the_destination_url() {
    let mut ctx = TestContext::new();
    load_bc_with_target(
        &mut ctx,
        "BID-INITIAL-TREE-METADATA",
        "TID-INITIAL-TREE-METADATA",
        "https://initial.test/not-committed",
    );
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .begin_active_target_initial_empty_document("about:blank".to_owned());
    for (id, method) in [(2010, "Page.getFrameTree"), (2011, "Page.getResourceTree")] {
        ctx.process_async(json!({ "id": id, "method": method }))
            .await;
        let reply = take_response_by_id(&mut ctx, id);
        let frame = &reply["result"]["frameTree"]["frame"];
        assert_eq!(frame["url"], ":");
        assert!(!frame["loaderId"].as_str().unwrap().is_empty());
    }
}

async fn complete_child_frame_lifecycle(ctx: &mut TestContext) {
    let owner = crate::conn::CommandOwnerScope::capture(&ctx.conn, None);
    let pending = ctx
        .conn
        .start_child_frame_lifecycle_work_for_owner(owner, std::time::Duration::from_secs(2))
        .expect("loaded page should expose child-frame lifecycle work");
    let completed = pending
        .wait()
        .await
        .expect("child-frame lifecycle work should complete");
    assert!(
        ctx.conn
            .complete_child_frame_lifecycle_work_for_session_owner(completed)
            .expect("child-frame lifecycle completion should apply"),
        "child-frame lifecycle should settle before inspecting final frame metadata"
    );
}

/// cdp.page: getFrameTree – no browser context
#[tokio::test(flavor = "multi_thread")]
async fn get_frame_tree_no_bc_error() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 10, "method": "Page.getFrameTree",
                           "params": {"targetId": "X"}}))
        .await;
    ctx.expect_error(10, -31998, "BrowserContextNotLoaded");
}
/// cdp.page: getFrameTree – target metadata exists without a current document
#[tokio::test(flavor = "multi_thread")]
async fn get_frame_tree_with_target_metadata_but_no_document() {
    let mut ctx = TestContext::new();
    load_bc_with_target(
        &mut ctx,
        "BID-9",
        "FID-000000000X",
        "http://127.0.0.1:9582/fixtures/hi.html",
    );
    ctx.process_async(json!({"id": 11, "method": "Page.getFrameTree"}))
        .await;
    ctx.expect_result(
        11,
        json!({
            "frameTree": {
                "frame": {
                    "id": "FID-000000000X",
                    "loaderId": "",
                    "url": "http://127.0.0.1:9582/fixtures/hi.html",
                    "domainAndRegistry": "",
                    "securityOrigin": crate::conn::URL_BASE,
                    "mimeType": "text/html",
                    "adFrameStatus": { "adFrameType": "none" },
                    "secureContextType": "Secure",
                    "crossOriginIsolatedContextType": "NotIsolated",
                    "gatedAPIFeatures": [],
                }
            }
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_tree_reports_the_current_committed_document_loader() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-FRAME-TREE-LOADER",
        "TID-FRAME-TREE-LOADER",
        "SID-FRAME-TREE-LOADER",
        "about:blank",
    );
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .begin_active_target_initial_empty_document("about:blank".to_owned());

    ctx.process_async(json!({
        "id": 1101,
        "method": "Page.getFrameTree",
        "sessionId": "SID-FRAME-TREE-LOADER"
    }))
    .await;
    let initial_loader_id =
        take_response_by_id(&mut ctx, 1101)["result"]["frameTree"]["frame"]["loaderId"]
            .as_str()
            .expect("initial document loaderId")
            .to_owned();

    ctx.process_async(json!({
        "id": 1102,
        "method": "Page.navigate",
        "sessionId": "SID-FRAME-TREE-LOADER",
        "params": { "url": "data:text/html,<title>navigated</title>" }
    }))
    .await;
    let navigation_loader_id = take_response_by_id(&mut ctx, 1102)["result"]["loaderId"]
        .as_str()
        .expect("Page.navigate loaderId")
        .to_owned();
    assert_ne!(
        initial_loader_id, navigation_loader_id,
        "a cross-document navigation must replace the initial DocumentLoader identity"
    );

    ctx.process_async(json!({
        "id": 1103,
        "method": "Page.getFrameTree",
        "sessionId": "SID-FRAME-TREE-LOADER"
    }))
    .await;
    let committed_loader_id =
        take_response_by_id(&mut ctx, 1103)["result"]["frameTree"]["frame"]["loaderId"]
            .as_str()
            .expect("committed document loaderId")
            .to_owned();
    assert_eq!(committed_loader_id, navigation_loader_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_tree_without_a_current_document_reports_an_empty_loader_id() {
    let mut ctx = TestContext::new();
    let target_id = "TID-FRAME-TREE-NO-DOCUMENT";
    let session_id = "SID-FRAME-TREE-NO-DOCUMENT";
    load_bc_with_session(
        &mut ctx,
        "BID-FRAME-TREE-NO-DOCUMENT",
        target_id,
        session_id,
        "about:blank",
    );
    let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
    browser_context.begin_active_target_initial_empty_document("about:blank".to_owned());
    browser_context.mark_target_initial_empty_document_exited(target_id);

    ctx.process_async(json!({
        "id": 1104,
        "method": "Page.getFrameTree",
        "sessionId": session_id
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 1104);
    assert_eq!(
        response["result"]["frameTree"]["frame"]["loaderId"],
        json!(""),
        "a missing DocumentLoader must not borrow a loader identity from another document"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_tree_includes_top_level_child_frames_from_loaded_page() {
    let mut ctx = TestContext::new();
    let page_url = "data:text/html,<iframe name='blank-child'></iframe><iframe name='srcdoc-child' srcdoc=\"<p>child</p>\"></iframe>";
    load_bc_with_target(&mut ctx, "BID-10", "FID-000000000Y", page_url);
    ctx.install_navigation_fixture_for_session_owner(page_url, None)
        .await;
    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        bc.set_target_security_origin("https://top.example".into());
        bc.set_target_secure_context_type("Secure".into());
    }
    wait_until_renderer_document_load(&mut ctx, None, "FID-000000000Y", LOADER_ID).await;

    ctx.process_async(json!({"id": 12, "method": "Page.getFrameTree"}))
        .await;
    let response = take_response_by_id(&mut ctx, 12);
    let child_frames = response["result"]["frameTree"]["childFrames"]
        .as_array()
        .expect("frame tree should include child frames");
    assert_eq!(child_frames.len(), 2);

    let first_frame = &child_frames[0]["frame"];
    assert!(
        first_frame["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("child-browsing-context-"))
    );
    assert_eq!(first_frame["name"], json!("blank-child"));
    assert_eq!(first_frame["parentId"], json!("FID-000000000Y"));
    assert_eq!(first_frame["url"], json!("about:blank"));
    assert_eq!(first_frame["securityOrigin"], json!("://"));
    assert_eq!(first_frame["secureContextType"], json!("Secure"));
    let first_loader_id = first_frame["loaderId"]
        .as_str()
        .expect("blank child loaderId")
        .to_owned();
    assert!(!first_loader_id.is_empty());

    let second_frame = &child_frames[1]["frame"];
    assert!(
        second_frame["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("child-browsing-context-"))
    );
    assert_eq!(second_frame["name"], json!("srcdoc-child"));
    assert_eq!(second_frame["parentId"], json!("FID-000000000Y"));
    assert_eq!(second_frame["url"], json!("about:srcdoc"));
    assert_eq!(second_frame["securityOrigin"], json!("://"));
    assert_eq!(second_frame["secureContextType"], json!("Secure"));
    let second_loader_id = second_frame["loaderId"]
        .as_str()
        .expect("srcdoc child loaderId")
        .to_owned();
    assert!(!second_loader_id.is_empty());
    assert_ne!(first_loader_id, second_loader_id);

    ctx.process_async(json!({"id": 14, "method": "Page.getFrameTree"}))
        .await;
    let repeated_response = take_response_by_id(&mut ctx, 14);
    let repeated_children = repeated_response["result"]["frameTree"]["childFrames"]
        .as_array()
        .expect("repeated frame tree should include child frames");
    assert_eq!(
        repeated_children[0]["frame"]["loaderId"],
        json!(first_loader_id)
    );
    assert_eq!(
        repeated_children[1]["frame"]["loaderId"],
        json!(second_loader_id)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_tree_uses_owner_element_id_when_frame_name_is_empty() {
    let mut ctx = TestContext::new();
    let page_url = "data:text/html,<iframe id='id-only' srcdoc=\"<p>child</p>\"></iframe>";
    load_bc_with_target(&mut ctx, "BID-ID-FALLBACK", "FID-ID-FALLBACK", page_url);
    let page = ctx
        .conn
        .load_page_via_runtime_async(page_url)
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .replace_loaded_page(Some(page));
    complete_child_frame_lifecycle(&mut ctx).await;

    ctx.process_async(json!({"id": 15, "method": "Page.getFrameTree"}))
        .await;
    let response = take_response_by_id(&mut ctx, 15);
    let child_frame = &response["result"]["frameTree"]["childFrames"][0]["frame"];
    assert_eq!(child_frame["name"], json!("id-only"));
    assert_eq!(child_frame["parentId"], json!("FID-ID-FALLBACK"));
    assert_eq!(child_frame["url"], json!("about:srcdoc"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_tree_projects_sandboxed_about_blank_from_document_url() {
    let mut ctx = TestContext::new();
    let page_url =
        "data:text/html,<iframe name='sandboxed-blank' sandbox src='about:blank'></iframe>";
    load_bc_with_target(&mut ctx, "BID-SANDBOX-BLANK", "TID-SANDBOX-BLANK", page_url);
    let page = ctx
        .conn
        .load_page_via_runtime_async(page_url)
        .await
        .expect("page should load");
    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        let _ = bc
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(Some(page));
        bc.set_target_security_origin("https://top.example".into());
        bc.set_target_secure_context_type("Secure".into());
    }

    ctx.process_async(json!({"id": 1204, "method": "Page.getFrameTree"}))
        .await;
    let response = take_response_by_id(&mut ctx, 1204);
    let child_frame = &response["result"]["frameTree"]["childFrames"][0]["frame"];
    assert_eq!(child_frame["name"], json!("sandboxed-blank"));
    assert_eq!(child_frame["url"], json!("about:blank"));
    // Chromium Page.Frame derives this field from the DocumentLoader URL
    // rather than exposing the sandboxed document's live opaque origin.
    assert_eq!(child_frame["securityOrigin"], json!("://"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_tree_recurses_into_nested_child_frames() {
    let mut ctx = TestContext::new();
    let page_url = r#"data:text/html,<iframe name="outer" srcdoc="<iframe name='inner' srcdoc='<p>nested</p>'></iframe>"></iframe>"#;
    load_bc_with_target(&mut ctx, "BID-11", "FID-000000000Z", page_url);
    let page = ctx
        .conn
        .load_page_via_runtime_async(page_url)
        .await
        .expect("page should load");
    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        let _ = bc
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(Some(page));
        bc.set_target_security_origin("https://top.example".into());
        bc.set_target_secure_context_type("Secure".into());
    }
    complete_child_frame_lifecycle(&mut ctx).await;

    ctx.process_async(json!({"id": 13, "method": "Page.getFrameTree"}))
        .await;
    let response = take_response_by_id(&mut ctx, 13);
    let child_frames = response["result"]["frameTree"]["childFrames"]
        .as_array()
        .expect("frame tree should include child frames");
    assert_eq!(child_frames.len(), 1);

    let outer_frame = &child_frames[0];
    let outer_id = outer_frame["frame"]["id"]
        .as_str()
        .expect("outer frame id should be present");
    assert_eq!(outer_frame["frame"]["name"], json!("outer"));
    assert_eq!(outer_frame["frame"]["parentId"], json!("FID-000000000Z"));
    assert_eq!(outer_frame["frame"]["url"], json!("about:srcdoc"));
    let outer_loader_id = outer_frame["frame"]["loaderId"]
        .as_str()
        .expect("outer frame loaderId");
    assert!(!outer_loader_id.is_empty());

    let nested_frames = outer_frame["childFrames"]
        .as_array()
        .expect("outer frame should include nested child frames");
    assert_eq!(nested_frames.len(), 1);

    let inner_frame = &nested_frames[0]["frame"];
    assert_eq!(inner_frame["name"], json!("inner"));
    assert_eq!(inner_frame["parentId"], json!(outer_id));
    assert_eq!(inner_frame["url"], json!("about:srcdoc"));
    assert_ne!(inner_frame["id"], json!(outer_id));
    assert_ne!(inner_frame["id"], json!("FID-000000000Z"));
    let inner_loader_id = inner_frame["loaderId"]
        .as_str()
        .expect("inner frame loaderId");
    assert!(!inner_loader_id.is_empty());
    assert_ne!(inner_loader_id, outer_loader_id);
    assert_eq!(inner_frame["securityOrigin"], json!("://"));
    assert_eq!(inner_frame["secureContextType"], json!("Secure"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_tree_projects_nested_sandboxed_srcdoc_from_document_urls() {
    let mut ctx = TestContext::new();
    let page_url = r#"data:text/html,<iframe sandbox srcdoc="<iframe srcdoc='<p>nested</p>'></iframe>"></iframe>"#;
    load_bc_with_target(&mut ctx, "BID-NESTED-ORIGIN", "FID-NESTED-ORIGIN", page_url);
    let page = ctx
        .conn
        .load_page_via_runtime_async(page_url)
        .await
        .expect("page should load");
    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        let _ = bc
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(Some(page));
        bc.set_target_security_origin("https://top.example".into());
        bc.set_target_secure_context_type("Secure".into());
    }
    complete_child_frame_lifecycle(&mut ctx).await;

    ctx.process_async(json!({"id": 16, "method": "Page.getFrameTree"}))
        .await;
    let response = take_response_by_id(&mut ctx, 16);
    let outer_frame = &response["result"]["frameTree"]["childFrames"][0];
    let inner_frame = &outer_frame["childFrames"][0]["frame"];
    assert_eq!(outer_frame["frame"]["securityOrigin"], json!("://"));
    assert_eq!(inner_frame["securityOrigin"], json!("://"));
    assert_eq!(inner_frame["parentId"], outer_frame["frame"]["id"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn async_get_frame_tree_includes_top_level_child_frames_from_loaded_page() {
    let mut ctx = TestContext::new();
    let page_url = "data:text/html,<iframe name='blank-child'></iframe><iframe name='srcdoc-child' srcdoc=\"<p>child</p>\"></iframe>";
    load_bc_with_target(&mut ctx, "BID-10-ASYNC", "FID-000000000YA", page_url);
    ctx.install_navigation_fixture_for_session_owner(page_url, None)
        .await;
    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        bc.set_target_security_origin("https://top.example".into());
        bc.set_target_secure_context_type("Secure".into());
    }
    wait_until_renderer_document_load(&mut ctx, None, "FID-000000000YA", LOADER_ID).await;

    ctx.process_async(json!({"id": 1201, "method": "Page.getFrameTree"}))
        .await;
    let response = take_response_by_id(&mut ctx, 1201);
    let child_frames = response["result"]["frameTree"]["childFrames"]
        .as_array()
        .expect("frame tree should include child frames");
    assert_eq!(child_frames.len(), 2);

    let first_frame = &child_frames[0]["frame"];
    assert!(
        first_frame["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("child-browsing-context-"))
    );
    assert_eq!(first_frame["name"], json!("blank-child"));
    assert_eq!(first_frame["parentId"], json!("FID-000000000YA"));
    assert_eq!(first_frame["url"], json!("about:blank"));
    assert_eq!(first_frame["securityOrigin"], json!("://"));
    assert_eq!(first_frame["secureContextType"], json!("Secure"));
    assert!(
        first_frame["loaderId"]
            .as_str()
            .is_some_and(|loader_id| !loader_id.is_empty())
    );

    let second_frame = &child_frames[1]["frame"];
    assert!(
        second_frame["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("child-browsing-context-"))
    );
    assert_eq!(second_frame["name"], json!("srcdoc-child"));
    assert_eq!(second_frame["parentId"], json!("FID-000000000YA"));
    assert_eq!(second_frame["url"], json!("about:srcdoc"));
    assert_eq!(second_frame["securityOrigin"], json!("://"));
    assert_eq!(second_frame["secureContextType"], json!("Secure"));
    assert!(
        second_frame["loaderId"]
            .as_str()
            .is_some_and(|loader_id| !loader_id.is_empty())
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn get_frame_tree_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    let page_url = "data:text/html,<iframe name='pending-child'></iframe>";
    load_bc_with_session(
        &mut ctx,
        "BID-PENDING-FRAME-TREE",
        "TID-PENDING-FRAME-TREE",
        "SID-PENDING-FRAME-TREE",
        page_url,
    );
    let page = ctx
        .conn
        .load_page_via_runtime_async(page_url)
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .replace_loaded_page(Some(page));

    let raw = json!({
        "id": 1203,
        "method": "Page.getFrameTree",
        "sessionId": "SID-PENDING-FRAME-TREE"
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("Page.getFrameTree should start as a pending command for loaded pages");
    let (messages, scheduler_events) =
        complete_pending_command_task_for_test(&mut ctx, pending).await;
    assert!(
        scheduler_events.is_empty(),
        "Page.getFrameTree should not enqueue scheduler events: {scheduler_events:?}"
    );
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(1203))
        .expect("Page.getFrameTree response");
    assert_eq!(response["sessionId"], json!("SID-PENDING-FRAME-TREE"));
    assert_eq!(
        response["result"]["frameTree"]["frame"]["id"],
        json!("TID-PENDING-FRAME-TREE")
    );
    assert_eq!(
        response["result"]["frameTree"]["childFrames"][0]["frame"]["name"],
        json!("pending-child")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn completed_get_frame_tree_survives_page_unload_before_projection() {
    let mut ctx = TestContext::new();
    let page_url = "data:text/html,<iframe name='pending-child'></iframe>";
    load_bc_with_session(
        &mut ctx,
        "BID-PENDING-FRAME-TREE-UNLOAD",
        "TID-PENDING-FRAME-TREE-UNLOAD",
        "SID-PENDING-FRAME-TREE-UNLOAD",
        page_url,
    );
    let page = ctx
        .conn
        .load_page_via_runtime_async(page_url)
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .replace_loaded_page(Some(page));

    let raw = json!({
        "id": 1206,
        "method": "Page.getFrameTree",
        "sessionId": "SID-PENDING-FRAME-TREE-UNLOAD"
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("Page.getFrameTree should start as a pending command for loaded pages");
    // The producer has committed a complete snapshot; protocol has not
    // projected it yet. Removing the Page now must neither rewrite that
    // result to an empty tree nor install its state onto a replacement Page.
    let completed = pending.wait().await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .clear_loaded_page_for_test_fixture();

    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("a published frame tree must require no further renderer work");
    };
    let (messages, scheduler_events) = ctx.route_completed_command_outcome_for_test(outcome).await;
    assert!(
        scheduler_events.is_empty(),
        "Page.getFrameTree should not enqueue scheduler events: {scheduler_events:?}"
    );
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(1206))
        .expect("Page.getFrameTree response");
    assert_eq!(
        response["sessionId"],
        json!("SID-PENDING-FRAME-TREE-UNLOAD")
    );
    assert_eq!(
        response["result"]["frameTree"]["frame"]["id"],
        json!("TID-PENDING-FRAME-TREE-UNLOAD")
    );
    assert_eq!(
        response["result"]["frameTree"]["frame"]["url"],
        json!(page_url)
    );
    assert_eq!(
        response["result"]["frameTree"]["childFrames"][0]["frame"]["name"],
        json!("pending-child")
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["id"] == json!(1206))
            .count(),
        1,
        "a committed snapshot must have exactly one response: {messages:#?}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn get_frame_tree_targets_loaded_background_owner_without_activation() {
    let mut ctx = TestContext::new();
    let page_url = "data:text/html,<iframe name='background-child'></iframe>";
    let background = PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut bc = BrowserContext::new("BID-1".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.set_target_url("data:text/html,<body>active</body>".to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.install_navigation_fixture_for_session_owner(page_url, Some("SID-background"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1202,
        "method": "Page.getFrameTree",
        "sessionId": "SID-background"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1202);
    assert_eq!(
        response["result"]["frameTree"]["frame"]["id"],
        json!("TID-background")
    );
    assert_eq!(
        response["result"]["frameTree"]["frame"]["url"],
        json!(page_url)
    );
    assert_eq!(
        response["result"]["frameTree"]["childFrames"][0]["frame"]["name"],
        json!("background-child")
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Page.getFrameTree should not activate the target"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn get_frame_tree_targets_inactive_loaded_owner_without_activation() {
    let mut ctx = TestContext::new();
    let page_url = "data:text/html,<iframe name='inactive-child'></iframe>";
    let mut inactive = BrowserContext::new("BID-inactive".to_owned());
    inactive.set_active_target_id("TID-inactive".to_owned());
    inactive.attach_active_session("SID-inactive".to_owned());
    inactive.set_target_url("about:blank".to_owned());
    ctx.conn
        .push_inactive_browser_context_fixture_for_test(inactive);
    ctx.install_navigation_fixture_for_session_owner(page_url, Some("SID-inactive"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1203,
        "method": "Page.getFrameTree",
        "sessionId": "SID-inactive"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1203);
    assert_eq!(
        response["result"]["frameTree"]["frame"]["id"],
        json!("TID-inactive")
    );
    assert_eq!(
        response["result"]["frameTree"]["childFrames"][0]["frame"]["name"],
        json!("inactive-child")
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "inactive Page.getFrameTree should not activate its browser context"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_tree_targets_service_worker_as_synthetic_context() {
    let mut ctx = TestContext::new();
    load_bc_with_service_worker_target(&mut ctx);

    let (result, scheduler_events) = ctx
        .conn
        .execute_automation_command(AutomationCommand::GetFrameTree(
            DevToolsGetFrameTreeCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: None,
                    target_id: Some(DevToolsTargetId::from("TID-service-worker")),
                    browser_context_id: None,
                },
                max_depth: None,
            },
        ))
        .await
        .into_parts();

    assert!(scheduler_events.is_empty());
    let AutomationResult::GetFrameTree(result) =
        result.expect("GetFrameTree should target service worker")
    else {
        panic!("expected GetFrameTree result");
    };
    assert_eq!(
        result.frame_tree,
        json!({
            "frame": {
                "id": "TID-service-worker",
                "url": "https://example.test/service-worker.js"
            }
        })
    );
    assert_eq!(
        result.target_info.as_ref().map(|info| info.kind),
        Some(crate::automation::DevToolsTargetKind::ServiceWorker)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_trees_includes_service_worker_target() {
    let mut ctx = TestContext::new();
    load_bc_with_service_worker_target(&mut ctx);

    let (result, scheduler_events) = ctx
        .conn
        .execute_automation_command(AutomationCommand::GetFrameTrees(
            DevToolsGetFrameTreesCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: None,
                    target_id: None,
                    browser_context_id: None,
                },
                max_depth: None,
            },
        ))
        .await
        .into_parts();

    assert!(scheduler_events.is_empty());
    let AutomationResult::GetFrameTrees(result) =
        result.expect("GetFrameTrees should include service worker targets")
    else {
        panic!("expected GetFrameTrees result");
    };
    assert_eq!(result.frame_trees.len(), 1);
    assert_eq!(
        result.frame_trees[0].frame_tree["frame"]["id"],
        json!("TID-service-worker")
    );
    assert_eq!(
        result.frame_trees[0].frame_tree["frame"]["url"],
        json!("https://example.test/service-worker.js")
    );
    assert_eq!(
        result.frame_trees[0]
            .target_info
            .as_ref()
            .map(|info| info.kind),
        Some(crate::automation::DevToolsTargetKind::ServiceWorker)
    );
}
