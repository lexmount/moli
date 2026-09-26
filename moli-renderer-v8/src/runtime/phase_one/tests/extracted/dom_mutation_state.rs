use super::*;

#[test]
fn parser_reparent_focused_subtree_resets_focus_after_parser_step() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserFocusMoveEvents = [];
const a = document.createElement('div');
a.id = 'parser-focus-move-a';
const b = document.createElement('div');
b.id = 'parser-focus-move-b';
const parserTarget = document.createElement('input');
parserTarget.id = 'parser-focus-target';
const jsTarget = document.createElement('input');
jsTarget.id = 'js-focus-target';
window.parserFocusTarget = parserTarget;
window.jsFocusTarget = jsTarget;
for (const target of [parserTarget, jsTarget]) {
  target.addEventListener('blur', () => {
    window.parserFocusMoveEvents.push(`${target.id}:blur:${document.activeElement === target}`);
  });
  target.addEventListener('focusout', () => {
    window.parserFocusMoveEvents.push(`${target.id}:focusout:${document.activeElement === target}`);
  });
}
a.append(parserTarget, jsTarget);
document.body.append(a, b);
jsTarget.focus();
b.insertBefore(jsTarget, null);
parserTarget.focus();
"#,
                )
                .expect("focused move setup should evaluate");

            let (parent, target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-focus-move-b")
                        .expect("target parser parent should exist"),
                    runtime
                        .get_element_by_id("parser-focus-target")
                        .expect("focused parser target should exist"),
                )
            };

            let focus_reset_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent,
                        child: target,
                        reference_child: None,
                    },
                    "parser focused reparent mutation should apply",
                )
            };
            assert!(
                !focus_reset_roots.is_empty(),
                "moving a focused connected subtree should defer focus reset until the parser step returns"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(focus_reset_roots)
                .expect("parser focused reparent followups should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserFocusMoveEvents,
  parserParent: window.parserFocusTarget.parentNode && window.parserFocusTarget.parentNode.id,
  jsParent: window.jsFocusTarget.parentNode && window.jsFocusTarget.parentNode.id,
  parserFocused: document.activeElement === window.parserFocusTarget
})"#,
                )
                .expect("focused move result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-focus-target:blur:false","js-focus-target:focusout:false","parser-focus-target:blur:false","parser-focus-target:focusout:false"],"parserParent":"parser-focus-move-b","jsParent":"parser-focus-move-b","parserFocused":false}"#
                ),
                "parser reparent should match JS insertBefore focus reset for focused moved subtrees"
            );
        }));
}
#[test]
fn parser_append_child_focused_subtree_to_disconnected_parent_resets_focus_like_js() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserFocusAppendDetachEvents = [];
function observeFocusAppendDetach(target) {
  target.addEventListener('blur', () => {
    window.parserFocusAppendDetachEvents.push(`${target.id}:blur:${document.activeElement === target}`);
  });
  target.addEventListener('focusout', () => {
    window.parserFocusAppendDetachEvents.push(`${target.id}:focusout:${document.activeElement === target}`);
  });
}

const jsSource = document.createElement('div');
const jsDetachedParent = document.createElement('div');
const jsTarget = document.createElement('input');
jsTarget.id = 'js-focus-append-detach-target';
observeFocusAppendDetach(jsTarget);
jsSource.appendChild(jsTarget);

const parserSource = document.createElement('div');
const parserTarget = document.createElement('input');
parserTarget.id = 'parser-focus-append-detach-target';
window.parserFocusAppendDetachTarget = parserTarget;
observeFocusAppendDetach(parserTarget);
parserSource.appendChild(parserTarget);

document.body.append(jsSource, parserSource);

jsTarget.focus();
jsDetachedParent.appendChild(jsTarget);
window.parserFocusAppendDetachJsFocused = document.activeElement === jsTarget;

parserTarget.focus();
"#,
                )
                .expect("focused append-to-detached setup should evaluate");

            let (detached_parent, target) = {
                let runtime = &mut page_vm.vm_mut().document_runtime;
                let target = runtime
                    .get_element_by_id("parser-focus-append-detach-target")
                    .expect("focused parser append-to-detached target should exist");
                let dom_host = runtime.dom_host_mut();
                let detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    detached_parent,
                    "id",
                    "parser-focus-append-detach-parent"
                ));
                (detached_parent, target)
            };

            let focus_reset_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: detached_parent,
                    child: target,
                },
                "parser focused append-to-detached mutation should apply",
            );
            assert!(
                !focus_reset_roots.is_empty(),
                "parser AppendChild moving a focused connected subtree to a disconnected parent should defer focus reset"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(focus_reset_roots)
                .expect("parser focused append-to-detached followups should dispatch");

            {
                let dom_host = page_vm.vm().document_runtime.dom_host();
                assert_eq!(
                    dom_host.child_handles(detached_parent).collect::<Vec<_>>(),
                    vec![target],
                    "parser AppendChild should move the focused target under the native detached parent"
                );
            }

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserFocusAppendDetachEvents,
  jsFocused: window.parserFocusAppendDetachJsFocused,
  parserFocused: document.activeElement === window.parserFocusAppendDetachTarget
})"#,
                )
                .expect("focused append-to-detached result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-focus-append-detach-target:blur:false","js-focus-append-detach-target:focusout:false","parser-focus-append-detach-target:blur:false","parser-focus-append-detach-target:focusout:false"],"jsFocused":false,"parserFocused":false}"#
                ),
                "parser AppendChild to a disconnected parent should match JS appendChild focus reset for focused moved subtrees"
            );
        }));
}
#[test]
fn js_append_and_replace_child_reparent_focused_subtree_reset_focus_from_mutation_owner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            let result = page_vm
                .evaluate_expression(
                    r#"
window.jsFocusedReparentEvents = [];
function observe(target) {
  target.addEventListener('blur', () => {
    window.jsFocusedReparentEvents.push(`${target.id}:blur:${document.activeElement === target}`);
  });
  target.addEventListener('focusout', () => {
    window.jsFocusedReparentEvents.push(`${target.id}:focusout:${document.activeElement === target}`);
  });
}

const appendSource = document.createElement('div');
appendSource.id = 'js-focus-append-source';
const appendDest = document.createElement('div');
appendDest.id = 'js-focus-append-dest';
const appendTarget = document.createElement('input');
appendTarget.id = 'js-focus-append-target';
observe(appendTarget);
appendSource.appendChild(appendTarget);

const replaceSource = document.createElement('div');
replaceSource.id = 'js-focus-replace-source';
const replaceDest = document.createElement('div');
replaceDest.id = 'js-focus-replace-dest';
const replaceSlot = document.createElement('span');
replaceSlot.id = 'js-focus-replace-slot';
const replaceTarget = document.createElement('input');
replaceTarget.id = 'js-focus-replace-target';
observe(replaceTarget);
replaceSource.appendChild(replaceTarget);
replaceDest.appendChild(replaceSlot);

document.body.append(appendSource, appendDest, replaceSource, replaceDest);

appendTarget.focus();
appendDest.appendChild(appendTarget);
const appendFocused = document.activeElement === appendTarget;

replaceTarget.focus();
replaceDest.replaceChild(replaceTarget, replaceSlot);
const replaceFocused = document.activeElement === replaceTarget;

JSON.stringify({
  events: window.jsFocusedReparentEvents,
  appendFocused,
  replaceFocused,
  appendParent: appendTarget.parentNode && appendTarget.parentNode.id,
  replaceParent: replaceTarget.parentNode && replaceTarget.parentNode.id
})
"#,
                )
                .expect("focused append/replace reparent should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-focus-append-target:blur:false","js-focus-append-target:focusout:false","js-focus-replace-target:blur:false","js-focus-replace-target:focusout:false"],"appendFocused":false,"replaceFocused":false,"appendParent":"js-focus-append-dest","replaceParent":"js-focus-replace-dest"}"#
                ),
                "appendChild and replaceChild should reset focused moved subtrees like insertBefore"
            );
        }));
}
#[test]
fn js_and_parser_remove_focused_subtree_reset_focus_from_mutation_owner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserRemoveFocusEvents = [];
function observe(target) {
  target.addEventListener('blur', () => {
    window.parserRemoveFocusEvents.push(`${target.id}:blur:${document.activeElement === target}`);
  });
  target.addEventListener('focusout', () => {
    window.parserRemoveFocusEvents.push(`${target.id}:focusout:${document.activeElement === target}`);
  });
}

const jsParent = document.createElement('div');
jsParent.id = 'js-remove-focus-parent';
const jsTarget = document.createElement('input');
jsTarget.id = 'js-remove-focus-target';
observe(jsTarget);
jsParent.appendChild(jsTarget);

const parserParent = document.createElement('div');
parserParent.id = 'parser-remove-focus-parent';
const parserTarget = document.createElement('input');
parserTarget.id = 'parser-remove-focus-target';
observe(parserTarget);
parserParent.appendChild(parserTarget);
window.parserRemoveFocusParserTarget = parserTarget;

