use super::*;
use moli_page_types::LayoutPolicy;

mod arena_identity;
mod inline_compaction;
mod viewport_reuse;

fn realtime_vm(markup: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm("https://realtime-layout.test/", markup);
    vm.set_layout_policy(LayoutPolicy::Realtime);
    vm
}

#[test]
fn realtime_layout_recovers_after_rejecting_excessive_depth() {
    std::thread::Builder::new()
        .stack_size(crate::render_runtime::RENDER_RUNTIME_STACK_SIZE_BYTES)
        .spawn(|| {
            for publish_first in [false, true] {
                let mut vm = realtime_vm("<!doctype html><div id=target style='width:31px'></div>");
                if publish_first {
                    vm.publish_layout_for_test().unwrap();
                }
                let before = vm.layout_pass_observability_for_test().1;
                vm.eval(
                    "let node=target;for(let i=0;i<512;i++){\
                     const child=document.createElement('div');\
                     node.append(child);node=child}\
                     node.textContent='Deep';",
                )
                .unwrap();
                let host = vm.context_host_weak_for_test().upgrade().unwrap();
                assert!(
                    matches!(
                        host.borrow().ensure_observable_layout(),
                        Err(moli_layout::LayoutError::DepthLimitExceeded { limit: 512 })
                    ),
                    "publish_first={publish_first}"
                );
                assert_eq!(vm.layout_pass_observability_for_test().1, before);
                assert_eq!(
                    vm.eval("target.replaceChildren();String(target.offsetWidth)")
                        .unwrap(),
                    "31"
                );
                assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn realtime_reuses_boxes_after_observed_style_changes_revert() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>html.temporary,#branch.temporary{display:flex}</style>",
        "<div id=branch><div style='height:20px'></div></div><article>Stable</article>",
    ));
    let width = vm
        .eval("String(document.documentElement.offsetWidth)")
        .unwrap();
    let (_, mut previous_builds, _, metrics) = vm.layout_pass_observability_for_test();
    let box_count = metrics.unwrap().box_count;
    for target in ["document.documentElement", "branch"] {
        assert_eq!(
            vm.eval(&format!(
                "(() => {{
                    const element = {target};
                    element.classList.add('temporary');
                    const intermediate = getComputedStyle(element).display;
                    element.classList.remove('temporary');
                    const current = getComputedStyle(element).display;
                    return String([intermediate, current, document.documentElement.offsetWidth]);
                }})()"
            ))
            .unwrap(),
            format!("flex,block,{width}"),
        );
        let (_, builds, _, metrics) = vm.layout_pass_observability_for_test();
        if builds != previous_builds {
            let metrics = metrics.unwrap();
            assert_eq!(
                metrics.retained_box_count, box_count,
                "{target}: {metrics:?}"
            );
        }
        previous_builds = builds;
    }
}

#[test]
fn realtime_layout_writes_and_reads_in_one_call_share_only_current_inputs() {
    let mut vm = realtime_vm(
        "<!doctype html><div id=target style='width:100px;height:10px;padding:0;border:0'></div>",
    );
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "const widths=[target.offsetWidth];target.style.width='200px';\
             widths.push(target.offsetWidth,target.offsetWidth);\
             target.style.width='300px';widths.push(target.offsetWidth);JSON.stringify(widths)",
        )
        .unwrap(),
        "[100,200,200,300]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 3);
    let metrics = vm.layout_pass_observability_for_test().3.unwrap();
    assert_eq!(metrics.retained_box_count, metrics.box_count - 1);
    vm.eval("getComputedStyle(target).color;target.getBoundingClientRect();target.offsetWidth")
        .unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 3);
    assert_eq!(
        vm.layout_pass_observability_for_test()
            .3
            .unwrap()
            .paint_operation_count,
        0
    );
}

#[test]
fn realtime_html_attribute_changes_reuse_native_inputs_and_respect_css_selectors() {
    let mut vm = realtime_vm(
        r#"<!doctype html><style>
           #target { width:100px }
           #target[state="wide"] { width:240px }
           </style><div id=target></div>"#,
    );
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    let builds = vm.layout_pass_observability_for_test().1;
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let native_revision = || {
        let host = host.borrow();
        host.dom_host()
            .document_layout_version(host.document_handle())
    };
    let revision = native_revision();

    assert_eq!(
        vm.eval("target.setAttribute('x','123'); target.setAttribute('width','9'); String(target.offsetWidth)")
            .unwrap(),
        "100"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    assert_eq!(native_revision(), revision);
    assert_eq!(
        vm.eval("target.setAttribute('state','wide'); String(target.offsetWidth)")
            .unwrap(),
        "240"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    assert_eq!(native_revision(), revision);
    assert_eq!(
        vm.eval("target.removeAttribute('state'); String(target.offsetWidth)")
            .unwrap(),
        "100"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 2);
    assert_eq!(native_revision(), revision);
}

#[test]
fn retained_layout_world_belongs_to_realtime_screen_publication() {
    for mode in [
        LayoutPolicy::Mock,
        LayoutPolicy::Lazy,
        LayoutPolicy::Realtime,
    ] {
        let mut vm = new_parsed_test_vm(
            "https://realtime-layout.test/",
            "<!doctype html><div style='display:grid'><input value=Grid></div>",
        );
        vm.set_layout_policy(mode);
        vm.publish_layout_for_test().unwrap();
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        assert_eq!(host.has_retained_layout_for_test(), mode.is_realtime());
        let document = host.document_handle();
        let viewport = host.layout_viewport_for_document(document);
        let mut capture = host
            .build_layout_pass_for_document(
                document,
                moli_layout::LayoutPassRequest::with_paint(
                    viewport,
                    moli_layout::LayoutFlushReason::Print,
                ),
            )
            .unwrap()
            .unwrap();
        assert!(capture.take_retained_layout().is_none());
        assert_eq!(host.has_retained_layout_for_test(), mode.is_realtime());
    }
}

#[cfg(feature = "devtools")]
#[tokio::test(flavor = "current_thread")]
async fn realtime_canvas_pixels_update_without_replacing_geometry() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut vm =
                realtime_vm("<!doctype html><canvas id=canvas width=12 height=12></canvas>");
            let host = vm.context_host_weak_for_test().upgrade().unwrap();
            let document = host.borrow().document_handle();
            let viewport = host.borrow().layout_viewport_for_document(document);
            let session = crate::runtime::RendererScreencastPaintSession::default();
            let paint = |vm: &mut StandaloneScriptVmHarness| {
                vm.paint_current_screen_layout(
                    Some(&session),
                    viewport,
                    moli_layout::PaintCaptureRequest::viewport(),
                    moli_layout::LayoutFlushReason::Screenshot,
                )
                .unwrap()
                .unwrap()
            };
            vm.eval(
                "const ctx=canvas.getContext('2d'); ctx.fillStyle='red'; ctx.fillRect(0,0,12,12)",
            )
            .unwrap();
            let before = paint(&mut vm);
            let publications = vm.layout_pass_observability_for_test().1;
            vm.eval("ctx.fillStyle='blue'; ctx.fillRect(0,0,12,12)")
                .unwrap();
            let updated = paint(&mut vm);
            assert_ne!(before, updated);
            assert_eq!(vm.layout_pass_observability_for_test().1, publications);
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
            assert_eq!(Some(&updated), fresh.paint_snapshot());

            let publications = vm.layout_pass_observability_for_test().1;
            vm.eval("canvas.width=24").unwrap();
            paint(&mut vm);
            assert_eq!(vm.eval("String(canvas.offsetWidth)").unwrap(), "24");
            assert_eq!(vm.layout_pass_observability_for_test().1, publications + 1);
        })
        .await;
}

