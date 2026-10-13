use super::*;
use crate::{
    css_resource_urls::{
        CompletedStylesheetWebFont, StylesheetLoadBlockingResource,
        stylesheet_load_blocking_font_resources,
    },
    protocol_types::OptionalResourceFetchMask,
    script_vm::web_fonts::DocumentWebFontCompletion,
};
use moli_page_types::LayoutPolicy;

fn font_resource() -> StylesheetLoadBlockingResource {
    stylesheet_load_blocking_font_resources(
        "@font-face{font-family:RealtimeSourceFont;src:url(font.woff2)}",
        &Url::parse("https://realtime-font.test/style.css").unwrap(),
        OptionalResourceFetchMask::FONT,
    )
    .into_iter()
    .next()
    .unwrap()
}

fn font_response(resource: &StylesheetLoadBlockingResource) -> CompletedStylesheetWebFont {
    let (_, request) = resource.clone().into_parts();
    CompletedStylesheetWebFont::response(
        request.unwrap(),
        include_bytes!("../../../../moli-layout/tests/fixtures/moli-ahem.woff2").to_vec(),
    )
}

#[test]
fn realtime_layout_font_completion_and_revocation_refresh_without_dom_writes() {
    let mut vm = new_parsed_test_vm(
        "https://realtime-font.test/",
        "<!doctype html><style>@font-face{font-family:RealtimeSourceFont;src:url(font.woff2)}</style><span id=target style='display:inline-block;font:20px RealtimeSourceFont,serif'>iiii</span>",
    );
    vm.set_layout_policy(LayoutPolicy::Realtime);
    let document = vm._context_host.borrow().document_handle();
    let resource = vm
        ._context_host
        .borrow()
        .admit_document_web_font(document, font_resource())
        .unwrap();
    let fallback = vm.eval("String(target.offsetWidth)").unwrap();
    assert_ne!(
        fallback, "48",
        "the fixture must start with fallback advances"
    );
    let before = vm.layout_pass_observability_for_test().1;
    let dom_version = vm._context_host.borrow().dom_host().dom_version();

    assert!(matches!(
        vm._context_host
            .borrow()
            .complete_document_web_font(document, font_response(&resource)),
        DocumentWebFontCompletion::Registered(moli_layout::WebFontRegistrationOutcome::Added)
    ));
    assert_eq!(
        vm._context_host.borrow().dom_host().dom_version(),
        dom_version
    );
    // The project font has a 600/1000-em advance: 4 glyphs * 20px * 0.6.
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "48");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);

    assert!(matches!(
        vm._context_host
            .borrow()
            .complete_document_web_font(document, font_response(&resource)),
        DocumentWebFontCompletion::Registered(moli_layout::WebFontRegistrationOutcome::Unchanged)
    ));
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "48");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);

    vm.eval("document.styleSheets[0].deleteRule(0)").unwrap();
    vm._context_host
        .borrow()
        .retain_document_web_font_slots(document, []);
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), fallback);
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    assert!(matches!(
        vm._context_host
            .borrow()
            .complete_document_web_font(document, font_response(&resource)),
        DocumentWebFontCompletion::Stale
    ));
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), fallback);
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    assert_eq!(
        vm._context_host.borrow().dom_host().dom_version(),
        dom_version
    );
}

#[test]
fn realtime_layout_canvas_pixels_do_not_invalidate_geometry() {
    let mut vm = new_parsed_test_vm(
        "https://realtime-canvas.test/",
        "<!doctype html><canvas id=target width=100 height=50></canvas>",
    );
    vm.set_layout_policy(LayoutPolicy::Realtime);
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    let before = vm.layout_pass_observability_for_test().1;
    vm.eval("target.getContext('2d').fillRect(0,0,20,20)")
        .unwrap();
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    assert_eq!(
        vm.layout_pass_observability_for_test()
            .3
            .unwrap()
            .paint_operation_count,
        0
    );
}

