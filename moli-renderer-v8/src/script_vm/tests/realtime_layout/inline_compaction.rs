use super::realtime_vm;

#[test]
fn realtime_retains_paragraph_shaping_across_box_compaction() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;overflow:hidden}",
        "#prefix div{height:5px}main{width:180px}p{margin:0;font:16px/1.25 sans-serif}",
        "#nested{color:red;vertical-align:2px}#atomic{display:inline-block;width:40px}",
        "</style><section id=prefix><div></div></section><main id=content>",
        "<p id=plain>alpha <span id=nested>beta אבג gamma</span> delta epsilon zeta</p>",
        "<p id=mixed>before <span id=atomic>object</span> after more words</p></main>",
    ));
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let document = host.borrow().document_handle();
    let viewport = host.borrow().layout_viewport_for_document(document);
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();

    for (edit, expect_reused) in [
        ("prefix.prepend(document.createElement('div'))", true),
        ("prefix.firstChild.remove()", true),
        ("prefix.style.display='none'", true),
        ("prefix.style.display='block'", true),
        (
            "prefix.prepend(document.createElement('div'));content.style.width='95px'",
            true,
        ),
        (
            "nested.style.fontSize='22px';prefix.firstChild.remove()",
            false,
        ),
        (
            "nested.textContent='replacement text';prefix.prepend(document.createElement('div'))",
            false,
        ),
        (
            "const range=document.createRange();range.selectNodeContents(nested);\
             getSelection().removeAllRanges();getSelection().addRange(range);\
             prefix.firstChild.remove()",
            false,
        ),
    ] {
        vm.eval(edit).unwrap();
        let retained = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
        let metrics = vm.layout_pass_observability_for_test().3.unwrap();
        if expect_reused {
            assert_eq!(metrics.reused_inline_contexts, 3, "{edit}: {metrics:?}");
        }
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
        for name in ["plain", "nested", "mixed", "atomic"] {
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
