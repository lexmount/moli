use super::*;

#[test]
fn layout_renderer_constructs_phase_one_roles_from_native_dom_and_stylo() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
            r#"<!doctype html><html><head><style>
html, body, main { display: block; margin: 0 }
#contents { display: contents }
#grid { display: grid }
#grid::before { content: "before"; display: inline }
#cell { display: table-cell }
#item { display: list-item }
#item::marker { content: "marker" }
#unboxed-object { display: contents }
#hidden-control { display: block }
#float { float: left }
#clear { clear: both }
#fixed { position: fixed }
#sticky { position: sticky }
</style></head><body><main id="root">
<div id="contents"><section id="grid">direct <span id="grid-child">child</span></section></div>
<span id="cell">cell</span><li id="item">item</li><input id="control" type="checkbox">
<button id="button">button</button><input id="hidden-control" type="hidden">
<object id="unboxed-object"><span id="object-fallback">must-not-layout</span></object>
<picture id="picture"><span id="picture-child">picture child</span></picture>
<div id="float"></div><div id="clear"></div><div id="fixed"></div><div id="sticky"></div>
</main></body></html>"#,
        )
        .await;
        page_vm.vm_mut().sync_live_document_style_sources();

        let tree = page_vm
            .vm()
            .normalized_layout_box_tree_for_test()
            .expect("native layout construction should succeed")
            .expect("fixture should have a document element");
        let source_line = |source: &str| {
            tree.lines()
                .find(|line| line.contains(source))
                .unwrap_or_else(|| panic!("missing {source} in box tree:\n{tree}"))
        };
        assert!(tree.contains("principal-grid"), "{tree}");
        assert!(tree.contains("source=section#grid"), "{tree}");
        assert!(
            !source_line("source=section#grid").contains("capability=grid-layout-deferred"),
            "{tree}"
        );
        assert!(tree.contains("anonymous-grid-item"), "{tree}");
        assert!(tree.contains("pseudo-before"), "{tree}");
        assert!(tree.contains("anonymous-table-wrapper"), "{tree}");
        assert!(tree.contains("anonymous-table-row-group"), "{tree}");
        assert!(tree.contains("anonymous-table-row"), "{tree}");
        assert!(tree.contains("source=span#cell"), "{tree}");
        assert!(
            !source_line("source=span#cell").contains("capability=table-layout-deferred"),
            "{tree}"
        );
        assert!(tree.contains("pseudo-marker"), "{tree}");
        assert!(
            !source_line("source=li#item").contains("capability=list-marker-layout-deferred"),
            "{tree}"
        );
        assert!(tree.contains("source=input#control"), "{tree}");
        assert!(tree.contains("category=form-input-checkbox"), "{tree}");
        assert!(tree.contains("replaced=form-control"), "{tree}");
        assert!(
            tree.contains("display=inline-block source=button#button"),
            "{tree}"
        );
        assert!(!tree.contains("source=div#contents"), "{tree}");
        assert!(!tree.contains("source=input#hidden-control"), "{tree}");
        assert!(!tree.contains("source=object#unboxed-object"), "{tree}");
        assert!(!tree.contains("source=span#object-fallback"), "{tree}");
        assert!(!tree.contains("source=picture#picture"), "{tree}");
        assert!(tree.contains("source=span#picture-child"), "{tree}");
        for source in ["source=div#float", "source=div#clear"] {
            assert!(
                !source_line(source).contains("capability=float-or-clear-layout-deferred"),
                "{tree}"
            );
        }
        assert!(
            !source_line("source=div#fixed").contains("capability=fixed-position-layout-deferred"),
            "{tree}"
        );
        assert!(
            !source_line("source=div#sticky")
                .contains("capability=sticky-position-layout-deferred"),
            "{tree}"
        );
    }));
}
#[test]
fn layout_renderer_computes_phase_two_grid_calc_and_positioned_geometry_from_stylo() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { display: block; margin: 0; padding: 0 }
html { scrollbar-width: none }
body { background: rgb(0, 0, 255) }
#grid { display: grid; width: 400px; height: 200px; gap: 20px 10px;
  grid-template-columns: 100px 1fr; grid-template-rows: 50px 1fr; grid-auto-rows: 30px }
#first { background: rgb(255, 0, 0); grid-column: 1; grid-row: 1 }
#second { background: rgb(0, 255, 0); grid-column: 2; grid-row: 1 / 3 }
#implicit { background: rgb(0, 128, 128); grid-column: 1; grid-row: 3 }
#calc { background: rgb(255, 255, 0); width: calc(50% - 10px); height: 20px }
#positioned { box-sizing: content-box; position: relative; margin: 30px;
  width: 400px; height: 300px; padding: 20px; border: 5px solid transparent }
#static { margin: 10px; width: 200px; height: 100px }
#absolute { position: absolute; left: 10%; top: 25%; width: 50%; height: 10px;
  background: rgb(0, 255, 255) }
#fixed { position: fixed; right: 10px; bottom: 20px; width: 30px; height: 40px;
  background: rgb(255, 0, 255) }
</style></head><body><div id="grid"><div id="first"></div><div id="second"></div><div id="implicit"></div></div><div id="calc"></div><div id="positioned"><div id="static"><div id="absolute"></div></div><div id="fixed"></div></div></body></html>"#,
            )
            .await;
            page_vm.vm_mut().sync_live_document_style_sources();

            let snapshot = page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("native layout should succeed")
                .expect("fixture should have a document element");
            assert_eq!(
                snapshot.canvas_color,
                moli_layout::PaintColor::new(0.0, 0.0, 1.0, 1.0)
            );
            assert_eq!(
                snapshot.content_size,
                moli_layout::PaintSize::new(800.0, 630.0)
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(1.0, 0.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(0.0, 0.0, 100.0, 50.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(0.0, 1.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(110.0, 0.0, 290.0, 150.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(
                        0.0,
                        128.0 / 255.0,
                        128.0 / 255.0,
                        1.0,
                    ),
                ),
                moli_layout::PaintRect::new(0.0, 170.0, 100.0, 30.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(1.0, 1.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(0.0, 200.0, 390.0, 20.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(0.0, 1.0, 1.0, 1.0),
                ),
                moli_layout::PaintRect::new(79.0, 340.0, 220.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(1.0, 0.0, 1.0, 1.0),
                ),
                moli_layout::PaintRect::new(760.0, 540.0, 30.0, 40.0),
            );
            assert!(snapshot.diagnostics.iter().all(|diagnostic| {
                diagnostic.code != "grid-layout-deferred"
                    && diagnostic.code != "fixed-position-layout-deferred"
            }));
        }));
}
#[test]
fn layout_renderer_projects_static_position_and_flex_order_from_stylo() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { display: block; margin: 0; padding: 0 }
#static { position: static; left: 80px; top: 40px; width: 20px; height: 10px;
  background: rgb(255, 0, 0) }
#flex { display: flex; width: 300px; height: 40px; align-items: center }
#late { order: 2; flex: 1 1 100px; height: 20px; background: rgb(0, 0, 255) }
#early { order: -1; flex: 3 1 100px; height: 40px; background: rgb(0, 255, 0) }
#cb { position: relative; width: 200px; height: 100px }
#mid { margin: 10px; width: 80px; height: 30px }
#auto-static { position: absolute; width: 20px; height: 10px;
  background: rgb(255, 255, 0) }
#intrinsic { width: min-content; height: 10px; min-height: min-content }
#basis { display: flex }
#basis-child { flex: 0 0 content; min-width: 0; width: 10px; height: 10px;
  background: rgb(255, 128, 0) }
#basis-content { width: 80px; height: 10px }
#clamped { width: 90%; max-width: 240px; height: 10px; margin-left: 30px;
  background: rgb(128, 0, 128) }
