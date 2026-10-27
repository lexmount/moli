use super::*;

#[test]
fn realtime_grid_retains_the_1024_item_graph_and_refreshes_layout_inputs() {
    const ITEMS: usize = 1024;
    let markup = format!(
        "<!doctype html><meta charset=utf-8><style>body{{margin:0}}\
         #grid{{display:grid;width:1280px;grid-template-columns:repeat(4,1fr);gap:4px}}\
         .item{{display:block;width:100px;height:20px;font:16px MoliAhem}}\
         </style><div id=grid>{}</div>",
        "<div class=item>Alpha Beta Gamma</div>".repeat(ITEMS),
    );
    let mut vm = new_parsed_test_vm("https://realtime-grid-owned.test/", &markup);
    vm.set_layout_policy(LayoutPolicy::Realtime);
    vm.set_viewport_surface_for_bootstrap(Some(crate::protocol_types::ViewportSurface {
        inner_width: 1280,
        inner_height: 720,
        device_pixel_ratio: 1.0,
        ..Default::default()
    }));
    vm._context_host.borrow().register_layout_test_font(
        "MoliAhem",
        include_bytes!("../../../../../moli-layout/tests/fixtures/moli-ahem.ttf").to_vec(),
    );
    let read = "JSON.stringify({items:Array.from(document.querySelectorAll('.item'),n=>{const r=n.getBoundingClientRect();return [r.x,r.y,r.width,r.height,n.scrollWidth,n.scrollHeight]}),tracks:getComputedStyle(grid).gridTemplateColumns,scroll:[document.documentElement.scrollWidth,document.documentElement.scrollHeight]})";
    let initial = vm.eval(read).unwrap();
    let cold = main_stats(&vm).unwrap();
    assert!(cold.retained_boxes >= 2 * ITEMS, "{cold:?}");
    assert_eq!(cold.retained_owned_boxes, cold.retained_boxes, "{cold:?}");
    assert!(cold.estimated_retained_bytes <= moli_layout::DEFAULT_DOCUMENT_WORKING_SET_BYTES);
    let before = vm.layout_pass_observability_for_test().1;
    vm.eval("grid.firstElementChild.style.width='333px'")
        .unwrap();
    let local_geometry = vm.eval(read).unwrap();
    assert_ne!(local_geometry, initial);
    let local = main_stats(&vm).unwrap();
    assert_eq!(local.reused_owned_boxes, local.retained_boxes, "{local:?}");
    assert!(local.reused_inline_contexts >= ITEMS - 1, "{local:?}");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);

    // Parent track changes can invalidate every inline input conservatively.
    // They must still preserve the graph and match a fresh numeric layout.
    let before = vm.layout_pass_observability_for_test().1;
    vm.eval("grid.style.gridTemplateColumns='[start] 160px [middle] 240px [last] 1fr [end]';const n=grid.firstElementChild;n.style.order='2';n.style.gridColumn='middle'").unwrap();
    let warm_geometry = vm.eval(read).unwrap();
    assert_ne!(warm_geometry, local_geometry);
    let warm = main_stats(&vm).unwrap();
    assert_eq!(warm.reused_owned_boxes, warm.retained_boxes, "{warm:?}");
    assert_eq!(
        warm.reused_construction_boxes, warm.retained_boxes,
        "{warm:?}"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(vm.eval(read).unwrap(), warm_geometry);
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);

    vm._context_host
        .borrow()
        .clear_realtime_layout_working_sets_for_test();
    let screen = viewport(&vm);
    vm.with_fresh_layout_pass(
        LayoutPassRequest::new(screen, LayoutFlushReason::Test),
        LayoutPublicationRequest::Screen,
        |_| Ok(()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(vm.eval(read).unwrap(), warm_geometry);
    assert_eq!(main_stats(&vm).unwrap().reused_construction_boxes, 0);
}

#[test]
fn realtime_grid_display_and_anonymous_changes_rebuild_current_geometry() {
    let mut vm = fixture();
    vm.eval("document.body.style.display='grid';document.body.style.gridTemplateColumns='120px 240px';changing.offsetWidth").unwrap();
    assert!(main_stats(&vm).unwrap().retained_owned_boxes > 0);
    vm.eval("document.body.prepend(document.createTextNode('anonymous grid item'));changing.offsetWidth").unwrap();
    assert_eq!(main_stats(&vm).unwrap().reused_construction_boxes, 0);
    assert_eq!(main_stats(&vm).unwrap().retained_owned_boxes, 0);
    vm.eval("document.body.firstChild.remove();changing.offsetWidth")
        .unwrap();
    assert!(main_stats(&vm).unwrap().retained_owned_boxes > 0);
    vm.eval("document.body.style.gridTemplateColumns='180px 1fr';changing.offsetWidth")
        .unwrap();
    assert!(main_stats(&vm).unwrap().reused_owned_boxes > 0);
    vm.eval("document.body.style.display='block';changing.offsetWidth")
        .unwrap();
    assert_eq!(main_stats(&vm).unwrap().reused_construction_boxes, 0);
    assert_eq!(count(&vm), 1);
}
