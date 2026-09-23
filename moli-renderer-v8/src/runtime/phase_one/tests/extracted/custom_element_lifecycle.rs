use super::*;

#[test]
fn parser_insert_nested_custom_element_to_connected_parent_matches_js_order() {
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
window.parserNestedInsertEvents = [];
class ParserNestedInsertedElement extends HTMLElement {
  connectedCallback() {
    window.parserNestedInsertEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
}
customElements.define('parser-nested-inserted-element', ParserNestedInsertedElement);
const jsOuter = document.createElement('parser-nested-inserted-element');
jsOuter.id = 'js-inserted-outer';
const jsInner = document.createElement('parser-nested-inserted-element');
jsInner.id = 'js-inserted-inner';
jsOuter.appendChild(jsInner);
const parserOuter = document.createElement('parser-nested-inserted-element');
parserOuter.id = 'parser-inserted-outer';
const parserInner = document.createElement('parser-nested-inserted-element');
parserInner.id = 'parser-inserted-inner';
parserOuter.appendChild(parserInner);
document.body.append(jsOuter, parserOuter);
window.parserNestedInsertedJsOuter = jsOuter;
window.parserNestedInsertedOuter = parserOuter;
window.parserNestedInsertEvents.length = 0;
"#,
                )
                .expect("nested custom-element insertion setup should evaluate");

            let target = {
                let runtime = &page_vm.vm().document_runtime;
                runtime
                    .get_element_by_id("parser-inserted-outer")
                    .expect("parser nested custom element should exist before insertion")
            };

            page_vm
                .evaluate_expression(
                    r#"
window.parserNestedInsertedJsOuter.remove();
window.parserNestedInsertedOuter.remove();
window.parserNestedInsertEvents.length = 0;
document.body.appendChild(window.parserNestedInsertedJsOuter);
window.parserNestedInsertJsEvents = window.parserNestedInsertEvents.slice();
window.parserNestedInsertEvents.length = 0;
"#,
                )
                .expect("nested custom-element detached baseline should evaluate");

            let had_pending_work = apply_parser_dom_mutation_and_run_post_step_work_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: body,
                    child: target,
                },
                "parser nested insertion mutation should apply",
                "parser nested connected reactions should dispatch",
            );
            assert!(
                had_pending_work,
                "inserting a disconnected upgraded custom-element subtree should defer connected reactions"
            );

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsEvents: window.parserNestedInsertJsEvents,
  parserEvents: window.parserNestedInsertEvents,
  same: window.parserNestedInsertJsEvents.map(event => event.replace('js-inserted', 'target')).join('|') ===
    window.parserNestedInsertEvents.map(event => event.replace('parser-inserted', 'target')).join('|'),
  parserConnected: window.parserNestedInsertedOuter.isConnected,
  parserParent: window.parserNestedInsertedOuter.parentNode && window.parserNestedInsertedOuter.parentNode.nodeName
})"#,
                )
                .expect("nested custom-element insertion result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-inserted-outer:connected:true","js-inserted-inner:connected:true"],"parserEvents":["parser-inserted-outer:connected:true","parser-inserted-inner:connected:true"],"same":true,"parserConnected":true,"parserParent":"BODY"}"#
                ),
                "parser insertion should match JS appendChild connected callback preorder for upgraded subtrees"
            );
        }));
}
#[test]
fn parser_insert_document_fragment_custom_elements_matches_js_order() {
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
window.parserFragmentInsertEvents = [];
class ParserFragmentInsertedElement extends HTMLElement {
  connectedCallback() {
    window.parserFragmentInsertEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
}
customElements.define('parser-fragment-inserted-element', ParserFragmentInsertedElement);
const jsFragment = document.createDocumentFragment();
const jsOuter = document.createElement('parser-fragment-inserted-element');
jsOuter.id = 'js-fragment-outer';
const jsInner = document.createElement('parser-fragment-inserted-element');
jsInner.id = 'js-fragment-inner';
jsOuter.appendChild(jsInner);
const jsSecond = document.createElement('parser-fragment-inserted-element');
jsSecond.id = 'js-fragment-second';
const parserOuter = document.createElement('parser-fragment-inserted-element');
parserOuter.id = 'parser-fragment-outer';
const parserInner = document.createElement('parser-fragment-inserted-element');
parserInner.id = 'parser-fragment-inner';
parserOuter.appendChild(parserInner);
const parserSecond = document.createElement('parser-fragment-inserted-element');
parserSecond.id = 'parser-fragment-second';
document.body.append(jsOuter, jsSecond, parserOuter, parserSecond);
window.parserFragmentJsFragment = jsFragment;
window.parserFragmentJsOuter = jsOuter;
window.parserFragmentJsSecond = jsSecond;
window.parserFragmentParserOuter = parserOuter;
window.parserFragmentParserSecond = parserSecond;
window.parserFragmentInsertEvents.length = 0;
"#,
                )
                .expect("fragment custom-element insertion setup should evaluate");

            let (parser_outer, parser_second) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-fragment-outer")
                        .expect("parser fragment outer custom element should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-second")
                        .expect("parser fragment second custom element should exist"),
                )
            };

            page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentJsOuter.remove();
