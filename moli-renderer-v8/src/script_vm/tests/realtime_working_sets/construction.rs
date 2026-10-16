use super::*;

fn construction_fixture(policy: LayoutPolicy) -> StandaloneScriptVmHarness {
    construction_fixture_with_items(policy, 400)
}

fn construction_fixture_with_items(
    policy: LayoutPolicy,
    items: usize,
) -> StandaloneScriptVmHarness {
    let markup = format!(
        "<!doctype html><style>body{{margin:0}}.item{{display:block;width:100px;height:20px}}</style><body>{}",
        "<div class=item>Alpha Beta Gamma</div>".repeat(items)
    );
    let mut vm = new_parsed_test_vm("https://construction-revisions.test/", &markup);
    vm.set_layout_policy(policy);
    vm.eval("globalThis.first=document.querySelector('.item');first.offsetWidth")
        .unwrap();
    vm
}

#[test]
fn block_tail_construction_preserves_unchanged_text_and_retires_removed_sources() {
    let mut vm = construction_fixture(LayoutPolicy::Realtime);
    let before = main_stats(&vm).unwrap();
    vm.eval("globalThis.added=document.createElement('div');added.className='item';added.textContent='new tail';document.body.append(added);added.offsetWidth").unwrap();
    let appended = main_stats(&vm).unwrap();
    assert_eq!(
        appended.reused_owned_boxes, before.retained_boxes,
        "{appended:?}"
    );
    assert!(appended.reused_inline_contexts > 300, "{appended:?}");
    compare_evicted(
        &mut vm,
        "JSON.stringify(Array.from(document.querySelectorAll('.item'),n=>{const r=n.getBoundingClientRect();return [r.x,r.y,r.width,r.height]}))",
    );
    vm.eval("added.remove();first.offsetWidth").unwrap();
    let removed = main_stats(&vm).unwrap();
    assert_eq!(removed.retained_boxes, before.retained_boxes, "{removed:?}");
    assert_eq!(
        removed.reused_owned_boxes, before.retained_boxes,
        "{removed:?}"
    );
    assert!(removed.reused_inline_contexts > 300, "{removed:?}");
    assert_eq!(vm.eval("added.getClientRects().length").unwrap(), "0");
    compare_evicted(
        &mut vm,
        "JSON.stringify([document.body.scrollHeight,first.offsetWidth,document.querySelectorAll('.item').length])",
    );
}

#[test]
fn construction_admission_covers_small_and_over_budget_documents() {
    for items in [64, 256, 1024, 2048, 4096] {
        let mut vm = construction_fixture_with_items(LayoutPolicy::Realtime, items);
        vm.eval("first.style.width='150px';first.offsetWidth")
            .unwrap();
        let stats = main_stats(&vm);
        eprintln!("construction_scale items={items} stats={stats:?}");
        if items >= 2048 {
            assert!(items * 2 + 2 > moli_layout::LayoutWorkingSetLimits::default().max_boxes);
            assert!(stats.is_none());
            assert_eq!(
                count(&vm),
                0,
                "rejected candidates must not retain a working set"
            );
        } else {
            let stats = stats.unwrap();
            assert_eq!(
                stats.admission,
                moli_layout::LayoutWorkingSetAdmission::Retained
            );
            assert!(stats.reused_owned_boxes > 0, "{stats:?}");
        }
        compare_evicted(
            &mut vm,
            "JSON.stringify([first.offsetWidth,document.body.scrollHeight,document.querySelectorAll('.item').length])",
        );
    }
}

fn compare_evicted(vm: &mut StandaloneScriptVmHarness, script: &str) {
    let warm = vm.eval(script).unwrap();
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
    assert_eq!(vm.eval(script).unwrap(), warm);
}

#[test]
fn retained_construction_observes_cssom_and_pseudos() {
    let mut vm = construction_fixture(LayoutPolicy::Realtime);
    vm.eval("first.style.width='150px';first.offsetWidth")
        .unwrap();
    assert_eq!(vm.eval("first.offsetWidth").unwrap(), "150");
    let warm = main_stats(&vm).unwrap();
    assert!(warm.reused_owned_boxes > 0, "{warm:?}");
    compare_evicted(
        &mut vm,
        "JSON.stringify(Array.from(document.querySelectorAll('.item'),n=>[n.offsetWidth,n.offsetHeight]))",
    );

    vm.eval("first.style.removeProperty('width');document.styleSheets[0].cssRules[1].style.width='180px';first.offsetWidth").unwrap();
    assert_eq!(vm.eval("first.offsetWidth").unwrap(), "180");
    compare_evicted(
        &mut vm,
        "JSON.stringify([first.offsetWidth,first.offsetHeight])",
    );

    vm.eval("document.styleSheets[0].insertRule('.item::before {content:\"prefix\"}',2);first.offsetWidth").unwrap();
    assert_eq!(main_stats(&vm).unwrap().reused_owned_boxes, 0);
    compare_evicted(
        &mut vm,
        "JSON.stringify([first.offsetWidth,first.scrollWidth,first.offsetHeight])",
    );
}

#[test]
fn retained_construction_observes_shadow_tree_geometry() {
    let mut vm = construction_fixture(LayoutPolicy::Realtime);
    vm.eval("globalThis.h=document.createElement('div');document.body.append(h);h.attachShadow({mode:'open'}).innerHTML='<div style=width:70px>shadow</div>';h.offsetWidth").unwrap();
    vm.eval("first.style.width='160px';first.offsetWidth")
        .unwrap();
    compare_evicted(
        &mut vm,
        "JSON.stringify([first.offsetWidth,h.offsetWidth,h.shadowRoot.firstChild.offsetWidth])",
    );
}