</style></head><body><div id="static"></div><div id="flex"><div id="late"></div><div id="early"></div></div><div id="cb"><div id="mid"><div id="auto-static"></div></div></div><div id="intrinsic"></div><div id="basis"><div id="basis-child"><div id="basis-content"></div></div></div><div id="clamped"></div></body></html>"#,
            )
            .await;
            page_vm.vm_mut().sync_live_document_style_sources();

            let snapshot = page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("native layout should succeed")
                .expect("fixture should have a document element");

            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(1.0, 128.0 / 255.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(0.0, 170.0, 80.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(1.0, 0.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(0.0, 0.0, 20.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(0.0, 1.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(0.0, 10.0, 175.0, 40.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(0.0, 0.0, 1.0, 1.0),
                ),
                moli_layout::PaintRect::new(175.0, 20.0, 125.0, 20.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(1.0, 1.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(10.0, 60.0, 20.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(
                        128.0 / 255.0,
                        0.0,
                        128.0 / 255.0,
                        1.0,
                    ),
                ),
                moli_layout::PaintRect::new(30.0, 180.0, 240.0, 10.0),
            );
            assert!(snapshot.diagnostics.iter().all(|diagnostic| {
                diagnostic.code != "positioned-static-position-deferred"
                    || !diagnostic.message.contains("div#auto-static")
            }));
            assert!(snapshot.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "intrinsic-sizing-keyword-deferred"
                    && diagnostic.message.contains("div#intrinsic")
            }));
            assert!(snapshot
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "flex-basis-content-deferred"));
        }));
}
#[test]
fn layout_renderer_preserves_flex_static_position_edges_across_containing_blocks() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
            r#"<!doctype html><html><head><style>
html,body{display:block;margin:0;padding:0}
.case{display:flex;box-sizing:border-box;width:200px;height:100px;padding:10px 20px}
.case>div{position:absolute;width:20px;height:10px}
#start{background:red}
#center-parent{justify-content:center;align-items:center}#center{background:green}
#end-parent{justify-content:flex-end;align-items:flex-end}#end{background:blue}
#reverse-parent{flex-direction:row-reverse}#reverse{background:yellow}
#wrap-reverse-parent{flex-wrap:wrap-reverse}#wrap-reverse{background:magenta}
#rtl-parent{direction:rtl}#rtl{background:cyan}
#rtl-reverse-parent{direction:rtl;flex-direction:row-reverse}#rtl-reverse{background:maroon}
#column-parent{flex-direction:column;justify-content:center;align-items:center}
#column{background:darkgreen}
#vertical-parent{writing-mode:vertical-rl}#vertical{background:navy}
#vertical-rtl-parent{writing-mode:vertical-rl;direction:rtl}#vertical-rtl{background:olive}
#safe-parent{align-items:safe center}#safe{height:100px;background:rgb(123,45,67)}
</style></head><body>
<div class=case><div id=start></div></div>
<div class=case id=center-parent><div id=center></div></div>
<div class=case id=end-parent><div id=end></div></div>
<div class=case id=reverse-parent><div id=reverse></div></div>
<div class=case id=wrap-reverse-parent><div id=wrap-reverse></div></div>
<div class=case id=rtl-parent><div id=rtl></div></div>
<div class=case id=rtl-reverse-parent><div id=rtl-reverse></div></div>
<div class=case id=column-parent><div id=column></div></div>
<div class=case id=vertical-parent><div id=vertical></div></div>
<div class=case id=vertical-rtl-parent><div id=vertical-rtl></div></div>
<div class=case id=safe-parent><div id=safe></div></div>
</body></html>"#,
        )
        .await;
        page_vm.vm_mut().sync_live_document_style_sources();

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 1200, 1.0))
            .expect("native layout should succeed")
            .expect("fixture should have a document element");

        let cases = [
            (
                moli_layout::PaintColor::new(1.0, 0.0, 0.0, 1.0),
                moli_layout::PaintRect::new(20.0, 10.0, 20.0, 10.0),
            ),
            (
                moli_layout::PaintColor::new(0.0, 128.0 / 255.0, 0.0, 1.0),
                moli_layout::PaintRect::new(90.0, 145.0, 20.0, 10.0),
            ),
            (
                moli_layout::PaintColor::new(0.0, 0.0, 1.0, 1.0),
                moli_layout::PaintRect::new(160.0, 280.0, 20.0, 10.0),
            ),
            (
                moli_layout::PaintColor::new(1.0, 1.0, 0.0, 1.0),
                moli_layout::PaintRect::new(160.0, 310.0, 20.0, 10.0),
            ),
            (
                moli_layout::PaintColor::new(1.0, 0.0, 1.0, 1.0),
                moli_layout::PaintRect::new(20.0, 480.0, 20.0, 10.0),
            ),
            (
                moli_layout::PaintColor::new(0.0, 1.0, 1.0, 1.0),
                moli_layout::PaintRect::new(160.0, 510.0, 20.0, 10.0),
            ),
            (
                moli_layout::PaintColor::new(128.0 / 255.0, 0.0, 0.0, 1.0),
                moli_layout::PaintRect::new(20.0, 610.0, 20.0, 10.0),
            ),
            (
                moli_layout::PaintColor::new(0.0, 100.0 / 255.0, 0.0, 1.0),
                moli_layout::PaintRect::new(90.0, 745.0, 20.0, 10.0),
            ),
            (
                moli_layout::PaintColor::new(0.0, 0.0, 128.0 / 255.0, 1.0),
                moli_layout::PaintRect::new(160.0, 810.0, 20.0, 10.0),
            ),
            (
                moli_layout::PaintColor::new(128.0 / 255.0, 128.0 / 255.0, 0.0, 1.0),
                moli_layout::PaintRect::new(160.0, 980.0, 20.0, 10.0),
            ),
            (
                moli_layout::PaintColor::new(123.0 / 255.0, 45.0 / 255.0, 67.0 / 255.0, 1.0),
                moli_layout::PaintRect::new(20.0, 1010.0, 20.0, 100.0),
            ),
        ];
        for (color, expected) in cases {
            assert_paint_rect(solid_paint_rect(&snapshot, color), expected);
        }
        assert!(
            snapshot
                .diagnostics
                .iter()
                .all(|diagnostic| { diagnostic.code != "positioned-static-position-deferred" })
        );
    }));
}
#[test]
fn layout_renderer_preserves_calc_min_width_in_float_intrinsic_contribution() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
            r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