window.parserFragmentJsSecond.remove();
window.parserFragmentParserOuter.remove();
window.parserFragmentParserSecond.remove();
window.parserFragmentInsertEvents.length = 0;
window.parserFragmentJsFragment.append(window.parserFragmentJsOuter, window.parserFragmentJsSecond);
document.body.appendChild(window.parserFragmentJsFragment);
window.parserFragmentJsEvents = window.parserFragmentInsertEvents.slice();
window.parserFragmentInsertEvents.length = 0;
"#,
                )
                .expect("fragment custom-element JS baseline should evaluate");

            let fragment = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let fragment = dom_host.create_document_fragment();
                assert!(dom_host.append_child(fragment, parser_outer));
                assert!(dom_host.append_child(fragment, parser_second));
                fragment
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: fragment,
                    },
                    "parser fragment insertion mutation should apply",
                )
            };
            assert!(
                !custom_element_reaction_roots.is_empty(),
                "inserting a fragment with upgraded custom-element roots should defer connected reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment insertion should hoist and empty the fragment"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser fragment connected reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsEvents: window.parserFragmentJsEvents,
  parserEvents: window.parserFragmentInsertEvents,
  same: window.parserFragmentJsEvents.map(event => event.replace('js-fragment', 'target')).join('|') ===
    window.parserFragmentInsertEvents.map(event => event.replace('parser-fragment', 'target')).join('|'),
  jsFragmentEmpty: window.parserFragmentJsFragment.childNodes.length,
  parserOuterParent: window.parserFragmentParserOuter.parentNode && window.parserFragmentParserOuter.parentNode.nodeName,
  parserSecondParent: window.parserFragmentParserSecond.parentNode && window.parserFragmentParserSecond.parentNode.nodeName
})"#,
                )
                .expect("fragment custom-element insertion result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-fragment-outer:connected:true","js-fragment-inner:connected:true","js-fragment-second:connected:true"],"parserEvents":["parser-fragment-outer:connected:true","parser-fragment-inner:connected:true","parser-fragment-second:connected:true"],"same":true,"jsFragmentEmpty":0,"parserOuterParent":"BODY","parserSecondParent":"BODY"}"#
                ),
                "parser DocumentFragment insertion should match JS appendChild connected callback preorder across hoisted roots"
            );
        }));
}
#[test]
fn parser_insert_before_document_fragment_custom_elements_matches_js_order() {
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
window.parserFragmentBeforeEvents = [];
class ParserFragmentBeforeElement extends HTMLElement {
  connectedCallback() {
    window.parserFragmentBeforeEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
}
customElements.define('parser-fragment-before-element', ParserFragmentBeforeElement);
const jsFragment = document.createDocumentFragment();
const jsOuter = document.createElement('parser-fragment-before-element');
jsOuter.id = 'js-before-outer';
const jsInner = document.createElement('parser-fragment-before-element');
jsInner.id = 'js-before-inner';
jsOuter.appendChild(jsInner);
const jsSecond = document.createElement('parser-fragment-before-element');
jsSecond.id = 'js-before-second';
const jsReference = document.createElement('span');
jsReference.id = 'js-before-reference';
const parserOuter = document.createElement('parser-fragment-before-element');
parserOuter.id = 'parser-before-outer';
const parserInner = document.createElement('parser-fragment-before-element');
parserInner.id = 'parser-before-inner';
parserOuter.appendChild(parserInner);
const parserSecond = document.createElement('parser-fragment-before-element');
parserSecond.id = 'parser-before-second';
const parserReference = document.createElement('span');
parserReference.id = 'parser-before-reference';
document.body.append(jsOuter, jsSecond, jsReference, parserOuter, parserSecond, parserReference);
window.parserFragmentBeforeJsFragment = jsFragment;
window.parserFragmentBeforeJsOuter = jsOuter;
window.parserFragmentBeforeJsSecond = jsSecond;
window.parserFragmentBeforeJsReference = jsReference;
window.parserFragmentBeforeParserOuter = parserOuter;
window.parserFragmentBeforeParserSecond = parserSecond;
window.parserFragmentBeforeParserReference = parserReference;
window.parserFragmentBeforeEvents.length = 0;
"#,
                )
                .expect("fragment insert-before setup should evaluate");

            let (parser_outer, parser_second, parser_reference) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-before-outer")
                        .expect("parser fragment outer custom element should exist"),
                    runtime
                        .get_element_by_id("parser-before-second")
                        .expect("parser fragment second custom element should exist"),
                    runtime
                        .get_element_by_id("parser-before-reference")
                        .expect("parser fragment reference should exist"),
                )
            };

            page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentBeforeJsOuter.remove();
