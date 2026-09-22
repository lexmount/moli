use super::*;

fn copy_command_vm(tag: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm("https://clipboard-command.test/");
    vm.eval(&r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.control = document.createElement('$TAG');
document.body.appendChild(control); control.value = 'A🦊BC';control.focus();control.setSelectionRange(1,3);
globalThis.events=[];globalThis.savedTransfer=null;
for(const type of ['beforecopy','copy'])document.addEventListener(type,e=>{
  savedTransfer=e.clipboardData;
  events.push([type,e.target===control,e.constructor.name,e.isTrusted,e.bubbles,e.cancelable,e.composed,Array.from(e.clipboardData.types),e.clipboardData.getData('text/plain')]);
});
navigator.clipboard.writeText('seed');'ready'
"#.replace("$TAG",tag)).unwrap();
    vm
}

fn activated_copy_eval(vm: &mut StandaloneScriptVmHarness, script: &str) -> String {
    vm._context_host
        .borrow_mut()
        .begin_protocol_user_gesture_activation();
    let result = vm.eval(script);
    vm._context_host
        .borrow_mut()
        .end_protocol_user_gesture_activation();
    result.expect("activated copy command")
}

fn copied_text(vm: &mut ScriptVm) -> String {
    vm.eval("navigator.clipboard.readText().then(value=>globalThis.copiedText=value);'read'")
        .unwrap();
    vm.eval("copiedText").unwrap()
}

#[test]
fn exec_copy_commits_contents_and_presentation_style_together() {
    for (listener, text, style) in [
        ("", "🦊", "unspecified"),
        (
            "document.addEventListener('copy', e => {e.clipboardData.setData('text/plain', 'author');e.preventDefault()})",
            "author",
            "unspecified",
        ),
        (
            "document.addEventListener('copy', e => e.preventDefault())",
            "seed",
            "inline",
        ),
    ] {
        let mut vm = copy_command_vm("input");
        vm.eval("navigator.clipboard.write([new ClipboardItem({'text/plain': 'seed'}, {presentationStyle: 'inline'})])")
            .unwrap();
        vm.eval(listener).unwrap();
        assert_eq!(
            activated_copy_eval(&mut vm, "document.execCommand('copy')"),
            "true"
        );
        assert_eq!(copied_text(&mut vm), text, "{listener}");
        vm.eval("navigator.clipboard.read().then(([item]) => globalThis.clipboardStyle = item.presentationStyle)")
            .unwrap();
        assert_eq!(vm.eval("clipboardStyle").unwrap(), style, "{listener}");
    }
}

#[test]
fn exec_copy_dispatches_trusted_events_and_copies_text_control_selections() {
    for tag in ["input", "textarea"] {
        let mut vm = copy_command_vm(tag);
        assert_eq!(
            activated_copy_eval(
                &mut vm,
                "JSON.stringify([document.queryCommandSupported('copy'),document.queryCommandEnabled('copy'),document.execCommand('copy')])"
            ),
            "[true,true,true]"
        );
        assert_eq!(copied_text(&mut vm), "🦊");
        let events: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(events)").unwrap()).unwrap();
        let expected = |name| {
            serde_json::json!([name, true, "ClipboardEvent", true, true, true, true, [], ""])
        };
        assert_eq!(
            events,
            serde_json::json!([
                expected("beforecopy"),
                expected("beforecopy"),
                expected("copy")
            ])
        );
        assert_eq!(vm.eval("JSON.stringify([savedTransfer.types.length,savedTransfer.items.length,savedTransfer.getData('text/plain'),control.value])").unwrap(),r#"[0,0,"","A🦊BC"]"#);
    }
}

#[test]
fn exec_copy_requires_activation_and_checks_the_receiver_document() {
    let mut vm = copy_command_vm("input");
    assert_eq!(vm.eval("JSON.stringify([document.queryCommandSupported('copy'),document.queryCommandEnabled('copy'),document.execCommand('copy'),events.length])").unwrap(),"[true,false,false,0]");
    assert_eq!(copied_text(&mut vm), "seed");
    assert_eq!(
        activated_copy_eval(
            &mut vm,
            r#"(() => {
const detached=document.implementation.createHTMLDocument('');
const xml=new DOMParser().parseFromString('<root/>','application/xml');
let error;try{xml.execCommand('copy')}catch(e){error=e.name}
return JSON.stringify([detached.queryCommandSupported('copy'),detached.queryCommandEnabled('copy'),detached.execCommand('copy'),error]);
})()"#
        ),
        r#"[false,false,false,"InvalidStateError"]"#
    );
    assert_eq!(copied_text(&mut vm), "seed");
}