document.body.append(jsParent, parserParent);
jsTarget.focus();
jsParent.removeChild(jsTarget);
window.parserRemoveFocusJsState = {
  focused: document.activeElement === jsTarget,
  parent: jsTarget.parentNode && jsTarget.parentNode.id
};
parserTarget.focus();
"#,
                )
                .expect("focused remove setup should evaluate");

            let (parser_parent, parser_target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-remove-focus-parent")
                        .expect("focused parser remove parent should exist"),
                    runtime
                        .get_element_by_id("parser-remove-focus-target")
                        .expect("focused parser remove target should exist"),
                )
            };

            let focus_reset_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::RemoveChild {
                        parent: parser_parent,
                        child: parser_target,
                    },
                    "parser focused remove mutation should apply",
                )
            };
            assert!(
                !focus_reset_roots.is_empty(),
                "removing a focused connected subtree should defer focus reset until the parser step returns"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(focus_reset_roots)
                .expect("parser focused remove followups should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserRemoveFocusEvents,
  js: window.parserRemoveFocusJsState,
  parser: {
    focused: document.activeElement === window.parserRemoveFocusParserTarget,
    parent: window.parserRemoveFocusParserTarget.parentNode &&
      window.parserRemoveFocusParserTarget.parentNode.id
  }
})"#,
                )
                .expect("focused remove result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-remove-focus-target:blur:false","js-remove-focus-target:focusout:false","parser-remove-focus-target:blur:false","parser-remove-focus-target:focusout:false"],"js":{"focused":false,"parent":null},"parser":{"focused":false,"parent":null}}"#
                ),
                "JS and parser removeChild should reset focused removed subtrees"
            );
        }));
}
#[test]
fn parser_remove_pending_pointer_capture_target_clears_like_js_remove() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserPointerCaptureLog = [];
function installPendingPointerCaptureTarget(prefix, removeWithJs) {
  const button = document.createElement('div');
  button.setAttribute('id', `${prefix}-button`);
  button.textContent = `${prefix} button`;
  const target = document.createElement('div');
  target.setAttribute('id', `${prefix}-target`);
  target.textContent = `${prefix} target`;
  button.addEventListener('pointerdown', event => {
    window.parserPointerCaptureLog.push(`${prefix}:pointerdown`);
    target.setPointerCapture(event.pointerId);
    window.parserPointerCaptureLog.push(
      `${prefix}:has-before:${target.hasPointerCapture(event.pointerId)}`
    );
    if (removeWithJs) {
      target.remove();
      window.parserPointerCaptureLog.push(
        `${prefix}:has-after-js-remove:${target.hasPointerCapture(event.pointerId)}`
      );
    }
  });
  button.addEventListener('pointerup', () => {
    window.parserPointerCaptureLog.push(`${prefix}:pointerup`);
  });
  target.addEventListener('gotpointercapture', () => {
    window.parserPointerCaptureLog.push(`${prefix}:gotpointercapture`);
  });
  target.addEventListener('lostpointercapture', () => {
    window.parserPointerCaptureLog.push(`${prefix}:lostpointercapture`);
  });
  document.body.append(button, target);
  return { button, target };
}
window.jsPendingPointerCapture = installPendingPointerCaptureTarget('js-pointer', true);
window.parserPendingPointerCapture = installPendingPointerCaptureTarget('parser-pointer', false);
"#,
                )
                .expect("pending pointer capture parser mutation setup should evaluate");

            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mousedown",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("JS baseline mousedown should set then clear pending capture");
            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mouseup",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("JS baseline mouseup should not use removed pending capture");
            let js_log = page_vm
                .evaluate_expression(
                    r#"(() => {
  const log = window.parserPointerCaptureLog.splice(0).join('|');
  window.jsPendingPointerCapture.button.remove();
  return log;
})()"#,
                )
                .expect("JS pending pointer capture remove baseline should evaluate");

            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mousedown",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("parser baseline mousedown should set pending capture");

            let parser_target = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-pointer-target")
                .expect("parser pending pointer capture target should exist");
            let reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: body,
                    child: parser_target,
                },
                "parser pending pointer capture target removal should apply",
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(reaction_roots)
                .expect("parser pending pointer capture followups should dispatch");
            page_vm
                .evaluate_expression(
                    r#"window.parserPointerCaptureLog.push(
  `parser-pointer:has-after-parser-remove:${
    window.parserPendingPointerCapture.target.hasPointerCapture(1)
  }`
)"#,
                )
                .expect("parser pending pointer capture state should evaluate");
            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mouseup",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("parser baseline mouseup should not use removed pending capture");
            let parser_log = page_vm
                .evaluate_expression("window.parserPointerCaptureLog.join('|')")
                .expect("parser pending pointer capture remove log should evaluate");

            assert_eq!(
                js_log.get("value").and_then(serde_json::Value::as_str),
                Some(
                    "js-pointer:pointerdown|js-pointer:has-before:true|js-pointer:has-after-js-remove:false|js-pointer:pointerup"
                ),
                "JS remove baseline should clear pending pointer capture immediately"
            );
            assert_eq!(
                parser_log.get("value").and_then(serde_json::Value::as_str),
                Some(
                    "parser-pointer:pointerdown|parser-pointer:has-before:true|parser-pointer:has-after-parser-remove:false|parser-pointer:pointerup"
                ),
                "parser remove should clear pending pointer capture before the next pointer event"
            );
        }));
}
#[test]
fn parser_reparent_pending_pointer_capture_target_to_disconnected_parent_clears_like_js() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserPointerCaptureReparentLog = [];
function installPendingPointerCaptureReparentTarget(prefix, reparentWithJs) {
  const button = document.createElement('div');
  button.setAttribute('id', `${prefix}-button`);
  button.textContent = `${prefix} button`;
  const target = document.createElement('div');
  target.setAttribute('id', `${prefix}-target`);
  target.textContent = `${prefix} target`;
  const detachedParent = document.createElement('div');
  detachedParent.setAttribute('id', `${prefix}-detached-parent`);
  button.addEventListener('pointerdown', event => {
    window.parserPointerCaptureReparentLog.push(`${prefix}:pointerdown`);
    target.setPointerCapture(event.pointerId);
    window.parserPointerCaptureReparentLog.push(
      `${prefix}:has-before:${target.hasPointerCapture(event.pointerId)}`
    );
    if (reparentWithJs) {
      detachedParent.appendChild(target);
      window.parserPointerCaptureReparentLog.push(
        `${prefix}:has-after-js-reparent:${target.hasPointerCapture(event.pointerId)}`
      );
    }
  });
  button.addEventListener('pointerup', () => {
    window.parserPointerCaptureReparentLog.push(`${prefix}:pointerup`);
  });
  target.addEventListener('gotpointercapture', () => {
    window.parserPointerCaptureReparentLog.push(`${prefix}:gotpointercapture`);
  });
  target.addEventListener('lostpointercapture', () => {
    window.parserPointerCaptureReparentLog.push(`${prefix}:lostpointercapture`);
  });
  document.body.append(button, target);
  return { button, target, detachedParent };
}
window.jsPendingPointerCaptureReparent =
  installPendingPointerCaptureReparentTarget('js-pointer-reparent', true);
window.parserPendingPointerCaptureReparent =
  installPendingPointerCaptureReparentTarget('parser-pointer-reparent', false);
"#,
                )
                .expect("pending pointer capture reparent setup should evaluate");

            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mousedown",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("JS baseline mousedown should set then clear pending capture by reparent");
            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mouseup",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("JS baseline mouseup should not use reparented pending capture");
            let js_log = page_vm
                .evaluate_expression(
                    r#"(() => {
  const log = window.parserPointerCaptureReparentLog.splice(0).join('|');
  window.jsPendingPointerCaptureReparent.button.remove();
  return log;
})()"#,
                )
                .expect("JS pending pointer capture reparent baseline should evaluate");

            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mousedown",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("parser baseline mousedown should set pending capture");

            let parser_target = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-pointer-reparent-target")
                .expect("parser pending pointer capture reparent target should exist");
            let parser_detached_parent = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                )
            };
            let reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_detached_parent,
                    child: parser_target,
                },
                "parser pending pointer capture target reparent should apply",
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(reaction_roots)
                .expect("parser pending pointer capture reparent followups should dispatch");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .node(parser_target)
                    .and_then(Node::parent_node),
                Some(parser_detached_parent),
                "parser reparent should move the pending capture target under the native detached parent"
            );
            page_vm
                .evaluate_expression(
                    r#"window.parserPointerCaptureReparentLog.push(
  `parser-pointer-reparent:has-after-parser-reparent:${
    window.parserPendingPointerCaptureReparent.target.hasPointerCapture(1)
  }`
)"#,
                )
                .expect("parser pending pointer capture reparent state should evaluate");
            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mouseup",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("parser baseline mouseup should not use reparented pending capture");
            let parser_log = page_vm
                .evaluate_expression("window.parserPointerCaptureReparentLog.join('|')")
                .expect("parser pending pointer capture reparent log should evaluate");

            assert_eq!(
                js_log.get("value").and_then(serde_json::Value::as_str),
                Some(
                    "js-pointer-reparent:pointerdown|js-pointer-reparent:has-before:true|js-pointer-reparent:has-after-js-reparent:false|js-pointer-reparent:pointerup"
                ),
                "JS reparent to a disconnected parent should clear pending pointer capture immediately"
            );
            assert_eq!(
                parser_log.get("value").and_then(serde_json::Value::as_str),
                Some(
                    "parser-pointer-reparent:pointerdown|parser-pointer-reparent:has-before:true|parser-pointer-reparent:has-after-parser-reparent:false|parser-pointer-reparent:pointerup"
                ),
                "parser reparent to a disconnected parent should clear pending pointer capture before the next pointer event"
            );
        }));
}
#[test]
fn parser_remove_preserves_scroll_like_js_remove() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let mut page_vm = new_phase_one_page_vm_for_test();
        let body = create_connected_html_body_for_test(&mut page_vm);

        page_vm
            .evaluate_expression(
                r#"
const jsTarget = document.createElement('div');
jsTarget.id = 'js-scroll-preservation-remove';
jsTarget.style.height = '20px';
const parserTarget = document.createElement('div');
parserTarget.id = 'parser-scroll-preservation-remove';
parserTarget.style.height = '20px';
document.body.style.minHeight = '2000px';
document.body.append(jsTarget, parserTarget);
window.scrollTo(0, 30);
document.body.removeChild(jsTarget);
window.parserScrollJsOffset = window.pageYOffset;
window.scrollTo(0, 30);
window.parserScrollTarget = parserTarget;
"#,
            )
            .expect("scroll-preservation remove setup should evaluate");

        let parser_target = page_vm
            .vm()
            .document_runtime
            .get_element_by_id("parser-scroll-preservation-remove")
            .expect("parser scroll-preservation target should exist");
        let custom_element_reaction_roots = apply_parser_dom_mutation_for_test(
            &mut page_vm,
            ParserDomMutation::RemoveChild {
                parent: body,
                child: parser_target,
            },
            "parser scroll-preservation remove mutation should apply",
        );
        if !custom_element_reaction_roots.is_empty() {
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(
                    custom_element_reaction_roots,
                )
                .expect("parser scroll-preservation remove reactions should dispatch");
        }

        let result = page_vm
            .evaluate_expression(
                r#"JSON.stringify({
  jsOffset: window.parserScrollJsOffset,
  parserOffset: window.pageYOffset,
  parserParent: window.parserScrollTarget.parentNode
})"#,
            )
            .expect("scroll-preservation remove result should evaluate");
        assert_eq!(
            result.get("value").and_then(serde_json::Value::as_str),
            Some(r#"{"jsOffset":30,"parserOffset":30,"parserParent":null}"#),
            "parser RemoveChild should preserve the scroll offset like JS removeChild"
        );
    }));
}
#[test]
fn parser_reparent_preserves_scroll_like_js_reparent_to_disconnected_parent() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
const jsTarget = document.createElement('div');
jsTarget.id = 'js-scroll-preservation-reparent';
jsTarget.style.height = '20px';
const jsDetachedParent = document.createElement('div');
const parserTarget = document.createElement('div');
parserTarget.id = 'parser-scroll-preservation-reparent';
parserTarget.style.height = '20px';
document.body.style.minHeight = '2000px';
document.body.append(jsTarget, parserTarget);
window.scrollTo(0, 30);
jsDetachedParent.appendChild(jsTarget);
window.parserScrollReparentJsState = {
  offset: window.pageYOffset,
  parentIsDetached: jsTarget.parentNode === jsDetachedParent
};
window.scrollTo(0, 30);
window.parserScrollReparentTarget = parserTarget;
"#,
                )
                .expect("scroll-preservation reparent setup should evaluate");

            let parser_target = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-scroll-preservation-reparent")
                .expect("parser scroll-preservation reparent target should exist");
            let parser_detached_parent = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                )
            };
            let custom_element_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_detached_parent,
                    child: parser_target,
                },
                "parser scroll-preservation reparent mutation should apply",
            );
            if !custom_element_reaction_roots.is_empty() {
                page_vm
                    .vm_mut()
                    .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                    .expect("parser scroll-preservation reparent reactions should dispatch");
            }

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  js: window.parserScrollReparentJsState,
  parserOffset: window.pageYOffset
})"#,
                )
                .expect("scroll-preservation reparent result should evaluate");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .node(parser_target)
                    .and_then(Node::parent_node),
                Some(parser_detached_parent),
                "parser reparent should move the scroll-preservation target under the native detached parent"
            );
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"{"js":{"offset":30,"parentIsDetached":true},"parserOffset":30}"#),
                "parser reparent to a disconnected parent should preserve the scroll offset like JS reparent"
            );
        }));
}
#[test]
fn parser_insert_before_reparent_preserves_scroll_like_js() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
const jsTarget = document.createElement('div');
jsTarget.id = 'js-scroll-preservation-insert-before';
jsTarget.style.height = '20px';
const jsDetachedParent = document.createElement('div');
const jsReference = document.createElement('span');
jsDetachedParent.appendChild(jsReference);
const parserTarget = document.createElement('div');
parserTarget.id = 'parser-scroll-preservation-insert-before';
parserTarget.style.height = '20px';
document.body.style.minHeight = '2000px';
document.body.append(jsTarget, parserTarget);
window.scrollTo(0, 30);
jsDetachedParent.insertBefore(jsTarget, jsReference);
window.parserScrollInsertBeforeJsState = {
  offset: window.pageYOffset,
  parentIsDetached: jsTarget.parentNode === jsDetachedParent,
  nextIsReference: jsTarget.nextSibling === jsReference
};
window.scrollTo(0, 30);
"#,
                )
                .expect("scroll-preservation insertBefore setup should evaluate");

            let parser_target = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-scroll-preservation-insert-before")
                .expect("parser scroll-preservation insertBefore target should exist");
            let (parser_detached_parent, parser_reference) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.append_child(detached_parent, reference));
                (detached_parent, reference)
            };
            let custom_element_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_detached_parent,
                    child: parser_target,
                    reference_child: Some(parser_reference),
                },
                "parser scroll-preservation insertBefore reparent mutation should apply",
            );
            if !custom_element_reaction_roots.is_empty() {
                page_vm
                    .vm_mut()
                    .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                    .expect("parser scroll-preservation insertBefore reparent reactions should dispatch");
            }

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  js: window.parserScrollInsertBeforeJsState,
  parserOffset: window.pageYOffset
})"#,
                )
                .expect("scroll-preservation insertBefore result should evaluate");
            let parser_children = page_vm
                .vm()
                .document_runtime
                .dom_host()
                .child_handles(parser_detached_parent)
                .collect::<Vec<_>>();
            assert_eq!(
                parser_children,
                vec![parser_target, parser_reference],
                "parser insertBefore should move the scroll-preservation target before the native reference child"
            );
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"js":{"offset":30,"parentIsDetached":true,"nextIsReference":true},"parserOffset":30}"#
                ),
                "parser InsertBefore reparent to a disconnected parent should preserve the scroll offset like JS insertBefore"
            );
        }));
}
#[test]
fn parser_remove_and_reparent_update_child_list_style_invalidation_like_js() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
const style = document.createElement('style');
style.textContent = [
  '.parser-style-parent { color: rgb(1, 2, 3); }',
  '.parser-style-parent:empty { color: rgb(10, 20, 30); }',
  '.parser-style-old > .parser-style-target { color: rgb(4, 5, 6); }',
  '.parser-style-new > .parser-style-target { color: rgb(40, 50, 60); }'
].join('\n');
const head = document.head || document.documentElement.insertBefore(
  document.createElement('head'),
  document.body
);
head.appendChild(style);

