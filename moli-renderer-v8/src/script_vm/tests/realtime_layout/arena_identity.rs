use super::realtime_vm;

#[test]
fn stable_box_handles_preserve_fresh_geometry_and_paint_across_reparenting() {
    let mut vm = realtime_vm(concat!(
        "<!doctype html><style>body{margin:0;overflow:hidden;font:16px/1.25 sans-serif}",
        "#columns{display:flex;align-items:start}section{width:180px;flex:none}",
        "#right{width:125px;direction:rtl}p{margin:0}#atomic{display:inline-block;width:30px}",
        "#grid{display:grid;grid-template-columns:40px 60px}td{padding:2px}",
        "#positioned{position:relative;height:35px}#absolute{position:absolute;left:3px;top:4px}",
        "#float{float:left;width:35px;height:30px}#scroll{overflow:scroll;width:120px;height:30px}",
        "#tall{height:100px}</style><div id=prefix></div><div id=columns>",
        "<section id=left><p id=paragraph><span id=text>alpha אבג beta gamma</span>",
        "<span id=atomic>inline</span> more words</p><div id=grid><div>A</div><div>B</div></div></section>",
        "<section id=right><table id=table><tbody id=rows><tr><td>one</td><td>two</td></tr></tbody></table>",
        "<div id=positioned><span id=absolute>absolute</span></div></section></div>",
        "<div id=float></div><p id=wrapped>Wrapping alongside a float with more words.</p>",
        "<div id=scroll><div id=tall>scroll content</div></div><ol><li id=marker>marker</li></ol>",
    ));
    let host = vm.context_host_weak_for_test().upgrade().unwrap();
    let document = host.borrow().document_handle();
    let viewport = host.borrow().layout_viewport_for_document(document);
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
    for edit in [
        "prefix.prepend(document.createElement('div'));prefix.firstChild.style.height='7px'",
        "right.prepend(paragraph)",
        "window.removed=grid.firstChild;removed.remove()",
        "grid.append(removed);grid.style.gridTemplateColumns='55px 45px'",
        "rows.insertAdjacentHTML('afterbegin','<tr><td colspan=2>new row</td></tr>')",
        "left.append(table);right.style.width='95px'",
        "atomic.style.display='block';positioned.style.position='static'",
        "float.style.width='65px';prefix.firstChild.remove()",
        "document.getElementById('scroll').scrollTop=20;columns.style.transform='translateX(3.25px)'",
        "const range=document.createRange();range.selectNodeContents(text);getSelection().addRange(range)",
    ] {
        vm.eval(edit).unwrap();
        let retained = vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
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
        for name in [
            "paragraph",
            "text",
            "atomic",
            "grid",
            "table",
            "absolute",
            "wrapped",
            "scroll",
            "marker",
        ] {
            let node = vm.document_runtime.get_element_by_id(name).unwrap();
            let retained = host
                .with_latest_layout_tree_for_document(document, |tree| {
                    tree.client_rects_for_source(node)
                })
                .unwrap();
            assert_eq!(
                retained,
                fresh.client_rects_for_source(node),
                "{edit}: {name}"
            );
        }
    }
}
