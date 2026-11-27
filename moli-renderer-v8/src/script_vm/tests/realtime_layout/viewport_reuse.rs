use super::realtime_vm;
use crate::protocol_types::ViewportSurface;
use moli_layout::{LayoutFlushReason, LayoutPassRequest, LayoutViewport};

fn surface(width: u32, height: u32, scale: f32) -> ViewportSurface {
    ViewportSurface {
        inner_width: width,
        inner_height: height,
        outer_width: width,
        outer_height: height,
        device_pixel_ratio: f64::from(scale),
        screen_width: 1920,
        screen_height: 1080,
        screen_avail_width: 1920,
        screen_avail_height: 1040,
        ..Default::default()
    }
}

#[test]
fn viewport_changes_retain_boxes_and_shaping_but_recompute_wrapping() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;overflow:hidden;font:16px/1.25 sans-serif}",
        "p{margin:0}#fixed{width:160px}#fluid{width:90%}</style>",
        "<p id=fixed>Stable shaped words with several wrapped lines.</p>",
        "<p id=fluid>Fluid shaped words which wrap at the new available width.</p>",
    ));
    for (index, width) in [1000, 400, 800, 240, 1000].into_iter().enumerate() {
        vm.set_viewport_surface(Some(surface(width, 400, 1.0)))
            .unwrap();
        let viewport = LayoutViewport::new(width, 400, 1.0);
        let current = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
        let metrics = vm.layout_pass_observability_for_test().3.unwrap();
        if index > 0 {
            assert!(metrics.retained_box_count > 0, "{width}: {metrics:?}");
            assert!(metrics.reused_inline_contexts > 0, "{width}: {metrics:?}");
        }
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let document = host.document_handle();
        let fresh = host
            .build_layout_pass_for_document(
                document,
                LayoutPassRequest::with_paint(viewport, LayoutFlushReason::Screenshot),
            )
            .unwrap()
            .unwrap();
        assert_eq!(Some(&current), fresh.paint_snapshot(), "width={width}");
        for name in ["fixed", "fluid"] {
            let node = vm.document_runtime.get_element_by_id(name).unwrap();
            let rects = host
                .with_latest_layout_tree_for_document(document, |tree| {
                    tree.client_rects_for_source(node)
                })
                .unwrap();
            assert_eq!(
                rects,
                fresh.client_rects_for_source(node),
                "{width}: {name}"
            );
        }
    }
}

#[test]
fn retained_resize_revalidates_media_units_topology_and_scrollbar_constraints() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;font:16px/1.25 sans-serif}",
        "#fluid{width:65vw;height:30vh}#percent{width:55%;height:25%}",
        "#layout{display:grid;grid-template-columns:1fr 2fr}#appears{display:none}",
        "#scroll{width:45vw;height:45px;overflow:auto}#scroll p{margin:0}",
        "#fixed{position:fixed;right:4vw;bottom:3vh}",
        "@media(max-width:600px){html{font-size:20px}#layout{display:flex}",
        "#disappears{display:none}#appears{display:block}#scroll{height:28px}}",
        "@media(min-resolution:2dppx){#fluid{padding:3px}}</style>",
        "<div id=fluid><div id=percent>percent</div></div>",
        "<div id=layout><div id=disappears>large</div><div id=appears>small</div></div>",
        "<div id=scroll><p>alpha beta gamma delta epsilon zeta eta theta iota kappa</p></div>",
        "<span id=fixed>fixed</span><div style='height:120vh'>tall</div>",
    ));
    for (width, height, scale) in [
        (1000, 600, 1.0),
        (800, 400, 1.0),
        (400, 700, 1.0),
        (500, 350, 2.0),
        (1000, 600, 1.0),
    ] {
        vm.set_viewport_surface(Some(surface(width, height, scale)))
            .unwrap();
        let viewport = LayoutViewport::new(width, height, scale);
        let current = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let document = host.document_handle();
        let fresh = host
            .build_layout_pass_for_document(
                document,
                LayoutPassRequest::with_paint(viewport, LayoutFlushReason::Screenshot),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            Some(&current),
            fresh.paint_snapshot(),
            "{width}x{height}@{scale}"
        );
        for name in [
            "fluid",
            "percent",
            "layout",
            "disappears",
            "appears",
            "scroll",
            "fixed",
        ] {
            let node = vm.document_runtime.get_element_by_id(name).unwrap();
            let rects = host
                .with_latest_layout_tree_for_document(document, |tree| {
                    tree.client_rects_for_source(node)
                })
                .unwrap();
            assert_eq!(
                rects,
                fresh.client_rects_for_source(node),
                "{width}x{height}: {name}"
            );
        }
    }
}