#[test]
fn realtime_retained_numeric_layout_matches_fresh_geometry_and_paint() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;display:flex}section{flex:none}p{margin:0}</style>",
        "<section id=panel style='overflow:auto;width:140px;height:36px'>",
        "<div id=target style='height:70px;background:blue'></div><p>Some text</p></section>",
        "<section id=stable style='width:110px'><div style='height:48px;background:red'></div></section>",
    ));
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let document = host.borrow().document_handle();
    let viewport = host.borrow().layout_viewport_for_document(document);
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();

    // Changing the content height removes and restores the automatic gutter.
    let target = vm.document_runtime.get_element_by_id("target").unwrap();
    for (height, should_overflow) in [(2, "false"), (70, "true")] {
        vm.eval(&format!(
            "target.style.height='{height}px';getComputedStyle(target).color"
        ))
        .unwrap();
        assert!(
            host.borrow()
                .screen_style_damage_for_node_for_test(target)
                .is_some_and(|damage| damage
                    .contains(style::servo::restyle_damage::ServoRestyleDamage::RELAYOUT))
        );
        let incremental = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
        assert_eq!(
            host.borrow().screen_style_damage_for_node_for_test(target),
            None
        );
        let metrics = vm.layout_pass_observability_for_test().3.unwrap();
        assert!(metrics.retained_box_count > 0);
        assert!(metrics.retained_numeric_layout_hits > 0);
        assert!(metrics.reused_overflow_projection, "{metrics:?}");
        assert_eq!(
            vm.eval("String(panel.scrollHeight > panel.clientHeight)")
                .unwrap(),
            should_overflow,
        );
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
        assert_eq!(Some(&incremental), fresh.paint_snapshot());
        for name in ["panel", "target", "stable"] {
            let node = vm.document_runtime.get_element_by_id(name).unwrap();
            let retained = host
                .with_latest_layout_tree_for_document(document, |tree| {
                    tree.client_rects_for_source(node)
                })
                .unwrap();
            assert_eq!(retained, fresh.client_rects_for_source(node));
        }
    }
}

#[test]
fn realtime_refreshes_retained_overflow_after_transform_projection() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;overflow:hidden}",
        "#panel{overflow:scroll;width:160px;height:48px}",
        "#target{width:24px;height:16px;transform:translateX(8px)}",
        "#resized{width:40px;height:12px}</style>",
        "<div id=panel><div id=target></div></div><div id=resized></div>",
    ));
    vm.publish_layout_for_test().unwrap();
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "target.style.transform='translateX(185px)';\
             JSON.stringify([target.getBoundingClientRect().left,panel.scrollWidth])"
        )
        .unwrap(),
        "[185,209]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);

    assert_eq!(
        vm.eval(
            "resized.style.width='52px';JSON.stringify([resized.offsetWidth,\
             target.getBoundingClientRect().left,panel.scrollWidth])"
        )
        .unwrap(),
        "[52,185,209]"
    );
    let (_, passes, _, metrics) = vm.layout_pass_observability_for_test();
    assert_eq!(passes, before + 1);
    let metrics = metrics.unwrap();
    assert!(metrics.reused_overflow_projection, "{metrics:?}");

    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let host = host.borrow();
    let document = host.document_handle();
    let viewport = host.layout_viewport_for_document(document);
    let fresh = host
        .build_layout_pass_for_document(
            document,
            moli_layout::LayoutPassRequest::new(viewport, moli_layout::LayoutFlushReason::Test),
        )
        .unwrap()
        .unwrap();
    let panel = vm.document_runtime.get_element_by_id("panel").unwrap();
    let target = vm.document_runtime.get_element_by_id("target").unwrap();
    host.with_latest_layout_tree_for_document(document, |tree| {
        assert_eq!(
            tree.element_metrics_for_source(panel),
            fresh.element_metrics_for_source(panel)
        );
        assert_eq!(
            tree.client_rects_for_source(target),
            fresh.client_rects_for_source(target)
        );
    })
    .unwrap();
}

#[cfg(feature = "devtools")]
#[test]
fn realtime_hit_testing_follows_pointer_events_and_visibility_changes() {
    let mut vm = new_parsed_test_vm(
        "https://realtime-hit-test.test/",
        r#"<!doctype html>
           <style>body { margin:0 } #target { width:50px; height:50px }</style>
           <div id="target"></div>"#,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Realtime);
    let hit = "document.elementFromPoint(10, 10) === target";
    assert_eq!(vm.eval(hit).unwrap(), "true");
    for declaration in ["pointer-events:none", "visibility:hidden"] {
        vm.eval(&format!("target.style.cssText = '{declaration}'"))
            .unwrap();
        assert_eq!(vm.eval(hit).unwrap(), "false", "{declaration}");
        vm.eval("target.removeAttribute('style')").unwrap();
        assert_eq!(vm.eval(hit).unwrap(), "true", "{declaration}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn realtime_repaints_native_color_damage_without_rebuilding_geometry() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut vm = realtime_vm(concat!(
                "<!doctype html><style>p{background:red;color:black;border:1px solid green}",
                ".colored{background:blue;color:yellow;border-color:purple}",
                ".wider{width:70px;font-size:22px}</style>",
                "<p id=target>Recolor <em>this text</em></p>",
            ));
            let host = vm.context_host_weak_for_test().upgrade().unwrap();
            let document = host.borrow().document_handle();
            let viewport = host.borrow().layout_viewport_for_document(document);
            let session = crate::runtime::RendererScreencastPaintSession::default();
            let paint = |vm: &mut StandaloneScriptVmHarness| {
                vm.paint_current_screen_layout(
                    Some(&session),
                    viewport,
                    moli_layout::PaintCaptureRequest::viewport(),
                    moli_layout::LayoutFlushReason::Screenshot,
                )
                .unwrap()
                .unwrap()
            };
            let before = paint(&mut vm);
            for (edit, needs_layout) in [
                ("target.classList.add('colored')", false),
                ("target.classList.remove('colored')", false),
                ("target.classList.add('colored')", false),
                (
                    "target.style.backgroundColor='purple'; target.style.color='white'",
                    false,
                ),
                ("target.classList.add('wider')", true),
            ] {
                let publications = vm.layout_pass_observability_for_test().1;
                vm.eval(&format!("{edit}; getComputedStyle(target).color"))
                    .unwrap();
                let snapshot = paint(&mut vm);
                let expect_publications = publications + u64::from(needs_layout);
                assert_eq!(
                    vm.layout_pass_observability_for_test().1,
                    expect_publications,
                    "{edit}"
                );
                if !edit.contains("remove") {
                    assert_ne!(snapshot, before, "{edit}");
                } else {
                    assert_eq!(snapshot, before, "{edit}");
                }
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
                assert_eq!(Some(&snapshot), fresh.paint_snapshot(), "{edit}");
            }
        })
        .await;
}

