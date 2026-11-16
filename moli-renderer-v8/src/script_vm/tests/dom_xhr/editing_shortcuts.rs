use super::*;
fn key(vm: &mut StandaloneScriptVmHarness, key: &str, code: &str, modifiers: u8) {
    vm.dispatch_key_event("keydown", key, code, "", modifiers, false, false)
        .unwrap();
}
fn text(vm: &mut StandaloneScriptVmHarness, text: &str) {
    for ch in text.chars() {
        let value = ch.to_string();
        vm.dispatch_key_event("keydown", &value, "KeyA", &value, 0, false, true)
            .unwrap();
    }
}

#[test]
fn editing_shortcuts_rehome_removed_editable_selection_before_next_character() {
    for removal in [
        "editor.textContent=''",
        "editor.innerHTML=''",
        "editor.replaceChildren()",
        "editor.firstChild.remove()",
    ] {
        for select_all in [false, true] {
            let mut vm = new_parsed_test_vm(
                "https://editing-removal.test/",
                "<div id=editor contenteditable></div>",
            );
            vm.eval("globalThis.editor=document.getElementById('editor');editor.focus()")
                .unwrap();
            text(&mut vm, "a");
            if select_all {
                vm.eval("getSelection().selectAllChildren(editor.firstChild)")
                    .unwrap();
            }
            vm.eval(removal).unwrap();
            assert_eq!(
                vm.eval("getSelection().anchorNode===editor && getSelection().focusNode===editor && getSelection().isCollapsed").unwrap(),
                "true",
                "{removal}, select_all={select_all}: {}", vm.eval("JSON.stringify([getSelection().anchorNode?.nodeName,getSelection().anchorOffset,getSelection().focusNode?.nodeName,getSelection().focusOffset,getSelection().isCollapsed,getSelection().rangeCount])").unwrap(),
            );
            vm.dispatch_key_event("keydown", "A", "KeyA", "A", 8, false, true)
                .unwrap();
            assert_eq!(vm.eval("editor.textContent").unwrap(), "A", "{removal}");
        }
    }
}

#[test]
fn editing_shortcuts_undo_redo_group_typing_and_preserve_node_identity() {
    let mut vm = new_parsed_test_vm(
        "https://editing-shortcuts.test/",
        "<div id=editor contenteditable></div>",
    );
    vm.eval("globalThis.editor=document.getElementById('editor');editor.focus();globalThis.historyEvents=[];editor.addEventListener('input',e=>historyEvents.push([e.inputType,e.isTrusted]));").unwrap();
    text(&mut vm, "abc");
    vm.eval("globalThis.typed=editor.firstChild;typed.marker=7;editor.addEventListener('beforeinput',globalThis.cancelUndo=e=>{if(e.inputType==='historyUndo')e.preventDefault()});").unwrap();
    key(&mut vm, "z", "KeyZ", 2);
    assert_eq!(vm.eval("editor.textContent").unwrap(), "abc");
    vm.eval("editor.removeEventListener('beforeinput',cancelUndo)")
        .unwrap();
    key(&mut vm, "z", "KeyZ", 2);
    assert_eq!(vm.eval("editor.textContent").unwrap(), "");
    key(&mut vm, "z", "KeyZ", 10);
    assert_eq!(
        vm.eval("editor.textContent==='abc' && editor.firstChild===typed && typed.marker===7")
            .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("JSON.stringify(historyEvents.slice(-2))").unwrap(),
        r#"[["historyUndo",true],["historyRedo",true]]"#
    );
    key(&mut vm, "z", "KeyZ", 2);
    text(&mut vm, "new");
    key(&mut vm, "z", "KeyZ", 10);
    assert_eq!(vm.eval("editor.textContent").unwrap(), "new");
}

#[test]
fn editing_shortcuts_restore_rich_nodes_and_text_control_selection() {
    let mut vm = new_parsed_test_vm(
        "https://editing-rich.test/",
        "<div id=editor contenteditable><b id=bold>abc</b><i>tail</i></div><textarea id=control>abcdef</textarea>",
    );
    vm.eval("globalThis.editor=document.getElementById('editor');globalThis.bold=document.getElementById('bold');globalThis.original=bold.firstChild;original.marker=99;editor.focus();getSelection().selectAllChildren(editor);").unwrap();
    text(&mut vm, "X");
    key(&mut vm, "z", "KeyZ", 2);
    assert_eq!(vm.eval("editor.innerHTML==='<'+'b id=\"bold\">abc</b><i>tail</i>' && document.getElementById('bold')===bold && bold.firstChild===original && original.marker===99").unwrap(),"true");
    vm.eval("globalThis.control=document.getElementById('control');control.focus();control.setSelectionRange(1,3)").unwrap();
    text(&mut vm, "X");
    key(&mut vm, "z", "KeyZ", 2);
    assert_eq!(
        vm.eval("control.value+':'+control.selectionStart+':'+control.selectionEnd")
            .unwrap(),
        "abcdef:1:3"
    );
}