#[test]
fn exec_copy_preserves_unfocused_selections_and_respects_passwords_and_empty_selections() {
    for (setup, enabled, clipboard, event_count) in [
        ("control.readOnly=true", true, "🦊", 3),
        ("control.disabled=true", true, "🦊", 3),
        ("control.blur()", true, "🦊", 3),
        ("control.blur();control.select()", true, "A🦊BC", 3),
        ("control.type='password'", false, "seed", 0),
        ("control.setSelectionRange(1,1)", false, "seed", 3),
        ("getSelection().removeAllRanges()", false, "seed", 3),
    ] {
        let mut vm = copy_command_vm("input");
        vm.eval(setup).unwrap();
        assert_eq!(
            activated_copy_eval(
                &mut vm,
                "JSON.stringify([document.queryCommandEnabled('copy'),document.execCommand('copy')])"
            ),
            format!("[{enabled},true]"),
            "{setup}"
        );
        assert_eq!(copied_text(&mut vm), clipboard, "{setup}");
        assert_eq!(
            vm.eval("events.length").unwrap(),
            event_count.to_string(),
            "{setup}"
        );
    }
}

#[test]
fn exec_copy_honors_event_cancellation_and_selection_changes() {
    for (listener, clipboard) in [
        (
            "document.addEventListener('copy',e=>{e.clipboardData.setData('text/plain','author');e.preventDefault()})",
            "author",
        ),
        (
            "document.addEventListener('copy',e=>e.preventDefault())",
            "seed",
        ),
        (
            "document.addEventListener('copy',e=>e.clipboardData.setData('text/plain','ignored'))",
            "🦊",
        ),
        (
            "document.addEventListener('copy',()=>control.setSelectionRange(3,5))",
            "BC",
        ),
        (
            "document.addEventListener('copy',()=>control.remove())",
            "seed",
        ),
    ] {
        let mut vm = copy_command_vm("input");
        vm.eval(listener).unwrap();
        assert_eq!(
            activated_copy_eval(&mut vm, "document.execCommand('copy')"),
            "true"
        );
        assert_eq!(copied_text(&mut vm), clipboard, "{listener}");
    }
    let mut vm = copy_command_vm("input");
    vm.eval("control.setSelectionRange(1,1);document.addEventListener('beforecopy',e=>{e.clipboardData.setData('text/plain','before');e.preventDefault()})").unwrap();
    assert_eq!(
        activated_copy_eval(
            &mut vm,
            "JSON.stringify([document.queryCommandEnabled('copy'),document.execCommand('copy')])"
        ),
        "[true,true]"
    );
    assert_eq!(copied_text(&mut vm), "before");
}

