use super::*;

#[test]
fn selection_worlds_share_live_ranges_with_local_wrappers() {
    for main_first in [false, true] {
        let mut vm = new_storage_html_test_vm("https://selection-worlds.test/");
        vm.eval(
            "document.body.innerHTML='<div id=editor contenteditable=true>edit me</div>'; 'ready'",
        )
        .unwrap();
        if main_first {
            vm.eval("getSelection(); 'ready'").unwrap();
        }
        let isolated = vm.create_isolated_world("selection-worlds", false).unwrap();
        assert_eq!(
            vm.eval_in_isolated_context(
                isolated,
                r#"
                globalThis.selectedRange = document.createRange();
                selectedRange.selectNodeContents(document.getElementById('editor').firstChild);
                selectedRange.isolatedOnly = true;
                getSelection().isolatedOnly = true;
                getSelection().addRange(selectedRange);
                JSON.stringify([
                    getSelection() === document.getSelection(),
                    getSelection().getRangeAt(0) === selectedRange,
                    Object.getPrototypeOf(getSelection()) === Selection.prototype
                ])
            "#
            )
            .unwrap(),
            "[true,true,true]"
        );
        assert_eq!(
            vm.eval(
                r#"
                globalThis.mainRange = getSelection().getRangeAt(0);
                JSON.stringify([
                    String(getSelection()), getSelection().rangeCount,
                    getSelection().anchorNode === document.getElementById('editor').firstChild,
                    getSelection().focusOffset,
                    getSelection() === document.getSelection(),
                    Object.getPrototypeOf(getSelection()) === Selection.prototype,
                    Object.getPrototypeOf(mainRange) === Range.prototype,
                    getSelection().isolatedOnly === undefined,
                    mainRange.isolatedOnly === undefined
                ])
            "#
            )
            .unwrap(),
            r#"["edit me",1,true,7,true,true,true,true,true]"#,
            "main first: {main_first}"
        );
        vm.eval("mainRange.setEnd(document.getElementById('editor').firstChild, 4); 'changed'")
            .unwrap();
        assert_eq!(
            vm.eval_in_isolated_context(
                isolated,
                r#"
                JSON.stringify([
                    String(getSelection()), getSelection().focusOffset,
                    selectedRange.endOffset, getSelection().getRangeAt(0) === selectedRange
                ])
            "#
            )
            .unwrap(),
            r#"["edit",4,4,true]"#
        );
        vm.eval_in_isolated_context(
            isolated,
            "document.getElementById('editor').firstChild.insertData(2, '++'); 'changed'",
        )
        .unwrap();
        assert_eq!(
            vm.eval(
                r#"JSON.stringify([String(getSelection()), mainRange.endOffset,
                getSelection().getRangeAt(0) === mainRange])"#
            )
            .unwrap(),
            r#"["ed++it",6,true]"#
        );
        vm.eval("getSelection().removeAllRanges(); 'cleared'")
            .unwrap();
        assert_eq!(
            vm.eval_in_isolated_context(
                isolated,
                "JSON.stringify([getSelection().rangeCount, String(getSelection())])"
            )
            .unwrap(),
            r#"[0,""]"#
        );
        vm.eval("getSelection().setBaseAndExtent(document.getElementById('editor').firstChild, 6, document.getElementById('editor').firstChild, 2); 'selected'")
            .unwrap();
        assert_eq!(
            vm.eval_in_isolated_context(
                isolated,
                r#"
                JSON.stringify([String(getSelection()), getSelection().anchorOffset,
                    getSelection().focusOffset, getSelection().direction])
            "#
            )
            .unwrap(),
            r#"["++it",6,2,"backward"]"#
        );
        vm.eval_in_isolated_context(
            isolated,
            "getSelection().removeRange(getSelection().getRangeAt(0)); 'cleared'",
        )
        .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify([getSelection().rangeCount, String(getSelection())])")
                .unwrap(),
            r#"[0,""]"#
        );
    }
}

#[tokio::test]
async fn selection_worlds_keep_child_documents_independent() {
    let mut vm = new_storage_html_test_vm("https://selection-child-worlds.test/");
    vm.eval("document.body.innerHTML='<p id=parentText>parent</p>'; getSelection().selectAllChildren(document.getElementById('parentText')); 'ready'")
        .unwrap();
    let child =
        super::window_node_worlds::child_node_world(&mut vm, "selection-child-worlds").await;
    vm.eval_in_isolated_context(
        child,
        "getSelection().selectAllChildren(document.getElementById('childTarget')); 'selected'",
    )
    .unwrap();
    assert_eq!(
        vm.eval(r#"JSON.stringify([String(getSelection()),
            String(frame.contentWindow.getSelection()),
            frame.contentWindow.getSelection().anchorNode === frame.contentDocument.getElementById('childTarget')])"#)
            .unwrap(),
        r#"["parent","child",true]"#
    );
    vm.eval_in_isolated_context(child, "getSelection().removeAllRanges(); 'cleared'")
        .unwrap();
    assert_eq!(
        vm.eval(r#"JSON.stringify([String(getSelection()), frame.contentWindow.getSelection().rangeCount])"#)
            .unwrap(),
        r#"["parent",0]"#
    );
}

