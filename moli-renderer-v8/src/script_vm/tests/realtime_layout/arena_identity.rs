use super::realtime_vm;

#[test]
fn stable_box_handles_preserve_fresh_geometry_and_paint_across_reparenting() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;overflow:hidden;font:16px/1.25 sans-serif}",
        "#columns{display:flex;align-items:start}section{width:180px;flex:none}",
        "#right{width:125px;direction:rtl}p{margin:0}#atomic{display:inline-block;width:30px}",
        "#grid{display:grid;grid-template-columns:40px 60px}td{padding:2px}",
        "#positioned{position:relative;height:35px}#absolute{position:absolute;left:3px;top:4px}",
        "#float{float:left;width:35px;height:30px}#scroll{overflow:scroll;width:120px;height:30px}",
        "#tall{height:100px}</style><div id=prefix></div><div id=columns>",
        "<section id=left><p id=paragraph><span id=text>alpha אבג beta gamma</span>",
        "<span id=atomic>inline</span> more words</p><div id=grid><div>A</div><div>B</div></div></section>",
        "<section id=right><table id=table><tbody id=rows><tr><td>one</td><td>two</td></tr></tbody></table>",
        "<div id=positioned><span id=absolute>absolute</span></div></section></div>",
        "<div id=float></div><p id=wrapped>Wrapping alongside a float with more words.</p>",
        "<div id=scroll><div id=tall>scroll content</div></div><ol><li id=marker>marker</li></ol>",
    ));
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let document = host.borrow().document_handle();
    let viewport = host.borrow().layout_viewport_for_document(document);
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
    for edit in [
        "prefix.prepend(document.createElement('div'));prefix.firstChild.style.height='7px'",
        "right.prepend(paragraph)",
        "window.removed=grid.firstChild;removed.remove()",
        "grid.append(removed);grid.style.gridTemplateColumns='55px 45px'",
        "rows.insertAdjacentHTML('afterbegin','<tr><td colspan=2>new row</td></tr>')",
        "left.append(table);right.style.width='95px'",
        "atomic.style.display='block';positioned.style.position='static'",
        "float.style.width='65px';prefix.firstChild.remove()",
        "document.getElementById('scroll').scrollTop=20;columns.style.transform='translateX(3.25px)'",
        "const range=document.createRange();range.selectNodeContents(text);getSelection().addRange(range)",
    ] {
        vm.eval(edit).unwrap();
        let retained = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
        let host = host.borrow();
        let fresh = host
            .build_layout_pass_for_document(
                document,
                moli_layout::LayoutPassRequest::with_paint(
                    viewport,
                    moli_layout::LayoutFlushReason::Screenshot,
                ),
            )
            .unwrap()
            .unwrap();
        assert_eq!(Some(&retained), fresh.paint_snapshot(), "{edit}");
        for name in [
            "paragraph",
            "text",
            "atomic",
            "grid",
            "table",
            "absolute",
            "wrapped",
            "scroll",
            "marker",
        ] {
            let node = vm.document_runtime.get_element_by_id(name).unwrap();
            let retained = host
                .with_latest_layout_tree_for_document(document, |tree| {
                    tree.client_rects_for_source(node)
                })
                .unwrap();
            assert_eq!(
                retained,
                fresh.client_rects_for_source(node),
                "{edit}: {name}"
            );
        }
    }
}

fn assert_published_layout_matches_fresh(
    vm: &mut super::StandaloneScriptVmHarness,
    nodes: &[&str],
    step: &str,
) {
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let document = host.borrow().document_handle();
    let viewport = host.borrow().layout_viewport_for_document(document);
    let retained = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
    let host = host.borrow();
    let fresh = host
        .build_layout_pass_for_document(
            document,
            moli_layout::LayoutPassRequest::with_paint(
                viewport,
                moli_layout::LayoutFlushReason::Screenshot,
            ),
        )
        .unwrap()
        .unwrap();
    assert_eq!(Some(&retained), fresh.paint_snapshot(), "{step}");
    for name in nodes {
        let node = vm.document_runtime.get_element_by_id(name).unwrap();
        let current = host
            .with_latest_layout_tree_for_document(document, |tree| {
                tree.client_rects_for_source(node)
            })
            .unwrap();
        assert_eq!(
            current,
            fresh.client_rects_for_source(node),
            "{step}: {name}"
        );
    }
}