#[test]
fn exec_copy_rejects_nested_commands_and_uses_native_event_constructors_and_values() {
    let mut vm = copy_command_vm("input");
    vm.eval(r#"
globalThis.nested=[];
document.addEventListener('copy',()=>nested.push(document.execCommand('copy'),document.execCommand('insertText',false,'nested')));
globalThis.ClipboardEvent=function(){throw Error('author constructor')};
Object.defineProperty(control,'value',{get(){throw Error('author value')}});
Object.defineProperty(control,'selectionStart',{get(){throw Error('author selectionStart')}});
'installed'
"#).unwrap();
    for _ in 0..2 {
        assert_eq!(
            activated_copy_eval(&mut vm, "document.execCommand('copy')"),
            "true"
        )
    }
    assert_eq!(
        vm.eval("JSON.stringify(nested)").unwrap(),
        "[false,false,false,false]"
    );
    assert_eq!(copied_text(&mut vm), "🦊");
}

#[test]
fn exec_copy_uses_the_document_dom_selection_without_author_getters() {
    let mut vm = copy_command_vm("input");
    vm.eval(
        r#"
const text=document.createElement('div');text.textContent='A🦊BC';document.body.appendChild(text);
getSelection().selectAllChildren(text);
globalThis.getSelection=()=>{throw Error('author Selection')};
Selection.prototype.toString=()=>{throw Error('author Selection text')};
Range.prototype.toString=()=>{throw Error('author Range text')};
'installed'
"#,
    )
    .unwrap();
    assert_eq!(
        activated_copy_eval(&mut vm, "document.execCommand('copy')"),
        "true"
    );
    assert_eq!(copied_text(&mut vm), "A🦊BC");
}

#[test]
fn exec_copy_keeps_parent_and_child_document_selections_separate() {
    let mut vm = copy_command_vm("input");
    vm.eval(r#"
globalThis.iframe=document.createElement('iframe');document.body.appendChild(iframe);
globalThis.childDocument=iframe.contentDocument;
const child=childDocument.createElement('input');child.value='child';childDocument.body.appendChild(child);child.focus();child.setSelectionRange(1,4);
globalThis.childCopies=0;childDocument.addEventListener('copy',e=>{if(e.isTrusted && e instanceof iframe.contentWindow.ClipboardEvent)childCopies++});
'installed'
"#).unwrap();
    assert_eq!(
        activated_copy_eval(&mut vm, "childDocument.execCommand('copy')"),
        "true"
    );
    assert_eq!(copied_text(&mut vm), "hil");
    assert_eq!(vm.eval("childCopies").unwrap(), "1");
    assert_eq!(
        activated_copy_eval(&mut vm, "document.execCommand('copy')"),
        "true"
    );
    assert_eq!(copied_text(&mut vm), "🦊");
    vm.eval("iframe.remove()").unwrap();
    assert_eq!(
        activated_copy_eval(
            &mut vm,
            "JSON.stringify([childDocument.queryCommandSupported('copy'),childDocument.execCommand('copy')])"
        ),
        "[false,false]"
    );
    assert_eq!(copied_text(&mut vm), "🦊");
}

#[test]
fn exec_copy_preserves_preformatted_text_and_collapses_normal_whitespace() {
    for (white_space, expected) in [
        ("normal", "First Second"),
        ("pre", "First\r\nSecond"),
        ("pre-wrap", "First\r\nSecond"),
    ] {
        let mut vm = copy_command_vm("input");
        vm.eval(&format!(r#"
const text=document.createElement('div');document.body.appendChild(text);
text.style.whiteSpace='{white_space}';text.textContent='First\r\nSecond';getSelection().selectAllChildren(text);
'installed'
"#)).unwrap();
        assert_eq!(
            activated_copy_eval(&mut vm, "document.execCommand('copy')"),
            "true"
        );
        assert_eq!(copied_text(&mut vm), expected, "{white_space}");
    }
}

#[test]
fn native_copy_normalizes_lf_without_rewriting_lone_cr_in_preformatted_text() {
    for (text, expected) in [
        ("a\nb", "a\r\nb"),
        ("a\rb", "a\rb"),
        ("a\r\nb", "a\r\nb"),
        ("a\n\rb", "a\r\n\rb"),
    ] {
        let mut vm = copy_command_vm("input");
        let text = serde_json::to_string(text).unwrap();
        vm.eval(&format!(r#"const pre=document.createElement('pre');pre.textContent={text};document.body.appendChild(pre);getSelection().selectAllChildren(pre);'ready'"#)).unwrap();
        assert_eq!(
            activated_copy_eval(&mut vm, "document.execCommand('copy')"),
            "true"
        );
        assert_eq!(copied_text(&mut vm), expected);
    }
}