function installStyleRemove(prefix) {
  const parent = document.createElement('div');
  parent.setAttribute('id', `${prefix}-parent`);
  parent.setAttribute('class', 'parser-style-parent');
  const target = document.createElement('span');
  target.setAttribute('id', `${prefix}-target`);
  target.setAttribute('class', 'parser-style-target');
  parent.appendChild(target);
  document.body.appendChild(parent);
  return { parent, target };
}

function installStyleReparent(prefix) {
  const oldParent = document.createElement('div');
  oldParent.setAttribute('id', `${prefix}-old-parent`);
  oldParent.setAttribute('class', 'parser-style-parent parser-style-old');
  const newParent = document.createElement('div');
  newParent.setAttribute('id', `${prefix}-new-parent`);
  newParent.setAttribute('class', 'parser-style-parent parser-style-new');
  const target = document.createElement('span');
  target.setAttribute('id', `${prefix}-target`);
  target.setAttribute('class', 'parser-style-target');
  oldParent.appendChild(target);
  document.body.append(oldParent, newParent);
  return { oldParent, newParent, target };
}

window.parserStyleRemoveSummary = pair => [
  getComputedStyle(pair.parent).color,
  pair.target.parentNode && pair.target.parentNode.id
].join('|');
window.parserStyleReparentSummary = pair => [
  getComputedStyle(pair.oldParent).color,
  getComputedStyle(pair.newParent).color,
  getComputedStyle(pair.target).color,
  pair.target.parentNode && pair.target.parentNode.id
].join('|');