#[cfg(feature = "devtools")]
#[tokio::test]
async fn realtime_projects_transform_damage_until_auto_gutter_changes() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut vm = realtime_vm(concat!(
                "<!doctype html><style>body{margin:0}",
                "#target{width:20px;height:20px;transform:translateX(10px);background:red}",
                "#target.shifted{transform:translateX(100px);opacity:.5}",
                "#target.far{transform:translateX(2000px)}",
                "</style><div id=target></div>",
            ));
            let host = vm.context_host_weak_for_test().upgrade().unwrap();
            let document = host.borrow().document_handle();
            let viewport = host.borrow().layout_viewport_for_document(document);
            let session = crate::runtime::RendererScreencastPaintSession::default();
            let paint = |vm: &mut StandaloneScriptVmHarness| {
                vm.paint_current_screen_layout(
                    Some(&session),
                    viewport,
                    moli_layout::PaintCaptureRequest::viewport(),
                    moli_layout::LayoutFlushReason::Screenshot,
                ).unwrap().unwrap()
            };
            let initial = paint(&mut vm);
            let measure =
                "JSON.stringify([target.getBoundingClientRect().left,document.elementFromPoint(110,5).id])";
            assert_eq!(vm.eval(measure).unwrap(), "[10,\"\"]");

            for (edit, geometry) in [
                ("target.className='shifted'", "[100,\"target\"]"),
                ("target.style.transform='translateX(40px)'", "[40,\"\"]"),
            ] {
                let publications = vm.layout_pass_observability_for_test().1;
                vm.eval(edit).unwrap();
                assert_eq!(vm.eval(measure).unwrap(), geometry, "{edit}");
                assert_eq!(vm.layout_pass_observability_for_test().1, publications, "{edit}: geometry");
                let projected = paint(&mut vm);
                assert_eq!(vm.layout_pass_observability_for_test().1, publications, "{edit}: paint");
                assert_ne!(projected, initial, "{edit}");
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
                assert_eq!(Some(&projected), fresh.paint_snapshot(), "{edit}");
            }

            let publications = vm.layout_pass_observability_for_test().1;
            vm.eval("target.style.transform=''; target.className='far'")
                .unwrap();
            assert_eq!(
                vm.eval("String(target.getBoundingClientRect().left)")
                    .unwrap(),
                "2000",
            );
            assert_eq!(vm.layout_pass_observability_for_test().1, publications + 1);
        })
        .await;
}

#[test]
fn realtime_rebreaks_a_resized_paragraph_and_reshapes_after_a_font_change() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>p{width:240px;font:16px/1 sans-serif}</style>",
        "<p id=target>One two three four five six seven eight nine ten.</p>",
    ));
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let document = host.borrow().document_handle();
    let viewport = host.borrow().layout_viewport_for_document(document);
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
    let initial_height = vm
        .eval("target.offsetHeight")
        .unwrap()
        .parse::<u32>()
        .unwrap();

    for (edit, reused) in [
        ("target.style.width='78px'", 1),
        ("target.style.fontSize='22px'", 0),
    ] {
        vm.eval(edit).unwrap();
        let retained = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
        let metrics = vm.layout_pass_observability_for_test().3.unwrap();
        assert_eq!(
            metrics.reused_inline_contexts, reused,
            "{edit}: {metrics:?}"
        );
        let current_height = vm
            .eval("target.offsetHeight")
            .unwrap()
            .parse::<u32>()
            .unwrap();
        assert!(current_height > initial_height, "{edit}");
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
        assert_eq!(Some(&retained), fresh.paint_snapshot(), "{edit}");
    }
}

#[test]
fn realtime_reuses_numeric_links_until_box_roles_change() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;overflow:hidden}",
        "#flex{display:flex}#first,#second{flex:none}#first{width:40px}",
        "#second{width:30px}#context{margin-left:100px;position:relative}",
        "#positioned{position:absolute;left:7px;top:4px;width:4px;height:4px;background:red}",
        "</style><div id=flex><div id=first>First</div><div id=second>Second</div></div>",
        "<div id=context><div id=positioned></div></div>",
    ));
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let document = host.borrow().document_handle();
    let viewport = host.borrow().layout_viewport_for_document(document);
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();

    for (edit, reused_tree, geometry) in [
        ("first.style.width='60px'", true, "[0,60,107]"),
        ("first.style.order='2'", false, "[30,0,107]"),
        ("context.style.position='static'", false, "[30,0,7]"),
        ("second.style.width='34px'", true, "[34,0,7]"),
    ] {
        vm.eval(edit).unwrap();
        let retained = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
        let metrics = vm.layout_pass_observability_for_test().3.unwrap();
        assert_eq!(
            metrics.retained_numeric_tree, reused_tree,
            "{edit}: {metrics:?}"
        );
        assert_eq!(
            metrics.reused_overflow_projection, reused_tree,
            "{edit}: {metrics:?}"
        );
        assert_eq!(
            vm.eval(
                "JSON.stringify([first,second,positioned].map(node=>node.getBoundingClientRect().left))"
            ).unwrap(),
            geometry,
            "{edit}",
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
        assert_eq!(Some(&retained), fresh.paint_snapshot(), "{edit}");
    }
}

