use super::*;

#[test]
fn editing_shortcuts_delete_crosses_inline_text_nodes_and_undo_restores_them() {
    for (key_name, code, offset, caret_node, expected) in [
        (
            "Delete",
            "NumpadDecimal",
            2,
            "editor.firstChild.firstChild",
            "abd",
        ),
        (
            "Backspace",
            "Backspace",
            0,
            "editor.lastChild.firstChild",
            "acd",
        ),
    ] {
        let mut vm = new_parsed_test_vm(
            "https://editing-inline-delete.test/",
            "<div id=editor contenteditable><b>ab</b><i>cd</i></div>",
        );
        vm.eval(&format!("globalThis.editor=document.getElementById('editor');globalThis.bold=editor.firstChild;globalThis.italic=editor.lastChild;editor.focus();getSelection().collapse({caret_node},{offset})")).unwrap();
        key(&mut vm, key_name, code, 0);
        assert_eq!(vm.eval("editor.textContent").unwrap(), expected);
        key(&mut vm, "z", "KeyZ", 2);
        assert_eq!(vm.eval("editor.textContent==='abcd' && editor.firstChild===bold && editor.lastChild===italic").unwrap(), "true");
    }
    let mut vm = new_parsed_test_vm(
        "https://editing-inline-scalar.test/",
        "<div id=editor contenteditable><b>ab</b><i>😀d</i></div>",
    );
    vm.eval("globalThis.editor=document.getElementById('editor');editor.focus();getSelection().collapse(editor.firstChild.firstChild,2)").unwrap();
    key(&mut vm, "Delete", "Delete", 0);
    assert_eq!(vm.eval("editor.textContent").unwrap(), "abd");
}

#[test]
fn editing_shortcuts_delete_respects_adjacent_editing_boundaries() {
    for attributes in ["contenteditable=false", "inert"] {
        let mut vm = new_parsed_test_vm(
            "https://editing-delete-boundary.test/",
            &format!(
                "<div id=editor contenteditable><b>ab</b><span {attributes}>LOCKED</span><i>cd</i></div><div>outside</div>"
            ),
        );
        vm.eval("globalThis.editor=document.getElementById('editor');globalThis.locked=editor.querySelector('span').firstChild;editor.focus();getSelection().collapse(editor.firstChild.firstChild,2)").unwrap();
        key(&mut vm, "Delete", "Delete", 0);
        assert_eq!(vm.eval("locked.data").unwrap(), "LOCKED");
        vm.eval("getSelection().collapse(editor.lastChild.firstChild,2)")
            .unwrap();
        key(&mut vm, "Delete", "Delete", 0);
        assert_eq!(
            vm.eval("document.body.lastChild.textContent").unwrap(),
            "outside"
        );
    }
}
