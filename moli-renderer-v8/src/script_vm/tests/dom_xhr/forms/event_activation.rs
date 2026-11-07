use super::*;

#[test]
fn script_control_click_preserves_focus_and_pending_text_commit() {
    for dispatch in [false, true] {
        for target in [
            "button", "submit", "text", "check", "radio", "select", "textarea",
        ] {
            let mut vm = new_parsed_test_vm(
                "https://script-click-focus.test/",
                r#"<html><body><form>
<input id="origin"><button id="button" type="button">Button</button>
<input id="submit" type="submit"><input id="text"><input id="check" type="checkbox">
<input id="radio" type="radio"><select id="select"><option>A</option></select>
<textarea id="textarea"></textarea></form></body></html>"#,
            );
            vm.eval(
                r#"window.events = []; window.submits = []; window.clicks = [];
const origin = document.getElementById('origin');
for (const type of ['change', 'blur']) origin.addEventListener(type, () => events.push(type));
document.addEventListener('click', e => clicks.push([e.target.id, e.isTrusted, e.detail]));
document.querySelector('form').addEventListener('submit', e => {e.preventDefault(); submits.push(e.submitter.id)});
origin.focus();"#,
            )
            .expect("script activation fixture should initialize");
            vm.dispatch_key_event("keydown", "e", "KeyE", "edited", 0, false, true)
                .expect("actual text input should leave a pending change commit");
            let result = vm
                .eval(&format!(
                    r#"(() => {{
const target = document.getElementById('{target}');
if ({dispatch}) target.dispatchEvent(new MouseEvent('click', {{bubbles:true, cancelable:true}}));
else target.click();
return JSON.stringify({{active:document.activeElement.id, events, clicks, submits,
checked:document.getElementById('check').checked, radio:document.getElementById('radio').checked}});
}})()"#
                ))
                .expect("script activation should evaluate");
            let actual: serde_json::Value = serde_json::from_str(&result).unwrap();
            let submits = if target == "submit" {
                vec!["submit"]
            } else {
                vec![]
            };
            assert_eq!(
                actual,
                serde_json::json!({"active":"origin", "events":[],
                    "clicks":[[target,false,0]], "submits":submits,
                    "checked":target == "check", "radio":target == "radio"}),
                "target={target}, dispatch={dispatch}"
            );
            assert_eq!(
                vm.eval("origin.blur(); events.join('|')").unwrap(),
                "change|blur",
                "explicit blur must still commit the edited origin"
            );
        }
    }
}

#[test]
fn canceled_script_click_keeps_focus_and_rolls_back_activation() {
    for dispatch in [false, true] {
        for target in ["check", "radio", "submit"] {
            let mut vm = new_parsed_test_vm(
                "https://script-click-cancel-focus.test/",
                r#"<html><body><form><input id="origin">
<input id="check" type="checkbox"><input id="radio" type="radio">
<input id="submit" type="submit"></form></body></html>"#,
            );
            let result = vm.eval(&format!(r#"(() => {{
const origin = document.getElementById('origin'); const target = document.getElementById('{target}');
const seen = []; const submissions = [];
document.querySelector('form').addEventListener('submit', e => {{e.preventDefault(); submissions.push(e.submitter.id)}});
target.addEventListener('click', e => {{seen.push([target.checked, e.isTrusted]); e.preventDefault()}});
origin.focus();
if ({dispatch}) target.dispatchEvent(new MouseEvent('click', {{bubbles:true,cancelable:true}})); else target.click();
return JSON.stringify({{active:document.activeElement.id, checked:target.checked, seen, submissions}});
}})()"#)).unwrap();
            let actual: serde_json::Value = serde_json::from_str(&result).unwrap();
            assert_eq!(
                actual,
                serde_json::json!({"active":"origin", "checked":false,
                "seen":[[target != "submit",false]], "submissions":[]}),
                "target={target}, dispatch={dispatch}"
            );
        }
    }
}

