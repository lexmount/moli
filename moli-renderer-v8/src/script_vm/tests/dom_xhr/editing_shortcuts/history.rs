use super::*;

#[test]
fn editing_shortcuts_continuous_control_typing_includes_the_first_character() {
    for markup in ["<input id=a>", "<textarea id=a></textarea>"] {
        let mut vm = new_parsed_test_vm("https://editing-continuous.test/", markup);
        vm.eval("document.getElementById('a').focus()").unwrap();
        text(&mut vm, "first");
        key(&mut vm, "z", "KeyZ", 2);
        assert_eq!(vm.eval("document.getElementById('a').value").unwrap(), "");
    }
}

#[test]
fn editing_shortcuts_reentrant_same_control_typing_remains_a_single_group() {
    let mut vm = new_parsed_test_vm(
        "https://editing-same-control-reentrant.test/",
        "<textarea id=a></textarea>",
    );
    vm.eval("globalThis.a=document.getElementById('a');a.focus()")
        .unwrap();
    text(&mut vm, "first");
    vm.eval("a.addEventListener('beforeinput',function once(e){if(e.inputType!=='historyUndo')return;a.removeEventListener('beforeinput',once);document.execCommand('insertText',false,'second')})").unwrap();
    key(&mut vm, "z", "KeyZ", 2);
    assert_eq!(vm.eval("a.value").unwrap(), "");
    key(&mut vm, "z", "KeyZ", 10);
    assert_eq!(vm.eval("a.value").unwrap(), "firstsecond");
}

#[test]
fn editing_shortcuts_typing_groups_end_on_focus_and_selection_round_trips() {
    for markup in [
        "<input id=a>",
        "<textarea id=a></textarea>",
        "<div id=a contenteditable></div>",
    ] {
        for movement in ["focus", "arrows", "selection-api"] {
            let mut vm = new_parsed_test_vm(
                "https://editing-typing-groups.test/",
                &format!("{markup}<input id=b>"),
            );
            vm.eval("globalThis.a=document.getElementById('a');globalThis.b=document.getElementById('b');a.focus()").unwrap();
            text(&mut vm, "abc");
            match movement {
                "focus" => {
                    vm.eval("b.focus();a.focus()").unwrap();
                }
                "arrows" => {
                    key(&mut vm, "ArrowLeft", "ArrowLeft", 0);
                    key(&mut vm, "ArrowRight", "ArrowRight", 0);
                }
                _ => {
                    vm.eval("if(a.setSelectionRange){a.setSelectionRange(2,2);a.setSelectionRange(3,3)}else{getSelection().collapse(a.firstChild,2);getSelection().collapse(a.firstChild,3)}").unwrap();
                }
            }
            text(&mut vm, "d");
            key(&mut vm, "z", "KeyZ", 2);
            assert_eq!(
                vm.eval("a.value??a.textContent").unwrap(),
                "abc",
                "{markup}, {movement}"
            );
        }
    }
}

#[test]
fn editing_shortcuts_undo_reads_history_created_inside_beforeinput() {
    let mut vm = new_parsed_test_vm(
        "https://editing-history-reentrant.test/",
        "<textarea id=a></textarea><textarea id=b></textarea>",
    );
    vm.eval("globalThis.a=document.getElementById('a');globalThis.b=document.getElementById('b');a.focus()").unwrap();
    text(&mut vm, "first");
    vm.eval("a.addEventListener('beforeinput',function once(e){if(e.inputType!=='historyUndo')return;a.removeEventListener('beforeinput',once);b.focus();document.execCommand('insertText',false,'second');a.focus()});globalThis.targets=[];document.addEventListener('input',e=>{if(e.inputType==='historyUndo')targets.push(e.target.id)})").unwrap();
    key(&mut vm, "z", "KeyZ", 2);
    assert_eq!(
        vm.eval("a.value+':'+b.value+':'+document.activeElement.id")
            .unwrap(),
        "first::b"
    );
    key(&mut vm, "z", "KeyZ", 2);
    assert_eq!(
        vm.eval("a.value+':'+b.value+':'+targets.join(',')")
            .unwrap(),
        "::b,a"
    );
    key(&mut vm, "z", "KeyZ", 10);
    assert_eq!(vm.eval("a.value+':'+b.value").unwrap(), "first:");
    key(&mut vm, "z", "KeyZ", 10);
    assert_eq!(vm.eval("a.value+':'+b.value").unwrap(), "first:second");
}

