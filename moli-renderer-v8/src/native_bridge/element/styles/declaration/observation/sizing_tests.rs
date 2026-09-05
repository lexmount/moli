use super::super::sizing::{SizeQueryCounts, take_query_counts};
use super::*;
use crate::{
    dom::native::DomHost,
    parser::HtmlParser,
    script_vm::{ScriptVmDefaultWorldBootstrap, StandaloneScriptVmHarness},
};

mod sampling;

fn fixture(body: &str) -> StandaloneScriptVmHarness {
    let _runtime = crate::JsRuntime::initialize();
    let document = HtmlParser::SCRIPTING_ENABLED.parse(
        url::Url::parse("https://size-queries.test/").unwrap(),
        format!(r#"<!doctype html><style>html,body{{margin:0}}.sized{{width:500px;height:400px;max-width:120px;max-height:80px}}</style>{body}"#),
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
    vm.set_layout_policy(crate::real_layout_test_policy());
    vm
}

fn sample(vm: &mut StandaloneScriptVmHarness) {
    assert!(
        vm.screenshot_layout_snapshot(moli_layout::LayoutViewport::new(320, 240, 1.0))
            .unwrap()
            .is_some()
    );
}

fn sizes(read: &ComputedStyleRead<'_>) -> [String; 4] {
    ["width", "height", "inline-size", "block-size"].map(|property| read.property(property))
}

#[test]
fn computed_size_property_batch_looks_up_one_element_once_without_a_document_index() {
    let mut vm = fixture("<div id=target class=sized></div>");
    sample(&mut vm);
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let host = host.borrow();
    let target = host.dom_host().element_handle_by_id("target").unwrap();
    let before = host.layout_snapshot_cache_observability_for_test();
    take_query_counts();
    let read = ComputedStyleRead::new(&host, target);
    read.property("color");
    assert_eq!(
        take_query_counts(),
        SizeQueryCounts::default(),
        "non-size reads do no geometry work"
    );
    for _ in 0..20 {
        assert_eq!(sizes(&read), ["120px", "80px", "120px", "80px"]);
    }
    assert_eq!(
        take_query_counts(),
        SizeQueryCounts {
            source_queries: 1,
            ..SizeQueryCounts::default()
        }
    );
    assert_eq!(host.layout_snapshot_cache_observability_for_test(), before);
}

#[test]
fn computed_size_inspector_property_batch_shares_one_size_lookup() {
    let mut vm = fixture("<div id=target class=sized></div>");
    sample(&mut vm);
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let host = host.borrow();
    let target = host.dom_host().element_handle_by_id("target").unwrap();
    take_query_counts();
    let properties = crate::native_bridge::element::computed_style_properties_for_inspector_handle(
        &host, target,
    )
    .unwrap()
    .into_iter()
    .collect::<std::collections::HashMap<_, _>>();
    for (name, value) in [
        ("width", "120px"),
        ("height", "80px"),
        ("inline-size", "120px"),
        ("block-size", "80px"),
    ] {
        assert_eq!(
            properties.get(name).map(String::as_str),
            Some(value),
            "{name}"
        );
    }
    assert_eq!(
        take_query_counts(),
        SizeQueryCounts {
            source_queries: 1,
            ..SizeQueryCounts::default()
        }
    );
}

#[test]
fn computed_size_dom_snapshot_batches_all_nodes_in_one_observation() {
    let mut markup = String::new();
    for i in 0..128 {
        markup.push_str(&format!("<div class=sized id=box-{i}></div>"));
    }
    let mut vm = fixture(&markup);
    sample(&mut vm);
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let handles = {
        let host = host.borrow();
        (0..128)
            .map(|i| {
                host.dom_host()
                    .element_handle_by_id(&format!("box-{i}"))
                    .unwrap()
            })
            .collect::<Vec<_>>()
    };
    let properties = ["width", "height", "inline-size", "block-size"].map(String::from);
    take_query_counts();
    let values = vm.computed_style_property_values_for_document_snapshot(handles, &properties);
    assert_eq!(values.len(), 128);
    for sizes in values {
        assert_eq!(sizes, ["120px", "80px", "120px", "80px"]);
    }
    let counts = take_query_counts();
    assert_eq!(counts.index_builds, 1);
    assert_eq!(counts.source_queries, 0);
}

#[test]
fn computed_size_multi_element_batch_indexes_each_box_once_and_drops_its_lookup() {
    let mut markup = String::new();
    for i in 0..512 {
        markup.push_str(&format!("<div class=sized id=box-{i}></div>"));
    }
    markup.push_str("<div id=hidden class=sized style='display:none'></div>");
    let mut vm = fixture(&markup);
    sample(&mut vm);
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let host = host.borrow();
    let handles = (0..512)
        .map(|i| {
            host.dom_host()
                .element_handle_by_id(&format!("box-{i}"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    let hidden = host.dom_host().element_handle_by_id("hidden").unwrap();
    let before = host.layout_snapshot_cache_observability_for_test();
    let boxes = before.cached.unwrap().1.box_count;
    take_query_counts();
    let weak_inputs;
    {
        let mut observation = StyleObservation::new(&host);
        let first = observation.read(handles[0]);
        weak_inputs = Rc::downgrade(&first.observation_inputs);
        first.property("color");
        assert_eq!(take_query_counts(), SizeQueryCounts::default());
        // Non-box misses must also be answered from the one projection, not
        // repeatedly rescan the complete tree.
        for handle in handles
            .iter()
            .rev()
            .chain(handles.iter())
            .copied()
            .chain([hidden; 20])
        {
            let expected = if handle == hidden {
                ["500px", "400px", "500px", "400px"]
            } else {
                ["120px", "80px", "120px", "80px"]
            };
            assert_eq!(sizes(&observation.read(handle)), expected);
        }
        assert_eq!(
            take_query_counts(),
            SizeQueryCounts {
                source_queries: 0,
                index_builds: 1,
                indexed_boxes: boxes
            }
        );
        assert!(weak_inputs.upgrade().is_some());
    }
    assert!(
        weak_inputs.upgrade().is_none(),
        "no Document or wrapper retains the lookup"
    );
    assert_eq!(host.layout_snapshot_cache_observability_for_test(), before);
    let mut later = StyleObservation::new(&host);
    assert_eq!(
        sizes(&later.read(handles[0])),
        ["120px", "80px", "120px", "80px"]
    );
    assert_eq!(
        take_query_counts().index_builds,
        1,
        "a new observation derives its own projection"
    );
}

#[test]
fn computed_size_query_reuse_keeps_cold_mock_and_missing_box_reads_lazy() {
    let mut vm = fixture("<div id=target class=sized style='display:none'></div>");
    for sampled in [false, true] {
        if sampled {
            sample(&mut vm);
        }
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let target = host.dom_host().element_handle_by_id("target").unwrap();
        let before = host.layout_snapshot_cache_observability_for_test();
        take_query_counts();
        let read = ComputedStyleRead::new(&host, target);
        for _ in 0..20 {
            assert_eq!(sizes(&read), ["500px", "400px", "500px", "400px"]);
        }
        assert_eq!(
            take_query_counts().source_queries,
            usize::from(sampled),
            "cache an absent sizing box as well as a present one"
        );
        let mut batch = StyleObservation::new(&host);
        for _ in 0..20 {
            assert_eq!(
                sizes(&batch.read(target)),
                ["500px", "400px", "500px", "400px"]
            );
        }
        assert_eq!(take_query_counts().index_builds, usize::from(sampled));
        assert_eq!(host.layout_snapshot_cache_observability_for_test(), before);
    }
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let host = host.borrow();
    let target = host.dom_host().element_handle_by_id("target").unwrap();
    let before = host.layout_snapshot_cache_observability_for_test();
    take_query_counts();
    let read = ComputedStyleRead::new(&host, target);
    assert_eq!(sizes(&read), ["500px", "400px", "500px", "400px"]);
    let mut batch = StyleObservation::new(&host);
    assert_eq!(
        sizes(&batch.read(target)),
        ["500px", "400px", "500px", "400px"]
    );
    assert_eq!(take_query_counts(), SizeQueryCounts::default());
    assert_eq!(host.layout_snapshot_cache_observability_for_test(), before);
}

#[test]
fn computed_size_query_reuse_does_not_survive_mutation_refresh_or_document_replacement() {
    let mut vm = fixture("<div id=target class=sized></div>");
    sample(&mut vm);
    for (action, expected) in [
        ("'unchanged'", ["120px", "80px", "120px", "80px"]),
        (
            "document.getElementById('target').style.cssText='width:30px;height:20px;writing-mode:vertical-rl';'mutated'",
            ["120px", "80px", "120px", "80px"],
        ),
        ("'refresh'", ["30px", "20px", "20px", "30px"]),
        (
            "document.open();document.write('<!doctype html><div id=target style=\"width:90px;height:50px\"></div>');document.close();'replaced'",
            ["90px", "50px", "90px", "50px"],
        ),
    ] {
        vm.eval(action).unwrap();
        if action == "'refresh'" {
            sample(&mut vm);
        }
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let target = host.dom_host().element_handle_by_id("target").unwrap();
        let before = host.layout_snapshot_cache_observability_for_test();
        take_query_counts();
        assert_eq!(
            sizes(&ComputedStyleRead::new(&host, target)),
            expected,
            "{action}"
        );
        let mut observation = StyleObservation::new(&host);
        assert_eq!(sizes(&observation.read(target)), expected, "{action}");
        let counts = take_query_counts();
        let has_tree = before.cached.is_some();
        assert_eq!(counts.source_queries, usize::from(has_tree));
        assert_eq!(counts.index_builds, usize::from(has_tree));
        assert_eq!(host.layout_snapshot_cache_observability_for_test(), before);
    }
}

#[test]
fn computed_size_batch_partitions_parent_and_iframe_lookups() {
    let mut vm = fixture(
        "<div id=parent class=sized></div><iframe id=frame style='width:180px;height:120px;border:0'></iframe>",
    );
    vm.eval(r#"
const child=document.getElementById('frame').contentDocument;
child.body.innerHTML='<div id=target style="width:500px;height:400px;max-width:90px;max-height:30px;writing-mode:vertical-rl"></div>';
'installed'
"#).unwrap();
    sample(&mut vm);
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let host = host.borrow();
    let parent = host.dom_host().element_handle_by_id("parent").unwrap();
    let frame = host.dom_host().element_handle_by_id("frame").unwrap();
    let child = host.child_browsing_context_document_handle(frame).unwrap();
    let target = host
        .dom_host()
        .element_handle_by_id_in_subtree(child, "target")
        .unwrap();
    assert!(
        host.with_latest_layout_tree_for_document(child, |_| ())
            .is_some(),
        "the fixture must publish the child tree, not just the parent geometry"
    );
    let before = host.layout_snapshot_cache_observability_for_test();
    let mut observation = StyleObservation::new(&host);
    take_query_counts();
    for _ in 0..20 {
        assert_eq!(
            sizes(&observation.read(parent)),
            ["120px", "80px", "120px", "80px"]
        );
        assert_eq!(
            sizes(&observation.read(target)),
            ["90px", "30px", "30px", "90px"]
        );
    }
    let counts = take_query_counts();
    assert_eq!(counts.index_builds, 2);
    assert_eq!(counts.source_queries, 0);
    let parent_read = observation.read(parent);
    let child_size = parent_read.observation_inputs.used_size(target).unwrap();
    assert_eq!(
        [child_size.width, child_size.height],
        [90.0, 30.0],
        "a recursive cross-Document read must not use the parent table"
    );
    assert_eq!(take_query_counts().source_queries, 1);
    assert_eq!(host.layout_snapshot_cache_observability_for_test(), before);
}
