use std::cell::Cell;

use super::*;
use crate::{
    dom::native::DomHost,
    parser::HtmlParser,
    script_vm::{ScriptVmDefaultWorldBootstrap, StandaloneScriptVmHarness},
};

thread_local! {
    static FLAT_TREE_STEPS: Cell<usize> = const { Cell::new(0) };
    static SHADOW_TREE_STEPS: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn note_shadow_tree_step() {
    SHADOW_TREE_STEPS.with(|count| count.set(count.get() + 1));
}

pub(super) fn note_flat_tree_step() {
    FLAT_TREE_STEPS.with(|count| count.set(count.get() + 1));
}

fn take_flat_tree_steps() -> usize {
    FLAT_TREE_STEPS.replace(0)
}

fn fixture(body: &str) -> StandaloneScriptVmHarness {
    let _runtime = crate::JsRuntime::initialize();
    let document = HtmlParser::SCRIPTING_ENABLED.parse(
        url::Url::parse("https://markdown-depth.test/").unwrap(),
        format!("<!doctype html><html><body>{body}</body></html>"),
    );
    let queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        queue.owner_attached_runtime_page_task_sender_for_test(),
        queue.parser_boundary_sender(),
    )
    .unwrap()
    .finish()
    .unwrap();
    vm.install_page_task_residence_for_executor_test(queue.residence());
    vm
}

fn divs_in_subtree(host: &JsContextHost, root: DomHandle) -> Vec<DomHandle> {
    let mut nodes = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if host.dom_host().is_html_element_named(node, "div") {
            nodes.push(node);
        }
        stack.extend(host.dom_host().child_handles(node));
    }
    nodes
}

#[test]
fn document_export_checks_flat_tree_membership_linearly_in_either_node_order() {
    let depth = 256;
    let vm = fixture(&format!(
        "{}HELLO{}",
        "<div>".repeat(depth),
        "</div>".repeat(depth),
    ));
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let host = host.borrow();
    let mut nodes = divs_in_subtree(&host, host.document_handle());
    assert_eq!(nodes.len(), depth);
    let properties = ["display", "visibility"].map(str::to_owned);
    for _ in 0..2 {
        take_flat_tree_steps();
        let values = computed_style_export_values_for_document_snapshot(
            &host,
            nodes.iter().copied(),
            &properties,
        );
        let steps = take_flat_tree_steps();
        assert_eq!(values.len(), depth);
        for style in values {
            assert_eq!(style.values, ["block", "visible"]);
        }
        assert!(
            steps <= 4 * (depth + 4),
            "one export must share ancestor checks: {steps} steps for {depth} nodes",
        );
        nodes.reverse();
    }
}