window.jsStyleRemove = installStyleRemove('js-style-remove');
window.parserStyleRemove = installStyleRemove('parser-style-remove');
window.jsStyleReparent = installStyleReparent('js-style-reparent');
window.parserStyleReparent = installStyleReparent('parser-style-reparent');
"#,
                )
                .expect("style invalidation parser mutation setup should evaluate");

            let js_remove_result = page_vm
                .evaluate_expression(
                    r#"(() => {
  const before = window.parserStyleRemoveSummary(window.jsStyleRemove);
  window.jsStyleRemove.parent.removeChild(window.jsStyleRemove.target);
  return `${before}=>${window.parserStyleRemoveSummary(window.jsStyleRemove)}`;
})()"#,
                )
                .expect("JS style remove baseline should evaluate");

            let parser_remove_before = page_vm
                .evaluate_expression("window.parserStyleRemoveSummary(window.parserStyleRemove)")
                .expect("parser style remove precondition should evaluate");
            let (parser_remove_parent, parser_remove_child) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-style-remove-parent")
                        .expect("parser style remove parent should exist"),
                    runtime
                        .get_element_by_id("parser-style-remove-target")
                        .expect("parser style remove target should exist"),
                )
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: parser_remove_parent,
                    child: parser_remove_child,
                },
                "parser style remove mutation should apply",
            );
            let parser_remove_after = page_vm
                .evaluate_expression("window.parserStyleRemoveSummary(window.parserStyleRemove)")
                .expect("parser style remove result should evaluate");

            let js_reparent_result = page_vm
                .evaluate_expression(
                    r#"(() => {
  const before = window.parserStyleReparentSummary(window.jsStyleReparent);
  window.jsStyleReparent.newParent.appendChild(window.jsStyleReparent.target);
  return `${before}=>${window.parserStyleReparentSummary(window.jsStyleReparent)}`;
})()"#,
                )
                .expect("JS style reparent baseline should evaluate");

            let parser_reparent_before = page_vm
                .evaluate_expression("window.parserStyleReparentSummary(window.parserStyleReparent)")
                .expect("parser style reparent precondition should evaluate");
            let (parser_reparent_new_parent, parser_reparent_target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-style-reparent-new-parent")
                        .expect("parser style reparent new parent should exist"),
                    runtime
                        .get_element_by_id("parser-style-reparent-target")
                        .expect("parser style reparent target should exist"),
                )
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_reparent_new_parent,
                    child: parser_reparent_target,
                    reference_child: None,
                },
                "parser style reparent mutation should apply",
            );
            let parser_reparent_after = page_vm
                .evaluate_expression("window.parserStyleReparentSummary(window.parserStyleReparent)")
                .expect("parser style reparent result should evaluate");

            assert_eq!(
                js_remove_result
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(
                    "rgb(1, 2, 3)|js-style-remove-parent=>rgb(10, 20, 30)|"
                ),
                "JS remove baseline should invalidate :empty style"
            );
            assert_eq!(
                parser_remove_before
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some("rgb(1, 2, 3)|parser-style-remove-parent"),
                "parser remove precondition should populate the non-empty style cache"
            );
            assert_eq!(
                parser_remove_after
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some("rgb(10, 20, 30)|"),
                "parser remove should invalidate child-list-dependent :empty style"
            );
            assert_eq!(
                js_reparent_result
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(
                    "rgb(1, 2, 3)|rgb(10, 20, 30)|rgb(4, 5, 6)|js-style-reparent-old-parent=>rgb(10, 20, 30)|rgb(1, 2, 3)|rgb(40, 50, 60)|js-style-reparent-new-parent"
                ),
                "JS reparent baseline should invalidate removed and inserted child-list style"
            );
            assert_eq!(
                parser_reparent_before
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(
                    "rgb(1, 2, 3)|rgb(10, 20, 30)|rgb(4, 5, 6)|parser-style-reparent-old-parent"
                ),
                "parser reparent precondition should populate old-parent, new-parent, and target style caches"
            );
            assert_eq!(
                parser_reparent_after
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(
                    "rgb(10, 20, 30)|rgb(1, 2, 3)|rgb(40, 50, 60)|parser-style-reparent-new-parent"
                ),
                "parser reparent should invalidate removed and inserted child-list style like JS"
            );
        }));
}
#[test]
fn parser_remove_and_reparent_slotted_nodes_queue_slotchange_like_js() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let mut page_vm = new_phase_one_page_vm_for_test();
        create_connected_html_body_for_test(&mut page_vm);

        page_vm
            .evaluate_expression(
                r#"
window.parserSlotchangeLog = [];
function installSlottedTarget(prefix, label) {
  const host = document.createElement('div');
  host.id = `${prefix}-host`;
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<slot name="a"></slot>';
  const slot = shadow.querySelector('slot');
  slot.addEventListener('slotchange', () => {
    window.parserSlotchangeLog.push(`${label}:${slot.assignedNodes().length}`);
  });
  const target = document.createElement('span');
  target.id = `${prefix}-target`;
  target.slot = 'a';
  host.appendChild(target);
  document.body.appendChild(host);
  return { host, target };
}
function installReparentPair(prefix) {
  const oldPair = installSlottedTarget(`${prefix}-old`, 'old');
  const newHost = document.createElement('div');
  newHost.id = `${prefix}-new-host`;
  const shadow = newHost.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<slot name="a"></slot>';
  const newSlot = shadow.querySelector('slot');
  newSlot.addEventListener('slotchange', () => {
    window.parserSlotchangeLog.push(`new:${newSlot.assignedNodes().length}`);
  });
  document.body.appendChild(newHost);
  return { oldHost: oldPair.host, newHost, target: oldPair.target };
}

window.jsSlotRemove = installSlottedTarget('js-slot-remove', 'remove');
window.parserSlotRemove = installSlottedTarget('parser-slot-remove', 'remove');
window.jsSlotReparent = installReparentPair('js-slot-reparent');
window.parserSlotReparent = installReparentPair('parser-slot-reparent');
"#,
            )
            .expect("slotchange parser mutation setup should evaluate");

        page_vm
            .evaluate_expression("undefined")
            .expect("initial slotchange microtasks should drain");
        page_vm
            .evaluate_expression("window.parserSlotchangeLog = []")
            .expect("slotchange log reset should evaluate");

        page_vm
            .evaluate_expression(
                r#"window.jsSlotRemove.host.removeChild(window.jsSlotRemove.target)"#,
            )
            .expect("JS slot remove baseline should evaluate");
        page_vm
            .evaluate_expression("undefined")
            .expect("JS slot remove slotchange microtask should drain");
        let js_remove_log = page_vm
            .evaluate_expression("JSON.stringify(window.parserSlotchangeLog.splice(0))")
            .expect("JS slot remove log should evaluate");

        let (parser_remove_host, parser_remove_target) = {
            let runtime = &page_vm.vm().document_runtime;
            (
                runtime
                    .get_element_by_id("parser-slot-remove-host")
                    .expect("parser slot remove host should exist"),
                runtime
                    .get_element_by_id("parser-slot-remove-target")
                    .expect("parser slot remove target should exist"),
            )
        };
        let _ = apply_parser_dom_mutation_for_test(
            &mut page_vm,
            ParserDomMutation::RemoveChild {
                parent: parser_remove_host,
                child: parser_remove_target,
            },
            "parser slot remove mutation should apply",
        );
        page_vm
            .evaluate_expression("undefined")
            .expect("parser slot remove slotchange microtask should drain");
        let parser_remove_log = page_vm
            .evaluate_expression("JSON.stringify(window.parserSlotchangeLog.splice(0))")
            .expect("parser slot remove log should evaluate");

        page_vm
            .evaluate_expression(
                r#"window.jsSlotReparent.newHost.appendChild(window.jsSlotReparent.target)"#,
            )
            .expect("JS slot reparent baseline should evaluate");
        page_vm
            .evaluate_expression("undefined")
            .expect("JS slot reparent slotchange microtask should drain");
        let js_reparent_log = page_vm
            .evaluate_expression("JSON.stringify(window.parserSlotchangeLog.splice(0))")
            .expect("JS slot reparent log should evaluate");

        let (parser_reparent_new_host, parser_reparent_target) = {
            let runtime = &page_vm.vm().document_runtime;
            (
                runtime
                    .get_element_by_id("parser-slot-reparent-new-host")
                    .expect("parser slot reparent new host should exist"),
                runtime
                    .get_element_by_id("parser-slot-reparent-old-target")
                    .expect("parser slot reparent target should exist"),
            )
        };
        let _ = apply_parser_dom_mutation_for_test(
            &mut page_vm,
            ParserDomMutation::InsertBefore {
                parent: parser_reparent_new_host,
                child: parser_reparent_target,
                reference_child: None,
            },
            "parser slot reparent mutation should apply",
        );
        page_vm
            .evaluate_expression("undefined")
            .expect("parser slot reparent slotchange microtask should drain");
        let parser_reparent_log = page_vm
            .evaluate_expression("JSON.stringify(window.parserSlotchangeLog.splice(0))")
            .expect("parser slot reparent log should evaluate");

        assert_eq!(
            js_remove_log
                .get("value")
                .and_then(serde_json::Value::as_str),
            Some(r#"["remove:0"]"#),
            "JS remove baseline should queue one slotchange for removed slotted node"
        );
        assert_eq!(
            parser_remove_log
                .get("value")
                .and_then(serde_json::Value::as_str),
            js_remove_log
                .get("value")
                .and_then(serde_json::Value::as_str),
            "parser remove should queue the same slotchange signal as JS remove"
        );
        assert_eq!(
            js_reparent_log
                .get("value")
                .and_then(serde_json::Value::as_str),
            Some(r#"["old:0","new:1"]"#),
            "JS reparent baseline should queue old then new slotchange signals"
        );
        assert_eq!(
            parser_reparent_log
                .get("value")
                .and_then(serde_json::Value::as_str),
            js_reparent_log
                .get("value")
                .and_then(serde_json::Value::as_str),
            "parser reparent should queue the same slotchange sequence as JS reparent"
        );
    }));
}
#[test]
fn parser_remove_and_reparent_queue_mutation_observer_records_like_js() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
function parserMoRole(node) {
  if (!node) return '';
  return node.dataset && node.dataset.role ? node.dataset.role : node.nodeName;
}
window.parserMoSummarize = function(records) {
  return records.map(record => [
    parserMoRole(record.target),
    Array.from(record.addedNodes).map(parserMoRole).join(','),
    Array.from(record.removedNodes).map(parserMoRole).join(','),
    parserMoRole(record.previousSibling),
    parserMoRole(record.nextSibling)
  ].join(':')).join('|');
};
function installMoRemove(prefix) {
  const parent = document.createElement('div');
  parent.id = `${prefix}-parent`;
  parent.dataset.role = 'remove-parent';
  const child = document.createElement('span');
  child.id = `${prefix}-target`;
  child.dataset.role = 'target';
  parent.appendChild(child);
  document.body.appendChild(parent);
  const observer = new MutationObserver(() => {});
  observer.observe(parent, { childList: true });
  return { parent, child, observer };
}
function installMoReparent(prefix) {
  const oldParent = document.createElement('div');
  oldParent.id = `${prefix}-old-parent`;
  oldParent.dataset.role = 'old-parent';
  const newParent = document.createElement('div');
  newParent.id = `${prefix}-new-parent`;
  newParent.dataset.role = 'new-parent';
  const target = document.createElement('span');
  target.id = `${prefix}-target`;
  target.dataset.role = 'target';
  oldParent.appendChild(target);
  document.body.append(oldParent, newParent);
  const oldObserver = new MutationObserver(() => {});
  const newObserver = new MutationObserver(() => {});
  oldObserver.observe(oldParent, { childList: true });
  newObserver.observe(newParent, { childList: true });
  return { oldParent, newParent, target, oldObserver, newObserver };
}
window.jsMoRemove = installMoRemove('js-mo-remove');
window.parserMoRemove = installMoRemove('parser-mo-remove');
window.jsMoReparent = installMoReparent('js-mo-reparent');
window.parserMoReparent = installMoReparent('parser-mo-reparent');
"#,
                )
                .expect("mutation observer parser mutation setup should evaluate");

            let js_remove_records = page_vm
                .evaluate_expression(
                    r#"(() => {
  window.jsMoRemove.parent.removeChild(window.jsMoRemove.child);
  return window.parserMoSummarize(window.jsMoRemove.observer.takeRecords());
})()"#,
                )
                .expect("JS mutation observer remove baseline should evaluate");

            let (parser_remove_parent, parser_remove_child) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-mo-remove-parent")
                        .expect("parser mutation observer remove parent should exist"),
                    runtime
                        .get_element_by_id("parser-mo-remove-target")
                        .expect("parser mutation observer remove target should exist"),
                )
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: parser_remove_parent,
                    child: parser_remove_child,
                },
                "parser mutation observer remove mutation should apply",
            );
            let parser_remove_records = page_vm
                .evaluate_expression(
                    r#"window.parserMoSummarize(window.parserMoRemove.observer.takeRecords())"#,
                )
                .expect("parser mutation observer remove records should evaluate");

            let js_reparent_records = page_vm
                .evaluate_expression(
                    r#"(() => {
  window.jsMoReparent.newParent.appendChild(window.jsMoReparent.target);
  return JSON.stringify([
    window.parserMoSummarize(window.jsMoReparent.oldObserver.takeRecords()),
    window.parserMoSummarize(window.jsMoReparent.newObserver.takeRecords())
  ]);
})()"#,
                )
                .expect("JS mutation observer reparent baseline should evaluate");

            let (parser_reparent_parent, parser_reparent_child) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-mo-reparent-new-parent")
                        .expect("parser mutation observer reparent parent should exist"),
                    runtime
                        .get_element_by_id("parser-mo-reparent-target")
                        .expect("parser mutation observer reparent target should exist"),
                )
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_reparent_parent,
                    child: parser_reparent_child,
                    reference_child: None,
                },
                "parser mutation observer reparent mutation should apply",
            );
            let parser_reparent_records = page_vm
                .evaluate_expression(
                    r#"JSON.stringify([
  window.parserMoSummarize(window.parserMoReparent.oldObserver.takeRecords()),
  window.parserMoSummarize(window.parserMoReparent.newObserver.takeRecords())
])"#,
                )
                .expect("parser mutation observer reparent records should evaluate");

            assert_eq!(
                js_remove_records
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some("remove-parent::target::"),
                "JS remove baseline should queue one childList removal record"
            );
            assert_eq!(
                parser_remove_records
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                js_remove_records
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                "parser remove should queue the same MutationObserver childList record as JS remove"
            );
            assert_eq!(
                js_reparent_records
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["old-parent::target::","new-parent:target:::"]"#),
                "JS reparent baseline should queue old removal and new insertion records"
            );
            assert_eq!(
                parser_reparent_records
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                js_reparent_records
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                "parser reparent should queue the same MutationObserver childList records as JS reparent"
            );
        }));
}
#[test]
fn parser_document_fragment_insertion_queues_mutation_observer_records_like_js() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
function parserFragmentMoRole(node) {
  if (!node) return '';
  return node.id || node.nodeName;
}
window.parserFragmentMoSummarize = function(records) {
  return records.map(record => [
    parserFragmentMoRole(record.target),
    Array.from(record.addedNodes).map(parserFragmentMoRole).join(','),
    Array.from(record.removedNodes).map(parserFragmentMoRole).join(','),
    parserFragmentMoRole(record.previousSibling),
    parserFragmentMoRole(record.nextSibling)
  ].join(':')).join('|');
};
function parserFragmentMoParent(id) {
  const parent = document.createElement('div');
  parent.id = id;
  document.body.appendChild(parent);
  return parent;
}
const jsAppendFragment = document.createDocumentFragment();
jsAppendFragment.append(
  Object.assign(document.createElement('span'), { id: 'js-fragment-mo-append-a' }),
  Object.assign(document.createElement('span'), { id: 'js-fragment-mo-append-b' })
);
const jsAppendParent = parserFragmentMoParent('js-fragment-mo-append-parent');
const jsAppendObserver = new MutationObserver(() => {});
jsAppendObserver.observe(jsAppendParent, { childList: true });
jsAppendParent.appendChild(jsAppendFragment);
window.parserFragmentMoJsAppend = window.parserFragmentMoSummarize(jsAppendObserver.takeRecords());
window.parserFragmentMoJsAppendEmpty = jsAppendFragment.childNodes.length;