#[test]
fn realtime_shared_shadow_stylesheet_mutations_are_visible_on_first_read() {
    for first in ["color", "width", "offsetWidth"] {
        let mut vm = realtime_vm("<!doctype html><section></section><section></section>");
        let setup = r#"
            const roots = [...document.querySelectorAll('section')].map(
                node => node.attachShadow({mode:'open'})
            );
            const targets = roots.map(root => root.appendChild(document.createElement('div')));
            const sheet = new CSSStyleSheet();
            sheet.replaceSync('div {width:100px;height:18px;color:rgb(1,2,3)}');
            roots.forEach(root => { root.adoptedStyleSheets = [sheet]; });
            const styles = targets.map(node => getComputedStyle(node));
            function sample() {
                const first = firstRead === 'offsetWidth'
                    ? targets.map(node => node.offsetWidth)
                    : styles.map(style => style[firstRead]);
                return [first,styles.map(style=>style.color),targets.map(node=>node.offsetWidth)];
            }
            JSON.stringify(sample())
        "#;
        let expected = |color: &str, width: u32| {
            let first_value = match first {
                "color" => serde_json::json!(color),
                "width" => serde_json::json!(format!("{width}px")),
                _ => serde_json::json!(width),
            };
            serde_json::json!([[first_value, first_value], [color, color], [width, width]])
        };
        assert_eq!(
            vm.eval(&format!("const firstRead={first:?};{setup}"))
                .unwrap(),
            expected("rgb(1, 2, 3)", 100).to_string(),
        );
        assert_eq!(
            vm.eval(concat!(
                "sheet.replaceSync('div {width:200px;height:18px;color:rgb(4,5,6)}');",
                "JSON.stringify(sample())",
            ))
            .unwrap(),
            expected("rgb(4, 5, 6)", 200).to_string(),
        );
        assert_eq!(
            vm.eval(concat!(
                "sheet.cssRules[0].style.width='300px';sheet.cssRules[0].style.color='rgb(7,8,9)';",
                "JSON.stringify(sample())",
            ))
            .unwrap(),
            expected("rgb(7, 8, 9)", 300).to_string(),
        );
    }
}

#[test]
fn realtime_retains_stable_grid_and_table_box_branches() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{overflow:hidden}",
        "#grid{display:grid;grid-template-columns:30px 50px}",
        "#plain{border-collapse:separate;table-layout:auto}",
        "</style><div id=control style='width:20px'></div>",
        "<div id=grid><div>A</div><div>B</div><div>C</div><div>D</div></div>",
        "<table id=plain><tbody><tr><td id=cell>Text</td><td>Cell</td></tr>",
        "<tr><td>Other</td><td>Row</td></tr></tbody></table>",
    ));
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let document = host.borrow().document_handle();
    let viewport = host.borrow().layout_viewport_for_document(document);
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();

    for (index, edit) in [
        "control.style.width='32px'",
        "plain.tBodies[0].insertRow().insertCell().textContent='New row'",
        "grid.firstElementChild.style.order='2'",
    ]
    .into_iter()
    .enumerate()
    {
        vm.eval(edit).unwrap();
        let retained = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
        if index == 0 {
            let metrics = vm.layout_pass_observability_for_test().3.unwrap();
            assert!(metrics.pruned_source_count >= 13, "{metrics:?}");
        }
        assert_eq!(
            vm.eval("getComputedStyle(grid).gridTemplateColumns")
                .unwrap(),
            "30px 50px"
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
        assert_eq!(Some(&retained), fresh.paint_snapshot(), "{edit}");
    }
}

#[test]
fn realtime_table_span_edits_match_fresh_geometry() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;overflow:hidden}",
        "table{border-spacing:0;width:240px}td{width:80px;height:24px;padding:0}</style>",
        "<table><tr><td id=wide></td><td id=next></td></tr>",
        "<tr><td></td><td></td><td></td></tr></table>",
        "<table><tr><td id=tall><span></span></td><td></td></tr>",
        "<tr><td id=below></td></tr></table>",
    ));
    let sample =
        "JSON.stringify([wide.offsetWidth,next.offsetLeft,tall.offsetHeight,below.offsetLeft])";
    assert_eq!(vm.eval(sample).unwrap(), "[80,80,24,0]");
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let document = host.borrow().document_handle();
    let viewport = host.borrow().layout_viewport_for_document(document);

    for (edit, expected) in [
        ("wide.colSpan=2", "[160,160,24,0]"),
        ("tall.rowSpan=2", "[160,160,48,120]"),
        ("wide.removeAttribute('colspan')", "[80,80,48,120]"),
        ("tall.setAttribute('rowspan','1')", "[80,80,24,0]"),
    ] {
        assert_eq!(vm.eval(&format!("{edit};{sample}")).unwrap(), expected);
        let metrics = vm.layout_pass_observability_for_test().3.unwrap();
        assert!(!metrics.retained_numeric_tree, "{edit}: {metrics:?}");
        let host = host.borrow();
        let fresh = host
            .build_layout_pass_for_document(
                document,
                moli_layout::LayoutPassRequest::new(viewport, moli_layout::LayoutFlushReason::Test),
            )
            .unwrap()
            .unwrap();
        for id in ["wide", "next", "tall", "below"] {
            let node = vm.document_runtime.get_element_by_id(id).unwrap();
            let retained = host.with_latest_layout_tree_for_document(document, |tree| {
                tree.client_rects_for_source(node)
            });
            assert_eq!(
                retained.unwrap(),
                fresh.client_rects_for_source(node),
                "{edit} {id}"
            );
        }
    }
}

#[test]
fn realtime_retained_inline_roles_match_fresh_geometry_and_paint() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0}p{margin:0;width:180px}",
        "#shift{vertical-align:3px}#atomic{display:inline-block;width:30px;height:24px}",
        "#floatBox{float:left;width:24px;height:24px}#absoluteBox{position:absolute}",
        "#stable{width:120px;background:blue}</style>",
        "<p id=owner>   <span>alpha <span id=shift>beta </span></span>",
        "<span id=atomic>atomic</span><span id=floatBox>float</span>",
        "<span id=absoluteBox>absolute</span><br>tail   </p>",
        "<p id=stable>Unaffected text</p>",
    ));
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let document = host.borrow().document_handle();
    let viewport = host.borrow().layout_viewport_for_document(document);
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();

    for edit in [
        "atomic.style.width='45px';floatBox.style.width='49px'",
        "shift.textContent='changed nested text'",
        "owner.style.width='120px'",
        "shift.remove()",
        "owner.insertAdjacentHTML('beforeend',' <span id=added>new text</span>')",
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
        for name in ["owner", "atomic", "floatBox", "stable"] {
            let node = vm.document_runtime.get_element_by_id(name).unwrap();
            let retained = host
                .with_latest_layout_tree_for_document(document, |tree| {
                    tree.client_rects_for_source(node)
                })
                .unwrap();
            assert_eq!(
                retained,
                fresh.client_rects_for_source(node),
                "{edit} {name}"
            );
        }
    }
}

