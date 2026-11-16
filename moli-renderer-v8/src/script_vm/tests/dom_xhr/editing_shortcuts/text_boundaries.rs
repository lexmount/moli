use super::*;

#[test]
fn editing_shortcuts_delete_joins_paragraphs_without_removing_a_character() {
    for (open, close) in [
        ("<p>", "</p>"),
        ("<div>", "</div>"),
        ("<span style='display:block'>", "</span>"),
    ] {
        for backward in [false, true] {
            for separator in ["", "\n  "] {
                let mut vm = new_parsed_test_vm(
                    "https://editing-block-boundary.test/",
                    &format!(
                        "<div id=editor contenteditable>{open}ab{close}{separator}{open}cd{close}</div>"
                    ),
                );
                vm.eval("globalThis.editor=document.getElementById('editor');globalThis.before=editor.innerHTML;globalThis.left=editor.firstElementChild;globalThis.right=editor.lastElementChild;globalThis.rightText=right.firstChild;editor.focus()").unwrap();
                vm.eval(if backward {
                    "getSelection().collapse(right.firstChild,0)"
                } else {
                    "getSelection().collapse(left.firstChild,2)"
                })
                .unwrap();
                key(
                    &mut vm,
                    if backward { "Backspace" } else { "Delete" },
                    if backward { "Backspace" } else { "Delete" },
                    0,
                );
                assert_eq!(
                    vm.eval("editor.textContent+':'+editor.children.length")
                        .unwrap(),
                    "abcd:1"
                );
                key(&mut vm, "z", "KeyZ", 2);
                assert_eq!(vm.eval("editor.innerHTML===before && editor.firstElementChild===left && editor.lastElementChild===right && right.firstChild===rightText").unwrap(), "true");
                key(&mut vm, "z", "KeyZ", 10);
                assert_eq!(vm.eval("editor.textContent").unwrap(), "abcd");
            }
        }
    }
}

#[test]
fn editing_shortcuts_delete_crosses_hidden_subtrees_without_editing_hidden_text() {
    for attributes in [
        "hidden",
        "style='display:none'",
        "style='visibility:hidden'",
    ] {
        let mut vm = new_parsed_test_vm(
            "https://editing-hidden-boundary.test/",
            &format!(
                "<div id=editor contenteditable><b>ab</b><span id=hidden {attributes}>SECRET</span><i>cd</i></div>"
            ),
        );
        vm.eval("globalThis.editor=document.getElementById('editor');globalThis.hidden=document.getElementById('hidden');globalThis.hiddenText=hidden.firstChild;editor.focus();getSelection().collapse(editor.firstChild.firstChild,2)").unwrap();
        key(&mut vm, "Delete", "Delete", 0);
        assert_eq!(vm.eval("editor.textContent").unwrap(), "abd");
        assert_eq!(
            vm.eval("hiddenText.data+':'+hidden.isConnected").unwrap(),
            "SECRET:false"
        );
        key(&mut vm, "z", "KeyZ", 2);
        assert_eq!(
            vm.eval("editor.textContent+':'+(document.getElementById('hidden')===hidden)")
                .unwrap(),
            "abSECRETcd:true"
        );
    }
}

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

#[test]
fn editing_shortcuts_copy_serializes_visible_br_without_changing_range_text() {
    let mut vm = new_parsed_test_vm(
        "https://editing-copy-break.test/",
        "<div id=editor contenteditable>a<br>b</div><textarea id=output></textarea>",
    );
    vm.eval("globalThis.editor=document.getElementById('editor');globalThis.output=document.getElementById('output');editor.focus();getSelection().selectAllChildren(editor)").unwrap();
    assert_eq!(
        vm.eval("getSelection().getRangeAt(0).toString()").unwrap(),
        "ab"
    );
    key(&mut vm, "c", "KeyC", 2);
    vm.eval("output.focus()").unwrap();
    key(&mut vm, "v", "KeyV", 2);
    assert_eq!(vm.eval("output.value").unwrap(), "a\nb");
    vm.eval("editor.focus();const r=document.createRange();r.setStart(editor,0);r.setEnd(editor,1);getSelection().removeAllRanges();getSelection().addRange(r)").unwrap();
    key(&mut vm, "c", "KeyC", 2);
    vm.eval("output.value='';output.focus()").unwrap();
    key(&mut vm, "v", "KeyV", 2);
    assert_eq!(vm.eval("output.value").unwrap(), "a");
}