#[test]
fn retained_container_display_switches_match_fresh_after_warming_numeric_layout() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;overflow:hidden;font:16px sans-serif}",
        "#switcher{width:240px}#switcher.flex{display:flex;gap:6px}",
        "#switcher.grid{display:grid;grid-template-columns:75px 100px}",
        "#switcher.flex::before{content:'before';color:green}",
        "#switcher.grid::after{content:'after';color:blue}",
        "#side{width:130px}p{margin:0}</style><div id=warm style='height:10px'></div>",
        "<div id=switcher><p id=firstCell>first paragraph with text</p>",
        "<p id=secondCell>second paragraph with text</p></div>",
        "<aside id=side>unmodified sibling content for reuse</aside>",
        "<section style='width:70px;height:20px'><div style='width:15px;height:4px'></div></section>",
    ));
    vm.publish_layout_for_test().unwrap();
    vm.eval("warm.style.height='13px';warm.getBoundingClientRect().height")
        .unwrap();
    let warmed = vm.layout_pass_observability_for_test().3.unwrap();
    assert!(warmed.retained_numeric_tree, "{warmed:?}");
    assert!(warmed.retained_numeric_layout_hits > 0, "{warmed:?}");

    for mode in ["flex", "grid", "", "grid", "flex", ""] {
        let edit = format!("switcher.className='{mode}'");
        vm.eval(&edit).unwrap();
        assert_published_layout_matches_fresh(
            &mut vm,
            &["switcher", "firstCell", "secondCell", "side"],
            &edit,
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn reordered_box_storage_preserves_the_nested_scroll_projection() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut vm = realtime_vm(concat!(
                "<!doctype html><style>body{margin:0;overflow:hidden;font:16px sans-serif}",
                "#frame{overflow:auto;width:210px;height:140px}",
                "#scroller{overflow:auto;width:160px;height:90px}",
                "#deleted,#spacer{height:24px}#sticky{position:sticky;top:0;height:20px;background:lime}",
                "#fixed{position:fixed;top:7px;left:165px;width:14px;height:16px;background:blue}",
                "#tall{height:320px;background:yellow}</style><div id=deleted></div>",
                "<div id=frame><div id=spacer></div><div id=scroller><div id=sticky></div>",
                "<div id=tall>scroll content</div><div id=fixed></div></div></div>",
                "<section id=holding><p id=moving>reparent me</p></section>",
            ));
            let host = vm.context_host_weak_for_test().upgrade().unwrap();
            let (document, viewport) = {
                let host = host.borrow();
                let document = host.document_handle();
                (document, host.layout_viewport_for_document(document))
            };
            #[cfg(feature = "devtools")]
            let session = crate::runtime::RendererScreencastPaintSession::default();
            #[cfg(feature = "devtools")]
            let paint = |vm: &mut super::StandaloneScriptVmHarness, consumer| {
                vm.paint_current_screen_layout(
                    consumer,
                    viewport,
                    moli_layout::PaintCaptureRequest::viewport(),
                    moli_layout::LayoutFlushReason::RenderingOpportunity,
                )
                .unwrap()
                .unwrap()
            };
            #[cfg(feature = "devtools")]
            paint(&mut vm, Some(&session));
            #[cfg(not(feature = "devtools"))]
            vm.publish_layout_for_test().unwrap();
            vm.eval("deleted.remove()").unwrap();
            vm.publish_layout_for_test().unwrap();
            vm.eval("frame.prepend(document.createElement('hr'));scroller.prepend(moving)")
                .unwrap();
            vm.publish_layout_for_test().unwrap();

            let nodes = ["frame", "scroller", "sticky", "fixed", "moving", "tall"]
                .map(|name| (name, vm.document_runtime.get_element_by_id(name).unwrap()));
            for offset in [18, 61, 4] {
                let builds = vm.layout_pass_observability_for_test().1;
                let scroll_top = vm
                    .eval(&format!("scroller.scrollTop={offset};String(scroller.scrollTop)"))
                    .unwrap();
                assert_eq!(scroll_top, offset.to_string());

                // Sample the scroll projection before preparing the independent reference.
                let projected = {
                    let host = host.borrow();
                    host.ensure_observable_layout().unwrap();
                    host.with_latest_layout_tree_for_document(document, |tree| {
                        nodes.map(|(_, node)| tree.client_rects_for_source(node))
                    })
                    .unwrap()
                };
                #[cfg(feature = "devtools")]
                let projected_paint = paint(&mut vm, None);
                assert_eq!(
                    vm.layout_pass_observability_for_test().1,
                    builds,
                    "scrollTop={offset} must reuse the current layout"
                );

                let fresh = host
                    .borrow()
                    .build_layout_pass_for_document(
                        document,
                        moli_layout::LayoutPassRequest::with_paint(
                            viewport,
                            moli_layout::LayoutFlushReason::Screenshot,
                        ),
                    )
                    .unwrap()
                    .unwrap();
                for ((name, node), rects) in nodes.iter().zip(projected) {
                    assert_eq!(
                        rects,
                        fresh.client_rects_for_source(*node),
                        "scrollTop={offset}: {name}"
                    );
                }
                #[cfg(feature = "devtools")]
                assert_eq!(
                    Some(&projected_paint),
                    fresh.paint_snapshot(),
                    "scrollTop={offset}"
                );
            }
        })
        .await;
}

#[test]
fn unchanged_markers_keep_direct_item_paragraphs_and_rebuild_changed_text() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;font:16px sans-serif}",
        "#items{width:190px}li{list-style-position:inside}",
        "#items.custom li::marker{content:'item. ';color:blue}</style>",
        "<div id=prefix style='height:10px'></div>",
        "<ol id=items start=3><li id=first>first item keeps a direct paragraph",
        "<li id=second>second item keeps a direct paragraph</ol>",
    ));
    vm.publish_layout_for_test().unwrap();
    for height in [17, 19] {
        vm.eval(&format!(
            "prefix.style.height='{height}px';first.getBoundingClientRect().height"
        ))
        .unwrap();
        let metrics = vm.layout_pass_observability_for_test().3.unwrap();
        assert!(metrics.reused_inline_contexts >= 2, "{metrics:?}");
        // The first refresh reconciles the initial order, which still had
        // generated marker text appended to the end.
        if height == 19 {
            assert!(metrics.retained_numeric_tree, "{metrics:?}");
        }
    }
    for edit in [
        "items.start=21",
        "second.value=101;items.reversed=true",
        "items.prepend(second)",
        "items.className='custom'",
        "items.className='';second.style.listStyleType='none'",
        "second.style.listStyleType='decimal';second.style.listStylePosition='outside'",
    ] {
        vm.eval(edit).unwrap();
        assert_published_layout_matches_fresh(&mut vm, &["first", "second"], edit);
    }
}