#[test]
fn resize_keeps_descendants_when_containers_change_formatting_and_generated_content() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;font:16px/1.25 sans-serif}",
        "main{display:grid;grid-template-columns:1fr 2fr;gap:8px}",
        "main::before{content:'wide';display:block;background:green}",
        "article::after{content:'';display:block;clear:both;height:8px;background:red}",
        "p{margin:0}span{background:yellow}",
        "@media(max-width:600px){main{display:block}main::before{content:none}",
        "article::after{content:'narrow';display:contents}}",
        "@media(max-width:300px){main{display:flex}article::after{content:none}}</style>",
        "<main><span id=item>inline item</span><article id=content>",
        "<p id=first>first line and its continuation</p><p>more words in the article</p>",
        "<p>third line of the same article</p><p>fourth line of the article</p>",
        "</article></main>",
    ));
    for (index, width) in [900, 480, 240, 460, 900].into_iter().enumerate() {
        vm.set_viewport_surface(Some(surface(width, 400, 1.0)))
            .unwrap();
        let viewport = LayoutViewport::new(width, 400, 1.0);
        let current = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
        let metrics = vm.layout_pass_observability_for_test().3.unwrap();
        if index > 0 {
            assert!(metrics.reused_inline_contexts >= 4, "{width}: {metrics:?}");
        }
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let document = host.document_handle();
        let fresh = host
            .build_layout_pass_for_document(
                document,
                LayoutPassRequest::with_paint(viewport, LayoutFlushReason::Screenshot),
            )
            .unwrap()
            .unwrap();
        assert_eq!(Some(&current), fresh.paint_snapshot(), "width={width}");
        for name in ["item", "content", "first"] {
            let node = vm.document_runtime.get_element_by_id(name).unwrap();
            let rects = host
                .with_latest_layout_tree_for_document(document, |tree| {
                    tree.client_rects_for_source(node)
                })
                .unwrap();
            assert_eq!(
                rects,
                fresh.client_rects_for_source(node),
                "{width}: {name}"
            );
        }
    }
}

#[test]
fn retained_list_items_refresh_markers_for_new_styles_and_numbering() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;font:16px/1.25 sans-serif}",
        "li{list-style-position:inside}p{margin:0}li::marker{color:green}",
        "@media(max-width:600px){li::marker{color:blue}#first::marker{content:'one. '}}</style>",
        "<ol id=items start=4><li id=first><p>first item content</p></li>",
        "<li id=second><p>second item content</p></li>",
        "<li id=third><p>third item content</p></li></ol>",
    ));
    for (index, (width, edit)) in [
        (900, ""),
        (480, ""),
        (480, "items.reversed=true; items.start=12; second.value=6"),
        (480, "items.prepend(third)"),
        (
            900,
            "items.removeAttribute('start'); second.removeAttribute('value')",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        if !edit.is_empty() {
            vm.eval(edit).unwrap();
        }
        vm.set_viewport_surface(Some(surface(width, 400, 1.0)))
            .unwrap();
        let viewport = LayoutViewport::new(width, 400, 1.0);
        let current = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
        if index == 1 {
            let metrics = vm.layout_pass_observability_for_test().3.unwrap();
            assert!(metrics.reused_inline_contexts >= 3, "{metrics:?}");
        }
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let fresh = host
            .build_layout_pass_for_document(
                host.document_handle(),
                LayoutPassRequest::with_paint(viewport, LayoutFlushReason::Screenshot),
            )
            .unwrap()
            .unwrap();
        assert_eq!(Some(&current), fresh.paint_snapshot(), "case {index}");
    }
}
