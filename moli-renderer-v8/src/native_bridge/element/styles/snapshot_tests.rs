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

fn divs_in_subtree(host: &PageScriptHost, root: DomHandle) -> Vec<DomHandle> {
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

fn shadow_inheritance_fixture(mode: &str) -> (StandaloneScriptVmHarness, DomHandle) {
    let mut vm = fixture(
        "<main id=host style='font-weight:700;letter-spacing:3px;font-style:italic;text-transform:uppercase'></main>",
    );
    vm.eval(&format!(
        "const root = document.getElementById('host').attachShadow({{mode:'{mode}'}}); root.innerHTML = '<span>Hello</span>'; globalThis.styleTarget = root.firstElementChild",
    ))
    .unwrap();
    let handle = {
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let shadow_host = host.dom_host().element_handle_by_id("host").unwrap();
        let root = host.dom_host().shadow_root_handle(shadow_host).unwrap();
        host.dom_host().child_handles(root).next().unwrap()
    };
    (vm, handle)
}

#[test]
fn cold_document_export_inherits_from_shadow_host_like_single_node_resolution() {
    let properties = [
        "font-weight",
        "letter-spacing",
        "font-style",
        "text-transform",
    ]
    .map(str::to_owned);
    for mode in ["open", "closed"] {
        let expected = {
            let (vm, node) = shadow_inheritance_fixture(mode);
            let host = vm.context_host_weak_for_test().upgrade().unwrap();
            let host = host.borrow();
            let mut observation = StyleObservation::new(&host);
            let style = observation.read(node);
            properties
                .iter()
                .map(|property| style.property(property))
                .collect::<Vec<_>>()
        };
        assert_eq!(expected, ["700", "3px", "italic", "uppercase"]);

        // A separate DOM must reach the batch resolver with no styles warmed
        // by getComputedStyle or the single-node control above.
        let (mut vm, node) = shadow_inheritance_fixture(mode);
        {
            let host = vm.context_host_weak_for_test().upgrade().unwrap();
            let host = host.borrow();
            let values =
                computed_style_export_values_for_document_snapshot(&host, [node], &properties);
            assert_eq!(values[0].values, expected, "cold {mode} shadow tree");
        }
        assert_eq!(
            vm.eval("JSON.stringify(['font-weight','letter-spacing','font-style','text-transform'].map(name => getComputedStyle(styleTarget).getPropertyValue(name)))")
                .unwrap(),
            serde_json::to_string(&expected).unwrap(),
            "the batch result must also leave the canonical cache correct",
        );
    }
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

fn export_values(
    vm: &StandaloneScriptVmHarness,
    nodes: impl IntoIterator<Item = DomHandle>,
    properties: &[String],
) -> Vec<Vec<String>> {
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let host = host.borrow();
    computed_style_export_values_for_document_snapshot(&host, nodes, properties)
        .into_iter()
        .map(|style| style.values)
        .collect()
}

// Observable values below were calibrated against Chromium 145.0.7632.6.
#[test]
fn cold_document_export_matches_single_reads_across_contents_and_selector_branches() {
    let body = "<style>#scope {display:flex;--weight:700;--spacing:3px} #scope > .contents {display:contents;font-weight:var(--weight);letter-spacing:var(--spacing)} #scope > .contents > span {font-style:italic} #scope > .other > span {font-weight:600;letter-spacing:5px}</style><main id=scope><section id=contents class=contents><span id=first>A</span><span id=second>B</span></section><section class=other><span id=third>C</span></section></main>";
    let properties = ["font-weight", "letter-spacing", "font-style", "display"].map(str::to_owned);
    for reverse in [false, true] {
        let mut ids = ["first", "third", "second", "contents"];
        let mut expected = vec![
            vec!["700", "3px", "italic", "block"],
            vec!["600", "5px", "normal", "inline"],
            vec!["700", "3px", "italic", "block"],
            vec!["700", "3px", "normal", "contents"],
        ];
        if reverse {
            ids.reverse();
            expected.reverse();
        }
        for batch in [false, true] {
            // Each resolver/order gets a new, completely cold document.
            let vm = fixture(body);
            let nodes = {
                let host = vm.context_host_weak_for_test().upgrade().unwrap();
                let host = host.borrow();
                ids.map(|id| host.dom_host().element_handle_by_id(id).unwrap())
            };
            let values = if batch {
                export_values(&vm, nodes, &properties)
            } else {
                let host = vm.context_host_weak_for_test().upgrade().unwrap();
                let host = host.borrow();
                let mut observation = StyleObservation::new(&host);
                nodes
                    .into_iter()
                    .map(|node| {
                        let style = observation.read(node);
                        properties.iter().map(|name| style.property(name)).collect()
                    })
                    .collect::<Vec<Vec<String>>>()
            };
            assert_eq!(values, expected, "batch={batch}, reverse={reverse}");
            assert_eq!(export_values(&vm, nodes, &properties), expected);
        }
    }
}

#[test]
fn document_export_and_explicit_pseudo_queries_share_invalidation_in_either_order() {
    let body = "<style>main {display:flex;--weight:700;--spacing:3px;--ink:rgb(10,20,30)} section {display:contents;font-weight:var(--weight);letter-spacing:var(--spacing)} span::before {color:var(--ink);font-weight:600;content:none} .generated span::before {content:'before';font-weight:800}</style><main><section><span id=target>Hello</span></section></main>";
    let properties = ["font-weight", "letter-spacing", "display"].map(str::to_owned);
    for batch_first in [true, false] {
        let mut vm = fixture(body);
        let target = {
            let host = vm.context_host_weak_for_test().upgrade().unwrap();
            let host = host.borrow();
            host.dom_host().element_handle_by_id("target").unwrap()
        };
        for (mutation, content, weight, color) in [
            ("void 0", "none", "600", "rgb(10, 20, 30)"),
            (
                "document.body.className='generated'",
                "\"before\"",
                "800",
                "rgb(10, 20, 30)",
            ),
            (
                "document.querySelector('main').style.setProperty('--ink','rgb(40,50,60)')",
                "\"before\"",
                "800",
                "rgb(40, 50, 60)",
            ),
            (
                "document.body.className=''",
                "none",
                "600",
                "rgb(40, 50, 60)",
            ),
        ] {
            vm.eval(mutation).unwrap();
            if batch_first {
                assert_eq!(
                    export_values(&vm, [target], &properties),
                    [["700", "3px", "block"]]
                );
            }
            let actual = vm.eval("JSON.stringify((() => { const target=document.getElementById('target'), style=getComputedStyle(target), before=getComputedStyle(target,'::before'); return [style.fontWeight,style.letterSpacing,style.display,before.content,before.fontWeight,before.color]; })())").unwrap();
            assert_eq!(
                actual,
                serde_json::to_string(&["700", "3px", "block", content, weight, color]).unwrap(),
                "batch_first={batch_first}, after {mutation}",
            );
            if !batch_first {
                assert_eq!(
                    export_values(&vm, [target], &properties),
                    [["700", "3px", "block"]]
                );
            }
        }
    }
}

#[test]
fn document_export_refreshes_shadow_inheritance_after_slot_and_connection_changes() {
    let mut vm = fixture(
        "<main id=host style='--weight:700;--spacing:3px'><article id=assigned slot=content style='font-weight:var(--weight);letter-spacing:var(--spacing)'>Assigned</article></main>",
    );
    vm.eval("globalThis.exportHost=document.getElementById('host'); globalThis.exportRoot=exportHost.attachShadow({mode:'closed'}); exportRoot.innerHTML='<slot name=content style=\"font-weight:var(--weight);letter-spacing:var(--spacing)\"><span>Fallback</span></slot>'; globalThis.exportFallback=exportRoot.firstElementChild.firstElementChild").unwrap();
    let nodes = {
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let assigned = host.dom_host().element_handle_by_id("assigned").unwrap();
        let shadow_host = host.dom_host().element_handle_by_id("host").unwrap();
        let root = host.dom_host().shadow_root_handle(shadow_host).unwrap();
        let slot = host.dom_host().child_handles(root).next().unwrap();
        let fallback = host.dom_host().child_handles(slot).next().unwrap();
        [assigned, fallback, slot]
    };
    let properties = ["font-weight", "letter-spacing"].map(str::to_owned);
    for (mutation, selected, expected) in [
        ("void 0", 0, Some(["700", "3px"])),
        (
            "exportHost.style.setProperty('--weight','900')",
            0,
            Some(["900", "3px"]),
        ),
        (
            "exportRoot.firstElementChild.name='other'",
            1,
            Some(["900", "3px"]),
        ),
        (
            "exportRoot.firstElementChild.style.setProperty('--spacing','5px')",
            1,
            Some(["900", "5px"]),
        ),
        ("exportHost.remove()", 1, None),
        (
            "document.body.appendChild(exportHost)",
            1,
            Some(["900", "5px"]),
        ),
    ] {
        vm.eval(mutation).unwrap();
        // Live Markdown traversal supplies the slot and its current assigned
        // or fallback child, never the inactive fallback subtree.
        for order in [[2, selected], [selected, 2]] {
            let values = export_values(&vm, order.map(|i| nodes[i]), &properties);
            for value in values {
                if let Some(expected) = expected {
                    assert_eq!(value, expected, "after {mutation}");
                } else {
                    assert!(value.is_empty(), "after {mutation}");
                }
            }
        }
        if selected == 1
            && let Some(expected) = expected
        {
            assert_eq!(vm.eval("JSON.stringify([getComputedStyle(exportFallback).fontWeight,getComputedStyle(exportFallback).letterSpacing])").unwrap(), serde_json::to_string(&expected).unwrap());
        }
    }
}
