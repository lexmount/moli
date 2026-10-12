use super::*;
use crate::native_bridge::LayoutPublicationRequest;
use moli_layout::{LayoutError, LayoutFlushReason, LayoutPassRequest, LayoutViewport};

fn fixture(markup: &str) -> (StandaloneScriptVmHarness, LayoutViewport) {
    let mut vm = new_parsed_test_vm("https://realtime-publication.test/", markup);
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Realtime);
    let viewport = {
        let host = vm._context_host.borrow();
        host.layout_viewport_for_document(host.document_handle())
    };
    (vm, viewport)
}

#[test]
fn realtime_publication_screen_capture_satisfies_following_geometry_reads() {
    let (mut vm, viewport) = fixture("<!doctype html><div id=target style='width:100px'></div>");
    let before = vm.layout_pass_observability_for_test().1;
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);

    vm.eval("target.style.width='240px'").unwrap();
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "240");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
}

#[test]
fn realtime_publication_temporary_viewport_preserves_screen_and_child_geometry() {
    let (mut vm, viewport) = fixture(
        "<!doctype html><body style=margin:0><iframe id=frame style='width:50vw;height:100px;border:0'></iframe>",
    );
    assert!(viewport.css_width > 100);
    vm.eval(
        "const child=frame.contentWindow;child.document.open();\
         child.document.write('<!doctype html><body style=margin:0><div id=target style=width:100%;height:10px></div>');\
         child.document.close();globalThis.childTarget=child.document.getElementById('target')",
    ).unwrap();
    let read = "JSON.stringify([frame.offsetWidth,childTarget.offsetWidth,getComputedStyle(childTarget).width])";
    let initial = vm.eval(read).unwrap();
    let before = vm.layout_pass_observability_for_test().1;
    let publishes = vm.layout_snapshot_cache_observability_for_test().2;
    let frame = vm.document_runtime.get_element_by_id("frame").unwrap();
    let child_viewport = vm._context_host.borrow().frame_viewport(frame);

    vm.screenshot_layout_snapshot(LayoutViewport::new(100, 120, 1.0))
        .unwrap()
        .unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(
        vm.layout_snapshot_cache_observability_for_test().2,
        publishes
    );
    assert_eq!(
        vm._context_host.borrow().frame_viewport(frame),
        child_viewport
    );
    assert_eq!(vm.eval(read).unwrap(), initial);
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_publication_transient_intent_does_not_publish_even_with_screenshot_reason() {
    let (mut vm, viewport) = fixture("<!doctype html><div id=target style='width:100px'></div>");
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    let before = vm.layout_pass_observability_for_test().1;
    let publishes = vm.layout_snapshot_cache_observability_for_test().2;
    vm.eval("target.style.width='240px'").unwrap();
    vm.paint_layout_snapshot(
        viewport,
        LayoutFlushReason::Screenshot,
        LayoutPublicationRequest::Transient,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        vm.layout_snapshot_cache_observability_for_test().2,
        publishes
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "240");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "240");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
}

#[test]
fn realtime_publication_rejects_changed_inputs_without_overwriting_the_screen_tree() {
    let (mut vm, viewport) = fixture("<!doctype html><div id=target style='width:100px'></div>");
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    let before = vm.layout_pass_observability_for_test().1;
    let publishes = vm.layout_snapshot_cache_observability_for_test().2;
    let host = vm._context_host.clone();
    let error = vm
        .with_fresh_layout_pass(
            LayoutPassRequest::new(viewport, LayoutFlushReason::Test),
            LayoutPublicationRequest::Screen,
            |_| {
                // A source change during native output consumption must not be
                // assigned to geometry calculated before that change.
                host.borrow().note_realtime_layout_input_change();
                Ok(())
            },
        )
        .unwrap_err();
    assert!(matches!(error, LayoutError::SourceContract { .. }));
    assert_eq!(
        vm.layout_snapshot_cache_observability_for_test().2,
        publishes
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
}

#[test]
fn realtime_publication_temporary_media_failure_preserves_screen_geometry() {
    let (mut vm, viewport) = fixture(
        "<!doctype html><style>#target{width:40px}@media print{#target{width:80px}}</style><div id=target></div>",
    );
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "40");
    let builds = vm.layout_pass_observability_for_test().1;
    let publishes = vm.layout_snapshot_cache_observability_for_test().2;
    let environment = {
        let host = vm._context_host.borrow();
        let mut media = host.emulated_media().clone();
        media.media = Some("print".to_owned());
        crate::style_engine::StyloStyleEnvironment::from_emulated_media(&media)
    };
    let error = vm
        .with_fresh_layout_pass_in_environment::<()>(
            LayoutPassRequest::new(viewport, LayoutFlushReason::Print),
            LayoutPublicationRequest::Transient,
            Some(environment),
            |_| {
                Err(LayoutError::source_contract(
                    "test capture",
                    "output consumption failed",
                ))
            },
        )
        .unwrap_err();
    assert!(matches!(error, LayoutError::SourceContract { .. }));
    assert_eq!(vm.eval("JSON.stringify([target.offsetWidth,getComputedStyle(target).width,matchMedia('print').matches])").unwrap(), "[40,\"40px\",false]");
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    assert_eq!(
        vm.layout_snapshot_cache_observability_for_test().2,
        publishes
    );

    // A temporary environment must never receive a canonical screen stamp,
    // even when a caller provides an ordinary screenshot reason.
    let error = vm
        .with_fresh_layout_pass_in_environment(
            LayoutPassRequest::new(viewport, LayoutFlushReason::Screenshot),
            LayoutPublicationRequest::Screen,
            Some(environment),
            |_| Ok(()),
        )
        .unwrap_err();
    assert!(matches!(error, LayoutError::SourceContract { .. }));
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
}