window.parserFragmentBeforeJsSecond.remove();
window.parserFragmentBeforeParserOuter.remove();
window.parserFragmentBeforeParserSecond.remove();
window.parserFragmentBeforeEvents.length = 0;
window.parserFragmentBeforeJsFragment.append(
  window.parserFragmentBeforeJsOuter,
  window.parserFragmentBeforeJsSecond
);
document.body.insertBefore(window.parserFragmentBeforeJsFragment, window.parserFragmentBeforeJsReference);
window.parserFragmentBeforeJsEvents = window.parserFragmentBeforeEvents.slice();
window.parserFragmentBeforeEvents.length = 0;
"#,
                )
                .expect("fragment insert-before JS baseline should evaluate");

            let fragment = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let fragment = dom_host.create_document_fragment();
                assert!(dom_host.append_child(fragment, parser_outer));
                assert!(dom_host.append_child(fragment, parser_second));
                fragment
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: fragment,
                        reference_child: Some(parser_reference),
                    },
                    "parser fragment insert-before mutation should apply",
                )
            };
            assert!(
                !custom_element_reaction_roots.is_empty(),
                "insertBefore with a fragment of upgraded custom elements should defer connected reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment insertBefore should hoist and empty the fragment"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser fragment insert-before reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsEvents: window.parserFragmentBeforeJsEvents,
  parserEvents: window.parserFragmentBeforeEvents,
  same: window.parserFragmentBeforeJsEvents.map(event => event.replace('js-before', 'target')).join('|') ===
    window.parserFragmentBeforeEvents.map(event => event.replace('parser-before', 'target')).join('|'),
  jsFragmentEmpty: window.parserFragmentBeforeJsFragment.childNodes.length,
  parserReferencePrevious: window.parserFragmentBeforeParserReference.previousSibling &&
    window.parserFragmentBeforeParserReference.previousSibling.id,
  parserSecondNext: window.parserFragmentBeforeParserSecond.nextSibling &&
    window.parserFragmentBeforeParserSecond.nextSibling.id
})"#,
                )
                .expect("fragment insert-before result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-before-outer:connected:true","js-before-inner:connected:true","js-before-second:connected:true"],"parserEvents":["parser-before-outer:connected:true","parser-before-inner:connected:true","parser-before-second:connected:true"],"same":true,"jsFragmentEmpty":0,"parserReferencePrevious":"parser-before-second","parserSecondNext":"parser-before-reference"}"#
                ),
                "parser DocumentFragment insertBefore should match JS connected callback preorder and insert before the reference child"
            );
        }));
}
#[test]
fn parser_reparent_nested_custom_element_to_disconnected_parent_matches_js_order() {
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
window.parserNestedMoveEvents = [];
class ParserNestedMovedElement extends HTMLElement {
  connectedCallback() {
    window.parserNestedMoveEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
  disconnectedCallback() {
    window.parserNestedMoveEvents.push(`${this.id}:disconnected:${this.isConnected}`);
  }
}
customElements.define('parser-nested-moved-element', ParserNestedMovedElement);
function createNestedMoved(prefix) {
  const outer = document.createElement('parser-nested-moved-element');
  outer.id = `${prefix}-outer`;
  const inner = document.createElement('parser-nested-moved-element');
  inner.id = `${prefix}-inner`;
  outer.appendChild(inner);
  return outer;
}
const connectedParent = document.createElement('div');
const jsBeforeDetachedParent = document.createElement('div');
const jsBeforeReference = document.createElement('span');
jsBeforeDetachedParent.appendChild(jsBeforeReference);
const jsAppendDetachedParent = document.createElement('div');
const jsBeforeOuter = createNestedMoved('js-before-nested');
const jsAppendOuter = createNestedMoved('js-append-nested');
const parserBeforeOuter = createNestedMoved('parser-before-nested');
const parserAppendOuter = createNestedMoved('parser-append-nested');
connectedParent.append(jsBeforeOuter, jsAppendOuter, parserBeforeOuter, parserAppendOuter);
document.body.appendChild(connectedParent);
window.parserBeforeNestedOuter = parserBeforeOuter;
window.parserAppendNestedOuter = parserAppendOuter;
window.parserNestedMoveEvents.length = 0;
jsBeforeDetachedParent.insertBefore(jsBeforeOuter, jsBeforeReference);
jsAppendDetachedParent.appendChild(jsAppendOuter);
"#,
                )
                .expect("nested custom-element reparent setup should evaluate");

            let (
                before_detached_parent,
                before_reference,
                before_target,
                append_detached_parent,
                append_target,
            ) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let before_detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let before_reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.append_child(before_detached_parent, before_reference));
                let before_target = dom_host
                    .element_handle_by_id("parser-before-nested-outer")
                    .expect("parser insertBefore nested custom element should exist before reparent");
                let append_detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let append_target = dom_host
                    .element_handle_by_id("parser-append-nested-outer")
                    .expect("parser append nested custom element should exist before reparent");
                (
                    before_detached_parent,
                    before_reference,
                    before_target,
                    append_detached_parent,
                    append_target,
                )
            };

            let before_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: before_detached_parent,
                        child: before_target,
                        reference_child: Some(before_reference),
                    },
                    "parser nested insertBefore reparent mutation should apply",
                )
            };
            assert!(
                !before_reaction_roots.is_empty(),
                "parser insertBefore moving a connected custom-element subtree to a disconnected parent should defer disconnected reactions"
            );
            let append_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: append_detached_parent,
                        child: append_target,
                    },
                    "parser nested append reparent mutation should apply",
                )
            };
            assert!(
                !append_reaction_roots.is_empty(),
                "parser AppendChild moving a connected custom-element subtree to a disconnected parent should defer disconnected reactions"
            );
            let custom_element_reaction_roots =
                ParserPostStepRuntimeWorkForTest::merge_for_test([
                    before_reaction_roots,
                    append_reaction_roots,
                ]);

            {
                let dom_host = page_vm.vm().document_runtime.dom_host();
                assert_eq!(
                    dom_host
                        .child_handles(before_detached_parent)
                        .collect::<Vec<_>>(),
                    vec![before_target, before_reference],
                    "parser insertBefore should move the nested custom element before the native reference"
                );
                assert_eq!(
                    dom_host
                        .child_handles(append_detached_parent)
                        .collect::<Vec<_>>(),
                    vec![append_target],
                    "parser AppendChild should move the nested custom element under the native detached parent"
                );
            }

            assert!(
                !custom_element_reaction_roots.is_empty(),
                "moving connected custom-element subtrees to disconnected parents should defer disconnected reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser nested disconnected reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserNestedMoveEvents,
  parserBeforeParentConnected: window.parserBeforeNestedOuter.parentNode && window.parserBeforeNestedOuter.parentNode.isConnected,
  parserAppendParentConnected: window.parserAppendNestedOuter.parentNode && window.parserAppendNestedOuter.parentNode.isConnected
})"#,
                )
                .expect("nested custom-element reparent result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-before-nested-outer:disconnected:false","js-before-nested-inner:disconnected:false","js-append-nested-outer:disconnected:false","js-append-nested-inner:disconnected:false","parser-before-nested-outer:disconnected:false","parser-before-nested-inner:disconnected:false","parser-append-nested-outer:disconnected:false","parser-append-nested-inner:disconnected:false"],"parserBeforeParentConnected":false,"parserAppendParentConnected":false}"#
                ),
                "parser reparent to a disconnected parent should match JS disconnected callback preorder for insertBefore and appendChild"
            );
        }));
}
#[test]
fn parser_mutation_owner_dispatches_removed_custom_element_reactions() {
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
window.parserRemoveEvents = [];
class ParserRemovedElement extends HTMLElement {
  connectedCallback() {
    window.parserRemoveEvents.push(`connected:${this.isConnected}`);
  }
  disconnectedCallback() {
    window.parserRemoveEvents.push(`disconnected:${this.isConnected}`);
  }
}
customElements.define('parser-removed-element', ParserRemovedElement);
const parserRemoveTarget = document.createElement('parser-removed-element');
parserRemoveTarget.id = 'parser-remove-target';
window.parserRemoveTarget = parserRemoveTarget;
document.body.appendChild(parserRemoveTarget);
"#,
                )
                .expect("custom element setup should evaluate");

            let target = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-remove-target")
                .expect("custom element should be connected before parser removal");

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::RemoveChild {
                        parent: body,
                        child: target,
                    },
                    "parser remove mutation should apply",
                )
            };
            assert!(
                !custom_element_reaction_roots.is_empty(),
                "parser removal should defer disconnected reactions until the parser pump returns"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser removal reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserRemoveEvents,
  connected: window.parserRemoveTarget.isConnected,
  parent: window.parserRemoveTarget.parentNode && window.parserRemoveTarget.parentNode.nodeName
})"#,
                )
                .expect("custom element removal result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"{"events":["connected:true","disconnected:false"],"connected":false,"parent":null}"#),
                "parser remove should run the same disconnected lifecycle reaction as JS removeChild"
            );
        }));
}
#[test]
fn parser_reparent_connected_custom_element_does_not_reconnect_callback() {
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
window.parserMoveEvents = [];
class ParserMovedElement extends HTMLElement {
  connectedCallback() {
    window.parserMoveEvents.push(`${this.id}:connected:${this.parentNode && this.parentNode.id}`);
  }
  disconnectedCallback() {
    window.parserMoveEvents.push(`${this.id}:disconnected:${this.parentNode && this.parentNode.id}`);
  }
}
customElements.define('parser-moved-element', ParserMovedElement);
const a = document.createElement('div');
a.id = 'parser-move-a';
const b = document.createElement('div');
b.id = 'parser-move-b';
const parserTarget = document.createElement('parser-moved-element');
parserTarget.id = 'parser-target';
const jsTarget = document.createElement('parser-moved-element');
jsTarget.id = 'js-target';
window.parserTarget = parserTarget;
window.jsTarget = jsTarget;
a.append(parserTarget, jsTarget);
document.body.append(a, b);
b.appendChild(jsTarget);
"#,
                )
                .expect("custom element move setup should evaluate");

            let (parent, target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-move-b")
                        .expect("target parser parent should exist"),
                    runtime
                        .get_element_by_id("parser-target")
                        .expect("custom element should be connected before parser reparent"),
                )
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent,
                        child: target,
                        reference_child: None,
                    },
                    "parser reparent mutation should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "moving an already-connected custom element should not defer a connected reaction"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("empty parser move reactions should dispatch as a no-op");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserMoveEvents,
  parserParent: window.parserTarget.parentNode && window.parserTarget.parentNode.id,
  jsParent: window.jsTarget.parentNode && window.jsTarget.parentNode.id
})"#,
                )
                .expect("custom element move result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["parser-target:connected:parser-move-a","js-target:connected:parser-move-a"],"parserParent":"parser-move-b","jsParent":"parser-move-b"}"#
                ),
                "parser reparent should match JS insertBefore and avoid reconnecting an already-connected custom element"
            );
        }));
}
#[test]
fn parser_reparent_connected_custom_element_does_not_dispatch_atomic_move_callback() {
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
window.parserAtomicMoveEvents = [];
class ParserAtomicMoveElement extends HTMLElement {
  connectedCallback() {
    window.parserAtomicMoveEvents.push(`${this.id}:connected:${this.parentNode && this.parentNode.id}`);
  }
  disconnectedCallback() {
    window.parserAtomicMoveEvents.push(`${this.id}:disconnected:${this.parentNode && this.parentNode.id}`);
  }
  connectedMoveCallback() {
    window.parserAtomicMoveEvents.push(`${this.id}:move:${this.parentNode && this.parentNode.id}`);
  }
}
customElements.define('parser-atomic-move-element', ParserAtomicMoveElement);
const a = document.createElement('div');
a.id = 'parser-atomic-a';
const b = document.createElement('div');
b.id = 'parser-atomic-b';
const c = document.createElement('div');
c.id = 'parser-atomic-c';
const parserTarget = document.createElement('parser-atomic-move-element');
parserTarget.id = 'parser-atomic-target';
const jsInsertTarget = document.createElement('parser-atomic-move-element');
jsInsertTarget.id = 'js-insert-target';
const jsMoveTarget = document.createElement('parser-atomic-move-element');
jsMoveTarget.id = 'js-move-target';
window.parserAtomicTarget = parserTarget;
window.jsInsertTarget = jsInsertTarget;
window.jsMoveTarget = jsMoveTarget;
a.append(parserTarget, jsInsertTarget, jsMoveTarget);
document.body.append(a, b, c);
window.parserAtomicMoveEvents.length = 0;
b.insertBefore(jsInsertTarget, null);
c.moveBefore(jsMoveTarget, null);
"#,
                )
                .expect("custom element atomic move setup should evaluate");

            let (parent, target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-atomic-b")
                        .expect("target parser parent should exist"),
                    runtime
                        .get_element_by_id("parser-atomic-target")
                        .expect("parser custom element should be connected before reparent"),
                )
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent,
                        child: target,
                        reference_child: None,
                    },
                    "parser atomic-move comparison reparent mutation should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "parser ordinary reparent should not defer connected or atomic move reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("empty parser atomic move comparison reactions should dispatch as a no-op");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserAtomicMoveEvents,
  parserParent: window.parserAtomicTarget.parentNode && window.parserAtomicTarget.parentNode.id,
  jsInsertParent: window.jsInsertTarget.parentNode && window.jsInsertTarget.parentNode.id,
  jsMoveParent: window.jsMoveTarget.parentNode && window.jsMoveTarget.parentNode.id
})"#,
                )
                .expect("custom element atomic move comparison result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-move-target:move:parser-atomic-c"],"parserParent":"parser-atomic-b","jsInsertParent":"parser-atomic-b","jsMoveParent":"parser-atomic-c"}"#
                ),
                "parser reparent should match JS insertBefore and must not dispatch connectedMoveCallback"
            );
        }));
}
#[test]
fn parser_reparent_form_associated_custom_element_dispatches_form_reactions() {
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
window.parserFaceMoveEvents = [];
class ParserMovedFaceElement extends HTMLElement {
  static formAssociated = true;
  connectedCallback() {
    window.parserFaceMoveEvents.push(`${this.id}:connected:${this.parentNode && this.parentNode.id}`);
  }
  formAssociatedCallback(form) {
    window.parserFaceMoveEvents.push(`${this.id}:form:${form && form.id}`);
  }
  formDisabledCallback(disabled) {
    window.parserFaceMoveEvents.push(`${this.id}:disabled:${disabled}`);
  }
}
customElements.define('parser-moved-face-element', ParserMovedFaceElement);
const formA = document.createElement('form');
formA.id = 'parser-face-form-a';
const formB = document.createElement('form');
formB.id = 'parser-face-form-b';
const fieldset = document.createElement('fieldset');
fieldset.id = 'parser-face-fieldset';
fieldset.disabled = true;
formB.appendChild(fieldset);
const parserTarget = document.createElement('parser-moved-face-element');
parserTarget.id = 'parser-face-target';
const jsTarget = document.createElement('parser-moved-face-element');
jsTarget.id = 'js-face-target';
window.parserFaceTarget = parserTarget;
window.jsFaceTarget = jsTarget;
formA.append(parserTarget, jsTarget);
document.body.append(formA, formB);
window.parserFaceMoveEvents.length = 0;
fieldset.appendChild(jsTarget);
"#,
                )
                .expect("FACE move setup should evaluate");

            let (parent, target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-face-fieldset")
                        .expect("target parser fieldset should exist"),
                    runtime
                        .get_element_by_id("parser-face-target")
                        .expect("parser FACE target should exist"),
                )
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent,
                        child: target,
                        reference_child: None,
                    },
                    "parser FACE reparent mutation should apply",
                )
            };
            assert!(
                !custom_element_reaction_roots.is_empty(),
                "moving an already-connected FACE should defer form state reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser FACE reparent reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserFaceMoveEvents,
  parserParent: window.parserFaceTarget.parentNode && window.parserFaceTarget.parentNode.id,
  jsParent: window.jsFaceTarget.parentNode && window.jsFaceTarget.parentNode.id
})"#,
                )
                .expect("FACE move result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-face-target:form:parser-face-form-b","js-face-target:disabled:true","parser-face-target:form:parser-face-form-b","parser-face-target:disabled:true"],"parserParent":"parser-face-fieldset","jsParent":"parser-face-fieldset"}"#
                ),
                "parser FACE reparent should match JS insertion form association and disabled callbacks without reconnecting"
            );
        }));
}
#[test]
fn parser_remove_form_associated_custom_element_matches_js_reactions() {
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
window.parserFaceRemoveEvents = [];
class ParserRemovedFaceElement extends HTMLElement {
  static formAssociated = true;
  connectedCallback() {
    window.parserFaceRemoveEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
  disconnectedCallback() {
    window.parserFaceRemoveEvents.push(`${this.id}:disconnected:${this.isConnected}`);
  }
  formAssociatedCallback(form) {
    window.parserFaceRemoveEvents.push(`${this.id}:form:${form && form.id}`);
  }
  formDisabledCallback(disabled) {
    window.parserFaceRemoveEvents.push(`${this.id}:disabled:${disabled}`);
  }
}
customElements.define('parser-removed-face-element', ParserRemovedFaceElement);
const form = document.createElement('form');
form.id = 'parser-face-remove-form';
const fieldset = document.createElement('fieldset');
fieldset.id = 'parser-face-remove-fieldset';
fieldset.disabled = true;
const jsTarget = document.createElement('parser-removed-face-element');
jsTarget.id = 'js-face-remove-target';
const parserTarget = document.createElement('parser-removed-face-element');
parserTarget.id = 'parser-face-remove-target';
window.parserFaceRemoveTarget = parserTarget;
fieldset.append(jsTarget, parserTarget);
form.appendChild(fieldset);
document.body.appendChild(form);
window.parserFaceRemoveEvents.length = 0;
fieldset.removeChild(jsTarget);
window.parserFaceRemoveJsEvents = window.parserFaceRemoveEvents.slice();
window.parserFaceRemoveEvents.length = 0;
"#,
                )
                .expect("FACE remove setup should evaluate");

            let (parent, target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-face-remove-fieldset")
                        .expect("parser FACE remove parent should exist"),
                    runtime
                        .get_element_by_id("parser-face-remove-target")
                        .expect("parser FACE remove target should exist"),
                )
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::RemoveChild {
                        parent,
                        child: target,
                    },
                    "parser FACE remove mutation should apply",
                )
            };
            assert!(
                !custom_element_reaction_roots.is_empty(),
                "removing a connected FACE should defer lifecycle and form state reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser FACE remove reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsEvents: window.parserFaceRemoveJsEvents,
  parserEvents: window.parserFaceRemoveEvents,
  same: window.parserFaceRemoveJsEvents.map(event => event.replace('js-face-remove-target', 'target')).join('|') ===
    window.parserFaceRemoveEvents.map(event => event.replace('parser-face-remove-target', 'target')).join('|'),
  parserConnected: window.parserFaceRemoveTarget.isConnected,
  parserParent: window.parserFaceRemoveTarget.parentNode && window.parserFaceRemoveTarget.parentNode.id
})"#,
                )
                .expect("FACE remove result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-face-remove-target:disconnected:false","js-face-remove-target:form:null","js-face-remove-target:disabled:false"],"parserEvents":["parser-face-remove-target:disconnected:false","parser-face-remove-target:form:null","parser-face-remove-target:disabled:false"],"same":true,"parserConnected":false,"parserParent":null}"#
                ),
                "parser FACE remove should match JS removeChild lifecycle, form association, and disabled callbacks"
            );
        }));
}
#[test]
fn disconnected_remove_form_associated_custom_element_matches_js_disabled_state_reactions() {
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
window.parserDisconnectedFaceRemoveEvents = [];
class ParserDisconnectedRemovedFaceElement extends HTMLElement {
  static formAssociated = true;
  connectedCallback() {
    window.parserDisconnectedFaceRemoveEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
  disconnectedCallback() {
    window.parserDisconnectedFaceRemoveEvents.push(`${this.id}:disconnected:${this.isConnected}`);
  }
  formAssociatedCallback(form) {
    window.parserDisconnectedFaceRemoveEvents.push(`${this.id}:form:${form && form.id}`);
  }
  formDisabledCallback(disabled) {
    window.parserDisconnectedFaceRemoveEvents.push(`${this.id}:disabled:${disabled}`);
  }
}
customElements.define('parser-disconnected-removed-face-element', ParserDisconnectedRemovedFaceElement);
const form = document.createElement('form');
form.id = 'parser-face-disconnected-remove-form';
const fieldset = document.createElement('fieldset');
fieldset.id = 'parser-face-disconnected-remove-fieldset';
fieldset.disabled = true;
const jsTarget = document.createElement('parser-disconnected-removed-face-element');
jsTarget.id = 'js-face-disconnected-remove-target';
const parserTarget = document.createElement('parser-disconnected-removed-face-element');
parserTarget.id = 'parser-face-disconnected-remove-target';
window.parserDisconnectedFaceRemoveFieldset = fieldset;
window.parserDisconnectedFaceRemoveJsTarget = jsTarget;
window.parserDisconnectedFaceRemoveTarget = parserTarget;
fieldset.append(jsTarget, parserTarget);
form.appendChild(fieldset);
document.body.appendChild(form);
"#,
                )
                .expect("disconnected FACE remove setup should evaluate");

            let (parent, target) = {
                let dom_host = page_vm.vm().document_runtime.dom_host();
                (
                    dom_host
                        .element_handle_by_id("parser-face-disconnected-remove-fieldset")
                        .expect("parser disconnected FACE remove parent should exist"),
                    dom_host
                        .element_handle_by_id("parser-face-disconnected-remove-target")
                        .expect("parser disconnected FACE remove target should exist"),
                )
            };

            page_vm
                .evaluate_expression(
                    r#"
document.body.querySelector('#parser-face-disconnected-remove-form')
  .removeChild(window.parserDisconnectedFaceRemoveFieldset);
window.parserDisconnectedFaceRemoveEvents.length = 0;
window.parserDisconnectedFaceRemoveFieldset
  .removeChild(window.parserDisconnectedFaceRemoveJsTarget);
window.parserDisconnectedFaceRemoveJsEvents =
  window.parserDisconnectedFaceRemoveEvents.slice();
window.parserDisconnectedFaceRemoveEvents.length = 0;
"#,
                )
                .expect("disconnected FACE remove JS baseline should evaluate");

            let reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent,
                    child: target,
                },
                "parser disconnected FACE remove mutation should apply",
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(reaction_roots)
                .expect("parser disconnected FACE remove reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsEvents: window.parserDisconnectedFaceRemoveJsEvents,
  parserEvents: window.parserDisconnectedFaceRemoveEvents,
  same: window.parserDisconnectedFaceRemoveJsEvents.map(event => event.replace('js-face-disconnected-remove-target', 'target')).join('|') ===
    window.parserDisconnectedFaceRemoveEvents.map(event => event.replace('parser-face-disconnected-remove-target', 'target')).join('|'),
  parserConnected: window.parserDisconnectedFaceRemoveTarget.isConnected,
  parserParent: window.parserDisconnectedFaceRemoveTarget.parentNode && window.parserDisconnectedFaceRemoveTarget.parentNode.id
})"#,
                )
                .expect("disconnected FACE remove result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-face-disconnected-remove-target:disabled:false"],"parserEvents":["parser-face-disconnected-remove-target:disabled:false"],"same":true,"parserConnected":false,"parserParent":null}"#
                ),
                "tree mutation removal from a disabled disconnected fieldset should match JS formDisabledCallback behavior"
            );
        }));
}
#[test]
fn parser_reparent_form_associated_custom_element_to_disconnected_parent_matches_js_reactions() {
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
window.parserFaceDetachEvents = [];
class ParserDetachedFaceElement extends HTMLElement {
  static formAssociated = true;
  connectedCallback() {
    window.parserFaceDetachEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
  disconnectedCallback() {
    window.parserFaceDetachEvents.push(`${this.id}:disconnected:${this.isConnected}`);
  }
  formAssociatedCallback(form) {
    window.parserFaceDetachEvents.push(`${this.id}:form:${form && form.id}`);
  }
  formDisabledCallback(disabled) {
    window.parserFaceDetachEvents.push(`${this.id}:disabled:${disabled}`);
  }
}
customElements.define('parser-detached-face-element', ParserDetachedFaceElement);
const form = document.createElement('form');
form.id = 'parser-face-detach-form';
const fieldset = document.createElement('fieldset');
fieldset.id = 'parser-face-detach-fieldset';
fieldset.disabled = true;
const jsDetachedParent = document.createElement('div');
const jsTarget = document.createElement('parser-detached-face-element');
jsTarget.id = 'js-face-detach-target';
const parserTarget = document.createElement('parser-detached-face-element');
parserTarget.id = 'parser-face-detach-target';
window.parserFaceDetachTarget = parserTarget;
fieldset.append(jsTarget, parserTarget);
form.appendChild(fieldset);
document.body.appendChild(form);
window.parserFaceDetachEvents.length = 0;
jsDetachedParent.insertBefore(jsTarget, null);
window.parserFaceDetachJsEvents = window.parserFaceDetachEvents.slice();
window.parserFaceDetachEvents.length = 0;
"#,
                )
                .expect("FACE detached reparent setup should evaluate");

            let (detached_parent, target) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let target = dom_host
                    .element_handle_by_id("parser-face-detach-target")
                    .expect("parser FACE detach target should exist before reparent");
                (detached_parent, target)
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: detached_parent,
                        child: target,
                        reference_child: None,
                    },
                    "parser FACE detached reparent mutation should apply",
                )
            };
            assert!(
                !custom_element_reaction_roots.is_empty(),
                "moving a connected FACE to a disconnected parent should defer lifecycle and form state reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser FACE detached reparent reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsEvents: window.parserFaceDetachJsEvents,
  parserEvents: window.parserFaceDetachEvents,
  same: window.parserFaceDetachJsEvents.map(event => event.replace('js-face-detach-target', 'target')).join('|') ===
    window.parserFaceDetachEvents.map(event => event.replace('parser-face-detach-target', 'target')).join('|'),
  parserConnected: window.parserFaceDetachTarget.isConnected,
  parserParentConnected: window.parserFaceDetachTarget.parentNode && window.parserFaceDetachTarget.parentNode.isConnected
})"#,
                )
                .expect("FACE detached reparent result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-face-detach-target:disconnected:false","js-face-detach-target:form:null","js-face-detach-target:disabled:false"],"parserEvents":["parser-face-detach-target:disconnected:false","parser-face-detach-target:form:null","parser-face-detach-target:disabled:false"],"same":true,"parserConnected":false,"parserParentConnected":false}"#
                ),
                "parser FACE reparent to a disconnected parent should match JS insertBefore lifecycle, form association, and disabled callbacks"
            );
        }));
}
#[test]
fn parser_reparent_custom_element_across_documents_dispatches_adoption_reactions() {
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
window.parserAdoptEvents = [];
class ParserAdoptedElement extends HTMLElement {
  connectedCallback() {
    window.parserAdoptEvents.push(`${this.id}:connected`);
  }
  disconnectedCallback() {
    window.parserAdoptEvents.push(`${this.id}:disconnected`);
  }
  adoptedCallback(oldDocument, newDocument) {
    window.parserAdoptEvents.push(`${this.id}:adopted:${oldDocument === document}:${newDocument === document}`);
  }
}
customElements.define('parser-adopted-element', ParserAdoptedElement);
const jsAdoptDoc = document.implementation.createHTMLDocument("");
const jsTarget = document.createElement('parser-adopted-element');
jsTarget.id = 'js-adopt-target';
const parserTarget = document.createElement('parser-adopted-element');
parserTarget.id = 'parser-adopt-target';
window.jsAdoptTarget = jsTarget;
window.parserAdoptTarget = parserTarget;
document.body.append(jsTarget, parserTarget);
window.parserAdoptEvents.length = 0;
jsAdoptDoc.documentElement.appendChild(jsTarget);
"#,
                )
                .expect("custom element adoption setup should evaluate");

            let (parent, target) = {
                let runtime = &mut page_vm.vm_mut().document_runtime;
                let target = runtime
                    .get_element_by_id("parser-adopt-target")
                    .expect("parser adoption target should exist");
                let dom_host = runtime.dom_host_mut();
                let detached_document = dom_host.create_detached_html_document();
                let detached_root = dom_host.create_parser_element_without_attributes_for_document(
                    detached_document,
                    "html".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(
                    dom_host.append_child(detached_document, detached_root),
                    "detached document root should be attached before parser adoption"
                );
                (detached_root, target)
            };

            let had_pending_work = apply_parser_dom_mutation_and_run_post_step_work_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent,
                    child: target,
                    reference_child: None,
                },
                "parser cross-document reparent mutation should apply",
                "parser adoption reactions should dispatch",
            );
            assert!(
                had_pending_work,
                "cross-document parser reparent should defer adoption reactions"
            );

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserAdoptEvents,
  parserOwnerIsMain: window.parserAdoptTarget.ownerDocument === document,
  jsOwnerIsMain: window.jsAdoptTarget.ownerDocument === document
})"#,
                )
                .expect("custom element adoption result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-adopt-target:disconnected","js-adopt-target:adopted:true:false","js-adopt-target:connected","parser-adopt-target:disconnected","parser-adopt-target:adopted:true:false","parser-adopt-target:connected"],"parserOwnerIsMain":false,"jsOwnerIsMain":false}"#
                ),
                "parser cross-document reparent should match JS insertion adoption reactions"
            );
        }));
}
#[test]
fn parser_cross_document_reparent_without_adopted_callback_still_dispatches_lifecycle_reactions() {
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
window.parserNoAdoptCallbackEvents = [];
class ParserNoAdoptCallbackElement extends HTMLElement {
  connectedCallback() {
    window.parserNoAdoptCallbackEvents.push(`${this.id}:connected:${this.ownerDocument === document}`);
  }
  disconnectedCallback() {
    window.parserNoAdoptCallbackEvents.push(`${this.id}:disconnected:${this.isConnected}`);
  }
}
customElements.define('parser-no-adopt-callback-element', ParserNoAdoptCallbackElement);
const jsDocWithoutAdopt = document.implementation.createHTMLDocument("");
const jsTarget = document.createElement('parser-no-adopt-callback-element');
jsTarget.id = 'js-no-adopt-callback-target';
const parserTarget = document.createElement('parser-no-adopt-callback-element');
parserTarget.id = 'parser-no-adopt-callback-target';
window.jsNoAdoptCallbackTarget = jsTarget;
window.parserNoAdoptCallbackTarget = parserTarget;
document.body.append(jsTarget, parserTarget);
window.parserNoAdoptCallbackEvents.length = 0;
jsDocWithoutAdopt.documentElement.appendChild(jsTarget);
"#,
                )
                .expect("custom element no-adopt cross-document setup should evaluate");

            let (parent, target) = {
                let runtime = &mut page_vm.vm_mut().document_runtime;
                let target = runtime
                    .get_element_by_id("parser-no-adopt-callback-target")
                    .expect("parser no-adopt target should exist");
                let dom_host = runtime.dom_host_mut();
                let detached_document = dom_host.create_detached_html_document();
                let detached_root = dom_host.create_parser_element_without_attributes_for_document(
                    detached_document,
                    "html".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(
                    dom_host.append_child(detached_document, detached_root),
                    "detached document root should be attached before parser no-adopt mutation"
                );
                (detached_root, target)
            };

            let had_pending_work = apply_parser_dom_mutation_and_run_post_step_work_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent,
                    child: target,
                },
                "parser cross-document reparent without adoptedCallback should apply",
                "parser no-adopt lifecycle reactions should dispatch",
            );
            assert!(
                had_pending_work,
                "cross-document parser reparent must defer lifecycle reactions even without adoptedCallback"
            );

            let result = page_vm
                .evaluate_expression("JSON.stringify(window.parserNoAdoptCallbackEvents)")
                .expect("custom element no-adopt result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"["js-no-adopt-callback-target:disconnected:true","js-no-adopt-callback-target:connected:false","parser-no-adopt-callback-target:disconnected:true","parser-no-adopt-callback-target:connected:false"]"#
                ),
                "parser cross-document reparent should match JS lifecycle reactions even when adoptedCallback is absent"
            );
        }));
}
#[test]
fn parser_adoption_reactions_disconnect_only_preconnected_roots() {
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
window.parserMixedAdoptEvents = [];
class ParserMixedAdoptElement extends HTMLElement {
  disconnectedCallback() {
    window.parserMixedAdoptEvents.push(`${this.id}:disconnected:${this.isConnected}`);
  }
  adoptedCallback() {
    window.parserMixedAdoptEvents.push(`${this.id}:adopted`);
  }
}
customElements.define('parser-mixed-adopt-element', ParserMixedAdoptElement);
const connected = document.createElement('parser-mixed-adopt-element');
connected.id = 'parser-mixed-adopt-connected';
const disconnected = document.createElement('parser-mixed-adopt-element');
disconnected.id = 'parser-mixed-adopt-disconnected';
document.body.append(connected, disconnected);
window.parserMixedAdoptConnected = connected;
window.parserMixedAdoptDisconnected = disconnected;
"#,
                )
                .expect("mixed adoption reaction setup should evaluate");

            let (parent, connected, disconnected) = {
                let runtime = &mut page_vm.vm_mut().document_runtime;
                let dom_host = runtime.dom_host_mut();
                let connected = dom_host
                    .element_handle_by_id("parser-mixed-adopt-connected")
                    .expect("connected mixed adoption target should exist");
                let disconnected = dom_host
                    .element_handle_by_id("parser-mixed-adopt-disconnected")
                    .expect("disconnected mixed adoption target should exist");
                let detached_document = dom_host.create_detached_html_document();
                let detached_root = dom_host.create_parser_element_without_attributes_for_document(
                    detached_document,
                    "html".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(
                    dom_host.append_child(detached_document, detached_root),
                    "detached document root should be attached before mixed adoption"
                );
                (detached_root, connected, disconnected)
            };

            page_vm
                .evaluate_expression(
                    r#"
document.body.removeChild(window.parserMixedAdoptDisconnected);
window.parserMixedAdoptEvents.length = 0;
"#,
                )
                .expect("mixed adoption disconnected setup should evaluate");

            let reaction_roots = ParserPostStepRuntimeWorkForTest::merge_for_test([
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent,
                        child: connected,
                    },
                    "connected mixed adoption parser mutation should apply",
                ),
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent,
                        child: disconnected,
                    },
                    "disconnected mixed adoption parser mutation should apply",
                ),
            ]);
            assert!(
                !reaction_roots.is_empty(),
                "mixed cross-document parser reparent should request adoption reaction checkpoint"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(reaction_roots)
                .expect("mixed adoption reactions should dispatch");

            let result = page_vm
                .evaluate_expression("JSON.stringify(window.parserMixedAdoptEvents)")
                .expect("mixed adoption reaction result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"["parser-mixed-adopt-connected:disconnected:true","parser-mixed-adopt-connected:adopted","parser-mixed-adopt-disconnected:adopted"]"#
                ),
                "parser adoption reactions should dispatch disconnectedCallback only for roots that were lifecycle-connected before insertion"
            );
        }));
}
#[test]
fn parser_custom_element_constructor_runs_before_following_siblings() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let _js_runtime = crate::JsRuntime::initialize();
        let final_url =
            Url::parse("https://parser-sync-custom-element.test/").expect("test url");
        let loader: &'static ResourceRequestClient =
            Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
        let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url.clone())));
        let mut driver = ParserDriver {
            loader,
            final_url: &state.final_url,
            parser_session: &mut state.parser_session,
            scheduler: &mut state.scheduler,
            buffered_document_preloads: &mut state.buffered_document_preloads,
            service_worker_preload_context: state.service_worker_preload_context.as_ref(),
            input_closed: &state.input_closed,
        };
        let html = r#"<!doctype html><script>