const parserAppendParent = parserFragmentMoParent('parser-fragment-mo-append-parent');
window.parserFragmentMoParserAppendObserver = new MutationObserver(() => {});
window.parserFragmentMoParserAppendObserver.observe(parserAppendParent, { childList: true });

const jsBeforeParent = parserFragmentMoParent('js-fragment-mo-before-parent');
const jsBeforeReference = Object.assign(document.createElement('span'), {
  id: 'js-fragment-mo-before-reference'
});
jsBeforeParent.appendChild(jsBeforeReference);
const jsBeforeFragment = document.createDocumentFragment();
jsBeforeFragment.append(
  Object.assign(document.createElement('span'), { id: 'js-fragment-mo-before-a' }),
  Object.assign(document.createElement('span'), { id: 'js-fragment-mo-before-b' })
);
const jsBeforeObserver = new MutationObserver(() => {});
jsBeforeObserver.observe(jsBeforeParent, { childList: true });
jsBeforeParent.insertBefore(jsBeforeFragment, jsBeforeReference);
window.parserFragmentMoJsBefore = window.parserFragmentMoSummarize(jsBeforeObserver.takeRecords());
window.parserFragmentMoJsBeforeEmpty = jsBeforeFragment.childNodes.length;

const parserBeforeParent = parserFragmentMoParent('parser-fragment-mo-before-parent');
const parserBeforeReference = Object.assign(document.createElement('span'), {
  id: 'parser-fragment-mo-before-reference'
});
parserBeforeParent.appendChild(parserBeforeReference);
"#,
                )
                .expect("fragment MutationObserver setup should evaluate");

            let (
                parser_append_parent,
                parser_append_fragment,
                parser_before_parent,
                parser_before_fragment,
                parser_before_reference,
            ) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let parser_append_parent = dom_host
                    .element_handle_by_id("parser-fragment-mo-append-parent")
                    .expect("parser fragment MutationObserver append parent should exist");
                let parser_append_fragment = dom_host.create_document_fragment();
                let parser_append_a = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    parser_append_a,
                    "id",
                    "parser-fragment-mo-append-a"
                ));
                let parser_append_b = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    parser_append_b,
                    "id",
                    "parser-fragment-mo-append-b"
                ));
                assert!(dom_host.append_child(parser_append_fragment, parser_append_a));
                assert!(dom_host.append_child(parser_append_fragment, parser_append_b));

                let parser_before_parent = dom_host
                    .element_handle_by_id("parser-fragment-mo-before-parent")
                    .expect("parser fragment MutationObserver before parent should exist");
                let parser_before_fragment = dom_host.create_document_fragment();
                let parser_before_a = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    parser_before_a,
                    "id",
                    "parser-fragment-mo-before-a"
                ));
                let parser_before_b = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    parser_before_b,
                    "id",
                    "parser-fragment-mo-before-b"
                ));
                assert!(dom_host.append_child(parser_before_fragment, parser_before_a));
                assert!(dom_host.append_child(parser_before_fragment, parser_before_b));

                let parser_before_reference = dom_host
                    .element_handle_by_id("parser-fragment-mo-before-reference")
                    .expect("parser fragment MutationObserver reference should exist");
                (
                    parser_append_parent,
                    parser_append_fragment,
                    parser_before_parent,
                    parser_before_fragment,
                    parser_before_reference,
                )
            };

            let append_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_append_parent,
                    child: parser_append_fragment,
                },
                "parser fragment MutationObserver append should apply",
            );
            assert!(
                append_reaction_roots.is_empty(),
                "plain fragment append should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(parser_append_fragment)
                    .count(),
                0,
                "parser fragment append should hoist and empty the fragment"
            );
            let parser_append_records = page_vm
                .evaluate_expression(
                    r#"window.parserFragmentMoSummarize(
  window.parserFragmentMoParserAppendObserver.takeRecords()
)"#,
                )
                .expect("parser fragment MutationObserver append records should evaluate");

            page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentMoParserBeforeObserver = new MutationObserver(() => {});
