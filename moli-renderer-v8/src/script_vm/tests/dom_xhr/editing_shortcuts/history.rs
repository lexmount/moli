use super::*;

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
fn editing_shortcuts_history_revalidates_target_after_beforeinput_and_focus() {
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
        assert_eq!(vm.eval("a.value+':'+b.value").unwrap(), "A:B", "{kind}");
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