#[test]
fn script_label_activation_keeps_its_independent_control_focus_rule() {
    for dispatch in [false, true] {
        for target in ["label", "nested"] {
            let mut vm = new_parsed_test_vm(
                "https://script-label-focus.test/",
                r#"<html><body><input id="origin"><label id="label" for="check">
Label <span id="nested">Nested</span></label><input id="check" type="checkbox"></body></html>"#,
            );
            let result = vm.eval(&format!(r#"(() => {{
const origin = document.getElementById('origin'); const check = document.getElementById('check');
const target = document.getElementById('{target}'); const seen = [];
check.addEventListener('click', () => seen.push(document.activeElement === check));
origin.focus();
if ({dispatch}) target.dispatchEvent(new MouseEvent('click', {{bubbles:true,cancelable:true}})); else target.click();
return JSON.stringify({{active:document.activeElement.id, checked:check.checked, seen}});
}})()"#)).unwrap();
            assert_eq!(
                result, r#"{"active":"check","checked":true,"seen":[true]}"#,
                "target={target}, dispatch={dispatch}"
            );
        }
    }
}

#[test]
fn pointer_release_does_not_reapply_canceled_mouse_focus() {
    for canceled_event in ["none", "mousedown", "pointerdown"] {
        for label in [false, true] {
            let target = if label { "label" } else { "button" };
            let mut vm = new_rendered_test_vm(
                "https://pointer-focus-ownership.test/",
                r#"<html><body><input id="origin" style="position:absolute;left:200px;top:100px">
<button id="button" type="button" style="position:absolute;left:10px;top:10px;width:100px;height:40px">Button</button>
<label id="label" for="check" style="position:absolute;left:10px;top:70px;width:100px;height:40px">Label</label>
<input id="check" type="checkbox" style="position:absolute;left:200px;top:70px"></body></html>"#,
            );
            vm.eval(&format!(
                r#"window.events = []; window.clicks = [];
const origin = document.getElementById('origin');
for (const type of ['change','blur']) origin.addEventListener(type, () => events.push(type));
document.addEventListener('click', e => clicks.push(e.target.id));
document.getElementById('{target}').addEventListener('{canceled_event}', e => e.preventDefault());
origin.focus();"#
            ))
            .unwrap();
            vm.dispatch_key_event("keydown", "e", "KeyE", "edited", 0, false, true)
                .expect("text edit should create a pending commit");
            let y = if label { 85.0 } else { 25.0 };
            vm.dispatch_mouse_event_at_point(25.0, y, "mousedown", 0, None, 0.0, 0.0)
                .expect("pointer down should dispatch");
            vm.dispatch_mouse_event_at_point(25.0, y, "mouseup", 0, None, 0.0, 0.0)
                .expect("pointer up and click should dispatch");
            let result = vm.eval("JSON.stringify({active:document.activeElement.id, events, clicks, checked:document.getElementById('check').checked})").unwrap();
            let actual: serde_json::Value = serde_json::from_str(&result).unwrap();
            let moves_focus = label || canceled_event == "none";
            let events = if moves_focus {
                vec!["change", "blur"]
            } else {
                vec![]
            };
            let clicks = if label {
                vec!["label", "check"]
            } else {
                vec!["button"]
            };
            let active = if label {
                "check"
            } else if moves_focus {
                "button"
            } else {
                "origin"
            };
            assert_eq!(
                actual,
                serde_json::json!({"active":active, "events":events,
                "clicks":clicks, "checked":label}),
                "target={target}, canceled={canceled_event}"
            );
        }
    }
}

#[test]
fn dispatched_bubbling_child_click_uses_ancestor_button_activation_behavior() {
    let mut vm = new_storage_test_vm("https://button-child-dispatched-click.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const form = document.createElement('form');
  const button = document.createElement('button');
  const child = document.createElement('span');
  button.appendChild(child);
  form.appendChild(button);
  host.appendChild(form);
  const submits = [];
  form.addEventListener('submit', event => {
    event.preventDefault();
    submits.push(event.submitter === button);
  });
  const allowed = child.dispatchEvent(new MouseEvent('click', {
    bubbles: true,
    cancelable: true
  }));
  const nonBubblingAllowed = child.dispatchEvent(new MouseEvent('click', {
    bubbles: false,
    cancelable: true
  }));
  return JSON.stringify({ allowed, nonBubblingAllowed, submits });
})()
"#,
        )
        .expect("bubbling child click activation probe should evaluate");

    assert_eq!(
        result,
        r#"{"allowed":true,"nonBubblingAllowed":true,"submits":[true]}"#
    );
}