#[test]
fn child_web_font_document_open_retires_registrations_and_rejects_old_owner() {
    let mut vm = new_parsed_test_vm(
        "https://realtime-font.test/",
        "<!doctype html><span id=parentTarget style='display:inline-block;font:20px RealtimeSourceFont,serif'>iiii</span><iframe id=frame></iframe>",
    );
    vm.set_layout_policy(LayoutPolicy::Realtime);
    let rewrite = "globalThis.child=frame.contentWindow;child.document.open();\
        child.document.write('<!doctype html><style>@font-face{font-family:RealtimeSourceFont;src:url(font.woff2)}</style><span id=target style=\"display:inline-block;font:20px RealtimeSourceFont,serif\">iiii</span>');\
        child.document.close();globalThis.childTarget=child.document.getElementById('target')";
    vm.eval(rewrite).unwrap();
    let (document, binding, resource) = {
        let mut host = vm._context_host.borrow_mut();
        let frame = host.dom_host().element_handle_by_id("frame").unwrap();
        let document = host.child_browsing_context_document_handle(frame).unwrap();
        let binding = host
            .accept_current_stylesheet_subresource_load_delay_for_document(document)
            .unwrap();
        let resource = host
            .admit_document_web_font(document, font_resource())
            .unwrap();
        (document, binding, resource)
    };
    let fallback = vm.eval("String(childTarget.offsetWidth)").unwrap();
    assert_ne!(fallback, "48");
    let parent = vm.eval("String(parentTarget.offsetWidth)").unwrap();
    vm.complete_document_web_font(binding, font_response(&resource));
    assert_eq!(vm.eval("String(childTarget.offsetWidth)").unwrap(), "48");
    assert_eq!(vm.eval("String(parentTarget.offsetWidth)").unwrap(), parent);

    vm.eval(rewrite).unwrap();
    assert_eq!(
        vm.eval("String(childTarget.offsetWidth)").unwrap(),
        fallback
    );
    let (replacement_binding, replacement_resource) = {
        let mut host = vm._context_host.borrow_mut();
        let frame = host.dom_host().element_handle_by_id("frame").unwrap();
        // document.open reuses the DOM handle; the exact task owner differs.
        assert_eq!(
            host.child_browsing_context_document_handle(frame),
            Some(document)
        );
        assert!(host.stylesheet_web_font_document(binding).is_none());
        let binding = host
            .accept_current_stylesheet_subresource_load_delay_for_document(document)
            .unwrap();
        let resource = host
            .admit_document_web_font(document, font_resource())
            .unwrap();
        (binding, resource)
    };
    assert_eq!(
        resource.web_font().unwrap().request_id(),
        replacement_resource.web_font().unwrap().request_id()
    );
    let before = vm.layout_pass_observability_for_test().1;
    vm.complete_document_web_font(binding, font_response(&resource));
    assert_eq!(
        vm.eval("String(childTarget.offsetWidth)").unwrap(),
        fallback
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    vm.complete_document_web_font(replacement_binding, font_response(&replacement_resource));
    assert_eq!(vm.eval("String(childTarget.offsetWidth)").unwrap(), "48");
    vm.eval("frame.remove()").unwrap();
    assert!(
        vm._context_host
            .borrow()
            .stylesheet_web_font_document(replacement_binding)
            .is_none()
    );
    vm.complete_document_web_font(replacement_binding, font_response(&replacement_resource));
    assert_eq!(vm.eval("String(parentTarget.offsetWidth)").unwrap(), parent);
}

#[tokio::test]
async fn child_document_fonts_ready_waits_for_its_own_resource() {
    let (font_url, request_rx, release_tx, server) = spawn_gated_font_resource_server().await;
    let document_url = font_url.replace("/print-only.woff2", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(OptionalResourceFetchMask::FONT);
    let mut vm = new_parsed_page_task_executor_test_vm(
        &document_url,
        "<!doctype html><span id=parentTarget style='display:inline-block;font:20px ChildReady,serif'>iiii</span><iframe id=frame></iframe>",
        &loader,
    );
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    vm.set_layout_policy(LayoutPolicy::Realtime);
    vm.eval(&format!(r#"
        globalThis.child=frame.contentWindow;
        child.document.open();
        child.document.write('<!doctype html><style>@font-face{{font-family:ChildReady;src:url({font_url})}}</style><span id=target style="display:inline-block;font:20px ChildReady,serif">iiii</span>');
        child.document.close();
        globalThis.childTarget=child.document.getElementById('target');
        globalThis.rootReady=false;globalThis.childReady=false;
        document.fonts.ready.then(fonts=>rootReady=fonts===document.fonts);
        globalThis.childPromise=child.document.fonts.ready;
        childPromise.then(fonts=>childReady=fonts===child.document.fonts);
        'installed'
    "#)).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), request_rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([rootReady,childReady])").unwrap(),
        "[false,false]"
    );
    assert!(
        vm.run_selected_task_executor_turn(
            crate::runtime::PageSelectedTaskTestSelector::FontLoading,
            &loader
        )
        .await
        .unwrap()
    );
    assert_eq!(vm.eval("JSON.stringify([rootReady,childReady,document.fonts.status,child.document.fonts.status,childPromise===child.document.fonts.ready])").unwrap(), r#"[true,false,"loaded","loading",true]"#);
    // Complete already queued child parser/lifecycle work, leaving its gated
    // resource outstanding. This is fixture setup, not task-order evidence.
    assert!(
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap()
            < 32
    );

    let fallback = vm.eval("String(childTarget.offsetWidth)").unwrap();
    assert_ne!(fallback, "48");
    let parent = vm.eval("String(parentTarget.offsetWidth)").unwrap();
    release_tx.send(()).unwrap();
    server.await.unwrap();
    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "child font terminal",
    )
    .await;
    assert_eq!(
        vm.eval("String(childReady)").unwrap(),
        "false",
        "resource completion cannot settle font readiness inline"
    );
    assert!(
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap()
            < 32
    );
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval("JSON.stringify([rootReady,childReady,child.document.fonts.status,childTarget.offsetWidth])").unwrap(), r#"[true,true,"loaded",48]"#);
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        builds,
        "readiness must already have published current glyph geometry"
    );
    assert_eq!(vm.eval("String(parentTarget.offsetWidth)").unwrap(), parent);
    assert_eq!(vm.document_web_font_counts_for_test(), (0, 0, 0));
    assert_eq!(
        vm.layout_pass_observability_for_test()
            .3
            .unwrap()
            .paint_operation_count,
        0
    );
}

#[tokio::test]
async fn realtime_font_ready_rechecks_new_sources_before_resolving() {
    use base64::Engine;
    let (font_url, request_rx, release_tx, server) = spawn_gated_font_resource_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(OptionalResourceFetchMask::FONT);
    let font = base64::engine::general_purpose::STANDARD.encode(include_bytes!(
        "../../../../moli-layout/tests/fixtures/moli-ahem.woff2"
    ));
    let mut vm = new_parsed_page_task_executor_test_vm(
        &font_url.replace("/print-only.woff2", "/page"),
        &format!(
            "<!doctype html><style id=sheet>@font-face{{font-family:ReadySwap;src:url(data:font/woff2;base64,{font})}}</style><span id=target style='display:inline-block;font:20px ReadySwap,serif'>iiii</span>"
        ),
        &loader,
    );
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    vm.set_layout_policy(LayoutPolicy::Realtime);
    vm.eval("globalThis.ready=false;globalThis.promise=document.fonts.ready;promise.then(()=>ready=true)").unwrap();
    assert!(
        !vm._context_host
            .borrow()
            .document_web_fonts_have_pending_requests(vm._context_host.borrow().document_handle()),
        "first data font is already available"
    );
    vm.eval(&format!(
        "sheet.textContent='@font-face{{font-family:ReadySwap;src:url({font_url})}}'"
    ))
    .unwrap();
    assert!(
        vm.run_selected_task_executor_turn(
            crate::runtime::PageSelectedTaskTestSelector::FontLoading,
            &loader
        )
        .await
        .unwrap()
    );
    tokio::time::timeout(std::time::Duration::from_secs(2), request_rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([ready,promise===document.fonts.ready,document.fonts.status])")
            .unwrap(),
        r#"[false,true,"loading"]"#
    );
    release_tx.send(()).unwrap();
    server.await.unwrap();
    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "replacement font",
    )
    .await;
    assert_eq!(vm.eval("String(ready)").unwrap(), "false");
    assert!(
        vm.run_selected_task_executor_turn(
            crate::runtime::PageSelectedTaskTestSelector::FontLoading,
            &loader
        )
        .await
        .unwrap()
    );
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("JSON.stringify([ready,target.offsetWidth,document.fonts.status])")
            .unwrap(),
        r#"[true,48,"loaded"]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    assert_eq!(
        vm.layout_pass_observability_for_test()
            .3
            .unwrap()
            .paint_operation_count,
        0
    );
}