#[test]
fn realtime_rebuilds_only_the_changed_box_branch() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;display:flex;height:200px;align-items:flex-start}",
        "section{width:140px;flex:none}",
        "section div{height:12px;background:blue}</style>",
        "<section id=edited><div id=first></div><div id=second></div></section>",
        "<section id=stable><div id=neighbor></div></section>",
    ));
    assert_eq!(vm.eval("String(edited.offsetHeight)").unwrap(), "24");
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let document = host.borrow().document_handle();
    let viewport = host.borrow().layout_viewport_for_document(document);
    let mut inserted = None;

    for (action, expected_height) in [
        ("edited.style.width='120px'", "24"),
        (
            "window.added=document.createElement('div');added.id='added';edited.prepend(added)",
            "36",
        ),
        ("edited.append(added)", "36"),
        ("added.remove()", "24"),
        ("first.style.display='none'", "12"),
        ("first.style.display='block'", "24"),
        ("stable.append(first)", "12"),
    ] {
        let height = vm
            .eval(&format!("{action};String(edited.offsetHeight)"))
            .unwrap();
        assert_eq!(height, expected_height);
        let metrics = vm.layout_pass_observability_for_test().3.unwrap();
        assert!(metrics.retained_box_count > 0, "{action}");
        assert!(metrics.retained_numeric_layout_hits > 0, "{action}");
        inserted = vm.document_runtime.get_element_by_id("added").or(inserted);
        let sources = ["edited", "first", "second", "stable", "neighbor"]
            .into_iter()
            .map(|id| vm.document_runtime.get_element_by_id(id).unwrap())
            .chain(inserted);
        let host = host.borrow();
        let fresh = host
            .build_layout_pass_for_document(
                document,
                moli_layout::LayoutPassRequest::new(viewport, moli_layout::LayoutFlushReason::Test),
            )
            .unwrap()
            .unwrap();
        for source in sources {
            let retained = host
                .with_latest_layout_tree_for_document(document, |tree| {
                    tree.client_rects_for_source(source)
                })
                .unwrap();
            assert_eq!(retained, fresh.client_rects_for_source(source), "{action}");
        }
    }
}

#[test]
fn realtime_dirty_box_tree_skips_clean_sources_and_matches_fresh_layout() {
    let markup = format!(
        "<!doctype html><style>
           body {{ margin: 0; overflow: hidden }}
           #edited {{ width: 30px; height: 10px; background: blue }}
           #trigger.active + #edited {{ width: 42px }}
           #scroller {{ width: 100px; height: 20px; overflow: scroll }}
           #scrolled {{ height: 80px; background: red }}
           article div {{ height: 8px }}
         </style>
         <div id=scroller><div id=scrolled></div></div>
         <div id=trigger></div><div id=edited></div><section id=local></section>
         <main>{}</main>",
        "<article><div><div></div></div></article>".repeat(1_024)
    );
    let mut vm = realtime_vm(&markup);
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let document = host.borrow().document_handle();
    let viewport = host.borrow().layout_viewport_for_document(document);
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();

    // Finish a scroll-only projection before changing an independent branch.
    assert_eq!(
        vm.eval("scroller.scrollTop=15; String(scrolled.getBoundingClientRect().top)")
            .unwrap(),
        "-15"
    );
    for (edit, width) in [
        ("edited.style.width='38px'", "38"),
        (
            "edited.style.removeProperty('width'); trigger.classList.add('active')",
            "42",
        ),
        ("trigger.classList.remove('active')", "30"),
        (
            "local.appendChild(document.createTextNode('inserted text'))",
            "30",
        ),
        ("local.firstChild.data='edited text'", "30"),
        ("local.firstChild.remove()", "30"),
    ] {
        // An earlier computed-style observation can already commit the style
        // change. Layout still has to consume its pending damage.
        vm.eval(&format!("{edit}; getComputedStyle(edited).color"))
            .unwrap();
        let retained = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
        let metrics = vm.layout_pass_observability_for_test().3.unwrap();
        assert!(metrics.pruned_source_count >= 3_072, "{edit}: {metrics:?}");
        assert!(
            metrics.retained_numeric_layout_hits > 0,
            "{edit}: {metrics:?}"
        );
        assert_eq!(vm.eval("String(edited.offsetWidth)").unwrap(), width);
        assert_eq!(
            vm.eval("String(scrolled.getBoundingClientRect().top)")
                .unwrap(),
            "-15"
        );
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
        let incremental = host
            .with_latest_layout_tree_for_document(document, |tree| {
                ["edited", "local", "scrolled"]
                    .map(|id| vm.document_runtime.get_element_by_id(id).unwrap())
                    .map(|source| (source, tree.client_rects_for_source(source)))
            })
            .unwrap();
        for (source, rects) in incremental {
            assert_eq!(rects, fresh.client_rects_for_source(source), "{edit}");
        }
    }
}

#[test]
fn realtime_retained_layout_rebuilds_complex_branches_without_losing_sibling_caches() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;background:white}",
        "#stable>div{width:100px;height:12px;background:blue}",
        "#grid{display:grid;grid-template-columns:30px 60px}#grid>div{height:8px}",
        "#tableBox{border-collapse:collapse}#cell{border:1px solid}",
        "#anchor{position:relative;height:20px}#positioned{position:absolute;left:20px}",
        "#contents{display:contents}#splitBlock{display:block;height:12px}</style>",
        "Unwrapped text <span id=split>before<div id=splitBlock></div>after</span>",
        "<section id=stable><div id=edited></div><div id=neighbor></div></section>",
        "<section id=grid><div id=gridItem></div><div></div></section>",
        "<table id=tableBox><tr><td id=cell>Table</td></tr></table>",
        "<img id=picture width=20 height=16 alt=Image><input id=field value='old value'>",
        "<div id=anchor><div id=positioned>Positioned</div></div>",
        "<div id=contents><div id=contentsChild>Flattened</div></div>",
    ));
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let document = host.borrow().document_handle();
    let viewport = host.borrow().layout_viewport_for_document(document);
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();

    for (action, reuse_numeric_layout) in [
        ("edited.style.width='121px'", true),
        ("grid.style.gridTemplateColumns='50px 60px'", true),
        (
            "cell.textContent='Wider table';picture.width=42;field.value='new value'",
            true,
        ),
        (
            "positioned.style.left='53px';splitBlock.style.display='inline'",
            true,
        ),
        (
            "anchor.style.position='static';contents.style.display='block'",
            true,
        ),
        (
            "contents.style.display='contents';splitBlock.style.display='block'",
            true,
        ),
        ("grid.style.float='left'", false),
        ("grid.style.float='none'", false),
        ("edited.style.width='122px'", true),
    ] {
        vm.eval(action).unwrap();
        let incremental = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
        let metrics = vm.layout_pass_observability_for_test().3.unwrap();
        assert!(metrics.retained_box_count >= 3, "{action}: {metrics:?}");
        assert_eq!(
            metrics.retained_numeric_layout_hits > 0,
            reuse_numeric_layout,
            "{action}: {metrics:?}"
        );
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
        assert_eq!(Some(&incremental), fresh.paint_snapshot(), "{action}");
        for name in [
            "edited",
            "neighbor",
            "gridItem",
            "cell",
            "picture",
            "field",
            "positioned",
            "split",
            "splitBlock",
            "contents",
            "contentsChild",
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
                "{action}: {name}"
            );
        }
    }
}

