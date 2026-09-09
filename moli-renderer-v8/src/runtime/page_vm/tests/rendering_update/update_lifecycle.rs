// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test(flavor = "current_thread")]
async fn main_document_autofocus_runs_as_a_rendering_update_after_domcontentloaded() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/post-parse-autofocus").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
document.body.innerHTML = `<input id="defaultFocus" autofocus>`;
globalThis.__autofocusOrder = [];
document.addEventListener("DOMContentLoaded", () => {
  __autofocusOrder.push("dcl");
  Promise.resolve().then(() => __autofocusOrder.push("dcl-microtask"));
});
defaultFocus.addEventListener("focus", () => {
  __autofocusOrder.push("focus");
  Promise.resolve().then(() => __autofocusOrder.push("focus-microtask"));
});
"installed"
"#,
        )?;
        let owner =
            dispatch_main_document_domcontentloaded_for_rendering_test(&mut page_vm).await?;

        assert_eq!(
            page_vm.vm_mut().eval("__autofocusOrder.join('|')")?,
            "dcl|dcl-microtask",
            "DOMContentLoaded and its checkpoint must finish before autofocus"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("document.activeElement === defaultFocus")?,
            "false",
            "the lifecycle body must only publish autofocus rendering work"
        );

        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("post-parse autofocus should publish one exact rendering task");
        let (selected_owner, selected_kind) = claimed
            .rendering_update_owner_and_kind()
            .expect("rendering selector must preserve the exact task identity");
        assert_eq!(
            selected_owner.target().owner(),
            crate::native_bridge::WindowDocumentOwner::Frame(owner)
        );
        assert_eq!(
            selected_kind,
            RendererPageRenderingUpdateTaskKind::PostParseAutofocus
        );
        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__autofocusOrder.join('|')")?,
            "dcl|dcl-microtask|focus|focus-microtask",
            "the selected rendering task must own autofocus callback completion"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("document.activeElement === defaultFocus")?,
            "true"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("post-parse autofocus rendering-update test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn domcontentloaded_microtask_focus_prevents_autofocus_task_admission() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/manual-focus-before-autofocus").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
document.body.innerHTML = `
  <input id="defaultFocus" autofocus>
  <input id="manualFocus">
`;
document.addEventListener("DOMContentLoaded", () => {
  Promise.resolve().then(() => manualFocus.focus());
});
"installed"
"#,
        )?;
        dispatch_main_document_domcontentloaded_for_rendering_test(&mut page_vm).await?;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("document.activeElement === manualFocus")?,
            "true",
            "DOMContentLoaded's checkpoint must settle manual focus before admission"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::RenderingUpdate,
                )
                .is_none(),
            "a Document that acquired focus must not publish redundant autofocus work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("manual focus admission test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn claimed_autofocus_rendering_task_does_not_retarget_document_open_replacement() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/stale-post-parse-autofocus").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
document.body.innerHTML = `<input id="retiredFocus" autofocus>`;
globalThis.__retiredAutofocusEvents = 0;
retiredFocus.addEventListener("focus", () => __retiredAutofocusEvents++);
"installed"
"#,
        )?;
        let retired_owner =
            dispatch_main_document_domcontentloaded_for_rendering_test(&mut page_vm).await?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("retired Document should publish one exact autofocus task");
        let (claimed_owner, claimed_kind) = claimed
            .rendering_update_owner_and_kind()
            .expect("rendering claim must retain its exact owner and kind");
        assert_eq!(
            claimed_owner.target().owner(),
            crate::native_bridge::WindowDocumentOwner::Frame(retired_owner)
        );
        assert_eq!(
            claimed_kind,
            RendererPageRenderingUpdateTaskKind::PostParseAutofocus
        );

        page_vm.vm_mut().eval(
            r#"
document.open();
document.write('<!doctype html><body><input id="replacementFocus" autofocus></body>');
document.close();
"replaced"
"#,
        )?;
        let replacement_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement Document owner");
        assert_ne!(retired_owner, replacement_owner);

        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("String(__retiredAutofocusEvents)")?,
            "0",
            "a claimed old-Document task must not dispatch into the replacement"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("document.activeElement === replacementFocus")?,
            "false",
            "stale settlement must not reuse the payload against a colliding replacement"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale autofocus rendering-update test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn rendering_update_body_cleans_up_callbacks_before_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/rendering-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__renderingTaskBoundary = [];
