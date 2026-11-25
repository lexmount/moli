use super::*;

fn table_fixture(items: usize) -> StandaloneScriptVmHarness {
    let row = format!(
        "<tr>{}</tr>",
        "<td><div class=item>Alpha Beta Gamma</div></td>".repeat(4),
    );
    let markup = format!(
        "<!doctype html><meta charset=utf-8><style>body{{margin:0}}\
         #sheet{{width:1280px;border-spacing:2px;font:16px/20px MoliAhem}}\
         td{{padding:0}}.item{{display:block;width:100px;height:20px}}\
         </style><table id=sheet>{}</table>",
        row.repeat(items / 4),
    );
    let mut vm = new_parsed_test_vm("https://realtime-table-measure.test/", &markup);
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
    vm
}

const READ: &str = "JSON.stringify({boxes:Array.from(sheet.querySelectorAll('td,.item'),n=>{const r=n.getBoundingClientRect();return [r.x,r.y,r.width,r.height,n.scrollWidth,n.scrollHeight]}),table:[sheet.offsetWidth,sheet.offsetHeight],scroll:[document.documentElement.scrollWidth,document.documentElement.scrollHeight]})";

fn force_cold(vm: &mut StandaloneScriptVmHarness) {
    vm._context_host
        .borrow()
        .clear_realtime_layout_working_sets_for_test();
    let screen = viewport(vm);
    vm.with_fresh_layout_pass(
        LayoutPassRequest::new(screen, LayoutFlushReason::Test),
        LayoutPublicationRequest::Screen,
        |_| Ok(()),
    )
    .unwrap()
    .unwrap();
}

#[test]
fn realtime_table_reuses_actual_box_storage() {
    const ITEMS: usize = 256;
    let mut vm = table_fixture(ITEMS);
    // Exercise owned construction separately from the larger text fixture.
    let initial = vm.eval(READ).unwrap();
    let cold = main_stats(&vm).unwrap();
    assert!(cold.retained_boxes >= 2 * ITEMS, "{cold:?}");
    vm.eval("sheet.querySelector('.item').style.width='333px'")
        .unwrap();
    let updated = vm.eval(READ).unwrap();
    assert_ne!(initial, updated);
    let warm = main_stats(&vm).unwrap();
    assert_eq!(warm.reused_owned_boxes, cold.retained_boxes, "{warm:?}");
    force_cold(&mut vm);
    assert_eq!(vm.eval(READ).unwrap(), updated);
}

#[test]
fn realtime_table_constraint_and_role_changes_match_evicted_layout() {
    let mut vm = table_fixture(16);
    vm.eval(READ).unwrap();
    for change in [
        "sheet.style.width='760px'",
        "sheet.style.height='180px';sheet.querySelector('.item').style.height='50%'",
        "sheet.querySelector('td').style.verticalAlign='bottom'",
        "sheet.style.tableLayout='fixed';sheet.querySelector('td').colSpan=2",
        "sheet.querySelector('td').rowSpan=2;sheet.style.borderCollapse='collapse'",
        "sheet.style.writingMode='vertical-rl'",
        "sheet.querySelector('.item').firstChild.data='changed text content'",
        "sheet.style.writingMode='horizontal-tb';sheet.querySelector('td').style.display='block'",
    ] {
        vm.eval(change).unwrap();
        let warm = vm.eval(READ).unwrap();
        force_cold(&mut vm);
        assert_eq!(vm.eval(READ).unwrap(), warm, "{change}");
    }
}