#[test]
fn editing_shortcuts_paste_plain_line_breaks_as_nodes_and_restore_history() {
    for contents in ["first\nsecond", "<b>&\r\nlast"] {
        let mut vm = new_parsed_test_vm(
            "https://editing-paste-break.test/",
            "<textarea id=source></textarea><div id=editor contenteditable><b>keep</b></div>",
        );
        vm.eval(&format!("globalThis.source=document.getElementById('source');globalThis.editor=document.getElementById('editor');source.value={};source.focus();source.select()", serde_json::to_string(contents).unwrap())).unwrap();
        key(&mut vm, "c", "KeyC", 2);
        vm.eval("globalThis.original=editor.firstChild;editor.focus();getSelection().selectAllChildren(editor)").unwrap();
        key(&mut vm, "v", "KeyV", 2);
        assert_eq!(
            vm.eval("editor.querySelectorAll('br').length").unwrap(),
            "1"
        );
        assert_eq!(
            vm.eval("editor.textContent").unwrap(),
            contents.replace("\r\n", "\n").replace('\n', "")
        );
        key(&mut vm, "z", "KeyZ", 2);
        assert_eq!(
            vm.eval("editor.innerHTML==='<b>keep</b>' && editor.firstChild===original")
                .unwrap(),
            "true"
        );
        key(&mut vm, "z", "KeyZ", 10);
        assert_eq!(
            vm.eval("editor.querySelectorAll('br').length").unwrap(),
            "1"
        );
    }
}

#[test]
fn editing_shortcuts_multiline_paste_undo_restores_a_split_text_node() {
    let mut vm = new_parsed_test_vm(
        "https://editing-split-paste.test/",
        "<textarea id=source>first\nsecond</textarea><div id=editor contenteditable>abcd</div>",
    );
    vm.eval("globalThis.source=document.getElementById('source');globalThis.editor=document.getElementById('editor');globalThis.original=editor.firstChild;source.focus();source.select()").unwrap();
    key(&mut vm, "c", "KeyC", 2);
    vm.eval("editor.focus();getSelection().collapse(original,2)")
        .unwrap();
    key(&mut vm, "v", "KeyV", 2);
    assert_eq!(vm.eval("editor.textContent").unwrap(), "abfirstsecondcd");
    key(&mut vm, "z", "KeyZ", 2);
    assert_eq!(vm.eval("editor.textContent==='abcd' && editor.childNodes.length===1 && editor.firstChild===original").unwrap(), "true");
}

#[test]
fn editing_shortcuts_paste_retains_text_newlines_in_plaintext_and_preformatted_hosts() {
    for attributes in [
        "contenteditable=plaintext-only",
        "contenteditable style='white-space:pre'",
        "contenteditable style='white-space:pre-wrap'",
    ] {
        let mut vm = new_parsed_test_vm(
            "https://editing-pre-paste.test/",
            &format!(
                "<textarea id=source>first\nsecond</textarea><div id=editor {attributes}></div>"
            ),
        );
        vm.eval("globalThis.source=document.getElementById('source');globalThis.editor=document.getElementById('editor');source.focus();source.select()").unwrap();
        key(&mut vm, "c", "KeyC", 2);
        vm.eval("editor.focus()").unwrap();
        key(&mut vm, "v", "KeyV", 2);
        assert_eq!(vm.eval("editor.textContent").unwrap(), "first\nsecond");
        assert_eq!(
            vm.eval("editor.querySelector('br')===null").unwrap(),
            "true"
        );
    }
}