#[test]
fn editing_shortcuts_history_preserves_unrelated_connected_iframe_and_custom_element() {
    let mut vm = new_parsed_test_vm(
        "https://editing-iframe.test/",
        "<div id=editor contenteditable><iframe id=frame></iframe><x-sibling></x-sibling><b>abc</b><i>def</i></div>",
    );
    vm.eval(
        r#"
        globalThis.editor=document.getElementById('editor');
        globalThis.frame=document.getElementById('frame');
        globalThis.win=frame.contentWindow;
        globalThis.doc=frame.contentDocument;
        win.marker=42;
        globalThis.disconnects=0;
        customElements.define('x-sibling', class extends HTMLElement {
            disconnectedCallback() { disconnects++; }
        });
        editor.focus();
        const range=document.createRange();
        range.selectNode(editor.querySelector('b'));
        getSelection().removeAllRanges();getSelection().addRange(range);
    "#,
    )
    .unwrap();
    text(&mut vm, "X");
    for (modifiers, expected) in [(2, "abcdef"), (10, "Xdef")] {
        key(&mut vm, "z", "KeyZ", modifiers);
        assert_eq!(vm.eval("editor.textContent").unwrap(), expected);
        assert_eq!(vm.eval("frame.contentWindow===win && frame.contentDocument===doc && win.marker===42 && disconnects===0").unwrap(), "true");
    }
}

#[test]
fn editing_shortcuts_history_uses_document_stack_and_skips_removed_targets() {
    for removed in [false, true] {
        let mut vm = new_parsed_test_vm(
            "https://editing-history-controls.test/",
            "<input id=a><input id=b>",
        );
        vm.eval("globalThis.a=document.getElementById('a');globalThis.b=document.getElementById('b');a.focus()")
            .unwrap();
        text(&mut vm, "A");
        vm.eval("b.focus()").unwrap();
        text(&mut vm, "B");
        if removed {
            vm.eval("b.remove()").unwrap();
        }
        vm.eval("a.focus();globalThis.events=[];for(const kind of ['focus','beforeinput','input'])document.addEventListener(kind,e=>events.push([kind,e.target.id,document.activeElement.id]),true)").unwrap();
        key(&mut vm, "z", "KeyZ", 2);
        assert_eq!(
            vm.eval("JSON.stringify([a.value,b.value,document.activeElement.id,events])")
                .unwrap(),
            if removed {
                r#"["","B","a",[["beforeinput","a","a"],["input","a","a"]]]"#
            } else {
                r#"["A","","b",[["beforeinput","b","a"],["focus","b","b"],["input","b","b"]]]"#
            },
        );
        key(&mut vm, "z", "KeyZ", 10);
        assert_eq!(vm.eval("a.value+':'+b.value").unwrap(), "A:B");
    }
}

#[test]
fn editing_shortcuts_history_callbacks_use_current_stack_and_document_ownership() {
    for (kind, action) in [
        ("beforeinput", "b.remove()"),
        ("focus", "b.remove()"),
        (
            "beforeinput",
            "document.implementation.createHTMLDocument('').body.appendChild(b)",
        ),
    ] {
        let mut vm = new_parsed_test_vm(
            "https://editing-history-callback.test/",
            "<input id=a><input id=b>",
        );
        vm.eval("globalThis.a=document.getElementById('a');globalThis.b=document.getElementById('b');a.focus()").unwrap();
        text(&mut vm, "A");
        vm.eval("b.focus()").unwrap();
        text(&mut vm, "B");
        vm.eval(&format!(
            "a.focus();b.addEventListener('{kind}',()=>{action})"
        ))
        .unwrap();
        key(&mut vm, "z", "KeyZ", 2);
        assert_eq!(
            vm.eval("a.value+':'+b.value").unwrap(),
            if kind == "focus" { "A:" } else { ":B" },
            "{kind}"
        );
        vm.eval("a.focus()").unwrap();
        key(&mut vm, "z", "KeyZ", 2);
        assert_eq!(vm.eval("a.value").unwrap(), "");
    }
}

#[test]
fn editing_shortcuts_history_does_not_rewrite_or_steal_moved_nodes() {
    for move_node in [
        "outside.appendChild(editor.firstChild)",
        "detached.appendChild(editor.firstChild)",
    ] {
        let mut vm = new_parsed_test_vm(
            "https://editing-history-moved.test/",
            "<div id=editor contenteditable>abc</div><div id=outside></div>",
        );
        vm.eval("globalThis.editor=document.getElementById('editor');globalThis.outside=document.getElementById('outside');globalThis.detached=document.createElement('div');editor.focus();getSelection().collapse(editor.firstChild,2)").unwrap();
        text(&mut vm, "X");
        vm.eval(move_node).unwrap();
        vm.eval("editor.focus()").unwrap();
        key(&mut vm, "z", "KeyZ", 2);
        assert_eq!(
            vm.eval("outside.textContent+detached.textContent").unwrap(),
            "abXc"
        );
    }
    let mut vm = new_parsed_test_vm(
        "https://editing-history-reparent.test/",
        "<div id=editor contenteditable><b id=bold>abc</b></div><div id=outside></div>",
    );
    vm.eval("globalThis.editor=document.getElementById('editor');globalThis.bold=document.getElementById('bold');editor.focus();getSelection().selectAllChildren(editor)").unwrap();
    text(&mut vm, "X");
    vm.eval("document.getElementById('outside').appendChild(bold)")
        .unwrap();
    key(&mut vm, "z", "KeyZ", 2);
    assert_eq!(
        vm.eval("editor.textContent+':'+bold.parentNode.id")
            .unwrap(),
        "X:outside"
    );
}