window.parserFragmentMoParserBeforeObserver.observe(
  document.getElementById('parser-fragment-mo-before-parent'),
  { childList: true }
);
"#,
                )
                .expect("parser fragment MutationObserver before observer should install");

            let before_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_before_parent,
                    child: parser_before_fragment,
                    reference_child: Some(parser_before_reference),
                },
                "parser fragment MutationObserver insertBefore should apply",
            );
            assert!(
                before_reaction_roots.is_empty(),
                "plain fragment insertBefore should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(parser_before_fragment)
                    .count(),
                0,
                "parser fragment insertBefore should hoist and empty the fragment"
            );
            let parser_before_records = page_vm
                .evaluate_expression(
                    r#"window.parserFragmentMoSummarize(
  window.parserFragmentMoParserBeforeObserver.takeRecords()
)"#,
                )
                .expect("parser fragment MutationObserver insertBefore records should evaluate");

            let js_records = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsAppend: window.parserFragmentMoJsAppend,
  jsAppendEmpty: window.parserFragmentMoJsAppendEmpty,
  jsBefore: window.parserFragmentMoJsBefore,
  jsBeforeEmpty: window.parserFragmentMoJsBeforeEmpty
})"#,
                )
                .expect("JS fragment MutationObserver records should evaluate");

            let parser_append = parser_append_records
                .get("value")
                .and_then(serde_json::Value::as_str)
                .expect("parser append records should be string");
            let parser_before = parser_before_records
                .get("value")
                .and_then(serde_json::Value::as_str)
                .expect("parser insertBefore records should be string");
            let js_records = js_records
                .get("value")
                .and_then(serde_json::Value::as_str)
                .expect("JS fragment records should be string");
            let js_records: serde_json::Value =
                serde_json::from_str(js_records).expect("JS fragment records should parse");
            let normalize = |value: &str| {
                value
                    .replace("js-fragment-mo", "fragment-mo")
                    .replace("parser-fragment-mo", "fragment-mo")
            };
            assert_eq!(
                js_records.get("jsAppendEmpty").and_then(serde_json::Value::as_u64),
                Some(0),
                "JS append baseline should empty the inserted DocumentFragment"
            );
            assert_eq!(
                js_records.get("jsBeforeEmpty").and_then(serde_json::Value::as_u64),
                Some(0),
                "JS insertBefore baseline should empty the inserted DocumentFragment"
            );
            assert_eq!(
                normalize(
                    js_records
                        .get("jsAppend")
                        .and_then(serde_json::Value::as_str)
                        .expect("JS append records should be string")
                ),
                normalize(parser_append),
                "parser DocumentFragment append should queue MutationObserver records like JS appendChild"
            );
            assert_eq!(
                normalize(
                    js_records
                        .get("jsBefore")
                        .and_then(serde_json::Value::as_str)
                        .expect("JS insertBefore records should be string")
                ),
                normalize(parser_before),
                "parser DocumentFragment insertBefore should queue MutationObserver records like JS insertBefore"
            );
        }));
}
#[test]
fn parser_remove_open_popover_dispatches_forced_close_events_like_js() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            let setup = page_vm
                .evaluate_expression(
                    r#"
window.parserPopoverRemoveEvents = [];
function installOpenPopover(prefix) {
  const parent = document.createElement('div');
  parent.id = `${prefix}-parent`;
  const popover = document.createElement('div');
  popover.id = `${prefix}-popover`;
  popover.popover = 'manual';
  popover.addEventListener('beforetoggle', event => {
    window.parserPopoverRemoveEvents.push(
      `${prefix}:${event.type}:${event.oldState}->${event.newState}:${event.cancelable}:${popover.matches(':popover-open')}`
    );
  });
  popover.addEventListener('toggle', event => {
    window.parserPopoverRemoveEvents.push(
      `${prefix}:${event.type}:${event.oldState}->${event.newState}:${event.cancelable}:${popover.matches(':popover-open')}`
    );
  });
  parent.appendChild(popover);
  document.body.appendChild(parent);
  popover.showPopover();
  return { parent, popover };
}
window.jsPopoverRemove = installOpenPopover('js');
window.parserPopoverRemove = installOpenPopover('parser');
JSON.stringify([
  window.jsPopoverRemove.popover.matches(':popover-open'),
  window.parserPopoverRemove.popover.matches(':popover-open')
])
"#,
                )
                .expect("popover removal parser mutation setup should evaluate");
            assert_eq!(
                setup.get("value").and_then(serde_json::Value::as_str),
                Some("[true,true]"),
                "both JS and parser popover removal targets should start open"
            );

            let loader = page_vm.main_document_resource_loader();
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                2,
                "initial JS/parser popover show tasks should run",
            )
            .await;
            page_vm
                .evaluate_expression("window.parserPopoverRemoveEvents = []")
                .expect("popover removal event log reset should evaluate");

            let js_sync = page_vm
                .evaluate_expression(
                    r#"(() => {
  window.jsPopoverRemove.parent.removeChild(window.jsPopoverRemove.popover);
  return JSON.stringify([
    window.parserPopoverRemoveEvents.splice(0),
    window.jsPopoverRemove.popover.matches(':popover-open')
  ]);
})()"#,
                )
                .expect("JS popover removal baseline should evaluate");
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                1,
                "JS popover removal toggle task should run",
            )
            .await;
            let js_after_task = page_vm
                .evaluate_expression("JSON.stringify(window.parserPopoverRemoveEvents.splice(0))")
                .expect("JS popover removal task events should evaluate");

            let (parser_parent, parser_popover) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-parent")
                        .expect("parser popover parent should exist"),
                    runtime
                        .get_element_by_id("parser-popover")
                        .expect("parser popover should exist"),
                )
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: parser_parent,
                    child: parser_popover,
                },
                "parser popover removal mutation should apply",
            );
            let parser_sync = page_vm
                .evaluate_expression(
                    r#"JSON.stringify([
  window.parserPopoverRemoveEvents.splice(0),
  window.parserPopoverRemove.popover.matches(':popover-open')
])"#,
                )
                .expect("parser popover removal sync events should evaluate");
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                1,
                "parser popover removal toggle task should run",
            )
            .await;
            let parser_after_task = page_vm
                .evaluate_expression("JSON.stringify(window.parserPopoverRemoveEvents.splice(0))")
                .expect("parser popover removal task events should evaluate");

            assert_eq!(
                js_sync.get("value").and_then(serde_json::Value::as_str),
                Some(r#"[["js:beforetoggle:open->closed:false:false"],false]"#),
                "JS removal baseline should synchronously dispatch beforetoggle and clear open state"
            );
            assert_eq!(
                parser_sync
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"[["parser:beforetoggle:open->closed:false:false"],false]"#),
                "parser removal should synchronously dispatch beforetoggle and clear open state"
            );
            assert_eq!(
                js_after_task
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["js:toggle:open->closed:false:false"]"#),
                "JS removal baseline should queue a close toggle event"
            );
            assert_eq!(
                parser_after_task
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["parser:toggle:open->closed:false:false"]"#),
                "parser removal should queue the same close toggle event"
            );
        }));
}
#[test]
fn parser_reparent_open_popover_to_disconnected_parent_dispatches_forced_close_events_like_js() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            let setup = page_vm
                .evaluate_expression(
                    r#"
window.parserPopoverReparentEvents = [];
function installOpenPopoverReparent(prefix) {
  const parent = document.createElement('div');
  parent.id = `${prefix}-reparent-parent`;
  const detachedParent = document.createElement('div');
  detachedParent.id = `${prefix}-reparent-detached-parent`;
  const popover = document.createElement('div');
  popover.id = `${prefix}-reparent-popover`;
  popover.popover = 'manual';
  popover.addEventListener('beforetoggle', event => {
    window.parserPopoverReparentEvents.push(
      `${prefix}:${event.type}:${event.oldState}->${event.newState}:${event.cancelable}:${popover.matches(':popover-open')}`
    );
  });
  popover.addEventListener('toggle', event => {
    window.parserPopoverReparentEvents.push(
      `${prefix}:${event.type}:${event.oldState}->${event.newState}:${event.cancelable}:${popover.matches(':popover-open')}`
    );
  });
  parent.appendChild(popover);
  document.body.appendChild(parent);
  popover.showPopover();
  return { parent, detachedParent, popover };
}
window.jsPopoverReparent = installOpenPopoverReparent('js');
window.parserPopoverReparent = installOpenPopoverReparent('parser');
JSON.stringify([
  window.jsPopoverReparent.popover.matches(':popover-open'),
  window.parserPopoverReparent.popover.matches(':popover-open')
])
"#,
                )
                .expect("popover reparent parser mutation setup should evaluate");
            assert_eq!(
                setup.get("value").and_then(serde_json::Value::as_str),
                Some("[true,true]"),
                "both JS and parser popover reparent targets should start open"
            );

            let loader = page_vm.main_document_resource_loader();
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                2,
                "initial JS/parser popover reparent show tasks should run",
            )
            .await;
            page_vm
                .evaluate_expression("window.parserPopoverReparentEvents = []")
                .expect("popover reparent event log reset should evaluate");

            let js_sync = page_vm
                .evaluate_expression(
                    r#"(() => {
  window.jsPopoverReparent.detachedParent.appendChild(window.jsPopoverReparent.popover);
  return JSON.stringify([
    window.parserPopoverReparentEvents.splice(0),
    window.jsPopoverReparent.popover.matches(':popover-open'),
    window.jsPopoverReparent.popover.parentNode === window.jsPopoverReparent.detachedParent
  ]);
})()"#,
                )
                .expect("JS popover reparent baseline should evaluate");
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                1,
                "JS popover reparent toggle task should run",
            )
            .await;
            let js_after_task = page_vm
                .evaluate_expression("JSON.stringify(window.parserPopoverReparentEvents.splice(0))")
                .expect("JS popover reparent task events should evaluate");

            let parser_popover = {
                let runtime = &page_vm.vm().document_runtime;
                runtime
                    .get_element_by_id("parser-reparent-popover")
                    .expect("parser popover should exist")
            };
            let parser_detached_parent = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    detached_parent,
                    "id",
                    "parser-reparent-native-detached-parent"
                ));
                detached_parent
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_detached_parent,
                    child: parser_popover,
                },
                "parser popover reparent mutation should apply",
            );
            let parser_sync = page_vm
                .evaluate_expression(
                    r#"JSON.stringify([
  window.parserPopoverReparentEvents.splice(0),
  window.parserPopoverReparent.popover.matches(':popover-open')
])"#,
                )
                .expect("parser popover reparent sync events should evaluate");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .node(parser_popover)
                    .and_then(Node::parent_node),
                Some(parser_detached_parent),
                "parser reparent should move the popover under the native detached parent"
            );
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                1,
                "parser popover reparent toggle task should run",
            )
            .await;
            let parser_after_task = page_vm
                .evaluate_expression("JSON.stringify(window.parserPopoverReparentEvents.splice(0))")
                .expect("parser popover reparent task events should evaluate");

            assert_eq!(
                js_sync.get("value").and_then(serde_json::Value::as_str),
                Some(r#"[["js:beforetoggle:open->closed:false:false"],false,true]"#),
                "JS reparent to a disconnected parent should synchronously force-close the popover"
            );
            assert_eq!(
                parser_sync
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"[["parser:beforetoggle:open->closed:false:false"],false]"#),
                "parser reparent to a disconnected parent should synchronously force-close the popover"
            );
            assert_eq!(
                js_after_task
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["js:toggle:open->closed:false:false"]"#),
                "JS reparent baseline should queue a close toggle event"
            );
            assert_eq!(
                parser_after_task
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["parser:toggle:open->closed:false:false"]"#),
                "parser reparent should queue the same close toggle event"
            );
        }));
}
#[test]
fn parser_insert_before_open_popover_reparent_forces_close_like_js() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            let setup = page_vm
                .evaluate_expression(
                    r#"
window.parserPopoverBeforeEvents = [];
function installOpenPopoverBefore(prefix) {
  const parent = document.createElement('div');
  parent.id = `${prefix}-before-parent`;
  const detachedParent = document.createElement('div');
  detachedParent.id = `${prefix}-before-detached-parent`;
  const reference = document.createElement('span');
  reference.id = `${prefix}-before-reference`;
  detachedParent.appendChild(reference);
  const popover = document.createElement('div');
  popover.id = `${prefix}-before-popover`;
  popover.popover = 'manual';
  popover.addEventListener('beforetoggle', event => {
    window.parserPopoverBeforeEvents.push(
      `${prefix}:${event.type}:${event.oldState}->${event.newState}:${event.cancelable}:${popover.matches(':popover-open')}`
    );
  });
  popover.addEventListener('toggle', event => {
    window.parserPopoverBeforeEvents.push(
      `${prefix}:${event.type}:${event.oldState}->${event.newState}:${event.cancelable}:${popover.matches(':popover-open')}`
    );
  });
  parent.appendChild(popover);
  document.body.appendChild(parent);
  popover.showPopover();
  return { parent, detachedParent, reference, popover };
}
window.jsPopoverBefore = installOpenPopoverBefore('js');
window.parserPopoverBefore = installOpenPopoverBefore('parser');
JSON.stringify([
  window.jsPopoverBefore.popover.matches(':popover-open'),
  window.parserPopoverBefore.popover.matches(':popover-open')
])
"#,
                )
                .expect("popover insertBefore parser mutation setup should evaluate");
            assert_eq!(
                setup.get("value").and_then(serde_json::Value::as_str),
                Some("[true,true]"),
                "both JS and parser popover insertBefore targets should start open"
            );

            let loader = page_vm.main_document_resource_loader();
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                2,
                "initial JS/parser insertBefore popover show tasks should run",
            )
            .await;
            page_vm
                .evaluate_expression("window.parserPopoverBeforeEvents = []")
                .expect("popover insertBefore event log reset should evaluate");

            let js_sync = page_vm
                .evaluate_expression(
                    r#"(() => {
  window.jsPopoverBefore.detachedParent.insertBefore(
    window.jsPopoverBefore.popover,
    window.jsPopoverBefore.reference
  );
  return JSON.stringify([
    window.parserPopoverBeforeEvents.splice(0),
    window.jsPopoverBefore.popover.matches(':popover-open'),
    window.jsPopoverBefore.popover.parentNode === window.jsPopoverBefore.detachedParent,
    window.jsPopoverBefore.popover.nextSibling === window.jsPopoverBefore.reference
  ]);
})()"#,
                )
                .expect("JS popover insertBefore baseline should evaluate");
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                1,
                "JS popover insertBefore toggle task should run",
            )
            .await;
            let js_after_task = page_vm
                .evaluate_expression("JSON.stringify(window.parserPopoverBeforeEvents.splice(0))")
                .expect("JS popover insertBefore task events should evaluate");

            let parser_popover = {
                let runtime = &page_vm.vm().document_runtime;
                runtime
                    .get_element_by_id("parser-before-popover")
                    .expect("parser popover should exist")
            };
            let (parser_detached_parent, parser_reference) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.append_child(detached_parent, reference));
                (detached_parent, reference)
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_detached_parent,
                    child: parser_popover,
                    reference_child: Some(parser_reference),
                },
                "parser popover insertBefore mutation should apply",
            );
            let parser_sync = page_vm
                .evaluate_expression(
                    r#"JSON.stringify([
  window.parserPopoverBeforeEvents.splice(0),
  window.parserPopoverBefore.popover.matches(':popover-open')
])"#,
                )
                .expect("parser popover insertBefore sync events should evaluate");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(parser_detached_parent)
                    .collect::<Vec<_>>(),
                vec![parser_popover, parser_reference],
                "parser insertBefore should move the popover before the native reference child"
            );
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                1,
                "parser popover insertBefore toggle task should run",
            )
            .await;
            let parser_after_task = page_vm
                .evaluate_expression("JSON.stringify(window.parserPopoverBeforeEvents.splice(0))")
                .expect("parser popover insertBefore task events should evaluate");

            assert_eq!(
                js_sync.get("value").and_then(serde_json::Value::as_str),
                Some(r#"[["js:beforetoggle:open->closed:false:false"],false,true,true]"#),
                "JS insertBefore to a disconnected parent should synchronously force-close the popover"
            );
            assert_eq!(
                parser_sync
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"[["parser:beforetoggle:open->closed:false:false"],false]"#),
                "parser insertBefore to a disconnected parent should synchronously force-close the popover"
            );
            assert_eq!(
                js_after_task
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["js:toggle:open->closed:false:false"]"#),
                "JS insertBefore baseline should queue a close toggle event"
            );
            assert_eq!(
                parser_after_task
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["parser:toggle:open->closed:false:false"]"#),
                "parser insertBefore should queue the same close toggle event"
            );
        }));
}
#[test]
fn parser_remove_and_reparent_selected_subtrees_match_js_selection_ranges() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserSelectionStates = {};
function selectionState(targetText) {
  const selection = getSelection();
  const range = selection.rangeCount ? selection.getRangeAt(0) : null;
  const nodeLabel = node => {
    if (node === null) return null;
    if (node === targetText) return 'target-text';
    if (node.nodeType === Node.TEXT_NODE) {
      return `${node.parentNode && node.parentNode.id}#text`;
    }
    return node.id || node.nodeName;
  };
  return {
    rangeCount: selection.rangeCount,
    anchor: nodeLabel(selection.anchorNode),
    anchorOffset: selection.anchorOffset,
    focus: nodeLabel(selection.focusNode),
    focusOffset: selection.focusOffset,
    start: range ? nodeLabel(range.startContainer) : null,
    startOffset: range ? range.startOffset : null,
    end: range ? nodeLabel(range.endContainer) : null,
    endOffset: range ? range.endOffset : null,
    text: selection.toString()
  };
}