window.__containerChildNodesInConstructor = [];
window.__containerNextSiblingInConstructor = "unset";
window.__attributeCountInConstructor = -1;
class MyCustomElement extends HTMLElement {
  constructor() {
    super();
    window.__attributeCountInConstructor = this.attributes.length;
    const container = document.getElementById('custom-element-container');
    for (let i = 0; i < container.childNodes.length; i++)
      window.__containerChildNodesInConstructor.push(container.childNodes[i]);
    window.__containerNextSiblingInConstructor = container.nextSibling;
  }
}
customElements.define('my-custom-element', MyCustomElement);
</script><div id="custom-element-container">
    <span id="custom-element-previous-element"></span>
    <my-custom-element id="candidate"></my-custom-element>
    <div id="custom-element-next-element"></div>
</div><script>
const instance = document.querySelector('my-custom-element');
document.body.setAttribute('data-result', [
  window.__containerChildNodesInConstructor.length,
  window.__containerChildNodesInConstructor[0] === instance.parentNode.firstChild,
  window.__containerChildNodesInConstructor[1] === document.getElementById('custom-element-previous-element'),
  window.__containerChildNodesInConstructor[2] === instance.previousSibling,
  window.__containerNextSiblingInConstructor === null,
  window.__attributeCountInConstructor,
  instance.getAttribute('id')
].join('|'));
</script>"#;
        let crate::parser::ParserPumpOutcome {
            result,
            discovered_async_prefetch_scripts: _,
            discovered_preload_link_candidates: _,
            discovered_blocking_stylesheet_inputs: _,
        } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(html);
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at initial inline script handoff");
        };

        let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
        let local_executor = JsLocalExecutor::new();
        let mut page_vm = PageVm::new(
            PageId::new_for_testing(104),
            local_executor.clone(),
            loader,
            &default_test_page_vm_env_config(),
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
            parser_dom_host,
            Instant::now(),
        )
        .expect("page vm");
        let page_vm_ptr: *mut PageVm = &mut page_vm;
        let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
        let outcome = super::access::run_named_owner_local_task(
            local_executor.clone(),
            "phase-one parser custom element sync setup handoff channel closed",
            async move {
                let page_vm = unsafe { &mut *page_vm_ptr };
                let driver = unsafe { &mut *driver_ptr };
                driver
                    .handle_parse_time_script_handoff(page_vm, *handoff, None)
                    .await
            },
        )
        .await
        .expect("initial customElements.define handoff should complete");
        assert!(matches!(outcome, ScriptHandoffOutcome::NoNavigation));

        let local_executor = page_vm.local_executor.clone();
        let page_vm_ptr: *mut PageVm = &mut page_vm;
        let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
        let outcome = super::access::run_named_owner_local_task(
            local_executor,
            "phase-one parser custom element continuation channel closed",
            async move {
                let page_vm = unsafe { &mut *page_vm_ptr };
                let driver = unsafe { &mut *driver_ptr };
                driver.advance_parser_step(page_vm, "", None).await
            },
        )
        .await
        .expect("parser continuation should complete");
        assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

        let snapshot = page_vm.vm().snapshot_live_document();
        let body = snapshot.document_body_handle().expect("body");
        let result = snapshot
            .node(body)
            .and_then(Node::as_element)
            .and_then(|element| element.attribute("data-result"));
        assert_eq!(result, Some("3|true|true|true|true|0|candidate"));
    }));
}
#[test]
fn parser_custom_element_inserts_constructor_returned_element() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let _js_runtime = crate::JsRuntime::initialize();
        let final_url =
            Url::parse("https://parser-returned-custom-element.test/").expect("test url");
        let loader: &'static ResourceRequestClient =
            Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
        let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url.clone())));
        let mut driver = ParserDriver {
            loader,
            final_url: &state.final_url,
            parser_session: &mut state.parser_session,
            scheduler: &mut state.scheduler,
            buffered_document_preloads: &mut state.buffered_document_preloads,
            service_worker_preload_context: state.service_worker_preload_context.as_ref(),
            input_closed: &state.input_closed,
        };
        let html = r#"<!doctype html><script>