#host{width:0}
#outer{float:left;height:30px;background:rgb(101,102,103)}
#inner{width:40px;min-width:calc(160px + 0%);height:20px;background:rgb(111,112,113)}
#definite{clear:both;width:200px;height:20px}
#definite-child{width:40px;min-width:calc(20px + 50%);height:20px;background:rgb(121,122,123)}
</style></head><body><div id=host><div id=outer><div id=inner></div></div></div>
<div id=definite><div id=definite-child></div></div></body></html>"#,
        )
        .await;
        page_vm.vm_mut().sync_live_document_style_sources();

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
            .expect("float intrinsic layout should succeed")
            .expect("fixture should have a document element");
        assert_paint_rect(
            solid_paint_rect(
                &snapshot,
                moli_layout::PaintColor::new(101.0 / 255.0, 102.0 / 255.0, 103.0 / 255.0, 1.0),
            ),
            moli_layout::PaintRect::new(0.0, 0.0, 160.0, 30.0),
        );
        assert_paint_rect(
            solid_paint_rect(
                &snapshot,
                moli_layout::PaintColor::new(111.0 / 255.0, 112.0 / 255.0, 113.0 / 255.0, 1.0),
            ),
            moli_layout::PaintRect::new(0.0, 0.0, 160.0, 20.0),
        );
        assert_paint_rect(
            solid_paint_rect(
                &snapshot,
                moli_layout::PaintColor::new(121.0 / 255.0, 122.0 / 255.0, 123.0 / 255.0, 1.0),
            ),
            moli_layout::PaintRect::new(0.0, 30.0, 120.0, 20.0),
        );
    }));
}
#[test]
fn layout_renderer_clamps_definite_ifc_probes_to_intrinsic_contributions() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
.host{display:flow-root}.zero{width:0}.wide{width:400px}
.outer{float:left;height:30px}.inner{height:20px}
.atom{display:inline-block;width:60px;height:20px}
#zero-outer{background:rgb(131,132,133)}#wide-outer{background:rgb(141,142,143)}
</style></head><body>
<div class="host zero"><div id=zero-outer class=outer><div class=inner><span class=atom></span></div></div></div>
<div class="host wide"><div id=wide-outer class=outer><div class=inner><span class=atom></span></div></div></div>
</body></html>"#,
            )
            .await;
            page_vm.vm_mut().sync_live_document_style_sources();

            let snapshot = page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("intrinsic IFC probe layout should succeed")
                .expect("fixture should have a document element");
            for (color, expected) in [
                (
                    moli_layout::PaintColor::new(
                        131.0 / 255.0,
                        132.0 / 255.0,
                        133.0 / 255.0,
                        1.0,
                    ),
                    moli_layout::PaintRect::new(0.0, 0.0, 60.0, 30.0),
                ),
                (
                    moli_layout::PaintColor::new(
                        141.0 / 255.0,
                        142.0 / 255.0,
                        143.0 / 255.0,
                        1.0,
                    ),
                    moli_layout::PaintRect::new(0.0, 30.0, 60.0, 30.0),
                ),
            ] {
                assert_paint_rect(solid_paint_rect(&snapshot, color), expected);
            }
        }));
}
#[test]
fn layout_renderer_resolves_cyclic_preferred_width_after_parent_contribution() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
.host{display:flow-root;width:0}.outer{float:left;width:auto;height:30px}
.inner{height:20px}.raw{width:50%}.calc{width:calc(40px + 0%)}
.atom{display:inline-block;width:60px;height:20px}
#raw-outer{background:rgb(151,152,153)}#raw-inner{background:rgb(161,162,163)}
#calc-outer{background:rgb(171,172,173)}#calc-inner{background:rgb(181,182,183)}
</style></head><body>
<div class=host><div id=raw-outer class=outer><div id=raw-inner class="inner raw"><span class=atom></span></div></div></div>
<div class=host><div id=calc-outer class=outer><div id=calc-inner class="inner calc"><span class=atom></span></div></div></div>
</body></html>"#,
            )
            .await;
            page_vm.vm_mut().sync_live_document_style_sources();

            let snapshot = page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("cyclic preferred-width layout should succeed")
                .expect("fixture should have a document element");
            for (color, expected) in [
                (
                    moli_layout::PaintColor::new(
                        151.0 / 255.0,
                        152.0 / 255.0,
                        153.0 / 255.0,
                        1.0,
                    ),
                    moli_layout::PaintRect::new(0.0, 0.0, 60.0, 30.0),
                ),
                (
                    moli_layout::PaintColor::new(
                        161.0 / 255.0,
                        162.0 / 255.0,
                        163.0 / 255.0,
                        1.0,
                    ),
                    moli_layout::PaintRect::new(0.0, 0.0, 30.0, 20.0),
                ),
                (
                    moli_layout::PaintColor::new(
                        171.0 / 255.0,
                        172.0 / 255.0,
                        173.0 / 255.0,
                        1.0,
                    ),
                    moli_layout::PaintRect::new(0.0, 30.0, 60.0, 30.0),
                ),
                (
                    moli_layout::PaintColor::new(
                        181.0 / 255.0,
                        182.0 / 255.0,
                        183.0 / 255.0,
                        1.0,
                    ),
                    moli_layout::PaintRect::new(0.0, 30.0, 40.0, 20.0),
                ),
            ] {
                assert_paint_rect(solid_paint_rect(&snapshot, color), expected);
            }
        }));
}
#[test]
fn fixed_table_layout_distributes_unresolved_columns_equally() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{border-spacing:0;width:300px;table-layout:fixed}
td{padding:0;border:0;height:10px}
#calc-cell{width:calc(50% - 20px);background:rgb(61,62,63)}
#remaining-cell{background:rgb(71,72,73)}
</style></head><body><table><tr><td id=calc-cell></td><td id=remaining-cell></td></tr></table></body></html>"#,
            )
            .await;

            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(61, 62, 63)),
                moli_layout::PaintRect::new(0.0, 0.0, 150.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(71, 72, 73)),
                moli_layout::PaintRect::new(150.0, 0.0, 150.0, 10.0),
            );
        }));
}
#[test]
fn fixed_percentage_table_exports_its_parent_facing_max_content_size() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}html{scrollbar-width:none}
.case{width:100px}.wrapper{display:table}.cell{display:table-cell}
.flex{display:flex;height:10px}.flex table{height:5px}
#percent-flex{background:rgb(201,11,11)}#percent-table{background:rgb(201,12,12)}
#calc-flex{background:rgb(202,11,11)}#calc-table{background:rgb(202,12,12)}
#length-flex{background:rgb(203,11,11)}#length-table{background:rgb(203,12,12)}
#auto-flex{background:rgb(204,11,11)}#auto-table{background:rgb(204,12,12)}
</style></head><body>
<div class=case><div class=wrapper><div class=cell><div id=percent-flex class=flex><table id=percent-table style="table-layout:fixed;width:100%"></table></div></div></div></div>
<div class=case><div class=wrapper><div class=cell><div id=calc-flex class=flex><table id=calc-table style="table-layout:fixed;width:calc(40px + 0%)"></table></div></div></div></div>
<div class=case><div class=wrapper><div class=cell><div id=length-flex class=flex><table id=length-table style="table-layout:fixed;width:40px"></table></div></div></div></div>
<div class=case><div class=wrapper><div class=cell><div id=auto-flex class=flex><table id=auto-table style="table-layout:auto;width:100%"></table></div></div></div></div>
</body></html>"#,
            )
            .await;

            // Chromium gives a percentage-dependent fixed table an
            // effectively unbounded max-content contribution. The wrapper
            // therefore fills the 100px opportunity, while final layout still
            // resolves the authored width normally. Definite and automatic
            // controls must retain their ordinary finite contributions.
            for (color, expected_width) in [
                (rgb(201, 11, 11), 100.0),
                (rgb(201, 12, 12), 100.0),
                (rgb(202, 11, 11), 100.0),
                (rgb(202, 12, 12), 40.0),
                (rgb(203, 11, 11), 40.0),
                (rgb(203, 12, 12), 40.0),
                (rgb(204, 11, 11), 4.0),
                (rgb(204, 12, 12), 4.0),
            ] {
                let rect = solid_paint_rect(&snapshot, color);
                assert!(
                    (rect.width - expected_width).abs() <= 0.01,
                    "expected width {expected_width}, got {rect:?}",
                );
            }
        }));
}
#[test]
fn table_layout_fixed_with_auto_width_uses_automatic_column_measurement() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}html{scrollbar-width:none}
table{table-layout:fixed;width:auto;border-spacing:0;background:rgb(205,11,11)}
td{padding:0;border:0;height:5px;background:rgb(205,12,12)}
.wide{width:80px;height:5px;background:rgb(205,13,13)}
</style></head><body><table><tr><td style="width:10px"></td></tr><tr><td><div class=wide></div></td></tr></table></body></html>"#,
            )
            .await;

            // `table-layout: fixed` selects the fixed algorithm only when the
            // table has an eligible non-auto preferred width. Chromium keeps
            // the computed property value but measures this table with the
            // automatic algorithm, so the second row contributes 80px.
            for color in [rgb(205, 11, 11), rgb(205, 12, 12), rgb(205, 13, 13)] {
                let rect = solid_paint_rect(&snapshot, color);
                assert!(
                    (rect.width - 80.0).abs() <= 0.01,
                    "auto-width table should measure every row: {rect:?}",
                );
            }
        }));
}
#[test]
fn automatic_table_layout_collects_authored_widths_after_a_colspan_header() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let snapshot = render_test_snapshot(
            r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{border-spacing:3px;background:rgb(10,11,12)}
td{box-sizing:border-box;padding:0;border:0}
.month{height:4px}
.day{width:10px;height:10px}
#first{background:rgb(21,31,41)}#second{background:rgb(22,32,42)}#third{background:rgb(23,33,43)}
</style></head><body><table>
<tr><td class=month colspan=3></td></tr>
<tr><td id=first class=day></td><td id=second class=day></td><td id=third class=day></td></tr>
</table></body></html>"#,
        )
        .await;

        // GitHub's contribution calendar has the same shape: a leading
        // month-label row spans columns, while the authored 10px day-cell
        // widths only appear in later rows. Automatic layout must collect
        // those later constraints; the 3px border spacing stays separate.
        assert_paint_rect(
            solid_paint_rect(&snapshot, rgb(10, 11, 12)),
            moli_layout::PaintRect::new(0.0, 0.0, 42.0, 23.0),
        );
        for (color, x) in [
            (rgb(21, 31, 41), 3.0),
            (rgb(22, 32, 42), 16.0),
            (rgb(23, 33, 43), 29.0),
        ] {
            assert_paint_rect(
                solid_paint_rect(&snapshot, color),
                moli_layout::PaintRect::new(x, 10.0, 10.0, 10.0),
            );
        }
    }));
}
#[test]
fn automatic_table_layout_matches_chromium_distribution_phases_and_colspans() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{table-layout:auto;border-spacing:0}
td{box-sizing:border-box;border:0;padding:0;height:10px}
#mixed{width:400px}#mixed-auto>div{width:90px;height:10px}
#mixed-percent{background:rgb(31,41,51)}#mixed-fixed{background:rgb(32,42,52)}#mixed-auto{background:rgb(33,43,53)}
#fixed-only{width:300px}#fixed-small{background:rgb(41,51,61)}#fixed-large{background:rgb(42,52,62)}
#percent-only{width:200px}#percent-left{background:rgb(51,61,71)}#percent-right{background:rgb(52,62,72)}
#spans{width:220px;border-spacing:10px 0}#span-left>div{width:40px;height:10px}#span-right>div{width:60px;height:10px}
#span-left{background:rgb(61,71,81)}#span-right{background:rgb(62,72,82)}
#percent-span{width:300px}#percent-span-left{background:rgb(71,81,91)}#percent-span-right{background:rgb(72,82,92)}
</style></head><body>
<table id=mixed><col style="width:25%"><col style="width:60px"><col><tr><td id=mixed-percent></td><td id=mixed-fixed></td><td id=mixed-auto><div></div></td></tr></table>
<table id=fixed-only><col style="width:50px"><col style="width:100px"><tr><td id=fixed-small></td><td id=fixed-large></td></tr></table>
<table id=percent-only><col style="width:25%"><col style="width:25%"><tr><td id=percent-left></td><td id=percent-right></td></tr></table>
<table id=spans><tr><td colspan=2 style="width:150px"></td></tr><tr><td id=span-left><div></div></td><td id=span-right><div></div></td></tr></table>
<table id=percent-span><tr><td colspan=2 style="width:60%"></td></tr><tr><td id=percent-span-left></td><td id=percent-span-right></td></tr></table>
</body></html>"#,
            )
            .await;

            // Captured with Chromium 147. The cases exercise, in order:
            // automatic-column priority above MAX, constrained-column growth,
            // percentage-only fallback, constrained colspan distribution with
            // separate border spacing, and percentage colspan projection.
            for (color, expected) in [
                (rgb(31, 41, 51), (0.0, 0.0, 100.0)),
                (rgb(32, 42, 52), (100.0, 0.0, 60.0)),
                (rgb(33, 43, 53), (160.0, 0.0, 240.0)),
                (rgb(41, 51, 61), (0.0, 10.0, 100.0)),
                (rgb(42, 52, 62), (100.0, 10.0, 200.0)),
                (rgb(51, 61, 71), (0.0, 20.0, 100.0)),
                (rgb(52, 62, 72), (100.0, 20.0, 100.0)),
                (rgb(61, 71, 81), (10.0, 40.0, 76.0)),
                (rgb(62, 72, 82), (96.0, 40.0, 114.0)),
                (rgb(71, 81, 91), (0.0, 60.0, 150.0)),
                (rgb(72, 82, 92), (150.0, 60.0, 150.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&snapshot, color),
                    moli_layout::PaintRect::new(expected.0, expected.1, expected.2, 10.0),
                );
            }
        }));
}
#[test]
fn automatic_table_layout_preserves_chromium_constraint_collection_edges() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{table-layout:auto;border-spacing:0}
td{box-sizing:border-box;border:0;padding:0;height:10px}
#over{width:300px}#over-a{background:rgb(81,91,101)}#over-b{background:rgb(82,92,102)}
#group{width:240px}#group-a{background:rgb(91,101,111)}#group-b{background:rgb(92,102,112)}#group-c{background:rgb(93,103,113)}
#rtl{width:200px;direction:rtl}#rtl-a{background:rgb(101,111,121)}#rtl-b{background:rgb(102,112,122)}
#padded{width:300px}#padded-a{box-sizing:content-box;width:50%;padding:0 10px;background:rgb(111,121,131)}#padded-b{background:rgb(112,122,132)}
#conflict{width:300px}#conflict-a{width:120px;background:rgb(121,131,141)}#conflict-b{background:rgb(122,132,142)}
</style></head><body>
<table id=over><col style="width:70%"><col style="width:60%"><tr><td id=over-a></td><td id=over-b></td></tr></table>
<table id=group><colgroup span=2 style="width:40px"></colgroup><col><tr><td id=group-a></td><td id=group-b></td><td id=group-c></td></tr></table>
<table id=rtl><col style="width:50px"><col><tr><td id=rtl-a></td><td id=rtl-b></td></tr></table>
<table id=padded><tr><td id=padded-a></td><td id=padded-b></td></tr></table>
<table id=conflict><col style="width:80px"><col><tr><td id=conflict-a></td><td id=conflict-b></td></tr></table>
</body></html>"#,
            )
            .await;

            // Captured with Chromium 147. These cases cross the DOM/style
            // collection boundary before exercising percentage clipping,
            // colgroup expansion, logical RTL placement, table-cell sizing
            // box semantics, and column/cell constraint precedence.
            for (color, expected) in [
                (rgb(81, 91, 101), (0.0, 0.0, 210.0)),
                (rgb(82, 92, 102), (210.0, 0.0, 90.0)),
                (rgb(91, 101, 111), (0.0, 10.0, 40.0)),
                (rgb(92, 102, 112), (40.0, 10.0, 40.0)),
                (rgb(93, 103, 113), (80.0, 10.0, 160.0)),
                (rgb(101, 111, 121), (150.0, 20.0, 50.0)),
                (rgb(102, 112, 122), (0.0, 20.0, 150.0)),
                (rgb(111, 121, 131), (0.0, 30.0, 150.0)),
                (rgb(112, 122, 132), (150.0, 30.0, 150.0)),
                (rgb(121, 131, 141), (0.0, 40.0, 120.0)),
                (rgb(122, 132, 142), (120.0, 40.0, 180.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&snapshot, color),
                    moli_layout::PaintRect::new(expected.0, expected.1, expected.2, 10.0),
                );
            }
        }));
}
#[test]
fn fixed_table_layout_distributes_first_row_colspan_constraints() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{table-layout:fixed;border-spacing:0}td{border:0;padding:0;height:10px}
#lengths{width:300px}#l0{background:rgb(10,20,30)}#l1{background:rgb(20,30,40)}#l2{background:rgb(30,40,50)}#l3{background:rgb(40,50,60)}
#percents{width:500px}#p0{background:rgb(50,60,70)}#p1{background:rgb(60,70,80)}#p2{background:rgb(70,80,90)}#p3{background:rgb(80,90,100)}#p4{background:rgb(90,100,110)}
#priority{width:300px}#c0{background:rgb(100,110,120)}#c1{background:rgb(110,120,130)}#c2{background:rgb(120,130,140)}
#spacing{width:350px;border-spacing:10px}#s0{background:rgb(130,140,150)}#s1{background:rgb(140,150,160)}#s2{background:rgb(150,160,170)}#s3{background:rgb(160,170,180)}
</style></head><body>
<table id=lengths><tr><td colspan=2 style="width:100px"></td><td colspan=2 style="width:200px"></td></tr><tr><td id=l0></td><td id=l1></td><td id=l2></td><td id=l3></td></tr></table>
<table id=percents><tr><td colspan=2 style="width:40%"></td><td colspan=2 style="width:20%"></td><td style="width:40%"></td></tr><tr><td id=p0></td><td id=p1></td><td id=p2></td><td id=p3></td><td id=p4></td></tr></table>
<table id=priority><col style="width:80px"><col><col><tr><td colspan=2 style="width:200px"></td><td></td></tr><tr><td id=c0></td><td id=c1></td><td id=c2></td></tr></table>
<table id=spacing><tr><td colspan=2 style="width:110px"></td><td colspan=2 style="width:210px"></td></tr><tr><td id=s0></td><td id=s1></td><td id=s2></td><td id=s3></td></tr></table>
</body></html>"#,
            )
            .await;

            // These values are Chromium's fixed-table constraint geometry.
            // Wide first-row cells contribute one constraint that is divided
            // over their tracks; explicit columns retain priority, and inner
            // border-spacing is removed before division.
            for (color, expected) in [
                (rgb(10, 20, 30), (0.0, 10.0, 50.0)),
                (rgb(20, 30, 40), (50.0, 10.0, 50.0)),
                (rgb(30, 40, 50), (100.0, 10.0, 100.0)),
                (rgb(40, 50, 60), (200.0, 10.0, 100.0)),
                (rgb(50, 60, 70), (0.0, 30.0, 100.0)),
                (rgb(60, 70, 80), (100.0, 30.0, 100.0)),
                (rgb(70, 80, 90), (200.0, 30.0, 50.0)),
                (rgb(80, 90, 100), (250.0, 30.0, 50.0)),
                (rgb(90, 100, 110), (300.0, 30.0, 200.0)),
                (rgb(100, 110, 120), (0.0, 50.0, 80.0)),
                (rgb(110, 120, 130), (80.0, 50.0, 100.0)),
                (rgb(120, 130, 140), (180.0, 50.0, 120.0)),
                (rgb(130, 140, 150), (10.0, 90.0, 50.0)),
                (rgb(140, 150, 160), (70.0, 90.0, 50.0)),
                (rgb(150, 160, 170), (130.0, 90.0, 100.0)),
                (rgb(160, 170, 180), (240.0, 90.0, 100.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&snapshot, color),
                    moli_layout::PaintRect::new(expected.0, expected.1, expected.2, 10.0),
                );
            }
        }));
}
#[test]
fn fixed_table_layout_grows_to_its_definite_column_minimum() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{border-spacing:0;width:300px;table-layout:fixed}
col{width:200px}td{padding:0;border:0;height:10px}
#first{background:rgb(81,82,83)}#second{background:rgb(91,92,93)}
#collapsed{border-collapse:collapse}
#collapsed-first{background:rgb(171,172,173)}#collapsed-second{background:rgb(181,182,183)}
</style></head><body>
<table><col><col><tr><td id=first></td><td id=second></td></tr></table>
<table id=collapsed><col><col><tr><td id=collapsed-first></td><td id=collapsed-second></td></tr></table>
</body></html>"#,
            )
            .await;

            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(81, 82, 83)),
                moli_layout::PaintRect::new(0.0, 0.0, 200.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(91, 92, 93)),
                moli_layout::PaintRect::new(200.0, 0.0, 200.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(171, 172, 173)),
                moli_layout::PaintRect::new(0.0, 10.0, 200.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(181, 182, 183)),
                moli_layout::PaintRect::new(200.0, 10.0, 200.0, 10.0),
            );
        }));
}
#[test]
fn fixed_table_layout_grows_fixed_columns_when_no_auto_column_remains() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{border-spacing:0;width:400px;table-layout:fixed}
td{padding:0;border:0;height:10px}
#first{width:50px;background:rgb(101,102,103)}
#second{width:100px;background:rgb(111,112,113)}
#third{width:25%;background:rgb(121,122,123)}
</style></head><body><table><tr><td id=first></td><td id=second></td><td id=third></td></tr></table></body></html>"#,
            )
            .await;

            for (color, expected) in [
                (rgb(101, 102, 103), (0.0, 100.0)),
                (rgb(111, 112, 113), (100.0, 200.0)),
                (rgb(121, 122, 123), (300.0, 100.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&snapshot, color),
                    moli_layout::PaintRect::new(expected.0, 0.0, expected.1, 10.0),
                );
            }
        }));
}
#[test]
fn fixed_table_layout_adds_content_box_padding_to_percent_cell_measure() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{border-spacing:0;width:300px;table-layout:fixed}
td{border:0;height:10px;padding-top:0;padding-bottom:0}
#first{box-sizing:content-box;width:50%;padding-left:10px;padding-right:10px;background:rgb(131,132,133)}
#second{padding-left:0;padding-right:0;background:rgb(141,142,143)}
</style></head><body><table><tr><td id=first></td><td id=second></td></tr></table></body></html>"#,
            )
            .await;

            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(131, 132, 133)),
                moli_layout::PaintRect::new(0.0, 0.0, 170.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(141, 142, 143)),
                moli_layout::PaintRect::new(170.0, 0.0, 130.0, 10.0),
            );
        }));
}
#[test]
fn fixed_table_layout_clamps_border_box_cell_width_to_its_insets() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{border-spacing:0;width:100px;table-layout:fixed}
td{border:0;height:10px;padding-top:0;padding-bottom:0}
#first{box-sizing:border-box;width:10px;padding-left:20px;padding-right:20px;background:rgb(151,152,153)}
#second{padding-left:0;padding-right:0;background:rgb(161,162,163)}
</style></head><body><table><tr><td id=first></td><td id=second></td></tr></table></body></html>"#,
            )
            .await;

            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(151, 152, 153)),
                moli_layout::PaintRect::new(0.0, 0.0, 40.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(161, 162, 163)),
                moli_layout::PaintRect::new(40.0, 0.0, 60.0, 10.0),
            );
        }));
}
#[test]
fn fixed_table_layout_preserves_native_and_explicit_box_sizing() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{position:absolute;left:0;border:4px solid black;border-spacing:0;width:100px;table-layout:fixed}
td{padding:0;border:0;height:10px}
#native{top:0}#content{top:30px;box-sizing:content-box}
#native-first{background:rgb(191,192,193)}#native-second{background:rgb(201,202,203)}
#content-first{background:rgb(211,212,213)}#content-second{background:rgb(221,222,223)}
</style></head><body>
<table id=native><tr><td id=native-first></td><td id=native-second></td></tr></table>
<table id=content><tr><td id=content-first></td><td id=content-second></td></tr></table>
</body></html>"#,
            )
            .await;

            for (color, expected) in [
                (rgb(191, 192, 193), (4.0, 4.0, 46.0)),
                (rgb(201, 202, 203), (50.0, 4.0, 46.0)),
                (rgb(211, 212, 213), (4.0, 34.0, 50.0)),
                (rgb(221, 222, 223), (54.0, 34.0, 50.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&snapshot, color),
                    moli_layout::PaintRect::new(expected.0, expected.1, expected.2, 10.0),
                );
            }
        }));
}
#[test]
fn layout_renderer_projects_current_blitz_taffy_parity_from_stylo() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let rgb = |red: u8, green: u8, blue: u8| {
                moli_layout::PaintColor::new(
                    f32::from(red) / 255.0,
                    f32::from(green) / 255.0,
                    f32::from(blue) / 255.0,
                    1.0,
                )
            };

            let mut inline_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
