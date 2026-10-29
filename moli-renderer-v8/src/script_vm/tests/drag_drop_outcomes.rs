use super::*;
use crate::runtime::{RendererDragData, RendererPointerEventProperties};

const PAGE: &str = r#"<html><head><meta charset=utf-8></head><body style="margin:0">
<div id=source draggable=true style="position:absolute;left:0;top:0;width:100px;height:40px"></div>
TARGET
</body></html>"#;

fn drag_vm(target: &str, allowed: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_rendered_test_vm(
        "https://drag-outcomes.test/",
        &PAGE.replace("TARGET", target),
    );
    vm.eval(&format!(
        r#"
(() => {{
  window.__ends=[];window.__events=[];window.__drops=0;window.__leaves=0;
  window.__target=document.getElementById('target');
  window.__target.style.cssText='position:absolute;left:0;top:100px;width:200px;height:100px';
  const source=document.getElementById('source');
  source.addEventListener('dragstart', e => {{
    e.dataTransfer.effectAllowed={};e.dataTransfer.setData('text/plain','payload');
    window.__sourceTransfer=e.dataTransfer;
  }});
  source.addEventListener('dragend', e => window.__ends.push(e.dataTransfer.dropEffect));
}})();
"#,
        serde_json::json!(allowed)
    ))
    .expect("configure drag source");
    vm
}

fn start_drag(vm: &mut ScriptVm) -> RendererDragData {
    vm.publish_layout_for_test().expect("publish drag geometry");
    for (event, y) in [("mousedown", 20.0), ("mousemove", 120.0)] {
        let outcome = vm
            .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                20.0,
                y,
                event,
                0,
                Some(1),
                0,
                0.0,
                0.0,
                RendererPointerEventProperties::default(),
                0,
                true,
            )
            .expect("dispatch native drag gesture");
        if event == "mousemove" {
            let intercepted = outcome.intercepted_drag.expect("native drag intercepted");
            return RendererDragData {
                items: intercepted.items,
                files: Vec::new(),
                directories: Vec::new(),
                drag_operations_mask: intercepted.drag_operations_mask,
            };
        }
    }
    unreachable!()
}

