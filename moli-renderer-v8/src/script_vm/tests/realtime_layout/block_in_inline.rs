use moli_layout::LayoutRect;

use super::{StandaloneScriptVmHarness, realtime_vm};

fn content_rects(vm: &StandaloneScriptVmHarness, id: &str) -> Vec<LayoutRect> {
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let host = host.borrow();
    let node = vm.document_runtime.get_element_by_id(id).unwrap();
    host.with_latest_layout_tree_for_document(host.document_handle(), |tree| {
        tree.content_quads_for_source(node)
            .into_iter()
            .map(|quad| quad.bounding_rect())
            .collect()
    })
    .unwrap()
}

#[test]
fn block_in_inline_grouping_and_sizing_match_chromium() {
    // Chromium 156 with a 300px containing block. CSSOM uses the anonymous
    // run while the inspector reports the actual member boxes.
    for (name, markup, bounds, offsets, rect_count, content) in [
        (
            "adjacent blocks separated by collapsible spaces",
            "<a id=a><div class=b></div>  <div class=c></div></a>",
            "[0,0,300,50]",
            "[0,0,300,50]",
            3,
            vec![
                LayoutRect::new(0.0, 0.0, 120.0, 20.0),
                LayoutRect::new(0.0, 20.0, 80.0, 30.0),
            ],
        ),
        (
            "empty inline separates block runs",
            "<a id=a><div class=b></div><span></span><div class=c></div></a>",
            "[0,0,300,50]",
            "[0,0,300,50]",
            5,
            vec![
                LayoutRect::new(0.0, 0.0, 120.0, 20.0),
                LayoutRect::new(0.0, 20.0, 0.0, 0.0),
                LayoutRect::new(0.0, 20.0, 80.0, 30.0),
            ],
        ),
        (
            "negative margin and degenerate bounds",
            "<a id=a><div class=b style='margin-bottom:-50px'></div><div class=c></div></a>",
            "[0,0,0,0]",
            "[0,0,300,0]",
            3,
            vec![
                LayoutRect::new(0.0, 0.0, 120.0, 20.0),
                LayoutRect::new(0.0, -30.0, 80.0, 30.0),
            ],
        ),
        (
            "collapsed margins and auto inline margins",
            concat!(
                "<div style='height:10px'></div><a id=a>",
                "<div class=b style='margin:30px auto 20px'></div>",
                "<div class=c style='margin:40px 0 10px'></div></a><div style='height:10px'></div>",
            ),
            "[0,40,300,90]",
            "[0,40,300,90]",
            3,
            vec![
                LayoutRect::new(90.0, 40.0, 120.0, 20.0),
                LayoutRect::new(0.0, 100.0, 80.0, 30.0),
            ],
        ),
        (
            "float clearance",
            "<div style='float:left;width:80px;height:40px'></div><a id=a><div class=b style='clear:both'></div></a>",
            "[0,0,300,60]",
            "[80,0,300,60]",
            3,
            vec![LayoutRect::new(0.0, 40.0, 120.0, 20.0)],
        ),
        (
            "definite containing height",
            "<a id=a><div class=b style='height:50%'></div></a>",
            "[0,0,300,50]",
            "[0,0,300,50]",
            3,
            vec![LayoutRect::new(0.0, 0.0, 120.0, 50.0)],
        ),
    ] {
        let fixture = format!(
            "<!doctype html><style>body{{margin:0;font:16px/20px Arial}}\
             section{{display:flow-root;width:300px;height:100px}}\
             .b{{width:120px;height:20px}}.c{{width:80px;height:30px}}</style>\
             <section>{markup}</section>"
        );
        let mut vm = realtime_vm(&fixture);
        assert_eq!(
            vm.eval("const r=a.getBoundingClientRect();JSON.stringify([r.x,r.y,r.width,r.height])")
                .unwrap(),
            bounds,
            "{name}"
        );
        assert_eq!(
            vm.eval("JSON.stringify([a.offsetLeft,a.offsetTop,a.offsetWidth,a.offsetHeight])")
                .unwrap(),
            offsets,
            "{name}"
        );
        assert_eq!(
            vm.eval("String(a.getClientRects().length)").unwrap(),
            rect_count.to_string(),
            "{name}"
        );
        assert_eq!(content_rects(&vm, "a"), content, "{name}");
    }
}

#[test]
fn split_wrapper_has_one_hit_owner_and_query_specific_coordinates() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;font:16px/20px Arial}",
        "section{display:flow-root;width:300px}",
        "#a{position:relative;left:10px;top:5px}",
        "#inner{position:relative;left:30px;top:7px}",
        "#block{width:120px;height:20px}</style>",
        "<section><a id=a><span id=inner><div id=block></div></span></a></section>",
    ));
    assert_eq!(
        vm.eval(
            "const rect=r=>[r.x,r.y,r.width,r.height];JSON.stringify([a,inner,block].map(n=>rect(n.getBoundingClientRect())))",
        )
        .unwrap(),
        "[[10,5,300,20],[40,12,300,20],[40,12,120,20]]"
    );
    assert_eq!(
        content_rects(&vm, "a"),
        [LayoutRect::new(40.0, 12.0, 120.0, 20.0)]
    );
    assert_eq!(
        vm.eval("JSON.stringify([document.elementFromPoint(200,20).id,document.elementFromPoint(200,8).id])")
            .unwrap(),
        "[\"inner\",\"\"]"
    );
    assert_eq!(
        vm.eval("inner.style.pointerEvents='none';document.elementFromPoint(200,20).id")
            .unwrap(),
        ""
    );
}