.host{width:200px;height:40px;font-size:0;line-height:0}
.atom{display:inline-block;vertical-align:top;width:20px;height:10px;position:relative}
#ltr-atomic{left:10%;right:40px;top:25%;bottom:20px;background:rgb(201,1,1)}
#rtl{direction:rtl;text-align:left}#rtl-atomic{direction:ltr;left:10px;right:10%;top:auto;bottom:5px;background:rgb(1,201,1)}
#right-row{display:flex;width:200px;height:20px;justify-content:right}#right-item{width:20px;height:10px;background:rgb(1,1,201)}
#column-row{display:flex;flex-direction:column;width:50px;height:100px;justify-content:right}#column-item{width:20px;height:20px;background:rgb(201,201,1)}
#self-grid{display:grid;width:200px;height:20px}#self-item{width:20px;height:10px;direction:rtl;justify-self:self-start;background:rgb(201,1,201)}
</style></head><body><div id=ltr class=host><div id=ltr-atomic class=atom></div></div>
<div id=rtl class=host><div id=rtl-atomic class=atom></div></div>
<div id=right-row><div id=right-item></div></div>
<div id=column-row><div id=column-item></div></div>
<div id=self-grid><div id=self-item></div></div></body></html>"#,
            )
            .await;
            inline_page.vm_mut().sync_live_document_style_sources();
            let inline = inline_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("inline/alignment layout should succeed")
                .expect("inline/alignment fixture should have a document element");
            for (color, expected) in [
                (rgb(201, 1, 1), (20.0, 10.0, 20.0, 10.0)),
                (rgb(1, 201, 1), (-20.0, 35.0, 20.0, 10.0)),
                (rgb(1, 1, 201), (180.0, 80.0, 20.0, 10.0)),
                (rgb(201, 201, 1), (0.0, 100.0, 20.0, 20.0)),
                (rgb(201, 1, 201), (180.0, 200.0, 20.0, 10.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&inline, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }

            let mut flow_grid_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
#flow{display:flow-root;width:100px;background:rgb(11,12,13)}#float{float:left;width:40px;height:30px;background:rgb(21,22,23)}#after{width:10px;height:5px;background:rgb(31,32,33)}
#areas{display:grid;width:300px;height:40px;grid-template-areas:'a a b';grid-template-columns:50px 100px 150px;grid-template-rows:40px}
#area-a{grid-area:a;background:rgb(41,42,43)}#area-b{grid-area:b;background:rgb(51,52,53)}
</style></head><body><div id=flow><div id=float></div></div><div id=after></div>
<div id=areas><div id=area-a></div><div id=area-b></div></div></body></html>"#,
            )
            .await;
            flow_grid_page
                .vm_mut()
                .sync_live_document_style_sources();
            let flow_grid = flow_grid_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("flow-root/grid-area layout should succeed")
                .expect("flow-root/grid-area fixture should have a document element");
            for (color, expected) in [
                (rgb(11, 12, 13), (0.0, 0.0, 100.0, 30.0)),
                (rgb(21, 22, 23), (0.0, 0.0, 40.0, 30.0)),
                (rgb(31, 32, 33), (0.0, 30.0, 10.0, 5.0)),
                (rgb(41, 42, 43), (0.0, 35.0, 150.0, 40.0)),
                (rgb(51, 52, 53), (150.0, 35.0, 150.0, 40.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&flow_grid, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }

            let mut replaced_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
#image{display:block;width:120px;height:auto;aspect-ratio:0/1;background:rgb(81,82,83)}
</style></head><body><svg id=image width=80 height=40 viewBox="0 0 80 40"></svg></body></html>"#,
            )
            .await;
            replaced_page
                .vm_mut()
                .sync_live_document_style_sources();
            let replaced = replaced_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("replaced ratio layout should succeed")
                .expect("replaced ratio fixture should have a document element");
            assert_paint_rect(
                solid_paint_rect(&replaced, rgb(81, 82, 83)),
                moli_layout::PaintRect::new(0.0, 0.0, 120.0, 60.0),
            );
        }));
}
#[test]
fn table_ua_defaults_match_chromium_spacing_indent_and_border_color() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let snapshot = render_test_snapshot(
            r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
body{text-indent:30px}
td{width:10px;height:10px;padding:0;border:0;font-size:0}
#first{background:rgb(21,31,41)}#second{background:rgb(22,32,42)}
#indent-probe{display:inline-block;width:4px;height:4px;background:rgb(23,33,43)}
#bordered{border-style:solid;border-width:1px}
</style></head><body>
<table><tr><td id=first><span id=indent-probe></span></td><td id=second></td></tr></table>
<table id=bordered><tr><td></td></tr></table>
</body></html>"#,
        )
        .await;

        let first = solid_paint_rect(&snapshot, rgb(21, 31, 41));
        let second = solid_paint_rect(&snapshot, rgb(22, 32, 42));
        let indent_probe = solid_paint_rect(&snapshot, rgb(23, 33, 43));
        assert_paint_rect(first, moli_layout::PaintRect::new(2.0, 2.0, 10.0, 10.0));
        assert_paint_rect(second, moli_layout::PaintRect::new(14.0, 2.0, 10.0, 10.0));
        assert!(
            (indent_probe.x - first.x).abs() <= 0.01,
            "table should reset inherited text-indent: first={first:?}, probe={indent_probe:?}"
        );

        let gray = rgb(128, 128, 128);
        assert!(
            snapshot.fragments.iter().any(|fragment| matches!(
                fragment,
                moli_layout::PaintFragment::Border { widths, colors, .. }
                    if widths.top == 1.0
                        && widths.right == 1.0
                        && widths.bottom == 1.0
                        && widths.left == 1.0
                        && colors.top == gray
                        && colors.right == gray
                        && colors.bottom == gray
                        && colors.left == gray
            )),
            "table should inherit Chromium's gray UA border color: {:?}",
            snapshot.fragments
        );
    }));
}
#[test]
fn layout_renderer_ignores_separated_table_part_border_padding_and_margin() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{border-spacing:0;background:rgb(81,41,141)}
colgroup{border:20px solid rgb(203,13,23);padding:12px;margin:9px}
col{border:20px solid rgb(204,14,24);padding:12px;margin:9px}
tbody{border:20px solid rgb(201,11,21);padding:12px;margin:9px;background:rgb(82,42,142)}
tr{border:20px solid rgb(202,12,22);padding:12px;margin:9px;background:rgb(83,43,143)}
td{box-sizing:border-box;width:50px;height:20px;padding:0;background:rgb(84,44,144)}
</style></head><body><table><colgroup><col></colgroup><tbody><tr><td></td></tr></tbody></table></body></html>"#,
            )
            .await;

            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(84, 44, 144)),
                moli_layout::PaintRect::new(0.0, 0.0, 50.0, 20.0),
            );
            for ignored in [
                rgb(201, 11, 21),
                rgb(202, 12, 22),
                rgb(203, 13, 23),
                rgb(204, 14, 24),
            ] {
                assert!(
                    snapshot.fragments.iter().all(|fragment| !matches!(
                        fragment,
                        moli_layout::PaintFragment::Border { colors, .. }
                            if colors.top == ignored
                                || colors.right == ignored
                                || colors.bottom == ignored
                                || colors.left == ignored
                    )),
                    "separated table-part border reached paint: {ignored:?}; fragments={:?}",
                    snapshot.fragments
                );
            }
        }));
}
#[test]
fn layout_renderer_computes_phase_four_special_formatting_geometry_from_native_dom_and_stylo() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let rgb = |red: u8, green: u8, blue: u8| {
                moli_layout::PaintColor::new(
                    f32::from(red) / 255.0,
                    f32::from(green) / 255.0,
                    f32::from(blue) / 255.0,
                    1.0,
                )
            };

            let mut table_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { margin: 0; padding: 0 }