#[test]
fn editing_shortcuts_numpad_delete_uses_live_selection_and_utf16_boundaries() {
    let mut vm = new_parsed_test_vm(
        "https://editing-delete.test/",
        "<div id=editor contenteditable>abcdef</div>",
    );
    vm.eval("globalThis.editor=document.getElementById('editor');editor.focus();getSelection().collapse(editor.firstChild,0)").unwrap();
    for _ in 0..3 {
        key(&mut vm, "Delete", "NumpadDecimal", 0);
    }
    assert_eq!(vm.eval("editor.textContent").unwrap(), "def");
    vm.eval("editor.textContent='a😀b';getSelection().collapse(editor.firstChild,1)")
        .unwrap();
    key(&mut vm, "Delete", "NumpadDecimal", 0);
    assert_eq!(vm.eval("editor.textContent").unwrap(), "ab");
    vm.eval("editor.addEventListener('beforeinput',e=>e.preventDefault())")
        .unwrap();
    key(&mut vm, "Delete", "NumpadDecimal", 0);
    assert_eq!(vm.eval("editor.textContent").unwrap(), "ab");
}

#[test]
fn editing_shortcuts_clipboard_transfers_and_honors_native_event_cancellation() {
    let mut vm = new_parsed_test_vm(
        "https://editing-clipboard.test/",
        "<div id=editor contenteditable>copy</div><textarea id=control></textarea>",
    );
    vm.eval("globalThis.editor=document.getElementById('editor');globalThis.control=document.getElementById('control');globalThis.events=[];for(const target of [editor,control])for(const type of ['copy','cut','paste'])target.addEventListener(type,e=>events.push([e.type,e.isTrusted,e.clipboardData instanceof DataTransfer]));editor.focus();").unwrap();
    key(&mut vm, "a", "KeyA", 2);
    key(&mut vm, "c", "KeyC", 2);
    vm.eval("control.focus()").unwrap();
    key(&mut vm, "v", "KeyV", 2);
    assert_eq!(vm.eval("control.value").unwrap(), "copy");
    key(&mut vm, "a", "KeyA", 2);
    key(&mut vm, "x", "KeyX", 2);
    assert_eq!(vm.eval("control.value").unwrap(), "");
    vm.eval("editor.focus();getSelection().selectAllChildren(editor)")
        .unwrap();
    key(&mut vm, "v", "KeyV", 2);
    assert_eq!(vm.eval("editor.textContent").unwrap(), "copy");
    vm.eval("editor.addEventListener('copy',e=>{e.clipboardData.setData('text/plain','custom');e.preventDefault()});editor.focus();").unwrap();
    key(&mut vm, "a", "KeyA", 2);
    key(&mut vm, "c", "KeyC", 2);
    vm.eval("control.focus()").unwrap();
    key(&mut vm, "v", "KeyV", 2);
    assert_eq!(vm.eval("control.value").unwrap(), "custom");
    vm.eval("control.addEventListener('paste',e=>e.preventDefault())")
        .unwrap();
    key(&mut vm, "v", "KeyV", 2);
    assert_eq!(vm.eval("control.value").unwrap(), "custom");
    assert_eq!(vm.eval("events.every(e=>e[1]&&e[2])").unwrap(), "true");
}

#[test]
fn editing_shortcuts_keep_author_changes_and_read_only_subtrees() {
    let mut vm = new_parsed_test_vm(
        "https://editing-shortcuts.test/",
        "<div id=editor contenteditable><span id=locked contenteditable=false>LOCKED</span><span id=ordinary>abc</span></div>",
    );
    vm.eval("globalThis.editor=document.getElementById('editor');globalThis.locked=document.getElementById('locked');editor.focus();getSelection().collapse(locked.firstChild,0)").unwrap();
    key(&mut vm, "Delete", "NumpadDecimal", 0);
    assert_eq!(vm.eval("locked.textContent").unwrap(), "LOCKED");
    vm.eval("locked.removeAttribute('contenteditable');locked.setAttribute('inert','');getSelection().collapse(locked.firstChild,0)").unwrap();
    key(&mut vm, "Delete", "NumpadDecimal", 0);
    assert_eq!(vm.eval("locked.textContent").unwrap(), "LOCKED");
    vm.eval("editor.textContent='';editor.focus();getSelection().collapse(editor,0)")
        .unwrap();
    text(&mut vm, "abc");
    vm.eval("editor.firstChild.data='AUTHOR';getSelection().collapse(editor.firstChild,6)")
        .unwrap();
    text(&mut vm, "X");
    key(&mut vm, "z", "KeyZ", 2);
    assert_eq!(vm.eval("editor.textContent").unwrap(), "AUTHOR");
    key(&mut vm, "y", "KeyY", 2);
    assert_eq!(vm.eval("editor.textContent").unwrap(), "AUTHORX");
}
