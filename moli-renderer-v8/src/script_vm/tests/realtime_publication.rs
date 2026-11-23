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
fn realtime_publication_temporary_viewport_preserves_screen_container_query_sizes() {
    let (mut vm, viewport) = fixture(
        r#"<!doctype html><style>
        #box { container-type: inline-size; width: 50vw }
        #target { color: red }
        @container (width >= 100px) { #target { color: green } }
        </style><div id=box><div id=target></div></div>"#,
    );
    assert!(viewport.css_width >= 200);
    let color = "getComputedStyle(target).color";
    assert_eq!(vm.eval(color).unwrap(), "rgb(0, 128, 0)");
    let builds = vm.layout_pass_observability_for_test().1;
    vm.screenshot_layout_snapshot(LayoutViewport::new(100, 120, 1.0))
        .unwrap()
        .unwrap();
    assert_eq!(vm.eval(color).unwrap(), "rgb(0, 128, 0)");
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
}

#[test]
fn realtime_publication_container_query_auto_scrollbars_stabilize_on_reflow() {
    let (mut vm, viewport) = fixture(
        r#"<!doctype html><style>
        .scroller { width: 200px; height: 100px; overflow-y: auto }
        .query { container-type: inline-size }
        .content { height: 200px }
        @container (width < 190px) { #content { height: 50px } }
        @container (width >= 180px) { #short { height: 50px } }
        </style>
        <div id=scroller class=scroller><div id=query class=query><div id=content class=content></div></div></div>
        <div id=other class=scroller><div class=query><div id=short class=content></div></div></div>"#,
    );
    let read = "JSON.stringify([scroller.clientWidth, query.offsetWidth, getComputedStyle(content).height, other.clientWidth])";
    let initial = vm.eval(read).unwrap();
    assert_eq!(initial, r#"[185,185,"50px",200]"#);
    let builds = vm.layout_pass_observability_for_test().1;
    vm.eval("content.style.borderBottom = '2px solid';")
        .unwrap();
    assert_eq!(vm.eval(read).unwrap(), initial);
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
    vm.eval("content.style.height = '40px'").unwrap();
    assert_eq!(vm.eval(read).unwrap(), r#"[200,200,"40px",200]"#);
}

#[test]
fn realtime_publication_container_query_can_reveal_three_scrollable_ancestors() {
    let (mut vm, viewport) = fixture(
        r#"<!doctype html><style>
        .scroll { overflow-y: auto }
        #outer { width: 200px; max-height: 100px }
        #middle { max-height: 200px }
        #inner { max-height: 300px }
        #query { container: q / inline-size }
        #content { height: 50px }
        @container q (width >= 195px) { #content { height: 150px } }
        @container q (width < 195px) { #content { height: 250px } }
        @container q (width < 180px) { #content { height: 350px } }
        </style><div id=outer class=scroll><div id=middle class=scroll><div id=inner class=scroll>
        <div id=query><div id=content></div></div></div></div></div>"#,
    );
    let read = "JSON.stringify([outer.clientWidth, middle.clientWidth, inner.clientWidth, query.offsetWidth, content.offsetHeight])";
    assert_eq!(vm.eval(read).unwrap(), "[185,170,155,155,350]");
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval(read).unwrap(), "[185,170,155,155,350]");
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
}

#[test]
fn realtime_publication_inline_block_query_size_excludes_child_intrinsic_width() {
    let (mut vm, viewport) = fixture(
        r#"<!doctype html><style>
        .query { display:inline-block; container-type:inline-size; vertical-align:top }
        #automatic { padding:3px; border:2px solid }
        #intrinsic { width:max-content; contain-intrinsic-width:120px }
        .content { width:200px; height:20px }
        @container (width >= 100px) { .content { width:50px } }
        </style><div id=automatic class=query><div id=child class=content></div></div>
        <div id=intrinsic class=query><div id=other class=content></div></div>"#,
    );
    let read = "JSON.stringify([automatic.offsetWidth, automatic.offsetHeight, child.offsetWidth, intrinsic.offsetWidth, other.offsetWidth])";
    assert_eq!(vm.eval(read).unwrap(), "[10,30,200,120,50]");
    vm.eval("child.style.width = '250px'").unwrap();
    assert_eq!(vm.eval(read).unwrap(), "[10,30,250,120,50]");
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
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
