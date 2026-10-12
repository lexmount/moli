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
        "<!doctype html><span id=target style='display:inline-block;font:20px RealtimeSourceFont,serif'>iiii</span>",
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
        child.document.write('<!doctype html><span id=target style=\"display:inline-block;font:20px RealtimeSourceFont,serif\">iiii</span>');\
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
    let (mut vm, mut completions) = new_parsed_test_vm_with_loader_and_resource_completion_queue(
        &document_url,
        "<!doctype html><span id=parentTarget style='display:inline-block;font:20px ChildReady,serif'>iiii</span><iframe id=frame></iframe>",
        &loader,
    );
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
    assert_eq!(vm.eval("JSON.stringify([rootReady,childReady,document.fonts.status,child.document.fonts.status,childPromise===child.document.fonts.ready])").unwrap(), r#"[true,false,"loaded","loading",true]"#);
    let fallback = vm.eval("String(childTarget.offsetWidth)").unwrap();
    assert_ne!(fallback, "48");
    let parent = vm.eval("String(parentTarget.offsetWidth)").unwrap();
    release_tx.send(()).unwrap();
    server.await.unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            completions.wait_for_arrival_without_timeout()
        )
        .await
        .unwrap()
    );
    let completion = completions.pop_next_async_subresource_event().unwrap();
    let _ = vm
        .complete_async_subresource_fetch_event_body(completion)
        .unwrap();
    vm.perform_owner_lane_task_microtask_checkpoints().unwrap();
    assert_eq!(vm.eval("JSON.stringify([rootReady,childReady,child.document.fonts.status,childTarget.offsetWidth])").unwrap(), r#"[true,true,"loaded",48]"#);
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