#[tokio::test]
async fn child_web_fonts_share_the_resource_policy_and_preserve_document_isolation() {
    use base64::Engine;

    let font = base64::engine::general_purpose::STANDARD.encode(include_bytes!(
        "../../../../moli-layout/tests/fixtures/moli-ahem.woff2"
    ));
    for policy in [
        LayoutPolicy::Mock,
        LayoutPolicy::OnDemand,
        LayoutPolicy::Realtime,
    ] {
        for mask in [
            OptionalResourceFetchMask::NONE,
            OptionalResourceFetchMask::FONT,
        ] {
            let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
            loader.set_optional_resource_fetch_mask(mask);
            let (mut vm, _completions) =
                new_parsed_test_vm_with_loader_and_resource_completion_queue(
                    "https://child-font-policy.test/",
                    "<!doctype html><span id=parentTarget style='display:inline-block;font:20px ChildPolicy,serif'>iiii</span><iframe id=frame></iframe>",
                    &loader,
                );
            vm.set_layout_policy(policy);
            vm.eval(&format!(r#"
                globalThis.child=frame.contentWindow;
                child.document.open();
                child.document.write('<!doctype html><style>@font-face{{font-family:ChildPolicy;src:url(data:font/woff2;base64,{font})}}</style><span id=target style="display:inline-block;font:20px ChildPolicy,serif">iiii</span>');
                child.document.close();
                globalThis.childTarget=child.document.getElementById('target');
            "#)).unwrap();
            vm.reconcile_document_web_fonts_for_layout();
            let fonts_enabled = mask == OptionalResourceFetchMask::FONT;
            {
                let host = vm._context_host.borrow();
                let frame = host.dom_host().element_handle_by_id("frame").unwrap();
                let document = host.child_browsing_context_document_handle(frame).unwrap();
                assert_eq!(
                    host.document_web_font_sidecar_is_pristine(document),
                    !fonts_enabled,
                    "{policy:?}, {mask:?}"
                );
                assert!(!host.document_web_fonts_have_pending_requests(document));
            }
            assert_eq!(vm.document_web_font_counts_for_test(), (0, 0, 0));
            if policy.uses_real_layout() {
                let child = vm.eval("String(childTarget.offsetWidth)").unwrap();
                assert_eq!(
                    child == "48",
                    fonts_enabled,
                    "{policy:?}, {mask:?}: {child}"
                );
                let parent = vm.eval("String(parentTarget.offsetWidth)").unwrap();
                assert_ne!(
                    parent, "48",
                    "a child font must not become a parent registration"
                );
                if policy.is_realtime() && fonts_enabled {
                    vm.eval("child.document.styleSheets[0].deleteRule(0)")
                        .unwrap();
                    vm.reconcile_document_web_fonts_for_layout();
                    assert_eq!(vm.eval("String(childTarget.offsetWidth)").unwrap(), parent);
                }
            }
        }
    }
}

fn font_selection_fixture(media: &str) -> StandaloneScriptVmHarness {
    use base64::Engine;

    let font = base64::engine::general_purpose::STANDARD.encode(include_bytes!(
        "../../../../moli-layout/tests/fixtures/moli-ahem.woff2"
    ));
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(OptionalResourceFetchMask::FONT);
    let markup = format!(
        "<!doctype html><style media='{media}'>@font-face{{font-family:CurrentFont;src:url(data:font/woff2;base64,{font})}}</style><span id=target style='display:inline-block;font:20px CurrentFont,serif'>iiii</span>"
    );
    let (mut vm, _completions) = new_parsed_test_vm_with_loader_and_resource_completion_queue(
        "https://font-selection.test/",
        &format!("{markup}<iframe id=frame style='width:50vw;border:0'></iframe>"),
        &loader,
    );
    vm.set_layout_policy(LayoutPolicy::Realtime);
    vm.eval(&format!("globalThis.child=frame.contentWindow;child.document.open();child.document.write({});child.document.close()", serde_json::to_string(&markup).unwrap())).unwrap();
    vm.eval("void child.document.body.offsetWidth").unwrap();
    vm.reconcile_document_web_fonts_for_layout();
    assert_eq!(vm.eval("JSON.stringify([target.offsetWidth,child.document.getElementById('target').offsetWidth])").unwrap(), "[48,48]");
    vm
}

#[tokio::test]
async fn realtime_font_selection_observes_cssom_removal_and_restore_in_the_same_call() {
    let mut vm = font_selection_fixture("all");
    let before = vm.layout_pass_observability_for_test().1;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval(
            r#"
        JSON.stringify([document,child.document].map(doc=>{
            const target=doc.getElementById('target');
            const sheet=doc.styleSheets[0];
            const rule=sheet.cssRules[0].cssText;
            const style=doc.defaultView.getComputedStyle(target);
            const before=target.getBoundingClientRect().width;
            sheet.deleteRule(0);
            const removed=target.getBoundingClientRect().width;
            const removedStyle=parseFloat(style.width);
            sheet.insertRule(rule,0);
            const restored=target.getBoundingClientRect().width;
            const again=target.getBoundingClientRect().width;
            return {before,removed,removedStyle,restored,again};
        }))
    "#,
        )
        .unwrap(),
    )
    .unwrap();
    for result in result.as_array().unwrap() {
        assert_eq!(result["before"], 48);
        assert_ne!(result["removed"], 48, "{result}");
        // CSS serialization rounds the fractional used value; DOMRect keeps
        // the layout precision. Both must observe the same font selection.
        assert!(
            (result["removedStyle"].as_f64().unwrap() - result["removed"].as_f64().unwrap()).abs()
                < 0.001,
            "{result}"
        );
        assert_eq!(result["restored"], 48, "{result}");
        assert_eq!(result["again"], 48);
    }
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 4);
    assert_eq!(
        vm.layout_pass_observability_for_test()
            .3
            .unwrap()
            .paint_operation_count,
        0
    );

    // Source mutation already made the selected geometry current. Later
    // resource retirement must not turn that same result stale again.
    let fallback = vm
        .eval("document.styleSheets[0].deleteRule(0);String(target.getBoundingClientRect().width)")
        .unwrap();
    let builds = vm.layout_pass_observability_for_test().1;
    vm.reconcile_document_web_fonts_for_layout();
    assert_eq!(
        vm.eval("String(target.getBoundingClientRect().width)")
            .unwrap(),
        fallback
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
}

#[tokio::test]
async fn realtime_font_selection_uses_the_new_child_viewport_in_the_same_call() {
    let mut vm = font_selection_fixture("screen and (min-width:200px)");
    let builds = vm.layout_pass_observability_for_test().1;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval(
            r#"
        const el=child.document.getElementById('target');
        frame.style.width='100px';
        const small=el.getBoundingClientRect().width;
        const root=target.getBoundingClientRect().width;
        frame.style.width='300px';
        JSON.stringify({small,root,restored:el.getBoundingClientRect().width});
    "#,
        )
        .unwrap(),
    )
    .unwrap();
    assert_ne!(result["small"], 48, "{result}");
    assert_eq!(result["root"], 48, "{result}");
    assert_eq!(result["restored"], 48, "{result}");
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 2);
}