const removeParent = document.createElement('div');
removeParent.id = 'parser-selection-remove-parent';
const removeJs = document.createElement('span');
removeJs.id = 'js-selection-remove-target';
removeJs.textContent = 'remove';
const removeParser = document.createElement('span');
removeParser.id = 'parser-selection-remove-target';
removeParser.textContent = 'remove';
removeParent.append(removeJs, removeParser);

const moveParent = document.createElement('div');
moveParent.id = 'parser-selection-move-parent';
const moveDest = document.createElement('div');
moveDest.id = 'parser-selection-move-dest';
const moveJs = document.createElement('span');
moveJs.id = 'js-selection-move-target';
moveJs.textContent = 'move';
const moveParser = document.createElement('span');
moveParser.id = 'parser-selection-move-target';
moveParser.textContent = 'move';
moveParent.append(moveJs, moveParser);
document.body.append(removeParent, moveParent, moveDest);
window.parserSelectionRemoveText = removeParser.firstChild;
window.parserSelectionMoveText = moveParser.firstChild;

const selection = getSelection();
selection.setBaseAndExtent(removeJs.firstChild, 0, removeJs.firstChild, removeJs.firstChild.data.length);
removeParent.removeChild(removeJs);
window.parserSelectionStates.jsRemove = selectionState(removeJs.firstChild);

selection.setBaseAndExtent(moveJs.firstChild, 0, moveJs.firstChild, moveJs.firstChild.data.length);
moveDest.insertBefore(moveJs, null);
window.parserSelectionStates.jsMove = selectionState(moveJs.firstChild);

selection.setBaseAndExtent(removeParser.firstChild, 0, removeParser.firstChild, removeParser.firstChild.data.length);
"#,
                )
                .expect("selection setup should evaluate");

            let (remove_parent, remove_target, move_parent, move_target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-selection-remove-parent")
                        .expect("parser remove parent should exist"),
                    runtime
                        .get_element_by_id("parser-selection-remove-target")
                        .expect("parser remove target should exist"),
                    runtime
                        .get_element_by_id("parser-selection-move-dest")
                        .expect("parser move destination should exist"),
                    runtime
                        .get_element_by_id("parser-selection-move-target")
                        .expect("parser move target should exist"),
                )
            };

            let remove_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::RemoveChild {
                        parent: remove_parent,
                        child: remove_target,
                    },
                    "parser remove selected subtree should apply",
                )
            };
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(remove_roots)
                .expect("parser remove followups should dispatch");
            page_vm
                .evaluate_expression(
                    r#"(() => {
window.parserSelectionStates.parserRemove = selectionState(window.parserSelectionRemoveText);
const parserMove = document.getElementById('parser-selection-move-target');
const selection = getSelection();
selection.setBaseAndExtent(parserMove.firstChild, 0, parserMove.firstChild, parserMove.firstChild.data.length);
})()"#,
                )
                .expect("parser remove selection state should evaluate");

            let move_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: move_parent,
                        child: move_target,
                        reference_child: None,
                    },
                    "parser reparent selected subtree should apply",
                )
            };
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(move_roots)
                .expect("parser move followups should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"
window.parserSelectionStates.parserMove = selectionState(window.parserSelectionMoveText);
JSON.stringify(window.parserSelectionStates)
"#,
                )
                .expect("selection comparison result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsRemove":{"rangeCount":1,"anchor":"target-text","anchorOffset":0,"focus":"target-text","focusOffset":6,"start":"parser-selection-remove-parent","startOffset":0,"end":"parser-selection-remove-parent","endOffset":0,"text":""},"jsMove":{"rangeCount":1,"anchor":"target-text","anchorOffset":0,"focus":"target-text","focusOffset":4,"start":"parser-selection-move-parent","startOffset":0,"end":"parser-selection-move-parent","endOffset":0,"text":""},"parserRemove":{"rangeCount":1,"anchor":"target-text","anchorOffset":0,"focus":"target-text","focusOffset":6,"start":"parser-selection-remove-parent","startOffset":0,"end":"parser-selection-remove-parent","endOffset":0,"text":""},"parserMove":{"rangeCount":1,"anchor":"target-text","anchorOffset":0,"focus":"target-text","focusOffset":4,"start":"parser-selection-move-parent","startOffset":0,"end":"parser-selection-move-parent","endOffset":0,"text":""}}"#
                ),
                "parser remove/reparent should update the selected live range like JS DOM mutation"
            );
        }));
}
#[test]
fn parser_textarea_child_list_mutations_reset_selection_like_js() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserTextareaSelectionStates = {};
function textareaState(textarea) {
  return {
    value: textarea.value,
    text: textarea.textContent,
    start: textarea.selectionStart,
    end: textarea.selectionEnd
  };
}
function makeTextarea(id, text, start, end) {
  const textarea = document.createElement('textarea');
  textarea.id = id;
  textarea.textContent = text;
  document.body.appendChild(textarea);
  textarea.setSelectionRange(start, end);
  return textarea;
}

const jsAppend = makeTextarea('js-textarea-append', 'abc', 2, 3);
const parserAppend = makeTextarea('parser-textarea-append', 'abc', 2, 3);
jsAppend.appendChild(document.createTextNode('d'));
window.parserTextareaSelectionStates.jsAppend = textareaState(jsAppend);

const jsBefore = makeTextarea('js-textarea-before', 'bc', 1, 2);
const parserBefore = makeTextarea('parser-textarea-before', 'bc', 1, 2);
jsBefore.insertBefore(document.createTextNode('a'), jsBefore.firstChild);
window.parserTextareaSelectionStates.jsInsertBefore = textareaState(jsBefore);

const jsRemove = makeTextarea('js-textarea-remove', 'abc', 2, 3);
const parserRemove = makeTextarea('parser-textarea-remove', 'abc', 2, 3);
jsRemove.removeChild(jsRemove.firstChild);
window.parserTextareaSelectionStates.jsRemove = textareaState(jsRemove);
"#,
                )
                .expect("textarea selection setup should evaluate");

            let (parser_append, parser_before, parser_remove) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-textarea-append")
                        .expect("parser append textarea should exist"),
                    runtime
                        .get_element_by_id("parser-textarea-before")
                        .expect("parser insertBefore textarea should exist"),
                    runtime
                        .get_element_by_id("parser-textarea-remove")
                        .expect("parser remove textarea should exist"),
                )
            };
            let (parser_before_reference, parser_remove_child) = {
                let dom_host = page_vm.vm().document_runtime.dom_host();
                (
                    dom_host
                        .child_handles(parser_before)
                        .next()
                        .expect("parser insertBefore textarea should have a text child"),
                    dom_host
                        .child_handles(parser_remove)
                        .next()
                        .expect("parser remove textarea should have a text child"),
                )
            };

            let (parser_append_text, parser_before_text) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                (
                    dom_host.create_text_node("d"),
                    dom_host.create_text_node("a"),
                )
            };

            let append_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_append,
                    child: parser_append_text,
                },
                "parser textarea append should apply",
            );
            assert!(
                append_reaction_roots.is_empty(),
                "plain textarea text append should not queue custom element reactions"
            );
            let before_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_before,
                    child: parser_before_text,
                    reference_child: Some(parser_before_reference),
                },
                "parser textarea insertBefore should apply",
            );
            assert!(
                before_reaction_roots.is_empty(),
                "plain textarea text insertBefore should not queue custom element reactions"
            );
            let remove_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: parser_remove,
                    child: parser_remove_child,
                },
                "parser textarea remove should apply",
            );
            if !remove_reaction_roots.is_empty() {
                page_vm
                    .vm_mut()
                    .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(remove_reaction_roots)
                    .expect("parser textarea remove followups should dispatch");
            }

            let result = page_vm
                .evaluate_expression(
                    r#"
window.parserTextareaSelectionStates.parserAppend =
  textareaState(document.getElementById('parser-textarea-append'));
window.parserTextareaSelectionStates.parserInsertBefore =
  textareaState(document.getElementById('parser-textarea-before'));
window.parserTextareaSelectionStates.parserRemove =
  textareaState(document.getElementById('parser-textarea-remove'));
JSON.stringify({
  jsAppend: window.parserTextareaSelectionStates.jsAppend,
  parserAppend: window.parserTextareaSelectionStates.parserAppend,
  appendSame: JSON.stringify(window.parserTextareaSelectionStates.jsAppend) ===
    JSON.stringify(window.parserTextareaSelectionStates.parserAppend),
  jsInsertBefore: window.parserTextareaSelectionStates.jsInsertBefore,
  parserInsertBefore: window.parserTextareaSelectionStates.parserInsertBefore,
  insertBeforeSame: JSON.stringify(window.parserTextareaSelectionStates.jsInsertBefore) ===
    JSON.stringify(window.parserTextareaSelectionStates.parserInsertBefore),
  jsRemove: window.parserTextareaSelectionStates.jsRemove,
  parserRemove: window.parserTextareaSelectionStates.parserRemove,
  removeSame: JSON.stringify(window.parserTextareaSelectionStates.jsRemove) ===
    JSON.stringify(window.parserTextareaSelectionStates.parserRemove)
})
"#,
                )
                .expect("textarea selection result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppend":{"value":"abcd","text":"abcd","start":0,"end":0},"parserAppend":{"value":"abcd","text":"abcd","start":0,"end":0},"appendSame":true,"jsInsertBefore":{"value":"abc","text":"abc","start":0,"end":0},"parserInsertBefore":{"value":"abc","text":"abc","start":0,"end":0},"insertBeforeSame":true,"jsRemove":{"value":"","text":"","start":0,"end":0},"parserRemove":{"value":"","text":"","start":0,"end":0},"removeSame":true}"#
                ),
                "parser textarea append/insertBefore/remove should reset non-dirty selection like JS child-list mutations"
            );
        }));
}
#[test]
fn parser_inserted_nonce_attribute_is_hidden_like_js_insertion() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
function nonceState(id, referenceId) {
  const element = document.getElementById(id);
  return {
    attr: element.getAttribute('nonce'),
    nonce: element.nonce,
    parent: element.parentNode && element.parentNode.nodeName,
    beforeReference: referenceId ? element.nextSibling === document.getElementById(referenceId) : null
  };
}

