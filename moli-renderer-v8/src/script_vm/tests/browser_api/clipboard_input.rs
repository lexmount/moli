use super::*;

fn clipboard_input_vm(tag: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm("https://clipboard-input.test/");
    vm.eval(
        &r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.control = document.createElement('$TAG');
document.body.appendChild(control);
control.value = 'A🦊BC';
control.focus();
control.setSelectionRange(1, 3);
globalThis.events = [];
globalThis.savedTransfer = null;
for (const type of ['copy', 'cut', 'paste', 'beforeinput', 'input']) {
  control.addEventListener(type, event => {
    const transfer = event.clipboardData;
    if (transfer) savedTransfer = transfer;
    events.push([type, event.constructor.name, event.isTrusted,
      event.bubbles, event.cancelable, event.composed,
      event.inputType ?? null, event.data ?? null,
      transfer ? Array.from(transfer.types) : null,
      transfer ? transfer.getData('text/plain') : null, control.value]);
  });
}
navigator.clipboard.writeText('seed');
'ready'
"#
        .replace("$TAG", tag),
    )
    .expect("clipboard input fixture");
    vm
}

fn clipboard_key(vm: &mut ScriptVm, key: &str) {
    let modifier = if cfg!(target_os = "macos") { 4 } else { 2 };
    vm.dispatch_key_event(
        "keydown",
        key,
        &format!("Key{}", key.to_ascii_uppercase()),
        "",
        modifier,
        false,
        false,
    )
    .expect("native clipboard shortcut");
}

fn clipboard_input_state(vm: &mut ScriptVm) -> serde_json::Value {
    vm.eval("navigator.clipboard.readText().then(text => globalThis.clipboardText = text); 'read'")
        .expect("read native clipboard");
    serde_json::from_str(&vm.eval(r#"JSON.stringify({
      value:control.value, selection:[control.selectionStart, control.selectionEnd],
      clipboard:clipboardText, events,
      saved:savedTransfer ? [savedTransfer.types.length, savedTransfer.items.length, savedTransfer.getData('text/plain')] : null,
    })"#).expect("clipboard input state")).expect("JSON clipboard state")
}

#[test]
fn clipboard_shortcuts_copy_cut_and_paste_utf16_text_control_selections() {
    for tag in ["input", "textarea"] {
        let mut vm = clipboard_input_vm(tag);
        clipboard_key(&mut vm, "c");
        let copied = clipboard_input_state(&mut vm);
        assert_eq!(copied["value"], "A🦊BC");
        assert_eq!(copied["clipboard"], "🦊");
        assert_eq!(copied["selection"], serde_json::json!([1, 3]));
        assert_eq!(
            copied["events"],
            serde_json::json!([[
                "copy",
                "ClipboardEvent",
                true,
                true,
                true,
                true,
                null,
                null,
                [],
                "",
                "A🦊BC"
            ]])
        );
        assert_eq!(copied["saved"], serde_json::json!([0, 0, ""]));

        vm.eval("events.length = 0").unwrap();
        clipboard_key(&mut vm, "x");
        let cut = clipboard_input_state(&mut vm);
        assert_eq!(cut["value"], "ABC");
        assert_eq!(cut["clipboard"], "🦊");
        assert_eq!(cut["selection"], serde_json::json!([1, 1]));
        assert_eq!(
            cut["events"],
            serde_json::json!([
                [
                    "cut",
                    "ClipboardEvent",
                    true,
                    true,
                    true,
                    true,
                    null,
                    null,
                    [],
                    "",
                    "A🦊BC"
                ],
                [
                    "beforeinput",
                    "InputEvent",
                    true,
                    true,
                    true,
                    true,
                    "deleteByCut",
                    null,
                    null,
                    null,
                    "A🦊BC"
                ],
                [
                    "input",
                    "InputEvent",
                    true,
                    true,
                    false,
                    true,
                    "deleteByCut",
                    null,
                    null,
                    null,
                    "ABC"
                ],
            ])
        );

        vm.eval("events.length = 0").unwrap();
        clipboard_key(&mut vm, "v");
        let pasted = clipboard_input_state(&mut vm);
        assert_eq!(pasted["value"], "A🦊BC");
        assert_eq!(pasted["selection"], serde_json::json!([3, 3]));
        assert_eq!(
            pasted["events"],
            serde_json::json!([
                [
                    "paste",
                    "ClipboardEvent",
                    true,
                    true,
                    true,
                    true,
                    null,
                    null,
                    ["text/plain"],
                    "🦊",
                    "ABC"
                ],
                [
                    "beforeinput",
                    "InputEvent",
                    true,
                    true,
                    true,
                    true,
                    "insertFromPaste",
                    "🦊",
                    null,
                    null,
                    "ABC"
                ],
                [
                    "input",
                    "InputEvent",
                    true,
                    true,
                    false,
                    true,
                    "insertFromPaste",
                    "🦊",
                    null,
                    null,
                    "A🦊BC"
                ],
            ])
        );
        assert_eq!(pasted["saved"], serde_json::json!([0, 0, ""]));
    }
}