let anotherElementCreatedBeforeSuperCall = undefined;
let elementCreatedBySuperCall = undefined;
let shouldCreateElementBeforeSuperCall = true;
class InstantiatesItselfBeforeSuper extends HTMLElement {
  constructor() {
    if (shouldCreateElementBeforeSuperCall) {
      shouldCreateElementBeforeSuperCall = false;
      anotherElementCreatedBeforeSuperCall = new InstantiatesItselfBeforeSuper();
    }
    super();
    elementCreatedBySuperCall = this;
  }
}
customElements.define('instantiates-itself-before-super', InstantiatesItselfBeforeSuper);

let shouldCreateAnotherInstance = true;
let anotherInstance = undefined;
let firstInstance = undefined;
class ReturnsAnotherInstance extends HTMLElement {
  constructor() {
    super();
    if (shouldCreateAnotherInstance) {
      shouldCreateAnotherInstance = false;
      firstInstance = this;
      anotherInstance = new ReturnsAnotherInstance();
      return anotherInstance;
    }
    return this;
  }
}
customElements.define('returns-another-instance', ReturnsAnotherInstance);
</script>
<instantiates-itself-before-super id="a"><span id="child-a"></span></instantiates-itself-before-super>
<returns-another-instance id="b"></returns-another-instance>
<script>
const instanceA = document.querySelector('instantiates-itself-before-super');
const instanceB = document.querySelector('returns-another-instance');
document.body.setAttribute('data-result', [
  instanceA instanceof InstantiatesItselfBeforeSuper,
  instanceA === elementCreatedBySuperCall,
  instanceA !== anotherElementCreatedBeforeSuperCall,
  anotherElementCreatedBeforeSuperCall.parentNode === null,
  instanceA.getAttribute('id'),
  instanceA.firstElementChild.id,
  instanceB instanceof ReturnsAnotherInstance,
  instanceB === anotherInstance,
  instanceB !== firstInstance,
  firstInstance.parentNode === null,
  instanceB.getAttribute('id')
].join('|'));
</script>"#;
        let crate::parser::ParserPumpOutcome {
            result,
            discovered_async_prefetch_scripts: _,
            discovered_preload_link_candidates: _,
            discovered_blocking_stylesheet_inputs: _,
        } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(html);
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at initial inline script handoff");
        };

        let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
        let local_executor = JsLocalExecutor::new();
        let mut page_vm = PageVm::new(
            PageId::new_for_testing(105),
            local_executor.clone(),
            loader,
            &default_test_page_vm_env_config(),
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
            parser_dom_host,
            Instant::now(),
        )
        .expect("page vm");
        let page_vm_ptr: *mut PageVm = &mut page_vm;
        let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
        let outcome = super::access::run_named_owner_local_task(
            local_executor.clone(),
            "phase-one parser custom element return setup handoff channel closed",
            async move {
                let page_vm = unsafe { &mut *page_vm_ptr };
                let driver = unsafe { &mut *driver_ptr };
                driver
                    .handle_parse_time_script_handoff(page_vm, *handoff, None)
                    .await
            },
        )
        .await
        .expect("initial customElements.define handoff should complete");
        assert!(matches!(outcome, ScriptHandoffOutcome::NoNavigation));

        let local_executor = page_vm.local_executor.clone();
        let page_vm_ptr: *mut PageVm = &mut page_vm;
        let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
        let outcome = super::access::run_named_owner_local_task(
            local_executor,
            "phase-one parser custom element return continuation channel closed",
            async move {
                let page_vm = unsafe { &mut *page_vm_ptr };
                let driver = unsafe { &mut *driver_ptr };
                driver.advance_parser_step(page_vm, "", None).await
            },
        )
        .await
        .expect("parser continuation should complete");
        assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

        let snapshot = page_vm.vm().snapshot_live_document();
        let body = snapshot.document_body_handle().expect("body");
        let result = snapshot
            .node(body)
            .and_then(Node::as_element)
            .and_then(|element| element.attribute("data-result"));
        assert_eq!(
            result,
            Some("true|true|true|true|a|child-a|true|true|true|true|b")
        );
    }));
}
#[test]
fn document_write_custom_element_direct_constructs_before_token_attributes() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),

                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
