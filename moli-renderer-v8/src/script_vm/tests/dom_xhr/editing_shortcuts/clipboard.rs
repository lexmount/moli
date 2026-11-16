use super::*;

#[test]
fn editing_shortcuts_cut_reads_selection_and_editability_after_callbacks() {
    let mut vm = new_parsed_test_vm(
        "https://editing-cut-callback.test/",
        "<input id=field value=ABCDEF><textarea id=output></textarea>",
    );
    vm.eval("globalThis.field=document.getElementById('field');globalThis.output=document.getElementById('output');field.focus();field.setSelectionRange(0,2);field.oncut=()=>field.setSelectionRange(2,4)").unwrap();
    key(&mut vm, "x", "KeyX", 2);
    assert_eq!(vm.eval("field.value").unwrap(), "ABEF");
    vm.eval("output.focus()").unwrap();
    key(&mut vm, "v", "KeyV", 2);
    assert_eq!(vm.eval("output.value").unwrap(), "CD");
    for readonly in ["field.readOnly=true", "field.oncut=()=>field.readOnly=true"] {
        vm.eval("field.value='ABC';field.readOnly=false;field.oncut=null;field.focus();field.setSelectionRange(0,2)").unwrap();
        vm.eval(readonly).unwrap();
        key(&mut vm, "x", "KeyX", 2);
        assert_eq!(vm.eval("field.value").unwrap(), "ABC");
        vm.eval("output.value='';output.focus()").unwrap();
        key(&mut vm, "v", "KeyV", 2);
        assert_eq!(vm.eval("output.value").unwrap(), "CD", "{readonly}");
    }
}

#[test]
fn editing_shortcuts_target_nested_controls_and_focus_after_keydown() {
    let mut vm = new_parsed_test_vm(
        "https://editing-shortcut-target.test/",
        "<div contenteditable><input id=a value=ABC></div><input id=b value=DEF>",
    );
    vm.eval("globalThis.a=document.getElementById('a');globalThis.b=document.getElementById('b');globalThis.events=[];for(const kind of ['cut','paste','beforeinput','input'])document.addEventListener(kind,e=>events.push(e.target.id));a.focus();a.setSelectionRange(0,2)").unwrap();
    key(&mut vm, "x", "KeyX", 2);
    assert_eq!(vm.eval("a.value+':'+events.join(',')").unwrap(), "C:a,a,a");
    key(&mut vm, "v", "KeyV", 2);
    assert_eq!(
        vm.eval("a.value+':'+events.slice(-3).join(',')").unwrap(),
        "ABC:a,a,a"
    );
    vm.eval("a.focus();a.setSelectionRange(0,2);a.onkeydown=e=>{if(e.key==='x'){b.focus();b.setSelectionRange(0,2)}}").unwrap();
    key(&mut vm, "x", "KeyX", 2);
    assert_eq!(
        vm.eval("a.value+':'+b.value+':'+events.slice(-3).join(',')")
            .unwrap(),
        "ABC:F:b,b,b"
    );
}

#[test]
fn editing_shortcuts_empty_clipboard_preserves_rich_selection_but_clears_controls() {
    for custom in [false, true] {
        for control in [false, true] {
            let markup = if control {
                "<textarea id=editor>keep</textarea><input id=source value=seed>"
            } else {
                "<div id=editor contenteditable><b>keep</b></div><input id=source value=seed>"
            };
            let mut vm = new_parsed_test_vm("https://editing-empty-paste.test/", markup);
            vm.eval("globalThis.editor=document.getElementById('editor');globalThis.source=document.getElementById('source');source.focus();source.select()").unwrap();
            vm.eval(if custom {
                "source.oncopy=e=>{e.clipboardData.setData('application/x-test','custom');e.preventDefault()}"
            } else {
                "source.oncopy=e=>{e.clipboardData.setData('text/plain','');e.preventDefault()}"
            }).unwrap();
            key(&mut vm, "c", "KeyC", 2);
            vm.eval(if control {
                "editor.focus();editor.select()"
            } else {
                "editor.focus();getSelection().selectAllChildren(editor)"
            })
            .unwrap();
            key(&mut vm, "v", "KeyV", 2);
            assert_eq!(
                vm.eval("editor.value??editor.innerHTML").unwrap(),
                if control { "" } else { "<b>keep</b>" },
                "custom={custom}, control={control}",
            );
        }
    }
}
