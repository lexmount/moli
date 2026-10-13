use super::*;
use crate::native_bridge::LayoutPublicationRequest;
use moli_layout::{
    LayoutError, LayoutFlushReason, LayoutPassRequest, LayoutViewport, LayoutWorkingSetStats,
};
use moli_page_types::LayoutPolicy;

fn fixture() -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm(
        "https://realtime-working-sets.test/",
        "<!doctype html><body style='margin:0;font:16px/20px MoliAhem'>\
         <div id=changing style='width:100px;height:20px'>AAAA</div>\
         <div id=stable style='width:200px'>AAAA BBBB CCCC DDDD</div>",
    );
    vm.set_layout_policy(LayoutPolicy::Realtime);
    vm._context_host.borrow().register_layout_test_font(
        "MoliAhem",
        include_bytes!("../../../../moli-layout/tests/fixtures/moli-ahem.ttf").to_vec(),
    );
    vm
}

fn main_stats(vm: &StandaloneScriptVmHarness) -> Option<LayoutWorkingSetStats> {
    let host = vm._context_host.borrow();
    host.realtime_layout_working_set_stats_for_test(host.document_handle())
}

fn count(vm: &StandaloneScriptVmHarness) -> usize {
    vm._context_host
        .borrow()
        .realtime_layout_working_set_count_for_test()
}

fn viewport(vm: &StandaloneScriptVmHarness) -> LayoutViewport {
    let host = vm._context_host.borrow();
    host.layout_viewport_for_document(host.document_handle())
}

fn install_child(vm: &mut StandaloneScriptVmHarness) -> DomHandle {
    vm.eval(
        "globalThis.frame=document.createElement('iframe');frame.id='frame';frame.style.cssText='width:200px;height:100px';\
         document.body.append(frame);globalThis.child=frame.contentWindow;child.document.open();\
         child.document.write('<!doctype html><body><div id=target style=width:80px>child text</div>');\
         child.document.close();frame.offsetWidth",
    ).unwrap();
    let frame = vm.document_runtime.get_element_by_id("frame").unwrap();
    let host = vm._context_host.borrow();
    host.child_browsing_context_document_handle(frame).unwrap()
}