#table { border-spacing: 5px 7px; width: 300px; table-layout: fixed; background: rgb(1,2,3) }
#caption { height: 20px; background: rgb(11,12,13) }
#columns { background: rgb(21,22,23) } #column-a { width: 80px; background: rgb(31,32,33) }
#column-bc { background: rgb(41,42,43) } #body { background: rgb(51,52,53) }
#first { height: 30px; background: rgb(61,62,63) }
#second { height: 40px; background: rgb(71,72,73) }
td { padding: 0; border: 0 } #a { background: rgb(81,82,83) }
#b { background: rgb(91,92,93) } #c { background: rgb(101,102,103) }
#d { background: rgb(111,112,113) }
</style></head><body><table id="table"><caption id="caption">cap</caption>
<colgroup id="columns"><col id="column-a"><col id="column-bc" span="2"></colgroup>
<tbody id="body"><tr id="first"><td id="a" rowspan="2">A</td><td id="b" colspan="2">B</td></tr>
<tr id="second"><td id="c">C</td><td id="d">D</td></tr></tbody></table></body></html>"#,
            )
            .await;
            table_page.vm_mut().sync_live_document_style_sources();
            let table = table_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 240, 1.0))
                .expect("table layout should succeed")
                .expect("table fixture should have a document element");
            for (color, expected) in [
                (rgb(1, 2, 3), (0.0, 0.0, 300.0, 111.0)),
                (rgb(11, 12, 13), (0.0, 0.0, 300.0, 20.0)),
                (rgb(21, 22, 23), (5.0, 27.0, 290.0, 77.0)),
                (rgb(31, 32, 33), (5.0, 27.0, 80.0, 77.0)),
                (rgb(41, 42, 43), (90.0, 27.0, 205.0, 77.0)),
                (rgb(51, 52, 53), (5.0, 27.0, 290.0, 77.0)),
                (rgb(61, 62, 63), (5.0, 27.0, 290.0, 30.0)),
                (rgb(71, 72, 73), (5.0, 64.0, 290.0, 40.0)),
                (rgb(81, 82, 83), (5.0, 27.0, 80.0, 77.0)),
                (rgb(91, 92, 93), (90.0, 27.0, 205.0, 30.0)),
                (rgb(101, 102, 103), (90.0, 64.0, 100.0, 40.0)),
                (rgb(111, 112, 113), (195.0, 64.0, 100.0, 40.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&table, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }

            let mut collapsed_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { margin: 0; padding: 0 }
#collapsed { border-collapse: collapse }
#collapsed td { width: 20px; height: 10px; border: 2px solid black }
</style></head><body><table id="collapsed"><tr><td>A</td><td>B</td></tr></table></body></html>"#,
            )
            .await;
            collapsed_page.vm_mut().sync_live_document_style_sources();
            let collapsed = collapsed_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(160, 100, 1.0))
                .expect("collapsed table should remain renderable")
                .expect("collapsed table fixture should have a document element");
            assert!(collapsed
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "collapsed-table-border-fallback"));
            assert!(collapsed.fragments.iter().any(|fragment| matches!(
                fragment,
                moli_layout::PaintFragment::Border { widths, .. }
                    if widths.top == 2.0 || widths.left == 2.0
            )));

            let mut list_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { margin: 0; padding: 0 }