const jsAppend = document.createElement('script');
jsAppend.id = 'js-nonce-append';
jsAppend.setAttribute('nonce', 'nonce-secret');
document.body.appendChild(jsAppend);

const jsBefore = document.createElement('script');
jsBefore.id = 'js-nonce-before';
jsBefore.setAttribute('nonce', 'nonce-secret');
const jsReference = document.createElement('span');
jsReference.id = 'js-nonce-reference';
document.body.appendChild(jsReference);
document.body.insertBefore(jsBefore, jsReference);

const parserReference = document.createElement('span');
parserReference.id = 'parser-nonce-reference';
document.body.appendChild(parserReference);

window.parserNonceInsertionStates = {
  jsAppend: nonceState('js-nonce-append', null),
  jsInsertBefore: nonceState('js-nonce-before', 'js-nonce-reference')
};
"#,
                )
                .expect("nonce insertion JS baseline should evaluate");

            let (parser_append, parser_before, parser_external, parser_reference) = {
                let runtime = &mut page_vm.vm_mut().document_runtime;
                let parser_reference = runtime
                    .get_element_by_id("parser-nonce-reference")
                    .expect("parser nonce reference should exist");
                let dom_host = runtime.dom_host_mut();
                let parser_append = dom_host.create_parser_element_without_attributes(
                    "script".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(parser_append, "id", "parser-nonce-append"));
                assert!(dom_host.set_attribute(parser_append, "nonce", "nonce-secret"));
                let parser_before = dom_host.create_parser_element_without_attributes(
                    "script".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(parser_before, "id", "parser-nonce-before"));
                assert!(dom_host.set_attribute(parser_before, "nonce", "nonce-secret"));
                let parser_external = dom_host.create_parser_element_without_attributes(
                    "script".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(parser_external, "id", "parser-nonce-external"));
                assert!(dom_host.set_attribute(parser_external, "nonce", "nonce-secret"));
                assert!(dom_host.set_attribute(parser_external, "src", "/parser-nonce.js"));
                (
                    parser_append,
                    parser_before,
                    parser_external,
                    parser_reference,
                )
            };

            let append_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: parser_append,
                    },
                    "parser nonce append should apply",
                )
            };
            assert!(
                append_roots.is_empty(),
                "plain script nonce append should not queue custom element reactions"
            );
            let insert_before_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: parser_before,
                        reference_child: Some(parser_reference),
                    },
                    "parser nonce insertBefore should apply",
                )
            };
            assert!(
                insert_before_roots.is_empty(),
                "plain script nonce insertBefore should not queue custom element reactions"
            );
            let external_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: parser_external,
                    },
                    "parser external nonce append should apply",
                )
            };
            assert!(
                external_roots.is_empty(),
                "plain external script nonce append should not queue custom element reactions"
            );
            assert!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .node(parser_external)
                    .and_then(|node| node.as_element())
                    .is_some_and(|element| !element.script_already_started()),
                "parser-created external nonce script should remain startable for parser handoff"
            );

            let result = page_vm
                .evaluate_expression(
                    r#"
window.parserNonceInsertionStates.parserAppend =
  nonceState('parser-nonce-append', null);
window.parserNonceInsertionStates.parserInsertBefore =
  nonceState('parser-nonce-before', 'parser-nonce-reference');
JSON.stringify({
  jsAppend: window.parserNonceInsertionStates.jsAppend,
  parserAppend: window.parserNonceInsertionStates.parserAppend,
  appendSame: JSON.stringify(window.parserNonceInsertionStates.jsAppend) ===
    JSON.stringify(window.parserNonceInsertionStates.parserAppend),
  jsInsertBefore: window.parserNonceInsertionStates.jsInsertBefore,
  parserInsertBefore: window.parserNonceInsertionStates.parserInsertBefore,
  insertBeforeSame: JSON.stringify(window.parserNonceInsertionStates.jsInsertBefore) ===
    JSON.stringify(window.parserNonceInsertionStates.parserInsertBefore)
})
"#,
                )
                .expect("nonce insertion comparison should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppend":{"attr":"","nonce":"nonce-secret","parent":"BODY","beforeReference":null},"parserAppend":{"attr":"","nonce":"nonce-secret","parent":"BODY","beforeReference":null},"appendSame":true,"jsInsertBefore":{"attr":"","nonce":"nonce-secret","parent":"BODY","beforeReference":true},"parserInsertBefore":{"attr":"","nonce":"nonce-secret","parent":"BODY","beforeReference":true},"insertBeforeSame":true}"#
                ),
                "parser insertion should hide nonce content attributes like JS insertion"
            );
        }));
}
#[test]
fn parser_document_fragment_inserted_nonce_subtree_is_hidden_like_js_insertion() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
function nonceSubtreeState(scriptId, containerId, referenceId) {
  const script = document.getElementById(scriptId);
  const container = document.getElementById(containerId);
  return {
    attr: script.getAttribute('nonce'),
    nonce: script.nonce,
    parent: script.parentNode && script.parentNode.nodeName,
    containerParent: container.parentNode && container.parentNode.nodeName,
    containerBeforeReference: referenceId ? container.nextSibling === document.getElementById(referenceId) : null
  };
}
function nonceFragment(containerId, scriptId) {
  const fragment = document.createDocumentFragment();
  const container = document.createElement('div');
  container.id = containerId;
  const script = document.createElement('script');
  script.id = scriptId;
  script.setAttribute('nonce', 'nonce-secret');
  container.appendChild(script);
  fragment.appendChild(container);
  return fragment;
}

const jsAppendFragment = nonceFragment('js-fragment-nonce-append-container', 'js-fragment-nonce-append');
document.body.appendChild(jsAppendFragment);

const jsBeforeReference = document.createElement('span');
jsBeforeReference.id = 'js-fragment-nonce-reference';
document.body.appendChild(jsBeforeReference);
const jsBeforeFragment = nonceFragment('js-fragment-nonce-before-container', 'js-fragment-nonce-before');
document.body.insertBefore(jsBeforeFragment, jsBeforeReference);

const parserBeforeReference = document.createElement('span');
parserBeforeReference.id = 'parser-fragment-nonce-reference';
document.body.appendChild(parserBeforeReference);

window.parserFragmentNonceStates = {
  jsAppend: nonceSubtreeState(
    'js-fragment-nonce-append',
    'js-fragment-nonce-append-container',
    null
  ),
  jsInsertBefore: nonceSubtreeState(
    'js-fragment-nonce-before',
    'js-fragment-nonce-before-container',
    'js-fragment-nonce-reference'
  )
};
"#,
                )
                .expect("fragment nonce JS baseline should evaluate");

            let (append_fragment, before_fragment, before_reference) = {
                let runtime = &mut page_vm.vm_mut().document_runtime;
                let before_reference = runtime
                    .get_element_by_id("parser-fragment-nonce-reference")
                    .expect("parser fragment nonce reference should exist");
                let dom_host = runtime.dom_host_mut();

                let append_fragment = dom_host.create_document_fragment();
                let append_container = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    append_container,
                    "id",
                    "parser-fragment-nonce-append-container"
                ));
                let append_script = dom_host.create_parser_element_without_attributes(
                    "script".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    append_script,
                    "id",
                    "parser-fragment-nonce-append"
                ));
                assert!(dom_host.set_attribute(append_script, "nonce", "nonce-secret"));
                assert!(dom_host.append_child(append_container, append_script));
                assert!(dom_host.append_child(append_fragment, append_container));

                let before_fragment = dom_host.create_document_fragment();
                let before_container = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    before_container,
                    "id",
                    "parser-fragment-nonce-before-container"
                ));
                let before_script = dom_host.create_parser_element_without_attributes(
                    "script".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    before_script,
                    "id",
                    "parser-fragment-nonce-before"
                ));
                assert!(dom_host.set_attribute(before_script, "nonce", "nonce-secret"));
                assert!(dom_host.append_child(before_container, before_script));
                assert!(dom_host.append_child(before_fragment, before_container));

                (append_fragment, before_fragment, before_reference)
            };

            let append_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: body,
                    child: append_fragment,
                },
                "parser fragment nonce append should apply",
            );
            assert!(
                append_roots.is_empty(),
                "plain fragment nonce append should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(append_fragment)
                    .count(),
                0,
                "parser fragment nonce append should hoist and empty the fragment"
            );

            let before_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: body,
                    child: before_fragment,
                    reference_child: Some(before_reference),
                },
                "parser fragment nonce insertBefore should apply",
            );
            assert!(
                before_roots.is_empty(),
                "plain fragment nonce insertBefore should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(before_fragment)
                    .count(),
                0,
                "parser fragment nonce insertBefore should hoist and empty the fragment"
            );

            let result = page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentNonceStates.parserAppend = nonceSubtreeState(
  'parser-fragment-nonce-append',
  'parser-fragment-nonce-append-container',
  null
);
window.parserFragmentNonceStates.parserInsertBefore = nonceSubtreeState(
  'parser-fragment-nonce-before',
  'parser-fragment-nonce-before-container',
  'parser-fragment-nonce-reference'
);
JSON.stringify({
  jsAppend: window.parserFragmentNonceStates.jsAppend,
  parserAppend: window.parserFragmentNonceStates.parserAppend,
  appendSame: JSON.stringify(window.parserFragmentNonceStates.jsAppend) ===
    JSON.stringify(window.parserFragmentNonceStates.parserAppend),
  jsInsertBefore: window.parserFragmentNonceStates.jsInsertBefore,
  parserInsertBefore: window.parserFragmentNonceStates.parserInsertBefore,
  insertBeforeSame: JSON.stringify(window.parserFragmentNonceStates.jsInsertBefore) ===
    JSON.stringify(window.parserFragmentNonceStates.parserInsertBefore)
})
"#,
                )
                .expect("fragment nonce parser comparison should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppend":{"attr":"","nonce":"nonce-secret","parent":"DIV","containerParent":"BODY","containerBeforeReference":null},"parserAppend":{"attr":"","nonce":"nonce-secret","parent":"DIV","containerParent":"BODY","containerBeforeReference":null},"appendSame":true,"jsInsertBefore":{"attr":"","nonce":"nonce-secret","parent":"DIV","containerParent":"BODY","containerBeforeReference":true},"parserInsertBefore":{"attr":"","nonce":"nonce-secret","parent":"DIV","containerParent":"BODY","containerBeforeReference":true},"insertBeforeSame":true}"#
                ),
                "parser DocumentFragment insertion should hide nonce-bearing subtree content attributes like JS insertion"
            );
        }));
}