#[test]
fn click_activation_target_survives_listener_tree_mutations() {
    for dispatch in [false, true] {
        for (mutation, expected) in [
            ("branch.remove()", vec!["owner:first"]),
            ("second.appendChild(branch)", vec!["owner:first"]),
            ("first.replaceChildren()", vec!["owner:first"]),
            ("first.type = 'button'", vec![]),
            ("first.disabled = true", vec![]),
            ("event.preventDefault(); branch.remove()", vec![]),
            ("first.remove()", vec![]),
            ("first.setAttribute('form', 'other')", vec!["other:first"]),
        ] {
            let mut vm = new_parsed_test_vm(
                "https://click-activation-target.test/",
                r#"<!doctype html><form id="owner"><button id="first"><span id="branch"><span id="child">click</span></span></button></form>
<form id="other"><button id="second">other</button></form>"#,
            );
            let result = vm
                .eval(&format!(
                    r#"(() => {{
const first = document.getElementById('first');
const second = document.getElementById('second');
const branch = document.getElementById('branch');
const child = document.getElementById('child');
const submits = [];
for (const form of document.forms) form.addEventListener('submit', event => {{
  event.preventDefault(); submits.push(form.id + ':' + event.submitter.id);
}});
first.addEventListener('click', event => {{ {mutation}; }});
if ({dispatch}) child.dispatchEvent(new MouseEvent('click', {{bubbles:true,cancelable:true}}));
else child.click();
return JSON.stringify(submits);
}})()"#
                ))
                .expect("activation must retain the original button and read its live form state");
            assert_eq!(
                serde_json::from_str::<Vec<String>>(&result).unwrap(),
                expected,
                "dispatch={dispatch}, mutation={mutation}"
            );
        }
    }
}

#[test]
fn click_activation_label_retains_identity_and_resolves_control_after_listeners() {
    for dispatch in [false, true] {
        for initial_for in ["first", "missing"] {
            let mut vm = new_parsed_test_vm(
                "https://click-activation-label.test/",
                &format!(
                    r#"<!doctype html><label id="label" for="{initial_for}"><span id="child">click</span></label>
<input id="first" type="checkbox"><input id="second" type="checkbox">"#
                ),
            );
            let result = vm
                .eval(&format!(
                    r#"(() => {{
const label = document.getElementById('label');
const child = document.getElementById('child');
const first = document.getElementById('first');
const second = document.getElementById('second');
const clicks = [];
first.addEventListener('click', () => clicks.push('first'));
second.addEventListener('click', () => clicks.push('second'));
label.addEventListener('click', () => {{ child.remove(); label.htmlFor = 'second'; }});
if ({dispatch}) child.dispatchEvent(new MouseEvent('click', {{bubbles:true,cancelable:true}}));
else child.click();
return JSON.stringify({{first:first.checked,second:second.checked,clicks}});
}})()"#
                ))
                .expect("label activation must use the original label's new associated control");
            assert_eq!(
                result, r#"{"first":false,"second":true,"clicks":["second"]}"#,
                "dispatch={dispatch}, initial_for={initial_for}"
            );
        }
    }
}

#[test]
fn click_activation_does_not_acquire_an_ancestor_inserted_by_a_listener() {
    for dispatch in [false, true] {
        let mut vm = new_parsed_test_vm(
            "https://click-activation-inserted-ancestor.test/",
            r#"<!doctype html><form><span id="child">click</span><button id="button">submit</button></form>"#,
        );
        let result = vm
            .eval(&format!(
                r#"(() => {{
const child = document.getElementById('child');
const button = document.getElementById('button');
let submits = 0;
document.forms[0].addEventListener('submit', event => {{ event.preventDefault(); submits++; }});
child.addEventListener('click', () => button.appendChild(child));
if ({dispatch}) child.dispatchEvent(new MouseEvent('click', {{bubbles:true,cancelable:true}}));
else child.click();
return String(submits);
}})()"#
            ))
            .expect("a listener cannot add an activation target to the original event path");
        assert_eq!(result, "0", "dispatch={dispatch}");
    }
}