#list { margin: 0; padding-left: 40px; width: 200px; font-size: 20px; line-height: 20px }
li::marker { color: rgb(201,202,203) }
#first { color: rgb(121,122,123); background: rgb(1,2,3) }
#valued { color: rgb(131,132,133); background: rgb(11,12,13) }
#inside { color: rgb(141,142,143); background: rgb(21,22,23); list-style-position: inside }
#custom { color: rgb(151,152,153); background: rgb(31,32,33); list-style: none }
#custom::marker { content: "X "; color: rgb(201,202,203) }
</style></head><body><ol id="list" reversed start="5"><li id="first">AA</li><li id="valued" value="9">BB</li>
<li id="inside">CC</li><li id="custom">DD</li></ol></body></html>"#,
            )
            .await;
            list_page.vm_mut().sync_live_document_style_sources();
            let list = list_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(280, 180, 1.0))
                .expect("list marker layout should succeed")
                .expect("list fixture should have a document element");
            for (color, expected) in [
                (rgb(1, 2, 3), (40.0, 0.0, 200.0, 20.0)),
                (rgb(11, 12, 13), (40.0, 20.0, 200.0, 20.0)),
                (rgb(21, 22, 23), (40.0, 40.0, 200.0, 20.0)),
                (rgb(31, 32, 33), (40.0, 60.0, 200.0, 20.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&list, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }
            let first_text_x = glyph_min_x(&list, rgb(121, 122, 123));
            let valued_text_x = glyph_min_x(&list, rgb(131, 132, 133));
            let inside_text_x = glyph_min_x(&list, rgb(141, 142, 143));
            let custom_text_x = glyph_min_x(&list, rgb(151, 152, 153));
            assert!((first_text_x - 40.0).abs() <= 0.01);
            assert!((valued_text_x - 40.0).abs() <= 0.01);
            assert!(inside_text_x > first_text_x);
            assert!((custom_text_x - 40.0).abs() <= 0.01);
            assert!(glyph_min_x(&list, rgb(201, 202, 203)) < 40.0);
            assert!(
                list.diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.code != "list-marker-layout-deferred")
            );

            let mut flow_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
/* A zero-size font makes the 4px half-leading below the baseline deterministic. */
html, body { margin: 0; padding: 0; font-size: 0; line-height: 8px }
#flow { display: flow-root; width: 200px }
#left { float: left; width: 60px; height: 40px; background: rgb(1,2,3) }
#right { float: right; width: 50px; height: 30px; background: rgb(11,12,13) }
.atom { display: inline-block; height: 20px }
#first-atom { width: 84px; background: rgb(21,22,23) }
#second-atom { width: 60px; background: rgb(31,32,33) }
#clear-root { width: 200px } #clear-float { float: left; width: 70px; height: 35px; background: rgb(41,42,43) }
#clear { clear: both; height: 10px; background: rgb(51,52,53) }
</style></head><body><div id="flow"><div id="left"></div><div id="right"></div><span id="first-atom" class="atom"></span><span id="second-atom" class="atom"></span></div>
<div id="clear-root"><div id="clear-float"></div><div id="clear"></div></div></body></html>"#,
            )
            .await;
            flow_page.vm_mut().sync_live_document_style_sources();
            let flow = flow_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(240, 180, 1.0))
                .expect("float layout should succeed")
                .expect("float fixture should have a document element");
            for (color, expected) in [
                (rgb(1, 2, 3), (0.0, 0.0, 60.0, 40.0)),
                (rgb(11, 12, 13), (150.0, 0.0, 50.0, 30.0)),
                (rgb(21, 22, 23), (60.0, 0.0, 84.0, 20.0)),
                (rgb(31, 32, 33), (60.0, 24.0, 60.0, 20.0)),
                (rgb(41, 42, 43), (0.0, 48.0, 70.0, 35.0)),
                (rgb(51, 52, 53), (0.0, 83.0, 200.0, 10.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&flow, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }

            let mut replaced_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { margin: 0; padding: 0 }
img, canvas, iframe, svg { display: block; margin: 0; border: 0; padding: 0 }
#image { width: 120px; height: auto; background: rgb(1,2,3) }
#canvas { background: rgb(11,12,13) } #frame { background: rgb(21,22,23) }
#svg { background: rgb(31,32,33) }
</style></head><body><img id="image" width="80" height="40" alt=""><canvas id="canvas" width="600"></canvas>
<iframe id="frame" width="90" height="45"></iframe><svg id="svg" width="70" height="35"></svg></body></html>"#,
            )
            .await;
            replaced_page.vm_mut().sync_live_document_style_sources();
            let replaced = replaced_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(700, 700, 1.0))
                .expect("replaced layout should succeed")
                .expect("replaced fixture should have a document element");
            for (color, expected) in [
                (rgb(1, 2, 3), (0.0, 0.0, 120.0, 60.0)),
                (rgb(11, 12, 13), (0.0, 60.0, 600.0, 150.0)),
                (rgb(21, 22, 23), (0.0, 210.0, 90.0, 45.0)),
                (rgb(31, 32, 33), (0.0, 255.0, 70.0, 35.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&replaced, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }
            assert_eq!(
                replaced
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.code == "replaced-content-placeholder")
                    .count(),
                0,
                "the unavailable HTML image now has real fallback content instead of a placeholder"
            );
            assert!(replaced.fragments.iter().any(|fragment| matches!(fragment,
                moli_layout::PaintFragment::SvgImage(image)
                    if image.destination.width == 16.0 && image.destination.height == 16.0)),
                "the fallback retains the attribute ratio and paints its broken-image icon");
            assert_eq!(
                replaced
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.code == "canvas-content-unavailable")
                    .count(),
                1,
                "unavailable canvas pixels must retain a transparent-fallback diagnostic"
            );

            let mut controls_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { margin: 0; padding: 0 }
