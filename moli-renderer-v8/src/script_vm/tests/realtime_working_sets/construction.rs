use super::*;

fn construction_fixture(policy: LayoutPolicy) -> StandaloneScriptVmHarness {
    let markup = format!(
        "<!doctype html><style>body{{margin:0}}.item{{display:block;width:100px;height:20px}}</style><body>{}",
        "<div class=item>Alpha Beta Gamma</div>".repeat(400)
    );
    let mut vm = new_parsed_test_vm("https://construction-revisions.test/", &markup);
    vm.set_layout_policy(policy);
    vm.eval("globalThis.first=document.querySelector('.item');first.offsetWidth")
        .unwrap();
    vm
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
fn retained_construction_skips_clean_storage_chunks_and_observes_cssom_and_pseudos() {
    let mut vm = construction_fixture(LayoutPolicy::Realtime);
    vm.eval("first.style.width='150px';first.offsetWidth")
        .unwrap();
    assert_eq!(vm.eval("first.offsetWidth").unwrap(), "150");
    let warm = main_stats(&vm).unwrap();
    assert!(warm.reused_owned_boxes > 0, "{warm:?}");
    assert!(warm.reused_construction_source_inputs > 400, "{warm:?}");
    compare_evicted(
        &mut vm,
        "JSON.stringify(Array.from(document.querySelectorAll('.item'),n=>[n.offsetWidth,n.offsetHeight]))",
    );

    vm.eval("first.style.removeProperty('width');document.styleSheets[0].cssRules[1].style.width='180px';first.offsetWidth").unwrap();
    assert_eq!(vm.eval("first.offsetWidth").unwrap(), "180");
    assert!(main_stats(&vm).unwrap().reused_construction_source_inputs > 400);
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
fn source_revision_tracking_is_absent_in_legacy_modes_and_shadow_uses_full_validation() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let vm = construction_fixture(policy);
        let host = vm._context_host.borrow();
        assert_eq!(
            host.dom_host()
                .dom()
                .node_mutation_revision(host.document_handle()),
            None
        );
    }
    let mut vm = construction_fixture(LayoutPolicy::Realtime);
    vm.eval("globalThis.h=document.createElement('div');document.body.append(h);h.attachShadow({mode:'open'}).innerHTML='<div style=width:70px>shadow</div>';h.offsetWidth").unwrap();
    vm.eval("first.style.width='160px';first.offsetWidth")
        .unwrap();
    assert_eq!(
        main_stats(&vm).unwrap().reused_construction_source_inputs,
        0
    );
    compare_evicted(
        &mut vm,
        "JSON.stringify([first.offsetWidth,h.offsetWidth,h.shadowRoot.firstChild.offsetWidth])",
    );
}