#[test]
fn realtime_layout_observes_cssom_sources_without_dom_attribute_mutations() {
    let mut vm = realtime_vm(
        "<!doctype html><style>#target{width:100px;height:10px}</style><div id=target></div>",
    );
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "document.styleSheets[0].cssRules[0].style.width='240px';String(target.offsetWidth)",
        )
        .unwrap(),
        "240"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(
        vm.eval(
            "const sheet=new CSSStyleSheet();sheet.replaceSync('#target{width:360px}');\
             document.adoptedStyleSheets=[sheet];String(target.offsetWidth)",
        )
        .unwrap(),
        "360"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "360");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
}

#[test]
fn realtime_layout_refresh_does_not_call_page_callbacks() {
    let mut vm = realtime_vm("<!doctype html><div id=target style='width:100px'></div>");
    assert_eq!(
        vm.eval(
            "window.callbacks=[];\
             new MutationObserver(()=>callbacks.push('mutation')).observe(target,{attributes:true});\
             requestAnimationFrame(()=>callbacks.push('frame'));\
             target.offsetWidth;target.style.width='200px';\
             JSON.stringify([target.offsetWidth,callbacks.slice()])",
        )
        .unwrap(),
        "[200,[]]"
    );
}

#[test]
fn realtime_rootless_queries_clear_old_geometry_and_rebuild_after_restore() {
    let mut vm = realtime_vm(
        "<!doctype html><style>html,body{margin:0}</style><div id=target style='width:100px;height:40px'></div>",
    );
    assert_eq!(
        vm.eval("document.elementFromPoint(10,10)===target")
            .unwrap(),
        "true"
    );
    let builds = vm.layout_pass_observability_for_test().1;
    vm.eval("window.root=document.documentElement;root.remove()")
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([document.elementFromPoint(10,10),document.elementsFromPoint(10,10).length])").unwrap(),
        "[null,0]"
    );
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let visual = host.borrow().visual_state_generation();
    assert_eq!(
        vm.eval("JSON.stringify([document.elementFromPoint(10,10),document.elementsFromPoint(10,10).length])").unwrap(),
        "[null,0]"
    );
    assert_eq!(host.borrow().visual_state_generation(), visual);
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    assert_eq!(
        vm.eval("document.append(root);target.style.width='200px';document.elementFromPoint(150,10)===target").unwrap(),
        "true"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
}

#[test]
fn realtime_layout_child_reads_refresh_parent_viewport_without_replacing_root() {
    let mut vm = realtime_vm(
        "<!doctype html><iframe id=frame style='width:200px;height:100px;border:0'></iframe>",
    );
    vm.eval(
        "window.child=frame.contentWindow;child.document.open();\
         child.document.write('<!doctype html><body style=margin:0><div id=target style=width:100%;height:10px></div>');\
         child.document.close();window.childTarget=child.document.getElementById('target')",
    )
    .unwrap();
    assert_eq!(vm.eval("String(childTarget.offsetWidth)").unwrap(), "200");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "frame.style.width='320px';JSON.stringify([childTarget.offsetWidth,frame.offsetWidth,childTarget.offsetWidth])",
        )
        .unwrap(),
        "[320,320,320]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_layout_does_not_keep_geometry_from_a_replaced_document() {
    let mut vm = realtime_vm("<!doctype html><div id=target style='width:100px'></div>");
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "document.open();document.write('<!doctype html><div id=newTarget style=width:270px></div>');\
             document.close();String(newTarget.offsetWidth)",
        )
        .unwrap(),
        "270"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_reuses_the_frame_layout_across_parent_changes() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><div id=outside style='width:100px'></div>",
        "<iframe id=child style='width:200px;height:100px;border:0'></iframe>",
    ));
    assert_eq!(
        vm.eval(concat!(
            "child.contentDocument.body.style.margin='0';",
            "child.contentDocument.body.innerHTML='<div id=inside style=width:50%>Text</div>';",
            "String(child.contentDocument.getElementById('inside').offsetWidth)",
        ))
        .unwrap(),
        "100",
    );
    assert_eq!(
        vm.eval("outside.style.width='110px'; String(outside.offsetWidth)")
            .unwrap(),
        "110",
    );
    let metrics = vm.layout_pass_observability_for_test().3.unwrap();
    assert!(metrics.retained_embedded_box_count > 0, "{metrics:?}");
    assert_eq!(
        vm.eval(concat!(
            "child.style.width='260px';",
            "String(child.contentDocument.getElementById('inside').offsetWidth)",
        ))
        .unwrap(),
        "130",
    );
    assert_eq!(
        vm.eval(concat!(
            "child.contentDocument.open();",
            "child.contentDocument.write('<div id=inside style=width:48px>Replacement</div>');",
            "child.contentDocument.close();",
            "String(child.contentDocument.getElementById('inside').offsetWidth)",
        ))
        .unwrap(),
        "48",
    );
}

#[test]
fn realtime_layout_is_current_inside_parser_and_custom_element_reactions() {
    let mut vm = realtime_vm("<!doctype html><body></body>");
    // The same fixture runs in Chromium and through the release CDP entry.
    // Parser writes must invalidate style/geometry before connected reactions;
    // subsequent writes and attribute reactions must be visible in this turn,
    // including through an already held CSSStyleDeclaration and in a child.
    let fixture = include_str!("../../../tests/fixtures/realtime-parser-reactions.js");
    let result = vm.eval(&format!("JSON.stringify({fixture})")).unwrap();
    let expected = serde_json::json!([
        ["initial", 100, "100px"],
        ["connected", 210, "210px", 210, 75],
        ["write", 320, "320px"],
        ["attribute", 430, "430px"],
        ["after", 430, "430px"]
    ]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([expected, expected])
    );
    assert_eq!(
        vm.layout_pass_observability_for_test()
            .3
            .unwrap()
            .paint_operation_count,
        0
    );
}

#[test]
fn realtime_layout_native_state_and_scroll_commit_before_geometry_reads() {
    let mut vm = realtime_vm(
        "<!doctype html><style>#scroller{width:100px}#flag:checked~#scroller{width:200px}</style>\
         <input id=flag type=checkbox><div id=scroller style='height:50px;overflow:auto'>\
         <div id=target style='width:20px;height:500px'></div></div>",
    );
    assert_eq!(vm.eval("String(scroller.offsetWidth)").unwrap(), "100");
    assert_eq!(
        vm.eval(
            "flag.checked=true;\
             const before=target.getBoundingClientRect().top;scroller.scrollTop=30;\
             JSON.stringify([scroller.offsetWidth,before-target.getBoundingClientRect().top])",
        )
        .unwrap(),
        "[200,30]"
    );
}