input, textarea, select, button { display: block; box-sizing: content-box; margin: 0; border: 0; padding: 0; font-size: 20px; line-height: 20px }
#input { background: rgb(1,2,3) } #textarea { background: rgb(11,12,13) }
#select { background: rgb(21,22,23) } #checkbox { background: rgb(31,32,33) }
#radio { background: rgb(41,42,43) } #button { width: 48px; background: rgb(51,52,53) }
</style></head><body><input id="input" size="4" value="AAAA"><textarea id="textarea" cols="4" rows="2">AAAA</textarea>
<select id="select" size="2"><option>A</option><option selected>AAAA</option></select>
<input id="checkbox" type="checkbox" checked><input id="radio" type="radio"><button id="button">AAAA</button></body></html>"#,
            )
            .await;
            controls_page.vm_mut().sync_live_document_style_sources();
            let controls = controls_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(500, 400, 1.0))
                .expect("form layout should succeed")
                .expect("form fixture should have a document element");
            for (color, expected) in [
                (rgb(1, 2, 3), (0.0, 0.0, 48.0, 20.0)),
                (rgb(11, 12, 13), (0.0, 20.0, 63.0, 40.0)),
                (rgb(21, 22, 23), (0.0, 60.0, 52.0, 50.0)),
                (rgb(31, 32, 33), (0.0, 110.0, 13.0, 13.0)),
                (rgb(41, 42, 43), (0.0, 123.0, 13.0, 13.0)),
                (rgb(51, 52, 53), (0.0, 136.0, 48.0, 20.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&controls, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }

            let mut positioned_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { margin: 0; padding: 0 }
#clip { overflow: clip; width: 100px; height: 50px; margin-top: 100px }
#clip-sticky { position: sticky; top: 10px; width: 30px; height: 20px; background: rgb(1,2,3) }
#scroll { overflow: hidden; width: 100px; height: 50px }
#scroll-sticky { position: sticky; top: 10px; width: 30px; height: 20px; background: rgb(11,12,13) }
#transform { transform: translate(0); margin-left: 50px; width: 100px; height: 100px }
#fixed { position: fixed; right: 0; top: 0; width: 20px; height: 20px; background: rgb(21,22,23) }
</style></head><body><div id="clip"><div id="clip-sticky"></div></div><div id="scroll"><div id="scroll-sticky"></div></div>
<div id="transform"><div id="fixed"></div></div></body></html>"#,
            )
            .await;
            positioned_page.vm_mut().sync_live_document_style_sources();
            let positioned = positioned_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 360, 1.0))
                .expect("positioned layout should succeed")
                .expect("positioned fixture should have a document element");
            for (color, expected) in [
                (rgb(1, 2, 3), (0.0, 100.0, 30.0, 20.0)),
                (rgb(11, 12, 13), (0.0, 160.0, 30.0, 20.0)),
                (rgb(21, 22, 23), (130.0, 200.0, 20.0, 20.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&positioned, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }
        }));
}
#[test]
fn parser_created_custom_element_direct_survives_table_foster_parenting() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),

                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