#[test]
fn range_uses_the_selected_element_or_its_contents() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0}section{width:300px}#block{width:120px;height:20px}</style>",
        "<section><a id=a><div id=block></div></a></section>",
    ));
    assert_eq!(
        vm.eval(concat!(
            "const r=document.createRange();",
            "r.selectNode(a);const selected=Array.from(r.getClientRects(),x=>x.width);",
            "r.selectNodeContents(a);JSON.stringify([selected,Array.from(r.getClientRects(),x=>x.width)])",
        ))
        .unwrap(),
        "[[0,300,0],[120]]"
    );
}

#[test]
fn block_in_inline_range_respects_text_and_child_offsets() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;font:16px Arial}section{width:300px}",
        "#block{width:120px;height:20px}</style>",
        "<section><a id=a><span id=first>ab</span><div id=block></div>",
        "<span id=last>cd</span></a></section>",
    ));
    assert_eq!(
        vm.eval(concat!(
            "const r=document.createRange();",
            "r.setStart(first.firstChild,1);r.setEnd(a,2);",
            "const left=Array.from(r.getClientRects(),x=>x.width);",
            "r.setStart(a,1);r.setEnd(last.firstChild,1);",
            "const right=Array.from(r.getClientRects(),x=>x.width);",
            "String(left.length===2 && right.length===2 && left[1]===120 && right[0]===120",
            " && left[0]>0 && left[0]<first.getBoundingClientRect().width",
            " && right[1]>0 && right[1]<last.getBoundingClientRect().width)",
        ))
        .unwrap(),
        "true"
    );
}

#[test]
fn block_in_inline_content_quads_include_positioned_children_in_order() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;font:16px/20px Arial}section{width:300px}",
        "a{position:relative}#positioned{position:absolute;left:80px;top:50px;width:20px;height:25px}",
        "#block{width:120px;height:20px}</style>",
        "<section><a id=a><span id=positioned></span><div id=block></div></a></section>",
    ));
    assert_eq!(
        vm.eval("String(a.getBoundingClientRect().height)").unwrap(),
        "20"
    );
    assert_eq!(
        content_rects(&vm, "a"),
        [
            LayoutRect::new(80.0, 50.0, 20.0, 25.0),
            LayoutRect::new(0.0, 0.0, 120.0, 20.0),
        ]
    );
}

#[test]
fn vertical_block_in_inline_preserves_its_child_flow() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0}section{width:300px;height:200px;writing-mode:vertical-rl}",
        "#block{width:120px;height:20px}</style>",
        "<section><a id=a><div id=block></div></a></section>",
    ));
    assert_eq!(
        vm.eval("const r=block.getBoundingClientRect();JSON.stringify([r.x,r.y,r.width,r.height])")
            .unwrap(),
        "[180,0,120,20]"
    );
    assert_eq!(
        content_rects(&vm, "a"),
        [LayoutRect::new(180.0, 0.0, 120.0, 20.0)]
    );
}

#[test]
fn split_edges_slice_decorations_and_box_model_insets() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;font:16px/20px Arial}",
        "section{width:300px}a{padding:2px 4px;border:1px solid}",
        "#block{width:120px;height:20px}</style>",
        "<section><a id=a><div id=block></div></a></section>",
    ));
    assert_eq!(
        vm.eval("JSON.stringify(Array.from(a.getClientRects(),r=>[r.x,r.y,r.width,r.height]))")
            .unwrap(),
        "[[0,-2,5,23],[0,20,300,20],[0,38,5,23]]"
    );
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let host = host.borrow();
    let node = vm.document_runtime.get_element_by_id("a").unwrap();
    let model = host
        .with_latest_layout_tree_for_document(host.document_handle(), |tree| {
            tree.box_model_for_source(node).unwrap()
        })
        .unwrap();
    assert_eq!(
        model.border.bounding_rect(),
        LayoutRect::new(0.0, -2.0, 300.0, 63.0)
    );
    assert_eq!(
        model.padding.bounding_rect(),
        LayoutRect::new(1.0, -1.0, 298.0, 61.0)
    );
    assert_eq!(
        model.content.bounding_rect(),
        LayoutRect::new(5.0, 1.0, 290.0, 57.0)
    );
}

#[test]
fn unadorned_split_inline_preserves_nested_positioned_text_contributions() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;font:16px/20px Arial}section{width:300px}",
        "span{position:relative;left:30px;top:7px}div{width:120px;height:20px}</style>",
        "<section><a id=a>first<span>inner<div></div>inside</span>after</a></section>",
    ));
    assert_eq!(
        vm.eval("JSON.stringify(Array.from(a.getClientRects(),r=>Math.round(r.y)))")
            .unwrap(),
        "[1,8,27,48,41]"
    );
    let quads = content_rects(&vm, "a");
    assert_eq!(quads.len(), 5);
    assert_eq!(quads[2], LayoutRect::new(30.0, 27.0, 120.0, 20.0));
    assert_eq!(quads[1].y, 8.0);
    assert_eq!(quads[3].y, 48.0);
}

#[test]
fn split_block_run_reprojects_when_scrolled_without_rebuilding_layout() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0}#scroll{width:300px;height:80px;overflow:auto;scrollbar-width:none}",
        "#before{height:60px}#block{width:120px;height:20px}#after{height:200px}</style>",
        "<section id=scroll><div id=before></div><a id=a><div id=block></div></a><div id=after></div></section>",
    ));
    assert_eq!(
        vm.eval("String(a.getBoundingClientRect().top)").unwrap(),
        "60"
    );
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("document.getElementById('scroll').scrollTop=40;const b=a.getBoundingClientRect();JSON.stringify([b.x,b.y,b.width,b.height])")
            .unwrap(),
        "[0,20,300,20]"
    );
    assert_eq!(
        content_rects(&vm, "a"),
        [LayoutRect::new(0.0, 20.0, 120.0, 20.0)]
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
}
