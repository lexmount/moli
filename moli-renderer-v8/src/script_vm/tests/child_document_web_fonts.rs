use super::*;
use crate::{
    css_resource_urls::{
        BoundStylesheetWebFont, CompletedStylesheetWebFont, StylesheetLoadBlockingResource,
        StylesheetSubresourceRequest, StylesheetSubresourceTarget,
        stylesheet_load_blocking_font_resources,
    },
    protocol_types::OptionalResourceFetchMask,
};
use moli_page_types::LayoutPolicy;

fn font_resource() -> StylesheetLoadBlockingResource {
    stylesheet_load_blocking_font_resources(
        "@font-face{font-family:ChildDocumentSourceFont;src:url(font.woff2)}",
        &Url::parse("https://child-document-font.test/style.css").unwrap(),
        OptionalResourceFetchMask::FONT,
    )
    .into_iter()
    .next()
    .unwrap()
}

fn font_request(resource: &StylesheetSubresourceRequest) -> &BoundStylesheetWebFont {
    let StylesheetSubresourceTarget::Font(request) = resource.target() else {
        panic!("fixture must prepare a font request");
    };
    request
}

fn font_response(resource: &StylesheetSubresourceRequest) -> CompletedStylesheetWebFont {
    CompletedStylesheetWebFont::response(
        font_request(resource).clone(),
        include_bytes!("../../../../moli-layout/tests/fixtures/moli-ahem.woff2").to_vec(),
    )
}

// These regressions exercise explicit OnDemand layout, rather than changing
// the legacy snapshot freshness contract to satisfy a Realtime fixture.
fn refresh_child_geometry_without_paint(vm: &StandaloneScriptVmHarness) {
    let host = vm._context_host.borrow();
    let document = host.document_handle();
    let pass = host
        .build_layout_pass_for_document(
            document,
            moli_layout::LayoutPassRequest::new(
                host.layout_viewport_for_document(document),
                moli_layout::LayoutFlushReason::Explicit,
            ),
        )
        .unwrap()
        .unwrap();
    assert_eq!(pass.metrics.paint_operation_count, 0);
    host.publish_layout_pass_for_document(document, pass);
}

#[test]
fn child_web_font_document_open_retires_registrations_and_rejects_old_owner() {
    let mut vm = new_parsed_test_vm(
        "https://child-document-font.test/",
        "<!doctype html><span id=parentTarget style='display:inline-block;font:20px ChildDocumentSourceFont,serif'>iiii</span><iframe id=frame></iframe>",
    );
    vm.set_layout_policy(LayoutPolicy::OnDemand);
    let rewrite = "globalThis.child=frame.contentWindow;child.document.open();\
        child.document.write('<!doctype html><style>@font-face{font-family:ChildDocumentSourceFont;src:url(font.woff2)}</style><span id=target style=\"display:inline-block;font:20px ChildDocumentSourceFont,serif\">iiii</span>');\
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
    refresh_child_geometry_without_paint(&vm);
    let fallback = vm.eval("String(childTarget.offsetWidth)").unwrap();
    assert_ne!(fallback, "48");
    let parent = vm.eval("String(parentTarget.offsetWidth)").unwrap();
    vm.complete_document_web_font(binding, font_response(&resource));
    refresh_child_geometry_without_paint(&vm);
    assert_eq!(vm.eval("String(childTarget.offsetWidth)").unwrap(), "48");
    assert_eq!(vm.eval("String(parentTarget.offsetWidth)").unwrap(), parent);

    vm.eval(rewrite).unwrap();
    refresh_child_geometry_without_paint(&vm);
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
        font_request(&resource).request_id(),
        font_request(&replacement_resource).request_id()
    );
    let before = vm.layout_pass_observability_for_test().1;
    vm.complete_document_web_font(binding, font_response(&resource));
    assert_eq!(
        vm.eval("String(childTarget.offsetWidth)").unwrap(),
        fallback
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    vm.complete_document_web_font(replacement_binding, font_response(&replacement_resource));
    refresh_child_geometry_without_paint(&vm);
    assert_eq!(vm.eval("String(childTarget.offsetWidth)").unwrap(), "48");
    vm.eval("frame.remove()").unwrap();
    assert!(
        vm._context_host
            .borrow()
            .stylesheet_web_font_document(replacement_binding)
            .is_none()
    );
    vm.complete_document_web_font(replacement_binding, font_response(&replacement_resource));
    refresh_child_geometry_without_paint(&vm);
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
    vm.set_layout_policy(LayoutPolicy::OnDemand);
    vm.eval(&format!(r#"
        globalThis.child=frame.contentWindow;
        child.document.open();
        child.document.write('<!doctype html><style>@font-face{{font-family:ChildReady;src:url(data:font/woff2;base64,YmFk),url({font_url})}}</style><span id=target style="display:inline-block;font:20px ChildReady,serif">iiii</span>');
        child.document.close();
        globalThis.childTarget=child.document.getElementById('target');
        globalThis.rootReady=false;globalThis.childReady=false;
        document.fonts.ready.then(fonts=>rootReady=fonts===document.fonts);
        globalThis.childPromise=child.document.fonts.ready;
        childPromise.then(fonts=>childReady=fonts===child.document.fonts);
        globalThis.childFace=[...child.document.fonts][0];
        globalThis.childFaceLoaded=false;
        childFace.loaded.then(face=>childFaceLoaded=face===childFace);
        'installed'
    "#)).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), request_rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(vm.eval("JSON.stringify([rootReady,childReady,document.fonts.status,child.document.fonts.status,childPromise===child.document.fonts.ready,childFace.status,childFaceLoaded])").unwrap(), r#"[true,false,"loaded","loading",true,"loading",false]"#);
    refresh_child_geometry_without_paint(&vm);
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
    refresh_child_geometry_without_paint(&vm);
    assert_eq!(vm.eval("childReady").unwrap(), "false");
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("JSON.stringify([rootReady,childReady,child.document.fonts.status,childTarget.offsetWidth,childFace.status,childFaceLoaded])").unwrap(), r#"[true,true,"loaded",48,"loaded",true]"#);
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
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
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
            vm.reconcile_document_font_resources();
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
            if policy == LayoutPolicy::OnDemand {
                refresh_child_geometry_without_paint(&vm);
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
            }
        }
    }
}