window.tableCeEvents = [];
window.WptTableTiming = class extends HTMLElement {
  constructor() {
    super();
    window.tableCeEvents.push([
      this.hasAttribute('data-token'),
      !!document.getElementById('after-table'),
      this.isConnected
    ].join('|'));
  }
  connectedCallback() {
    window.tableCeEvents.push([
      'connected',
      this.getAttribute('data-token'),
      this.parentElement && this.parentElement.localName,
      this.nextElementSibling && this.nextElementSibling.id,
      !!document.getElementById('after-table'),
      this.isConnected
    ].join('|'));
  }
};
customElements.define('wpt-table-timing', window.WptTableTiming);
</script>
<table id="table"><wpt-table-timing data-token="owned"></wpt-table-timing><tr><td>cell</td></tr></table><span id="after-table"></span>
<script>
const element = document.querySelector('wpt-table-timing');
document.body.setAttribute('data-first-event', window.tableCeEvents[0] || '');
document.body.setAttribute('data-second-event', window.tableCeEvents[1] || '');
document.body.setAttribute('data-token', element.getAttribute('data-token') || '');
document.body.setAttribute('data-parent', element.parentElement && element.parentElement.localName);
document.body.setAttribute('data-next', element.nextElementSibling && element.nextElementSibling.id);
document.body.setAttribute('data-instance', String(element instanceof window.WptTableTiming));
document.body.setAttribute('data-after-visible', String(!!document.getElementById('after-table')));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser table custom element direct regression local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-first-event"),
                Some("false|false|false"),
                "constructor must run before token attributes and later table siblings are visible"
            );
            assert_eq!(
                body_element.attribute("data-second-event"),
                Some("connected|owned|body|table|false|true"),
                "table foster parenting must connect the same parser-created handle before later parser siblings"
            );
            assert_eq!(body_element.attribute("data-token"), Some("owned"));
            assert_eq!(body_element.attribute("data-parent"), Some("body"));
            assert_eq!(body_element.attribute("data-next"), Some("table"));
            assert_eq!(body_element.attribute("data-instance"), Some("true"));
            assert_eq!(body_element.attribute("data-after-visible"), Some("true"));
        }));
}