#[test]
fn realtime_nested_local_scroll_matches_full_coordinate_projection() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;height:1800px}",
        "#outer{width:110px;height:80px;overflow:scroll;transform:scale(2);transform-origin:0 0}",
        "#inner{width:80px;height:50px;overflow:scroll}#mark{margin-top:30px;height:5px;background:red}",
        "#stable{position:fixed;top:120px;left:240px}</style>",
        "<div id=outer><div id=inner><div id=mark></div>",
        "<div style='height:200px'></div></div><div style='height:200px'></div></div>",
        "<div id=stable>Outside</div>",
    ));
    let read = "(() => {const rect=mark.getBoundingClientRect();return JSON.stringify([rect.top,\
                stable.getBoundingClientRect().top,document.elementFromPoint(rect.left+2,rect.top+2).id])})()";
    assert_eq!(vm.eval(read).unwrap(), "[60,120,\"mark\"]");
    let publishes = vm.layout_pass_observability_for_test().1;
    for (edit, top) in [
        ("outer.scrollTop=4;inner.scrollTop=7", 38),
        ("window.scrollTo(0,9)", 29),
        ("window.scrollTo(0,0)", 38),
        ("outer.scrollTop=0;inner.scrollTop=0", 60),
        ("outer.scrollTop=1;inner.scrollTop=2", 54),
    ] {
        assert_eq!(
            vm.eval(&format!("{edit};{read}")).unwrap(),
            format!("[{top},120,\"mark\"]"),
            "{edit}"
        );
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            publishes,
            "{edit}"
        );
    }
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let host = host.borrow();
    let document = host.document_handle();
    let viewport = host.layout_viewport_for_document(document);
    let fresh = host
        .build_layout_pass_for_document(
            document,
            moli_layout::LayoutPassRequest::new(viewport, moli_layout::LayoutFlushReason::Test),
        )
        .unwrap()
        .unwrap();
    for source in ["outer", "inner", "mark", "stable"] {
        let node = vm.document_runtime.get_element_by_id(source).unwrap();
        let current = host
            .with_latest_layout_tree_for_document(document, |tree| {
                tree.client_rects_for_source(node)
            })
            .unwrap();
        assert_eq!(current, fresh.client_rects_for_source(node), "{source}");
    }
}

#[test]
fn realtime_plain_scroll_matches_a_fresh_layout_without_rebuilding() {
    let mut vm = realtime_vm(
        r#"<!doctype html>
        <style>
            html,body {margin:0}
            body {height:2000px}
            #scroller {position:absolute;left:20px;top:120px;width:150px;height:100px;
                       overflow:auto;transform:translateX(5px)}
            #sticky {position:sticky;top:0;width:30px;height:20px}
            #moving {width:30px;height:20px}
            #pinned {position:fixed;top:10px;left:5px;width:10px;height:10px}
        </style>
        <div id=scroller>
            <div style="height:55px"></div>
            <div id=sticky></div>
            <div id=moving></div>
            <div style="height:600px"></div>
        </div>
        <div id=pinned></div>"#,
    );
    vm.eval("moving.getBoundingClientRect().top").unwrap();
    let initial_builds = vm.layout_pass_observability_for_test().1;
    let sample = r#"JSON.stringify([
        moving.getBoundingClientRect().top,
        sticky.getBoundingClientRect().top,
        sticky.offsetTop,
        pinned.getBoundingClientRect().top,
        scroller.scrollTop,
        scroller.scrollHeight
    ])"#;
    for (root, inner, moving_top, sticky_top) in
        [(100, 70, 25, 20), (110, 75, 10, 10), (0, 0, 195, 175)]
    {
        let reprojection = vm
            .eval(&format!(
                "scrollTo(0,{root});scroller.scrollTop={inner};{sample}"
            ))
            .unwrap();
        let values: Vec<i64> = serde_json::from_str(&reprojection).unwrap();
        assert_eq!(values[0], moving_top);
        assert_eq!(values[1], sticky_top);
        assert_eq!(values[3], 10);
        assert_eq!(values[4], inner);
        assert_eq!(vm.layout_pass_observability_for_test().1, initial_builds);
        if root != 0 {
            assert_eq!(
                vm.eval("document.elementFromPoint(30,sticky.getBoundingClientRect().top+2).id")
                    .unwrap(),
                "sticky"
            );
        }
    }

    let projected = vm
        .eval(&format!("scrollTo(0,100);scroller.scrollTop=70;{sample}"))
        .unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, initial_builds);
    vm._context_host
        .borrow()
        .note_realtime_layout_input_change();
    let rebuilt = vm.eval(sample).unwrap();
    assert_eq!(projected, rebuilt);
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        initial_builds + 1
    );
    assert_eq!(
        vm.eval("moving.style.width='60px';scroller.scrollTop=72;moving.offsetWidth")
            .unwrap(),
        "60"
    );
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        initial_builds + 2
    );
}

#[cfg(feature = "devtools")]
#[tokio::test(flavor = "current_thread")]
async fn realtime_scroll_reuses_layout_for_paint_only_hover_damage() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut vm = realtime_vm(
                r##"<!doctype html><style>
                    html,body { margin:0 }
                    body { height:4000px }
                    a { display:block; width:100px; height:40px; background:red }
                    #repaint:hover { background:lime }
                    #resize:hover { width:160px }
                </style>
                <a id=first href=#first></a>
                <a id=repaint href=#repaint></a>
                <a id=resize href=#resize></a>"##,
            );
            let host = vm.context_host_weak_for_test().upgrade().unwrap();
            let viewport = {
                let host = host.borrow();
                host.layout_viewport_for_document(host.document_handle())
            };
            let session = crate::runtime::RendererScreencastPaintSession::default();
            let paint = |vm: &mut StandaloneScriptVmHarness| {
                vm.paint_current_screen_layout(
                    Some(&session),
                    viewport,
                    moli_layout::PaintCaptureRequest::viewport(),
                    moli_layout::LayoutFlushReason::RenderingOpportunity,
                )
                .unwrap()
                .unwrap()
            };
            let update_hover_under_pointer = |vm: &StandaloneScriptVmHarness| {
                let hit = vm.mouse_input_surface_hit(16., 16., "mousemove").unwrap();
                assert!(host.borrow_mut().set_hovered_element_for_input(hit.target));
            };
            paint(&mut vm);
            let publications = vm.layout_pass_observability_for_test().1;

            update_hover_under_pointer(&vm);
            paint(&mut vm);
            assert_eq!(vm.layout_pass_observability_for_test().1, publications);

            vm.eval("scrollTo(0, 40)").unwrap();
            update_hover_under_pointer(&vm);
            let snapshot = paint(&mut vm);
            assert_eq!(vm.layout_pass_observability_for_test().1, publications);
            assert_eq!(
                vm.eval("String(repaint.getBoundingClientRect().top)")
                    .unwrap(),
                "0"
            );
            let image = moli_paint::raster_snapshot(&snapshot).unwrap();
            let offset = (16 * image.width as usize + 16) * 4;
            assert_eq!(&image.rgba[offset..offset + 4], [0, 255, 0, 255]);

            vm.eval("scrollTo(0, 80)").unwrap();
            update_hover_under_pointer(&vm);
            paint(&mut vm);
            assert_eq!(vm.eval("String(resize.offsetWidth)").unwrap(), "160");
            assert_eq!(vm.layout_pass_observability_for_test().1, publications + 1);
        })
        .await;
}