#[test]
fn realtime_working_sets_reuse_unchanged_text_after_mutation_and_allow_eviction() {
    let mut vm = fixture();
    let read = "JSON.stringify([changing.offsetWidth,stable.offsetHeight])";
    let initial = vm.eval(read).unwrap();
    let cold = main_stats(&vm).expect("first canonical pass retains computation state");
    assert!(cold.retained_boxes > 0);
    assert_eq!(cold.reused_box_slots, 0);
    assert_eq!(cold.reused_inline_contexts, 0);

    let builds = vm.layout_pass_observability_for_test().1;
    vm.eval("changing.style.width='150px'").unwrap();
    let updated = vm.eval(read).unwrap();
    assert_ne!(updated, initial);
    let warm = main_stats(&vm).unwrap();
    assert!(warm.reused_box_slots > 0, "{warm:?}");
    assert!(warm.reused_inline_contexts > 0, "{warm:?}");
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    assert_eq!(vm.eval(read).unwrap(), updated);
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);

    vm._context_host
        .borrow()
        .clear_realtime_layout_working_sets_for_test();
    assert_eq!(count(&vm), 0);
    // Evicting work does not invalidate published geometry or wake a frame.
    assert_eq!(vm.eval(read).unwrap(), updated);
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    let screen = viewport(&vm);
    vm.with_fresh_layout_pass(
        LayoutPassRequest::new(screen, LayoutFlushReason::Test),
        LayoutPublicationRequest::Screen,
        |_| Ok(()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(vm.eval(read).unwrap(), updated);
    assert_eq!(main_stats(&vm).unwrap().reused_box_slots, 0);
}

#[test]
fn realtime_working_sets_preserve_screen_state_across_transient_views() {
    let mut vm = fixture();
    vm.eval("changing.offsetWidth").unwrap();
    vm.eval("changing.style.width='150px';changing.offsetWidth")
        .unwrap();
    let before = main_stats(&vm).unwrap();
    let screen = viewport(&vm);
    vm.paint_layout_snapshot(
        screen,
        LayoutFlushReason::Screenshot,
        LayoutPublicationRequest::Transient,
    )
    .unwrap()
    .unwrap();
    assert_eq!(main_stats(&vm), Some(before));
    vm.screenshot_layout_snapshot(LayoutViewport::new(100, 120, 1.0))
        .unwrap()
        .unwrap();
    assert_eq!(main_stats(&vm), Some(before));

    let environment = {
        let host = vm._context_host.borrow();
        let mut media = host.emulated_media().clone();
        media.media = Some("print".to_owned());
        crate::style_engine::StyloStyleEnvironment::from_emulated_media(&media)
    };
    vm.with_fresh_layout_pass_in_environment(
        LayoutPassRequest::new(screen, LayoutFlushReason::Print),
        LayoutPublicationRequest::Transient,
        Some(environment),
        |_| Ok(()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(main_stats(&vm), Some(before));
    assert_eq!(vm.eval("String(changing.offsetWidth)").unwrap(), "150");
}

#[test]
fn realtime_working_sets_drop_failed_screen_transactions() {
    for change_inputs in [false, true] {
        let mut vm = fixture();
        vm.eval("changing.offsetWidth").unwrap();
        assert_eq!(count(&vm), 1);
        let host = vm._context_host.clone();
        let screen = viewport(&vm);
        let result = vm.with_fresh_layout_pass(
            LayoutPassRequest::new(screen, LayoutFlushReason::Test),
            LayoutPublicationRequest::Screen,
            |_| {
                if change_inputs {
                    host.borrow().note_realtime_layout_input_change();
                    Ok(())
                } else {
                    Err(LayoutError::source_contract(
                        "test output",
                        "consumer failed",
                    ))
                }
            },
        );
        assert!(matches!(result, Err(LayoutError::SourceContract { .. })));
        assert_eq!(count(&vm), 0);
        assert_eq!(vm.eval("String(changing.offsetWidth)").unwrap(), "100");
    }
}

#[test]
fn realtime_working_sets_retire_child_owners_and_only_keep_visited_documents() {
    let mut vm = fixture();
    let child = install_child(&mut vm);
    assert_eq!(count(&vm), 2);
    assert!(
        vm._context_host
            .borrow()
            .realtime_layout_working_set_stats_for_test(child)
            .is_some()
    );

    vm.eval("frame.style.display='none';changing.offsetWidth")
        .unwrap();
    assert_eq!(
        count(&vm),
        1,
        "hidden child did not participate in this pass"
    );
    vm.eval("frame.style.display='block';frame.offsetWidth")
        .unwrap();
    assert_eq!(count(&vm), 2);
    assert_eq!(
        vm._context_host
            .borrow()
            .realtime_layout_working_set_stats_for_test(child)
            .unwrap()
            .reused_box_slots,
        0
    );

    vm.eval("child.document.open()").unwrap();
    assert_eq!(count(&vm), 1, "document.open retires work synchronously");
    vm.eval("child.document.write('<!doctype html><div id=next style=width:90px>new child</div>');child.document.close();frame.offsetWidth").unwrap();
    assert_eq!(count(&vm), 2);
    assert_eq!(
        vm._context_host
            .borrow()
            .realtime_layout_working_set_stats_for_test(child)
            .unwrap()
            .reused_box_slots,
        0
    );

    vm.eval("child.document.documentElement.remove();changing.offsetWidth")
        .unwrap();
    assert_eq!(
        count(&vm),
        1,
        "rootless child must not keep its previous tree"
    );
    vm.eval("child.document.open();child.document.write('<!doctype html><div>again</div>');child.document.close();frame.offsetWidth").unwrap();
    assert_eq!(count(&vm), 2);
    vm.eval("frame.remove()").unwrap();
    assert_eq!(count(&vm), 1, "detaching a child retires its exact owner");
}

#[test]
fn realtime_working_sets_clear_on_main_replacement_missing_root_and_reset() {
    let mut vm = fixture();
    install_child(&mut vm);
    assert_eq!(count(&vm), 2);
    vm.eval("document.open()").unwrap();
    assert_eq!(count(&vm), 0);
    vm.eval("document.write('<!doctype html><div id=next style=width:123px>next</div>');document.close();next.offsetWidth").unwrap();
    assert_eq!(count(&vm), 1);
    assert_eq!(main_stats(&vm).unwrap().reused_box_slots, 0);
    vm.eval("document.documentElement.remove()").unwrap();
    let screen = viewport(&vm);
    let result = vm
        .with_fresh_layout_pass(
            LayoutPassRequest::new(screen, LayoutFlushReason::Test),
            LayoutPublicationRequest::Screen,
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(result, None);
    assert_eq!(count(&vm), 0);

    vm.eval("document.open();document.write('<!doctype html><div id=last style=width:70px>last</div>');document.close();last.offsetWidth").unwrap();
    assert_eq!(count(&vm), 1);
    vm._context_host.borrow().reset_document_layout_state();
    assert_eq!(count(&vm), 0);
    assert_eq!(vm.eval("String(last.offsetWidth)").unwrap(), "70");
    assert_eq!(main_stats(&vm).unwrap().reused_box_slots, 0);
}

#[test]
fn realtime_working_sets_lend_distinct_caches_through_nested_frames() {
    let mut vm = fixture();
    let child = install_child(&mut vm);
    vm.eval(
        "globalThis.inner=child.document.createElement('iframe');inner.style.cssText='width:100%;height:50px';\
         child.document.body.style.margin='0';child.document.body.append(inner);\
         globalThis.nested=inner.contentWindow;nested.document.open();\
         nested.document.write('<!doctype html><body style=margin:0><div id=deep style=width:100%>nested</div>');\
         nested.document.close();globalThis.deep=nested.document.getElementById('deep');deep.offsetWidth",
    ).unwrap();
    assert_eq!(count(&vm), 3);
    let read = "JSON.stringify([frame.offsetWidth,inner.offsetWidth,deep.offsetWidth])";
    let initial = vm.eval(read).unwrap();
    vm.eval("frame.style.width='300px'").unwrap();
    let updated = vm.eval(read).unwrap();
    assert_ne!(updated, initial);
    assert_eq!(count(&vm), 3);
    assert!(
        vm._context_host
            .borrow()
            .realtime_layout_working_set_stats_for_test(child)
            .unwrap()
            .reused_box_slots
            > 0
    );
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval(read).unwrap(), updated);
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    vm.eval("frame.style.display='none';changing.offsetWidth")
        .unwrap();
    assert_eq!(count(&vm), 1, "prune unvisited descendants at every depth");
}

#[test]
fn realtime_working_sets_apply_one_page_budget_without_losing_child_geometry() {
    fn publish_with_limit(vm: &StandaloneScriptVmHarness, limit: usize) {
        let host = vm._context_host.borrow();
        let document = host.document_handle();
        let screen = host.layout_viewport_for_document(document);
        let mut publication = host
            .prepare_layout_publication(document, screen, LayoutPublicationRequest::Screen)
            .unwrap();
        publication
            .working_sets_for_pass()
            .unwrap()
            .set_retention_limit_for_test(limit);
        let pass = host
            .build_layout_pass_for_document_in_environment(
                document,
                LayoutPassRequest::new(screen, LayoutFlushReason::Test),
                None,
                publication.working_sets_for_pass(),
            )
            .unwrap()
            .unwrap();
        host.finish_layout_publication(publication, pass).unwrap();
    }

    let mut vm = fixture();
    let child = install_child(&mut vm);
    let read = "JSON.stringify([changing.offsetWidth,stable.offsetHeight,frame.offsetWidth,child.document.getElementById('target').offsetWidth])";
    let expected = vm.eval(read).unwrap();
    let root_charge = main_stats(&vm).unwrap().estimated_retained_bytes;
    let child_charge = vm
        ._context_host
        .borrow()
        .realtime_layout_working_set_stats_for_test(child)
        .unwrap()
        .estimated_retained_bytes;
    assert!(root_charge > 0 && child_charge > 0);
    let limit = root_charge.max(child_charge);
    assert!(limit < root_charge + child_charge);

    // Each real document individually fits this limit. Their aggregate does
    // not: admission must refuse a working tree without dropping its frozen
    // geometry from the successfully published frame tree.
    publish_with_limit(&vm, limit);
    let charge = {
        let host = vm._context_host.borrow();
        [host.document_handle(), child]
            .into_iter()
            .filter_map(|document| host.realtime_layout_working_set_stats_for_test(document))
            .map(|stats| stats.estimated_retained_bytes)
            .sum::<usize>()
    };
    assert_eq!(count(&vm), 1);
    assert!(
        charge > 0 && charge <= limit,
        "retained {charge}, limit {limit}"
    );
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval(read).unwrap(), expected);
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);

    // Zero admission evicts even the previously reusable entry, but does not
    // change observable results or require a follow-up geometry build.
    publish_with_limit(&vm, 0);
    assert_eq!(count(&vm), 0);
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval(read).unwrap(), expected);
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);

    let screen = viewport(&vm);
    vm.with_fresh_layout_pass(
        LayoutPassRequest::new(screen, LayoutFlushReason::Test),
        LayoutPublicationRequest::Screen,
        |_| Ok(()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(count(&vm), 2);
    assert_eq!(main_stats(&vm).unwrap().reused_box_slots, 0);
    assert_eq!(vm.eval(read).unwrap(), expected);
}

#[test]
fn realtime_working_sets_release_all_documents_while_closed_host_is_retained() {
    let mut vm = fixture();
    install_child(&mut vm);
    vm.eval("changing.style.width='150px';changing.offsetWidth")
        .unwrap();
    assert_eq!(count(&vm), 2);
    assert!(main_stats(&vm).unwrap().reused_box_slots > 0);
    let host = vm._context_host.clone();
    let builds = vm.layout_pass_observability_for_test().1;

    vm.close_page_context_resources_for_context_teardown();
    assert_eq!(
        host.borrow().realtime_layout_working_set_count_for_test(),
        0
    );
    assert_eq!(host.borrow().layout_pass_observability_for_test().1, builds);
    vm.close_page_context_resources_for_context_teardown();
    assert_eq!(
        host.borrow().realtime_layout_working_set_count_for_test(),
        0
    );
}

#[test]
fn legacy_modes_never_install_layout_working_sets() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let mut vm = new_parsed_test_vm(
            "https://legacy-working-sets.test/",
            "<!doctype html><div id=target style=width:100px>text</div>",
        );
        vm.set_layout_policy(policy);
        vm.eval("target.offsetWidth;target.style.width='150px';target.offsetWidth")
            .unwrap();
        if policy == LayoutPolicy::OnDemand {
            let screen = viewport(&vm);
            vm.screenshot_layout_snapshot(screen).unwrap().unwrap();
        }
        assert_eq!(count(&vm), 0);
        assert!(main_stats(&vm).is_none());
        assert!(
            !vm._context_host
                .borrow()
                .realtime_layout_allocated_for_test()
        );
    }
}