#[test]
fn dnd_events_expose_source_preferences_and_negotiated_drop_effects() {
    let mut cases = vec![
        ("none", "none", None, "none", 0),
        ("copy", "copy", None, "copy", 0),
        ("link", "link", None, "link", 0),
        ("move", "move", None, "move", 0),
        ("copyLink", "copy", None, "copy", 0),
        ("copyMove", "move", None, "move", 0),
        ("linkMove", "move", None, "move", 0),
        ("all", "copy", None, "copy", 0),
        ("uninitialized", "copy", None, "copy", 0),
        ("copyLink", "copy", Some("link"), "link", 0),
        ("copyMove", "move", Some("copy"), "copy", 0),
        ("copy", "copy", Some("none"), "none", 0),
        ("copy", "copy", Some("move"), "none", 0),
    ];
    for (allowed, preferred) in [("copyMove", "move"), ("linkMove", "move"), ("all", "copy")] {
        for modifiers in [1, 2, 8, 10] {
            cases.push((allowed, preferred, None, preferred, modifiers));
        }
    }
    for (allowed, preferred, chosen, expected, modifiers) in cases {
        let mut vm = drag_vm("<div id=target></div>", allowed);
        vm.eval(&format!(r#"
window.__chosen={};
for(const type of ['dragenter','dragover','dragleave','drop'])window.__target.addEventListener(type,e => {{
  window.__events.push([type,e.dataTransfer.dropEffect,e.ctrlKey,e.altKey,e.shiftKey]);
  if(type==='dragover' && window.__chosen!==null)e.dataTransfer.dropEffect=window.__chosen;
  if(type!=='dragleave')e.preventDefault();
}});
"#,serde_json::json!(chosen))).expect("record target effects");
        let data = start_drag(&mut vm);
        for event in ["dragenter", "dragover", "drop"] {
            vm.dispatch_drag_event_at_point(20.0, 120.0, event, data.clone(), modifiers)
                .expect("dispatch replay command");
        }
        let observed: serde_json::Value = serde_json::from_str(&vm.eval(
            "JSON.stringify([window.__events,window.__ends,window.__sourceTransfer.getData('text/plain'),window.__sourceTransfer.types.length])",
        ).expect("effect result")).expect("effect JSON");
        let ctrl = modifiers & 2 != 0;
        let alt = modifiers & 1 != 0;
        let shift = modifiers & 8 != 0;
        let mut events = vec![
            serde_json::json!(["dragenter", preferred, ctrl, alt, shift]),
            serde_json::json!(["dragover", preferred, ctrl, alt, shift]),
            serde_json::json!(["dragover", preferred, ctrl, alt, shift]),
        ];
        if expected == "none" {
            events.push(serde_json::json!(["dragleave", "none", ctrl, alt, shift]));
        } else {
            events.push(serde_json::json!(["drop", expected, ctrl, alt, shift]));
        }
        assert_eq!(
            observed,
            serde_json::json!([events, [expected], "", 0]),
            "{allowed}: chosen={chosen:?}, modifiers={modifiers}"
        );
        assert!(vm.active_drag_session.is_none());
    }
}

#[test]
fn transition_leave_resets_the_shared_view_effect_before_dispatch() {
    let mut vm = drag_vm("<div id=target></div>", "all");
    vm.eval(r#"
const other=document.createElement('div');other.id='other';
other.style.cssText='position:absolute;left:220px;top:100px;width:150px;height:100px';document.body.append(other);
window.__target.addEventListener('dragenter',e => {window.__events.push(['enter-a',e.dataTransfer.dropEffect]);e.dataTransfer.dropEffect='link';e.preventDefault();});
other.addEventListener('dragenter',e => {window.__bView=e.dataTransfer;window.__events.push(['enter-b',e.dataTransfer.dropEffect]);e.dataTransfer.dropEffect='move';e.preventDefault();});
window.__target.addEventListener('dragleave',e => window.__events.push(['leave-a',e.dataTransfer.dropEffect,e.dataTransfer===window.__bView]));
other.addEventListener('dragleave',e => window.__events.push(['leave-b',e.dataTransfer.dropEffect]));
"#).expect("transition listeners");
    let data = start_drag(&mut vm);
    vm.dispatch_drag_event_at_point(20.0, 120.0, "dragenter", data.clone(), 0)
        .expect("enter A");
    vm.dispatch_drag_event_at_point(240.0, 120.0, "dragover", data, 0)
        .expect("enter B and leave A");
    vm.cancel_dragging().expect("cancel B");
    assert_eq!(
        vm.eval("JSON.stringify([window.__events,window.__ends])")
            .expect("transition result"),
        r#"[[["enter-a","copy"],["enter-b","copy"],["leave-a","none",true],["leave-b","none"]],["none"]]"#
    );
}

#[test]
fn html_drop_outcome_uses_cancellation_and_the_final_allowed_effect() {
    // HTML's drop algorithm uses the final effect after a canceled event and
    // rejects an uncanceled event without a successful default action.
    for (action, expected) in [
        ("e.preventDefault();", "copy"),
        (
            "e.preventDefault();e.dataTransfer.dropEffect='link';",
            "link",
        ),
        (
            "e.preventDefault();e.dataTransfer.dropEffect='none';",
            "none",
        ),
        (
            "e.preventDefault();e.dataTransfer.dropEffect='move';",
            "none",
        ),
        ("e.preventDefault();window.__target.remove();", "copy"),
        ("void 0;", "none"),
        ("e.dataTransfer.dropEffect='link';", "none"),
    ] {
        let mut vm = drag_vm("<div id=target></div>", "copyLink");
        vm.eval(&format!(r#"
window.__target.addEventListener('dragenter',e => {{window.__events.push('enter:'+e.dataTransfer.dropEffect);e.preventDefault();}});
window.__target.addEventListener('dragover',e => {{window.__events.push('over:'+e.dataTransfer.dropEffect);e.preventDefault();}});
window.__target.addEventListener('dragleave',() => window.__events.push('leave'));
window.__target.addEventListener('drop',e => {{window.__events.push('drop:'+e.dataTransfer.dropEffect);window.__dropView=e.dataTransfer;{action}}});
"#)).expect("drop outcome listeners");
        let data = start_drag(&mut vm);
        vm.dispatch_drag_event_at_point(20.0, 120.0, "dragenter", data.clone(), 0)
            .expect("enter target");
        vm.dispatch_drag_event_at_point(20.0, 120.0, "drop", data, 0)
            .expect("drop attempt");
        let observed: serde_json::Value = serde_json::from_str(&vm.eval(
            "JSON.stringify([window.__events,window.__ends,window.__dropView.getData('text/plain'),window.__dropView.types.length])",
        ).expect("drop outcome result")).expect("outcome JSON");
        assert_eq!(
            observed,
            serde_json::json!([["enter:copy", "over:copy", "drop:copy"], [expected], "", 0]),
            "{action}"
        );
        assert!(vm.active_drag_session.is_none());
    }
}

#[test]
fn uncanceled_drop_uses_default_target_acceptance_before_edit_callbacks() {
    for (
        markup,
        drop_action,
        before_action,
        empty_payload,
        expected_value,
        before_count,
        input_count,
        effect,
    ) in [
        ("<input id=target>", "", "", false, "payload", 1, 1, "copy"),
        (
            "<input id=target>",
            "window.__target.readOnly=true;",
            "",
            false,
            "",
            0,
            0,
            "none",
        ),
        (
            "<input id=target>",
            "window.__target.disabled=true;",
            "",
            false,
            "",
            0,
            0,
            "none",
        ),
        (
            "<input id=target>",
            "window.__target.remove();",
            "",
            false,
            "",
            0,
            0,
            "none",
        ),
        (
            "<input id=target>",
            "window.__other=document.implementation.createHTMLDocument('other');window.__other.body.appendChild(window.__target);",
            "",
            false,
            "",
            0,
            0,
            "none",
        ),
        ("<input id=target>", "", "", true, "", 0, 0, "none"),
        (
            "<div id=target contenteditable=true>base</div>",
            "",
            "e.preventDefault();",
            false,
            "base",
            1,
            0,
            "copy",
        ),
        (
            "<div id=target contenteditable=true>base</div>",
            "",
            "window.__target.contentEditable='false';",
            false,
            "base",
            1,
            0,
            "copy",
        ),
        (
            "<div id=target contenteditable=true>base</div>",
            "",
            "window.__other=document.implementation.createHTMLDocument('other');window.__other.body.appendChild(window.__target);",
            false,
            "base",
            1,
            0,
            "copy",
        ),
    ] {
        let mut vm = drag_vm(markup, "copy");
        vm.eval(&format!(r#"
window.__before=0;window.__input=0;
window.__target.addEventListener('drop',e => {{window.__drops++;window.__seenDropEffect=e.dataTransfer.dropEffect;{drop_action}}});
window.__target.addEventListener('dragleave',() => window.__leaves++);
window.__target.addEventListener('beforeinput',e => {{window.__before++;{before_action}}});
window.__target.addEventListener('input',() => window.__input++);
"#)).expect("default-action listeners");
        let mut data = start_drag(&mut vm);
        if empty_payload {
            data.items[0].data.clear();
        }
        vm.dispatch_drag_event_at_point(20.0, 120.0, "dragenter", data.clone(), 0)
            .expect("enter editable target");
        vm.dispatch_drag_event_at_point(20.0, 120.0, "drop", data, 0)
            .expect("perform default drop");
        let observed: serde_json::Value = serde_json::from_str(&vm.eval(
            "JSON.stringify([window.__drops,window.__seenDropEffect,window.__before,window.__input,window.__ends,'value' in window.__target ? window.__target.value : window.__target.textContent,window.__leaves])",
        ).expect("default-action result")).expect("default-action JSON");
        assert_eq!(
            observed,
            serde_json::json!([
                1,
                "copy",
                before_count,
                input_count,
                [effect],
                expected_value,
                0
            ]),
            "{markup}: drop={drop_action}, before={before_action}, empty={empty_payload}"
        );
        assert!(vm.active_drag_session.is_none());
    }
}

#[test]
fn drop_rejects_a_target_removed_or_adopted_during_the_final_over() {
    for mutation in [
        "window.__target.remove();",
        "window.__other=document.implementation.createHTMLDocument('other');window.__other.body.appendChild(window.__target);",
    ] {
        let mut vm = drag_vm("<div id=target></div>", "copy");
        vm.eval(&format!(
            r#"
window.__target.addEventListener('dragenter',e => e.preventDefault());
window.__target.addEventListener('dragover',e => {{e.preventDefault();{mutation}}});
window.__target.addEventListener('drop',e => {{window.__drops++;e.preventDefault();}});
"#
        ))
        .expect("invalidated over target listeners");
        let data = start_drag(&mut vm);
        vm.dispatch_drag_event_at_point(20.0, 120.0, "dragenter", data.clone(), 0)
            .expect("enter valid target");
        vm.dispatch_drag_event_at_point(20.0, 120.0, "drop", data, 0)
            .expect("final over invalidates target");
        assert_eq!(
            vm.eval("JSON.stringify([window.__drops,window.__ends])")
                .expect("invalidated target result"),
            r#"[0,["none"]]"#,
            "{mutation}",
        );
        assert!(vm.active_drag_session.is_none());
    }
}