#[test]
fn realtime_zoomed_rtl_scroll_matches_a_fresh_layout_without_rebuilding() {
    let mut vm = realtime_vm(
        "<!doctype html><div id=scroller style='direction:rtl;zoom:1.25;width:160px;\
         height:50px;overflow:auto'><div id=target style='width:400px;height:20px'></div></div>",
    );
    let sample = "JSON.stringify([scroller.scrollLeft,target.getBoundingClientRect().left])";
    let initial = vm.eval(sample).unwrap();
    let (_, initial_left): (f64, f64) = serde_json::from_str(&initial).unwrap();
    let mut builds = vm.layout_pass_observability_for_test().1;
    let mut previous_left = initial_left;
    for requested in [-37, -10_000, 0] {
        let projected = vm
            .eval(&format!("scroller.scrollLeft={requested};{sample}"))
            .unwrap();
        let (scroll_left, left): (f64, f64) = serde_json::from_str(&projected).unwrap();
        match requested {
            -37 => {
                assert_eq!(scroll_left, -37.0);
                assert!(left > initial_left);
            }
            -10_000 => {
                assert!((-10_000.0..-37.0).contains(&scroll_left));
                assert!(left > previous_left);
            }
            0 => assert_eq!(projected, initial),
            _ => unreachable!(),
        }
        assert_eq!(vm.layout_pass_observability_for_test().1, builds);
        vm._context_host
            .borrow()
            .note_realtime_layout_input_change();
        let fresh = vm.eval(sample).unwrap();
        assert_eq!(projected, fresh);
        builds += 1;
        assert_eq!(vm.layout_pass_observability_for_test().1, builds);
        previous_left = left;
    }
}

#[test]
fn legacy_layout_modes_do_not_allocate_realtime_state_or_refresh_after_writes() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::Lazy] {
        let mut vm = new_parsed_test_vm(
            "https://legacy-layout.test/",
            "<!doctype html><div id=target style='width:100px'></div><input id=field type=button value=A>",
        );
        vm.set_layout_policy(policy);
        assert!(
            !vm._context_host
                .borrow()
                .realtime_layout_allocated_for_test()
        );
        vm.eval("target.offsetWidth").unwrap();
        let before = vm.layout_pass_observability_for_test().1;
        let width = vm
            .eval("target.style.width='200px';field.value='ABCDEFGHIJKLMNOPQRST';String(target.offsetWidth)")
            .unwrap();
        assert_eq!(vm.eval("field.value").unwrap(), "ABCDEFGHIJKLMNOPQRST");
        assert_eq!(vm.layout_pass_observability_for_test().1, before);
        if policy == LayoutPolicy::Lazy {
            assert_eq!(width, "100");
        }
        assert!(
            !vm._context_host
                .borrow()
                .realtime_layout_allocated_for_test()
        );
    }
}

#[test]
fn select_value_refreshes_selected_peer_and_has_styles_in_every_layout_policy() {
    for policy in [
        LayoutPolicy::Mock,
        LayoutPolicy::Lazy,
        LayoutPolicy::Realtime,
    ] {
        for (case, change, same_before, same_after) in [
            (
                "selectedIndex",
                "selectCtl.selectedIndex=1",
                "selectCtl.selectedIndex=0",
                "selectCtl.selectedIndex=1",
            ),
            (
                "value",
                "selectCtl.value='b'",
                "selectCtl.value='a'",
                "selectCtl.value='b'",
            ),
        ] {
            let mut vm = new_parsed_test_vm(
                "https://select-value-style.test/",
                r#"<!doctype html><style>
                    option { color:rgb(1,2,3) }
                    option:checked { color:rgb(4,5,6) }
                    #target { color:rgb(7,8,9) }
                    body:has(#optionB:checked) #target { color:rgb(10,11,12) }
                </style><select id=selectCtl>
                    <option id=optionA value=a>Alpha</option>
                    <option id=optionB value=b>Beta</option>
                </select><div id=target></div>"#,
            );
            vm.set_layout_policy(policy);
            assert_eq!(
                vm.eval(
                    r#"
                    // Set both dirty flags before checking identical-value writes.
                    selectCtl.selectedIndex=0;
                    window.aStyle=getComputedStyle(optionA);
                    window.bStyle=getComputedStyle(optionB);
                    window.targetStyle=getComputedStyle(target);
                    window.styleColors=()=>[aStyle.color,bStyle.color,targetStyle.color];
                    JSON.stringify(styleColors())
                "#
                )
                .unwrap(),
                r#"["rgb(4, 5, 6)","rgb(1, 2, 3)","rgb(7, 8, 9)"]"#,
                "initial held declarations: {policy:?}, {case}"
            );
            let builds = vm.layout_pass_observability_for_test().1;
            let version = vm._context_host.borrow().dom_host().dom_version();
            vm.eval(same_before).unwrap();
            assert_eq!(vm._context_host.borrow().dom_host().dom_version(), version);
            assert_eq!(vm.layout_pass_observability_for_test().1, builds);

            vm.eval(change).unwrap();
            assert_eq!(
                vm.eval("JSON.stringify([selectCtl.value,optionA.selected,optionB.selected])")
                    .unwrap(),
                r#"["b",false,true]"#,
                "native selection must actually change: {policy:?}, {case}"
            );
            assert!(vm._context_host.borrow().dom_host().dom_version() > version);
            assert_eq!(
                vm.eval("JSON.stringify(styleColors())").unwrap(),
                r#"["rgb(1, 2, 3)","rgb(4, 5, 6)","rgb(10, 11, 12)"]"#,
                "selected peer and :has styles must be current: {policy:?}, {case}"
            );
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                builds,
                "pure color reads must not repair this through a layout pass"
            );
            let changed_version = vm._context_host.borrow().dom_host().dom_version();
            vm.eval(same_after).unwrap();
            assert_eq!(
                vm._context_host.borrow().dom_host().dom_version(),
                changed_version
            );
            assert_eq!(vm.layout_pass_observability_for_test().1, builds);
            assert_eq!(
                vm.eval("JSON.stringify(styleColors())").unwrap(),
                r#"["rgb(1, 2, 3)","rgb(4, 5, 6)","rgb(10, 11, 12)"]"#
            );
            if !policy.is_realtime() {
                assert!(
                    !vm._context_host
                        .borrow()
                        .realtime_layout_allocated_for_test()
                );
            }
        }
    }
}