#[test]
fn clipboard_cut_uses_the_selection_at_each_event_boundary() {
    for (listener, value, clipboard, event_types) in [
        (
            "control.addEventListener('cut', e => {e.clipboardData.setData('text/plain', 'author');e.preventDefault()})",
            "A🦊BC",
            "author",
            vec!["cut"],
        ),
        (
            "control.addEventListener('beforeinput', e => e.preventDefault())",
            "A🦊BC",
            "🦊",
            vec!["cut", "beforeinput"],
        ),
        (
            "control.addEventListener('cut', () => control.setSelectionRange(3, 5))",
            "A🦊",
            "BC",
            vec!["cut", "beforeinput", "input"],
        ),
        (
            "control.addEventListener('beforeinput', () => control.setSelectionRange(3, 5))",
            "A🦊",
            "🦊",
            vec!["cut", "beforeinput", "input"],
        ),
        (
            "control.addEventListener('beforeinput', () => control.readOnly = true)",
            "A🦊BC",
            "🦊",
            vec!["cut", "beforeinput"],
        ),
    ] {
        let mut vm = clipboard_input_vm("input");
        vm.eval(listener).expect("cut listener");
        clipboard_key(&mut vm, "x");
        let state = clipboard_input_state(&mut vm);
        assert_eq!(state["value"], value, "{listener}");
        assert_eq!(state["clipboard"], clipboard, "{listener}");
        assert_eq!(
            state["events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|event| event[0].as_str().unwrap())
                .collect::<Vec<_>>(),
            event_types
        );
    }
}

#[test]
fn clipboard_paste_data_is_read_only_and_items_are_disabled_after_dispatch() {
    let mut vm = clipboard_input_vm("input");
    vm.eval(
        r#"
control.addEventListener('paste', event => {
  const data = event.clipboardData;
  globalThis.savedItem = data.items[0];
  data.setData('text/plain', 'spoof');
  data.clearData();
  globalThis.added = data.items.add('spoof', 'text/plain');
  try {data.items.remove(0)} catch(error) {globalThis.removeError = error.name}
  data.items.clear();
  data.dropEffect = 'copy'; data.effectAllowed = 'all';
  globalThis.clipboardEffects = [data.dropEffect, data.effectAllowed];
  globalThis.duringPaste = data.getData('text/plain');
});
'installed'
"#,
    )
    .unwrap();
    clipboard_key(&mut vm, "v");
    let state = clipboard_input_state(&mut vm);
    assert_eq!(state["value"], "AseedBC");
    assert_eq!(state["clipboard"], "seed");
    assert_eq!(state["saved"], serde_json::json!([0, 0, ""]));
    assert_eq!(
        vm.eval("JSON.stringify(clipboardEffects)").unwrap(),
        r#"["none","uninitialized"]"#
    );
    assert_eq!(vm.eval("JSON.stringify([duringPaste, added, removeError, savedItem.kind, savedItem.type, savedItem.getAsFile()])").unwrap(),
        r#"["seed",null,"InvalidStateError","","",null]"#);
}

#[test]
fn clipboard_shortcuts_respect_password_readonly_and_keyboard_cancellation() {
    for (setup, key, events, clipboard) in [
        ("control.type='password'", "c", 0, "seed"),
        ("control.type='password'", "x", 0, "seed"),
        ("control.readOnly=true", "x", 1, "seed"),
        ("control.readOnly=true", "v", 1, "seed"),
        ("control.readOnly=true", "c", 1, "🦊"),
        ("control.setSelectionRange(1,1)", "x", 1, "seed"),
        (
            "control.addEventListener('keydown', e=>e.preventDefault())",
            "v",
            0,
            "seed",
        ),
    ] {
        let mut vm = clipboard_input_vm("input");
        vm.eval(setup).unwrap();
        clipboard_key(&mut vm, key);
        let state = clipboard_input_state(&mut vm);
        assert_eq!(state["value"], "A🦊BC", "{setup}");
        assert_eq!(state["clipboard"], clipboard, "{setup}");
        assert_eq!(state["events"].as_array().unwrap().len(), events, "{setup}");
    }
}

#[test]
fn clipboard_shortcuts_follow_keydown_focus_and_ignore_author_event_constructors() {
    let mut vm = clipboard_input_vm("input");
    vm.eval(r#"
globalThis.second = document.createElement('input');
second.value = 'destination';document.body.appendChild(second);
control.addEventListener('keydown', () => {second.focus();second.select()});
globalThis.originalInputEvent = InputEvent;
globalThis.originalClipboardEvent = ClipboardEvent;
second.addEventListener('paste', e => {globalThis.nativePaste = e instanceof originalClipboardEvent && e.isTrusted});
second.addEventListener('input', e => {globalThis.nativeInput = e instanceof originalInputEvent && e.isTrusted});
globalThis.ClipboardEvent = function() {throw Error('author ClipboardEvent')};
globalThis.InputEvent = function() {throw Error('author InputEvent')};
'installed'
"#).unwrap();
    clipboard_key(&mut vm, "v");
    assert_eq!(
        vm.eval("JSON.stringify([control.value, second.value, nativePaste, nativeInput])")
            .unwrap(),
        r#"["A🦊BC","seed",true,true]"#
    );
}

#[test]
fn clipboard_edits_follow_focus_and_reject_detached_targets_after_beforeinput() {
    let mut vm = clipboard_input_vm("input");
    vm.eval(
        r#"
globalThis.second = document.createElement('input');
second.value = 'other';document.body.appendChild(second);
control.addEventListener('beforeinput', () => {second.focus();second.select()});
'installed'
"#,
    )
    .unwrap();
    clipboard_key(&mut vm, "x");
    let state = clipboard_input_state(&mut vm);
    assert_eq!(state["value"], "A🦊BC");
    assert_eq!(state["clipboard"], "🦊");
    assert_eq!(state["events"].as_array().unwrap().len(), 2);
    assert_eq!(vm.eval("second.value").unwrap(), "");

    for (key, clipboard) in [("x", "🦊"), ("v", "seed")] {
        let mut vm = clipboard_input_vm("input");
        vm.eval("control.addEventListener('beforeinput', () => control.remove())")
            .unwrap();
        clipboard_key(&mut vm, key);
        let state = clipboard_input_state(&mut vm);
        assert_eq!(state["value"], "A🦊BC");
        assert_eq!(state["clipboard"], clipboard);
        assert_eq!(state["events"].as_array().unwrap().len(), 2);
    }
}

#[test]
fn clipboard_paste_reports_text_after_maxlength_and_single_line_normalization() {
    let mut vm = clipboard_input_vm("input");
    vm.eval("control.maxLength = 4").unwrap();
    clipboard_key(&mut vm, "v");
    let state = clipboard_input_state(&mut vm);
    assert_eq!(state["value"], "AsBC");
    assert_eq!(state["events"][1][7], "seed");
    assert_eq!(state["events"][2][7], "s");

    let mut vm = clipboard_input_vm("input");
    vm.eval("navigator.clipboard.writeText('a\\nb\\r\\nc')")
        .unwrap();
    clipboard_key(&mut vm, "v");
    let state = clipboard_input_state(&mut vm);
    assert_eq!(state["value"], "Aa b cBC");
    assert_eq!(state["events"][2][7], "a b c");
}