document.addEventListener("scroll", () => {
  __renderingTaskBoundary.push("callback");
  Promise.resolve().then(() => {
    __renderingTaskBoundary.push("microtask");
    const script = document.createElement("script");
    script.textContent = "__renderingTaskBoundary.push('runtime-script')";
    document.body.appendChild(script);
  });
});
scrollTo(0, 10);
"queued"
"#,
        )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let task = page_vm
            .take_rendering_update_body_task_for_test()
            .expect("one exact rendering-update task should be ready");
        let body = page_vm.apply_selected_page_rendering_update_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageRenderingUpdateTargetEffect::DispatchedToCurrentOwner
        );
        assert_eq!(
            page_vm.vm_mut().eval("__renderingTaskBoundary.join('|')")?,
            "callback|microtask|runtime-script",
            "listener cleanup must drain reactions before the selected task completes"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "the rendering-update body must not consume unrelated runtime residence"
        );

        page_vm
            .finish_selected_page_task_completion(body.action.into_page_task_completion(), &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__renderingTaskBoundary.join('|')")?,
            "callback|microtask|runtime-script",
            "selected task completion must not repeat callback reactions or their inline scripts"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("rendering-update body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn rendering_update_without_a_live_event_target_only_checkpoints() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/rendering-missing-event-target").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
document.head.appendChild(document.createElement("style")).textContent = `
  @keyframes removed { from { left: 0px; } to { left: 10px; } }
  #removed-animation { position: relative; animation: removed 1s linear; }
`;
const removed = document.createElement("div");
removed.id = "removed-animation";
removed.addEventListener("animationstart", () => {
  throw new Error("a removed animation target must not receive its queued scan");
});
document.body.appendChild(removed);
removed.remove();
"queued-then-removed"
"#,
        )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let task = page_vm
            .take_rendering_update_body_task_for_test()
            .expect("the removed target's exact animation scan should remain queued");
        assert_eq!(
            task.kind(),
            RendererPageRenderingUpdateTaskKind::AnimationStartScan
        );
        let body = page_vm.apply_selected_page_rendering_update_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageRenderingUpdateTargetEffect::CurrentOwnerHadNoEventTarget
        );
        page_vm
            .finish_selected_page_task_completion(body.action.into_page_task_completion(), &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "a current task with no callback only owns the agent checkpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("rendering-update checkpoint-only test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn scroll_rendering_update_is_document_exact_across_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/scroll-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let before_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");

        page_vm.vm_mut().eval(
            r#"
globalThis.__retiredScrollEvents = 0;
document.addEventListener("scroll", () => __retiredScrollEvents++);
document.addEventListener("scrollend", () => __retiredScrollEvents++);
scrollTo(0, 10);
document.open();
document.write("<!doctype html><title>replacement</title>");
document.close();
"replaced"
"#,
        )?;
        let after_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner should exist");
        assert_ne!(before_document, after_document);
        assert_eq!(
            before_document.local_window_id,
            after_document.local_window_id
        );

        let stale = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("retired Document rendering work should remain an exact source entry");
        let (stale_owner, stale_kind) = stale
            .rendering_update_owner_and_kind()
            .expect("rendering selector must retain the retired task identity");
        assert_eq!(
            stale_kind,
            RendererPageRenderingUpdateTaskKind::DocumentScrollEvents
        );
        assert_eq!(
            stale_owner.target().owner(),
            crate::native_bridge::WindowDocumentOwner::Frame(before_document),
            "the claimed task must retain the retired exact Document"
        );
        page_vm
            .run_claimed_selected_page_task_for_test(stale, &loader)
            .await?;
        assert_eq!(page_vm.vm_mut().eval("String(__retiredScrollEvents)")?, "0");

        page_vm.vm_mut().eval(
            r#"
globalThis.__currentScrollEvents = [];
document.addEventListener("scroll", () => __currentScrollEvents.push("scroll"));
document.addEventListener("scrollend", () => __currentScrollEvents.push("scrollend"));
scrollTo(0, 20);
"queued-current"
"#,
        )?;
        let current = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("replacement Document rendering work should remain runnable");
        let (current_owner, selected_kind) = current
            .rendering_update_owner_and_kind()
            .expect("rendering selector must retain its exact owner and kind");
        assert_ne!(stale_owner, current_owner);
        assert_eq!(
            current_owner.target().owner(),
            crate::native_bridge::WindowDocumentOwner::Frame(after_document)
        );
        assert_eq!(
            selected_kind,
            RendererPageRenderingUpdateTaskKind::DocumentScrollEvents
        );
        page_vm
            .run_claimed_selected_page_task_for_test(current, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__currentScrollEvents.join('|')")?,
            "scroll|scrollend"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Document-exact rendering update test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn animation_rendering_update_is_document_exact_across_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/animation-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let before_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");

        page_vm.vm_mut().eval(
            r#"
document.head.appendChild(document.createElement("style")).textContent = `
  @keyframes retired { from { left: 0px; } to { left: 10px; } }
  #retired-animation { position: relative; animation: retired 1s linear; }
`;
document.body.innerHTML = `<div id="retired-animation"></div>`;
globalThis.__retiredAnimationEvents = 0;
document.getElementById("retired-animation").addEventListener(
  "animationstart",
  () => __retiredAnimationEvents++
);
document.open();
document.write("<!doctype html><title>replacement</title><body></body>");
document.close();
"replaced"
"#,
        )?;
        let after_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner should exist");
        assert_ne!(before_document, after_document);

        let stale = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("retired Document animation work should remain an exact source entry");
        let (stale_owner, stale_kind) = stale
            .rendering_update_owner_and_kind()
            .expect("rendering selector must retain the retired task identity");
        assert_eq!(
            stale_kind,
            RendererPageRenderingUpdateTaskKind::AnimationStartScan
        );
        assert_eq!(
            stale_owner.target().owner(),
            crate::native_bridge::WindowDocumentOwner::Frame(before_document),
            "the claimed task must retain the retired exact Document"
        );
        page_vm
            .run_claimed_selected_page_task_for_test(stale, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("String(__retiredAnimationEvents)")?,
            "0"
        );

        page_vm.vm_mut().eval(
            r#"
document.head.appendChild(document.createElement("style")).textContent = `
  @keyframes current { from { left: 0px; } to { left: 10px; } }
  #current-animation { position: relative; animation: current 1s linear; }
`;
document.body.innerHTML = `<div id="current-animation"></div>`;
globalThis.__currentAnimationEvents = 0;
document.getElementById("current-animation").addEventListener(
  "animationstart",
  () => __currentAnimationEvents++
);
"queued-current"
"#,
        )?;
        assert!(!page_vm.vm().has_ready_timeout());
        let current = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("replacement Document animation work should remain runnable");
        let (current_owner, selected_kind) = current
            .rendering_update_owner_and_kind()
            .expect("rendering selector must retain its exact owner and kind");
        assert_eq!(
            selected_kind,
            RendererPageRenderingUpdateTaskKind::AnimationStartScan
        );
        assert_ne!(stale_owner, current_owner);
        assert_eq!(
            current_owner.target().owner(),
            crate::native_bridge::WindowDocumentOwner::Frame(after_document)
        );
        page_vm
            .run_claimed_selected_page_task_for_test(current, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("String(__currentAnimationEvents)")?,
            "1"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Document-exact animation rendering update test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn scroll_rendering_update_discards_a_retired_child_document() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/scroll-stale-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "scroll-stale-child";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "scroll-stale-child")?;
        page_vm.vm_mut().eval(
            r#"
globalThis.__retiredChildScrollEvents = 0;
const staleFrame = document.getElementById("scroll-stale-child");
staleFrame.contentDocument.addEventListener(
  "scroll",
  () => parent.__retiredChildScrollEvents++
);
staleFrame.contentDocument.addEventListener(
  "scrollend",
  () => parent.__retiredChildScrollEvents++
);
staleFrame.contentWindow.scrollTo(0, 12);
staleFrame.remove();
"retired"
"#,
        )?;

        assert!(!page_vm.vm().has_ready_timeout());
        let stale = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("retired child rendering work should remain an exact source entry");
        let (stale_owner, stale_kind) = stale
            .rendering_update_owner_and_kind()
            .expect("rendering selector must retain the retired child task identity");
        assert_eq!(
            stale_kind,
            RendererPageRenderingUpdateTaskKind::DocumentScrollEvents
        );
        assert_eq!(
            stale_owner.root_document(),
            page_vm.document_lifecycle.identity().document
        );
        page_vm
            .run_claimed_selected_page_task_for_test(stale, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(__retiredChildScrollEvents)")?,
            "0"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::RenderingUpdate,
                )
                .is_none(),
            "stale settlement must retire the child Host-local payload"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child-Document-exact rendering update test should run");
}

#[test]
fn rendering_update_rejects_a_real_page_vm_replacement_id_collision() {
    run_page_vm_large_stack_async_test(
        "rendering-update-page-vm-replacement-id-collision",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/replacement.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><body>replacement</body>".to_owned(),
                Duration::ZERO,
            )])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let (page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    page_vm.vm_mut().eval("scrollTo(0, 10); 'queued-retired'")?;
                    let retired_root = page_vm.document_lifecycle.identity().document;

                    let replacement_url = format!("{base_url}/replacement.html");
                    page_vm
                        .vm_mut()
                        .eval(&format!("location.href = {replacement_url:?}; 'navigating'"))?;
                    let mut pending_document_lifecycle_turn = None;
                    let navigation = page_vm
                        .follow_pending_location_navigation_one_turn_async(
                            &mut pending_document_lifecycle_turn,
                            PageVmInitStage::Load,
                        )
                        .await?;
                    assert!(matches!(
                        navigation,
                        crate::runtime::PageVmFollowNavigationTurnOutcome::Completed
                            | crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                                ..
                            }
                    ));

                    let current_root = page_vm.document_lifecycle.identity().document;
                    assert_ne!(retired_root, current_root);
                    page_vm.vm_mut().eval(
                        r#"
globalThis.__replacementScrollEvents = [];
document.addEventListener("scroll", () => __replacementScrollEvents.push("scroll"));
document.addEventListener("scrollend", () => __replacementScrollEvents.push("scrollend"));
scrollTo(0, 20);
"queued-current"
"#,
                    )?;

                    let stale = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::RenderingUpdate,
                        )
                        .expect("retired PageVm rendering task should consume one stale turn");
                    let (stale_owner, stale_kind) = stale
                        .rendering_update_owner_and_kind()
                        .expect("rendering selector must retain the retired task identity");
                    assert_eq!(
                        stale_owner.root_document(),
                        retired_root,
                        "the first selected task must remain bound to the retired PageVm"
                    );
                    assert_eq!(
                        stale_kind,
                        RendererPageRenderingUpdateTaskKind::DocumentScrollEvents
                    );
                    page_vm
                        .run_claimed_selected_page_task_for_test(stale, &loader)
                        .await?;

                    let current = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::RenderingUpdate,
                        )
                        .expect("replacement rendering task must survive stale-head settlement");
                    let (selected_owner, _) = current
                        .rendering_update_owner_and_kind()
                        .expect("rendering selector must retain its exact owner");
                    assert_eq!(selected_owner.root_document(), current_root);
                    assert_ne!(stale_owner, selected_owner);
                    assert_eq!(
                        stale_owner.target(),
                        selected_owner.target(),
                        "fresh PageVm counters should naturally reuse the local Document target"
                    );
                    page_vm
                        .run_claimed_selected_page_task_for_test(current, &loader)
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__replacementScrollEvents.join('|')")?,
                        "scroll|scrollend"
                    );
                    assert!(!page_vm.vm().has_ready_timeout());
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("rendering update replacement should use exact root arbitration");
            server
                .await
                .expect("rendering update replacement server should finish");
        },
    );
}

#[tokio::test(flavor = "current_thread")]
async fn rendering_update_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/rendering-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__renderingChildOrder = [];
document.addEventListener("scroll", () => {
  __renderingChildOrder.push("callback");
  Promise.resolve().then(() => {
    __renderingChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "rendering-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
});
scrollTo(0, 15);
"queued"
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate, &loader)
                .await?,
            "the exact rendering update should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__renderingChildOrder.join('|')")?,
            "callback|microtask",
            "the agent checkpoint must precede callback child-record synchronization"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "a reaction-created srcdoc frame must publish a typed navigation commit during callback completion"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit),
            "the microtask-created frame must remain a concrete later Page task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("rendering-update post-checkpoint child synchronization test should run");
}