#[test]
fn document_export_reuses_one_style_resolution_context_for_its_targets() {
    use crate::style_engine::take_style_resolution_context_count_for_test;

    let depth = 128;
    let body = format!("{}HELLO{}", "<div>".repeat(depth), "</div>".repeat(depth));
    let properties = ["display", "visibility"].map(str::to_owned);
    let unbatched = {
        let vm = fixture(&body);
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let nodes = divs_in_subtree(&host, host.document_handle());
        let mut observation = StyleObservation::new(&host);
        take_style_resolution_context_count_for_test();
        let values = nodes
            .into_iter()
            .map(|node| {
                let style = observation.read(node);
                properties
                    .iter()
                    .map(|property| style.property(property))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert!(
            take_style_resolution_context_count_for_test() >= depth,
            "the unbatched control must exercise repeated single-node resolution",
        );
        values
    };
    for reverse in [false, true] {
        let vm = fixture(&body);
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let mut nodes = divs_in_subtree(&host, host.document_handle());
        if reverse {
            nodes.reverse();
        }
        take_style_resolution_context_count_for_test();
        let values = computed_style_export_values_for_document_snapshot(&host, nodes, &properties);
        let contexts = take_style_resolution_context_count_for_test();
        assert!(
            contexts <= 3,
            "a cold export must share its resolver: {contexts} contexts for {depth} nodes",
        );
        assert_eq!(
            values
                .into_iter()
                .map(|style| style.values)
                .collect::<Vec<_>>(),
            unbatched
        );
    }
}

#[test]
fn document_export_shares_shadow_scope_queries_across_nodes_and_properties() {
    let depth = 256;
    let mut vm = fixture("<main id=host></main>");
    let contents = format!("{}HELLO{}", "<div>".repeat(depth), "</div>".repeat(depth));
    vm.eval(&format!(
        "document.getElementById('host').attachShadow({{mode:'closed'}}).innerHTML = {}",
        serde_json::to_string(&contents).unwrap(),
    ))
    .unwrap();
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let host = host.borrow();
    let shadow_host = host.dom_host().element_handle_by_id("host").unwrap();
    let root = host.dom_host().shadow_root_handle(shadow_host).unwrap();
    let mut nodes = divs_in_subtree(&host, root);
    assert_eq!(nodes.len(), depth);
    let properties = [
        "display",
        "visibility",
        "position",
        "float",
        "flex-direction",
        "baseline-shift",
    ]
    .map(str::to_owned);
    for _ in 0..2 {
        take_flat_tree_steps();
        SHADOW_TREE_STEPS.replace(0);
        let values = computed_style_export_values_for_document_snapshot(
            &host,
            nodes.iter().copied(),
            &properties,
        );
        let flat_steps = take_flat_tree_steps();
        let shadow_steps = SHADOW_TREE_STEPS.replace(0);
        assert_eq!(values.len(), depth);
        for style in values {
            assert_eq!(&style.values[..2], ["block", "visible"]);
        }
        assert!(
            flat_steps <= 4 * (depth + 5),
            "flat-tree membership repeated ancestors: {flat_steps} steps",
        );
        assert!(
            (depth..=4 * (depth + 1)).contains(&shadow_steps),
            "properties must share their observation's scope queries: {shadow_steps} steps",
        );
        nodes.reverse();
    }
}

#[test]
fn document_export_membership_cache_does_not_survive_tree_or_slot_changes() {
    let mut vm = fixture(
        "<main id=host><article id=assigned slot=content><section id=child>Assigned</section></article><article id=unassigned>Unassigned</article></main>",
    );
    vm.eval(
        "globalThis.exportHost = document.getElementById('host'); globalThis.exportRoot = exportHost.attachShadow({mode:'closed'}); exportRoot.innerHTML = '<slot name=content></slot>'",
    )
    .unwrap();
    let handles = {
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        ["assigned", "child", "unassigned"]
            .map(|id| host.dom_host().element_handle_by_id(id).unwrap())
    };
    let properties = ["display", "visibility"].map(str::to_owned);
    for (script, expected) in [
        ("void 0", [true, true, false]),
        (
            "exportRoot.querySelector('slot').name = 'other'",
            [false, false, false],
        ),
        (
            "document.getElementById('unassigned').slot = 'other'",
            [false, false, true],
        ),
        ("exportHost.remove()", [false, false, false]),
        (
            "document.body.appendChild(exportHost)",
            [false, false, true],
        ),
        (
            "document.getElementById('assigned').slot = 'other'",
            [true, true, true],
        ),
    ] {
        vm.eval(script).unwrap();
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        for nodes in [handles, [handles[2], handles[1], handles[0]]] {
            let values =
                computed_style_export_values_for_document_snapshot(&host, nodes, &properties);
            for (handle, value) in nodes.into_iter().zip(values) {
                let index = handles.iter().position(|&node| node == handle).unwrap();
                assert_eq!(
                    !value.values.is_empty(),
                    expected[index],
                    "node {handle:?} after {script}",
                );
                if expected[index] {
                    assert_eq!(value.values, ["block", "visible"]);
                }
            }
        }
    }
}