#[tokio::test]
async fn realtime_font_selection_temporary_media_and_viewport_preserve_screen_fonts() {
    use crate::native_bridge::LayoutPublicationRequest;
    use moli_layout::{LayoutError, LayoutFlushReason, LayoutPassRequest, LayoutViewport};

    let mut vm = font_selection_fixture("screen and (min-width:200px)");
    let target = vm.document_runtime.get_element_by_id("target").unwrap();
    let frame = vm.document_runtime.get_element_by_id("frame").unwrap();
    let (viewport, print, child_viewport) = {
        let host = vm._context_host.borrow();
        let viewport = host.layout_viewport_for_document(host.document_handle());
        let mut media = host.emulated_media().clone();
        media.media = Some("print".to_owned());
        (
            viewport,
            crate::style_engine::StyloStyleEnvironment::from_emulated_media(&media),
            host.frame_viewport(frame),
        )
    };
    let read = "JSON.stringify([target.getBoundingClientRect().width,child.document.getElementById('target').getBoundingClientRect().width])";
    let builds = vm.layout_pass_observability_for_test().1;
    let publishes = vm.layout_snapshot_cache_observability_for_test().2;
    for (index, (viewport, environment)) in [
        (viewport, Some(print)),
        (LayoutViewport::new(100, 120, 1.0), None),
    ]
    .into_iter()
    .enumerate()
    {
        let error = vm
            .with_fresh_layout_pass_in_environment::<()>(
                LayoutPassRequest::new(viewport, LayoutFlushReason::Test)
                    .with_frame_coverage(moli_layout::LayoutFrameCoverage::AllLayoutBoxes),
                LayoutPublicationRequest::Transient,
                environment,
                |pass| {
                    let rect = pass.tree.client_rects_for_source(target)[0].bounding_rect();
                    assert_ne!(
                        rect.width, 48.0,
                        "the temporary view must exclude the screen font"
                    );
                    Err(LayoutError::source_contract(
                        "test capture",
                        "output consumption failed",
                    ))
                },
            )
            .unwrap_err();
        assert!(matches!(error, LayoutError::SourceContract { .. }));
        assert_eq!(vm.eval(read).unwrap(), "[48,48]");
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            builds + index as u64 + 1
        );
        assert_eq!(
            vm.layout_snapshot_cache_observability_for_test().2,
            publishes
        );
        assert_eq!(
            vm._context_host.borrow().frame_viewport(frame),
            child_viewport
        );
    }
    // Force a subsequent real pass, so a cached screen tree cannot hide any
    // font registry damage from either failed temporary view.
    assert_eq!(
        vm.eval(&format!("target.style.marginLeft='1px';{read}"))
            .unwrap(),
        "[48,48]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 3);
}