window.writeCeEvents = [];
window.WptWrittenTiming = class extends HTMLElement {
  static get observedAttributes() { return ['data-token']; }
  constructor() {
    super();
    let writeResult = 'missing';
    try {
      document.write('<b id="bad-write">bad</b>');
      writeResult = 'ok';
    } catch (error) {
      writeResult = error.name;
    }
    window.writeCeEvents.push([
      this.hasAttribute('data-token'),
      !!document.getElementById('after-write'),
      this.isConnected,
      writeResult
    ].join('|'));
    new MutationObserver((records) => {
      for (const record of records) {
        window.writeCeEvents.push(
          'mo:' + record.attributeName + ':' +
          this.getAttribute(record.attributeName)
        );
      }
    }).observe(this, { attributes: true });
  }
  attributeChangedCallback(name, oldValue, newValue) {
    window.writeCeEvents.push('attr:' + name + ':' + oldValue + ':' + newValue);
    Promise.resolve().then(() => {
      window.writeCeEvents.push('promise:' + this.getAttribute(name));
    });
  }
  connectedCallback() {
    window.writeCeEvents.push([
      'connected',
      this.getAttribute('data-token'),
      this.childNodes.length,
      !!document.getElementById('after-write'),
      this.isConnected
    ].join('|'));
  }
};
customElements.define('wpt-written-timing', window.WptWrittenTiming);
document.write('<wpt-written-timing data-token="owned"></wpt-written-timing><span id="after-write"></span>');
window.writeCeEvents.push('after-write');
</script>
<script>
const element = document.querySelector('wpt-written-timing');
document.body.setAttribute('data-events', window.writeCeEvents.join('|'));
document.body.setAttribute('data-first-event', window.writeCeEvents[0] || '');
document.body.setAttribute(
  'data-connected-event',
  window.writeCeEvents.find((event) => event.startsWith('connected')) || ''
);
document.body.setAttribute('data-token', element.getAttribute('data-token') || '');
document.body.setAttribute('data-after-visible', String(!!document.getElementById('after-write')));
document.body.setAttribute('data-instance', String(element instanceof window.WptWrittenTiming));
document.body.setAttribute('data-bad-write', String(!!document.getElementById('bad-write')));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one document.write custom element direct regression local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-first-event"),
                Some("false|false|false|InvalidStateError"),
                "document.write constructor must run before token attributes and following siblings are visible"
            );
            assert_eq!(
                body_element.attribute("data-events"),
                Some("false|false|false|InvalidStateError|attr:data-token:null:owned|connected|owned|0|false|true|after-write|mo:data-token:owned|promise:owned"),
                "document.write must deliver reactions before following tokens, then defer mutation observers and promises until its outer script returns"
            );
            assert_eq!(
                body_element.attribute("data-connected-event"),
                Some("connected|owned|0|false|true"),
                "document.write connectedCallback must run before child and following sibling tokens"
            );
            assert_eq!(body_element.attribute("data-token"), Some("owned"));
            assert_eq!(body_element.attribute("data-after-visible"), Some("true"));
            assert_eq!(body_element.attribute("data-instance"), Some("true"));
            assert_eq!(body_element.attribute("data-bad-write"), Some("false"));
        }));
}