#[tokio::test]
async fn selection_worlds_native_input_uses_the_focused_child_document() {
    let mut vm = new_storage_html_test_vm("https://selection-child-input.test/");
    vm.eval("document.body.innerHTML='<p id=parentText>parent</p>'; getSelection().selectAllChildren(document.getElementById('parentText')); 'ready'")
        .unwrap();
    let child = super::window_node_worlds::child_node_world(&mut vm, "selection-child-input").await;
    vm.eval_in_isolated_context(
        child,
        r#"
        const editor = document.getElementById('childTarget');
        editor.contentEditable = 'true';
        editor.focus();
        getSelection().selectAllChildren(editor);
        'ready'
    "#,
    )
    .unwrap();
    assert!(vm.insert_text_into_active_control("typed").unwrap());
    assert_eq!(
        vm.eval(r#"JSON.stringify([String(getSelection()),
            frame.contentDocument.getElementById('childTarget').textContent,
            frame.contentWindow.getSelection().anchorNode.ownerDocument === frame.contentDocument])"#)
            .unwrap(),
        r#"["parent","typed",true]"#
    );
    vm.eval_in_isolated_context(
        child,
        "getSelection().selectAllChildren(editor); 'selected'",
    )
    .unwrap();
    vm.dispatch_key_event("keydown", "Delete", "Delete", "", 0, false, false)
        .unwrap();
    assert_eq!(
        vm.eval(r#"JSON.stringify([String(getSelection()), frame.contentDocument.getElementById('childTarget').textContent])"#)
            .unwrap(),
        r#"["parent",""]"#
    );
    vm.eval_in_isolated_context(child, "editor.textContent='reset'; 'ready'")
        .unwrap();
    vm.dispatch_key_event("keydown", "a", "KeyA", "", 2, false, false)
        .unwrap();
    assert!(vm.insert_text_into_active_control("new").unwrap());
    assert_eq!(
        vm.eval(r#"JSON.stringify([String(getSelection()), frame.contentDocument.getElementById('childTarget').textContent])"#)
            .unwrap(),
        r#"["parent","new"]"#
    );
}

#[tokio::test]
async fn selection_worlds_rebind_initial_child_document_together() {
    let mut vm = new_storage_html_test_vm("https://selection-worlds-rebind.test/");
    vm.eval(
        r#"
        globalThis.frame = document.body.appendChild(document.createElement('iframe'));
        frame.contentDocument.body.innerHTML = '<p id=oldText>old</p>';
        'ready'
    "#,
    )
    .unwrap();
    let child = materialize_single_child_default_realm_for_test(&mut vm, "Selection rebind");
    let frame_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .find(|realm| realm.context_id == child)
        .and_then(|realm| realm.frame_id)
        .unwrap();
    let isolated = vm
        .create_isolated_world_for_frame(&frame_id, "selection-rebind", false)
        .unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
        globalThis.savedSelection = getSelection();
        savedSelection.selectAllChildren(document.getElementById('oldText'));
        'ready'
    "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval("String(frame.contentWindow.getSelection())")
            .unwrap(),
        "old"
    );
    vm.eval("frame.srcdoc='<body><p id=newText>new</p>'; 'navigating'")
        .unwrap();
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "Selection rebind").await;
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"JSON.stringify([
            savedSelection === getSelection(), savedSelection.rangeCount, String(savedSelection)
        ])"#
        )
        .unwrap(),
        r#"[true,0,""]"#
    );
    vm.eval_in_isolated_context(
        isolated,
        r#"
        const range = document.createRange();
        range.selectNodeContents(document.getElementById('newText'));
        savedSelection.addRange(range);
        'selected'
        "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"JSON.stringify((() => {
            const selection = frame.contentWindow.getSelection();
            const node = frame.contentDocument.getElementById('newText');
            return [selection.rangeCount, selection.anchorNode === node,
                selection.anchorNode.ownerDocument === frame.contentDocument,
                selection.getRangeAt(0).toString()];
        })())"#
        )
        .unwrap(),
        r#"[1,true,true,"new"]"#
    );
}
