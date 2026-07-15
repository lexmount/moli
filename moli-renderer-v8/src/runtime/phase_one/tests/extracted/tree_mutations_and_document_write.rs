use super::*;

#[test]
fn parser_document_fragment_face_insertion_dispatches_form_reactions_like_js() {
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
window.parserFragmentFaceEvents = [];
class ParserFragmentFaceElement extends HTMLElement {
  static formAssociated = true;
  connectedCallback() {
    window.parserFragmentFaceEvents.push(`${this.dataset.role}:${this.dataset.name}:connected:${this.isConnected}`);
  }
  formAssociatedCallback(form) {
    window.parserFragmentFaceEvents.push(`${this.dataset.role}:${this.dataset.name}:form:${form && form.dataset.role}`);
  }
  formDisabledCallback(disabled) {
    window.parserFragmentFaceEvents.push(`${this.dataset.role}:${this.dataset.name}:disabled:${disabled}`);
  }
}
customElements.define('parser-fragment-face-element', ParserFragmentFaceElement);

function makeDisabledFieldset(idPrefix, role) {
  const form = document.createElement('form');
  form.id = `${idPrefix}-form`;
  form.dataset.role = role;
  const fieldset = document.createElement('fieldset');
  fieldset.id = `${idPrefix}-fieldset`;
  fieldset.disabled = true;
  form.appendChild(fieldset);
  document.body.appendChild(form);
  return fieldset;
}

function makeFace(id, role, name) {
  const element = document.createElement('parser-fragment-face-element');
  element.id = id;
  element.dataset.role = role;
  element.dataset.name = name;
  return element;
}

window.parserFragmentFaceJsAppendFieldset = makeDisabledFieldset('js-fragment-face-append', 'append');
window.parserFragmentFaceParserAppendFieldset = makeDisabledFieldset('parser-fragment-face-append', 'append');
window.parserFragmentFaceJsBeforeFieldset = makeDisabledFieldset('js-fragment-face-before', 'before');
window.parserFragmentFaceParserBeforeFieldset = makeDisabledFieldset('parser-fragment-face-before', 'before');
window.parserFragmentFaceJsBeforeReference = document.createElement('span');
window.parserFragmentFaceJsBeforeReference.id = 'js-fragment-face-before-reference';
window.parserFragmentFaceParserBeforeReference = document.createElement('span');
window.parserFragmentFaceParserBeforeReference.id = 'parser-fragment-face-before-reference';
window.parserFragmentFaceJsBeforeFieldset.appendChild(window.parserFragmentFaceJsBeforeReference);
window.parserFragmentFaceParserBeforeFieldset.appendChild(window.parserFragmentFaceParserBeforeReference);

window.parserFragmentFaceJsAppendA = makeFace('js-fragment-face-append-a', 'append', 'a');
window.parserFragmentFaceJsAppendB = makeFace('js-fragment-face-append-b', 'append', 'b');
window.parserFragmentFaceParserAppendA = makeFace('parser-fragment-face-append-a', 'append', 'a');
window.parserFragmentFaceParserAppendB = makeFace('parser-fragment-face-append-b', 'append', 'b');
window.parserFragmentFaceJsBeforeA = makeFace('js-fragment-face-before-a', 'before', 'a');
window.parserFragmentFaceJsBeforeB = makeFace('js-fragment-face-before-b', 'before', 'b');
window.parserFragmentFaceParserBeforeA = makeFace('parser-fragment-face-before-a', 'before', 'a');
window.parserFragmentFaceParserBeforeB = makeFace('parser-fragment-face-before-b', 'before', 'b');
document.body.append(
  window.parserFragmentFaceJsAppendA,
  window.parserFragmentFaceJsAppendB,
  window.parserFragmentFaceParserAppendA,
  window.parserFragmentFaceParserAppendB,
  window.parserFragmentFaceJsBeforeA,
  window.parserFragmentFaceJsBeforeB,
  window.parserFragmentFaceParserBeforeA,
  window.parserFragmentFaceParserBeforeB
);
window.parserFragmentFaceEvents.length = 0;
"#,
                )
                .expect("fragment FACE setup should evaluate");

            let (
                parser_append_fieldset,
                parser_append_a,
                parser_append_b,
                parser_before_fieldset,
                parser_before_a,
                parser_before_b,
                parser_before_reference,
            ) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-fragment-face-append-fieldset")
                        .expect("parser fragment FACE append fieldset should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-face-append-a")
                        .expect("parser fragment FACE append first element should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-face-append-b")
                        .expect("parser fragment FACE append second element should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-face-before-fieldset")
                        .expect("parser fragment FACE before fieldset should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-face-before-a")
                        .expect("parser fragment FACE before first element should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-face-before-b")
                        .expect("parser fragment FACE before second element should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-face-before-reference")
                        .expect("parser fragment FACE before reference should exist"),
                )
            };

            page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentFaceJsAppendA.remove();
window.parserFragmentFaceJsAppendB.remove();
window.parserFragmentFaceParserAppendA.remove();
window.parserFragmentFaceParserAppendB.remove();
window.parserFragmentFaceJsBeforeA.remove();
window.parserFragmentFaceJsBeforeB.remove();
window.parserFragmentFaceParserBeforeA.remove();
window.parserFragmentFaceParserBeforeB.remove();
window.parserFragmentFaceEvents.length = 0;

const jsAppendFragment = document.createDocumentFragment();
jsAppendFragment.append(window.parserFragmentFaceJsAppendA, window.parserFragmentFaceJsAppendB);
window.parserFragmentFaceJsAppendFieldset.appendChild(jsAppendFragment);
window.parserFragmentFaceJsAppendEvents = window.parserFragmentFaceEvents.slice();
window.parserFragmentFaceEvents.length = 0;

const jsBeforeFragment = document.createDocumentFragment();
jsBeforeFragment.append(window.parserFragmentFaceJsBeforeA, window.parserFragmentFaceJsBeforeB);
window.parserFragmentFaceJsBeforeFieldset.insertBefore(
  jsBeforeFragment,
  window.parserFragmentFaceJsBeforeReference
);
window.parserFragmentFaceJsBeforeEvents = window.parserFragmentFaceEvents.slice();
window.parserFragmentFaceEvents.length = 0;
"#,
                )
                .expect("fragment FACE JS baseline should evaluate");

            let parser_append_fragment = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let fragment = dom_host.create_document_fragment();
                assert!(dom_host.append_child(fragment, parser_append_a));
                assert!(dom_host.append_child(fragment, parser_append_b));
                fragment
            };
            let append_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_append_fieldset,
                    child: parser_append_fragment,
                },
                "parser fragment FACE append should apply",
            );
            assert!(
                !append_reaction_roots.is_empty(),
                "parser fragment FACE append should defer connected/form reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(parser_append_fragment)
                    .count(),
                0,
                "parser fragment FACE append should hoist and empty the fragment"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(append_reaction_roots)
                .expect("parser fragment FACE append reactions should dispatch");
            page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentFaceParserAppendEvents = window.parserFragmentFaceEvents.slice();
window.parserFragmentFaceEvents.length = 0;
"#,
                )
                .expect("fragment FACE parser append events should snapshot");

            let parser_before_fragment = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let fragment = dom_host.create_document_fragment();
                assert!(dom_host.append_child(fragment, parser_before_a));
                assert!(dom_host.append_child(fragment, parser_before_b));
                fragment
            };
            let before_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_before_fieldset,
                    child: parser_before_fragment,
                    reference_child: Some(parser_before_reference),
                },
                "parser fragment FACE insertBefore should apply",
            );
            assert!(
                !before_reaction_roots.is_empty(),
                "parser fragment FACE insertBefore should defer connected/form reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(parser_before_fragment)
                    .count(),
                0,
                "parser fragment FACE insertBefore should hoist and empty the fragment"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(before_reaction_roots)
                .expect("parser fragment FACE insertBefore reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"(() => {
  const parserBeforeEvents = window.parserFragmentFaceEvents.slice();
  const appendSame = JSON.stringify(window.parserFragmentFaceJsAppendEvents) ===
    JSON.stringify(window.parserFragmentFaceParserAppendEvents);
  const beforeSame = JSON.stringify(window.parserFragmentFaceJsBeforeEvents) ===
    JSON.stringify(parserBeforeEvents);
  return JSON.stringify({
    jsAppendEvents: window.parserFragmentFaceJsAppendEvents,
    parserAppendEvents: window.parserFragmentFaceParserAppendEvents,
    appendSame,
    jsBeforeEvents: window.parserFragmentFaceJsBeforeEvents,
    parserBeforeEvents,
    beforeSame,
    parserBeforeReferencePrevious: window.parserFragmentFaceParserBeforeReference.previousSibling &&
      window.parserFragmentFaceParserBeforeReference.previousSibling.id,
    parserBeforeBNext: window.parserFragmentFaceParserBeforeB.nextSibling &&
      window.parserFragmentFaceParserBeforeB.nextSibling.id
  });
})()"#,
                )
                .expect("fragment FACE result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppendEvents":["append:a:connected:true","append:a:form:append","append:a:disabled:true","append:b:connected:true","append:b:form:append","append:b:disabled:true"],"parserAppendEvents":["append:a:connected:true","append:a:form:append","append:a:disabled:true","append:b:connected:true","append:b:form:append","append:b:disabled:true"],"appendSame":true,"jsBeforeEvents":["before:a:connected:true","before:a:form:before","before:a:disabled:true","before:b:connected:true","before:b:form:before","before:b:disabled:true"],"parserBeforeEvents":["before:a:connected:true","before:a:form:before","before:a:disabled:true","before:b:connected:true","before:b:form:before","before:b:disabled:true"],"beforeSame":true,"parserBeforeReferencePrevious":"parser-fragment-face-before-b","parserBeforeBNext":"parser-fragment-face-before-reference"}"#
                ),
                "parser DocumentFragment insertion should dispatch FACE connected/form callbacks like JS fragment insertion for append and insertBefore"
            );
        }));
}
#[test]
fn parser_nested_face_insertion_dispatches_form_reactions_like_js() {
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
window.parserNestedFaceEvents = [];
class ParserNestedFaceElement extends HTMLElement {
  static formAssociated = true;
  connectedCallback() {
    window.parserNestedFaceEvents.push(`${this.dataset.mode}:${this.dataset.name}:connected:${this.isConnected}`);
  }
  formAssociatedCallback(form) {
    window.parserNestedFaceEvents.push(`${this.dataset.mode}:${this.dataset.name}:form:${form && form.dataset.mode}`);
  }
  formDisabledCallback(disabled) {
    window.parserNestedFaceEvents.push(`${this.dataset.mode}:${this.dataset.name}:disabled:${disabled}`);
  }
}
customElements.define('parser-nested-face-element', ParserNestedFaceElement);

function makeDisabledFieldset(idPrefix, mode) {
  const form = document.createElement('form');
  form.id = `${idPrefix}-form`;
  form.dataset.mode = mode;
  const fieldset = document.createElement('fieldset');
  fieldset.id = `${idPrefix}-fieldset`;
  fieldset.disabled = true;
  form.appendChild(fieldset);
  document.body.appendChild(form);
  return fieldset;
}

function makeNestedFace(prefix, mode) {
  const outer = document.createElement('parser-nested-face-element');
  outer.id = `${prefix}-outer`;
  outer.dataset.mode = mode;
  outer.dataset.name = 'outer';
  const inner = document.createElement('parser-nested-face-element');
  inner.id = `${prefix}-inner`;
  inner.dataset.mode = mode;
  inner.dataset.name = 'inner';
  outer.appendChild(inner);
  return outer;
}

window.parserNestedFaceJsAppendFieldset = makeDisabledFieldset('js-nested-face-append', 'append');
window.parserNestedFaceParserAppendFieldset = makeDisabledFieldset('parser-nested-face-append', 'append');
window.parserNestedFaceJsBeforeFieldset = makeDisabledFieldset('js-nested-face-before', 'before');
window.parserNestedFaceParserBeforeFieldset = makeDisabledFieldset('parser-nested-face-before', 'before');
window.parserNestedFaceJsBeforeReference = document.createElement('span');
window.parserNestedFaceJsBeforeReference.id = 'js-nested-face-before-reference';
window.parserNestedFaceParserBeforeReference = document.createElement('span');
window.parserNestedFaceParserBeforeReference.id = 'parser-nested-face-before-reference';
window.parserNestedFaceJsBeforeFieldset.appendChild(window.parserNestedFaceJsBeforeReference);
window.parserNestedFaceParserBeforeFieldset.appendChild(window.parserNestedFaceParserBeforeReference);

window.parserNestedFaceJsAppendOuter = makeNestedFace('js-nested-face-append', 'append');
window.parserNestedFaceParserAppendOuter = makeNestedFace('parser-nested-face-append', 'append');
window.parserNestedFaceJsBeforeOuter = makeNestedFace('js-nested-face-before', 'before');
window.parserNestedFaceParserBeforeOuter = makeNestedFace('parser-nested-face-before', 'before');
document.body.append(
  window.parserNestedFaceJsAppendOuter,
  window.parserNestedFaceParserAppendOuter,
  window.parserNestedFaceJsBeforeOuter,
  window.parserNestedFaceParserBeforeOuter
);
window.parserNestedFaceEvents.length = 0;
"#,
                )
                .expect("nested FACE setup should evaluate");

            let (
                parser_append_fieldset,
                parser_append_outer,
                parser_before_fieldset,
                parser_before_outer,
                parser_before_reference,
            ) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-nested-face-append-fieldset")
                        .expect("parser nested FACE append fieldset should exist"),
                    runtime
                        .get_element_by_id("parser-nested-face-append-outer")
                        .expect("parser nested FACE append outer should exist"),
                    runtime
                        .get_element_by_id("parser-nested-face-before-fieldset")
                        .expect("parser nested FACE before fieldset should exist"),
                    runtime
                        .get_element_by_id("parser-nested-face-before-outer")
                        .expect("parser nested FACE before outer should exist"),
                    runtime
                        .get_element_by_id("parser-nested-face-before-reference")
                        .expect("parser nested FACE before reference should exist"),
                )
            };

            page_vm
                .evaluate_expression(
                    r#"
window.parserNestedFaceJsAppendOuter.remove();
window.parserNestedFaceParserAppendOuter.remove();
window.parserNestedFaceJsBeforeOuter.remove();
window.parserNestedFaceParserBeforeOuter.remove();
window.parserNestedFaceEvents.length = 0;

window.parserNestedFaceJsAppendFieldset.appendChild(window.parserNestedFaceJsAppendOuter);
window.parserNestedFaceJsAppendEvents = window.parserNestedFaceEvents.slice();
window.parserNestedFaceEvents.length = 0;

window.parserNestedFaceJsBeforeFieldset.insertBefore(
  window.parserNestedFaceJsBeforeOuter,
  window.parserNestedFaceJsBeforeReference
);
window.parserNestedFaceJsBeforeEvents = window.parserNestedFaceEvents.slice();
window.parserNestedFaceEvents.length = 0;
"#,
                )
                .expect("nested FACE JS baseline should evaluate");

            let append_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_append_fieldset,
                    child: parser_append_outer,
                },
                "parser nested FACE append should apply",
            );
            assert!(
                !append_reaction_roots.is_empty(),
                "parser nested FACE append should defer connected/form reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(append_reaction_roots)
                .expect("parser nested FACE append reactions should dispatch");
            page_vm
                .evaluate_expression(
                    r#"
window.parserNestedFaceParserAppendEvents = window.parserNestedFaceEvents.slice();
window.parserNestedFaceEvents.length = 0;
"#,
                )
                .expect("nested FACE parser append events should snapshot");

            let before_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_before_fieldset,
                    child: parser_before_outer,
                    reference_child: Some(parser_before_reference),
                },
                "parser nested FACE insertBefore should apply",
            );
            assert!(
                !before_reaction_roots.is_empty(),
                "parser nested FACE insertBefore should defer connected/form reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(before_reaction_roots)
                .expect("parser nested FACE insertBefore reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"(() => {
  const parserBeforeEvents = window.parserNestedFaceEvents.slice();
  return JSON.stringify({
    jsAppendEvents: window.parserNestedFaceJsAppendEvents,
    parserAppendEvents: window.parserNestedFaceParserAppendEvents,
    appendSame: JSON.stringify(window.parserNestedFaceJsAppendEvents) ===
      JSON.stringify(window.parserNestedFaceParserAppendEvents),
    jsBeforeEvents: window.parserNestedFaceJsBeforeEvents,
    parserBeforeEvents,
    beforeSame: JSON.stringify(window.parserNestedFaceJsBeforeEvents) ===
      JSON.stringify(parserBeforeEvents),
    parserBeforeReferencePrevious: window.parserNestedFaceParserBeforeReference.previousSibling &&
      window.parserNestedFaceParserBeforeReference.previousSibling.id,
    parserBeforeOuterNext: window.parserNestedFaceParserBeforeOuter.nextSibling &&
      window.parserNestedFaceParserBeforeOuter.nextSibling.id
  });
})()"#,
                )
                .expect("nested FACE result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppendEvents":["append:outer:connected:true","append:outer:form:append","append:outer:disabled:true","append:inner:connected:true","append:inner:form:append","append:inner:disabled:true"],"parserAppendEvents":["append:outer:connected:true","append:outer:form:append","append:outer:disabled:true","append:inner:connected:true","append:inner:form:append","append:inner:disabled:true"],"appendSame":true,"jsBeforeEvents":["before:outer:connected:true","before:outer:form:before","before:outer:disabled:true","before:inner:connected:true","before:inner:form:before","before:inner:disabled:true"],"parserBeforeEvents":["before:outer:connected:true","before:outer:form:before","before:outer:disabled:true","before:inner:connected:true","before:inner:form:before","before:inner:disabled:true"],"beforeSame":true,"parserBeforeReferencePrevious":"parser-nested-face-before-outer","parserBeforeOuterNext":"parser-nested-face-before-reference"}"#
                ),
                "parser nested FACE insertion should dispatch connected/form callbacks like JS insertion for append and insertBefore"
            );
        }));
}
#[test]
fn parser_shadow_including_insertion_dispatches_connected_reactions_like_js() {
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
window.parserShadowInsertionEvents = [];
class ParserShadowInsertionElement extends HTMLElement {
  connectedCallback() {
    const root = this.getRootNode();
    window.parserShadowInsertionEvents.push(
      `${this.id}:connected:${this.isConnected}:${root instanceof ShadowRoot}:${root.host && root.host.id}`
    );
  }
  disconnectedCallback() {
    const root = this.getRootNode();
    window.parserShadowInsertionEvents.push(
      `${this.id}:disconnected:${this.isConnected}:${root instanceof ShadowRoot}:${root.host && root.host.id}`
    );
  }
}
customElements.define('parser-shadow-insertion-element', ParserShadowInsertionElement);

function makeParent(prefix) {
  const parent = document.createElement('section');
  parent.id = `${prefix}-parent`;
  document.body.appendChild(parent);
  return parent;
}

function makeBeforeParent(prefix) {
  const parent = makeParent(prefix);
  const reference = document.createElement('span');
  reference.id = `${prefix}-reference`;
  parent.appendChild(reference);
  return { parent, reference };
}

function makeShadowHost(prefix) {
  const host = document.createElement('div');
  host.id = `${prefix}-host`;
  const shadow = host.attachShadow({ mode: 'open' });
  const shadowChild = document.createElement('parser-shadow-insertion-element');
  shadowChild.id = `${prefix}-shadow-child`;
  shadow.appendChild(shadowChild);
  return host;
}

window.parserShadowJsAppendParent = makeParent('js-shadow-append');
window.parserShadowParserAppendParent = makeParent('parser-shadow-append');
const jsBefore = makeBeforeParent('js-shadow-before');
const parserBefore = makeBeforeParent('parser-shadow-before');
window.parserShadowJsBeforeParent = jsBefore.parent;
window.parserShadowJsBeforeReference = jsBefore.reference;
window.parserShadowParserBeforeParent = parserBefore.parent;
window.parserShadowParserBeforeReference = parserBefore.reference;

window.parserShadowJsAppendHost = makeShadowHost('js-shadow-append');
window.parserShadowParserAppendHost = makeShadowHost('parser-shadow-append');
window.parserShadowJsBeforeHost = makeShadowHost('js-shadow-before');
window.parserShadowParserBeforeHost = makeShadowHost('parser-shadow-before');
document.body.append(
  window.parserShadowJsAppendHost,
  window.parserShadowParserAppendHost,
  window.parserShadowJsBeforeHost,
  window.parserShadowParserBeforeHost
);
window.parserShadowInsertionEvents.length = 0;
"#,
                )
                .expect("shadow-including insertion setup should evaluate");

            let (
                parser_append_parent,
                parser_append_host,
                parser_before_parent,
                parser_before_host,
                parser_before_reference,
            ) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-shadow-append-parent")
                        .expect("parser shadow append parent should exist"),
                    runtime
                        .get_element_by_id("parser-shadow-append-host")
                        .expect("parser shadow append host should exist"),
                    runtime
                        .get_element_by_id("parser-shadow-before-parent")
                        .expect("parser shadow before parent should exist"),
                    runtime
                        .get_element_by_id("parser-shadow-before-host")
                        .expect("parser shadow before host should exist"),
                    runtime
                        .get_element_by_id("parser-shadow-before-reference")
                        .expect("parser shadow before reference should exist"),
                )
            };

            page_vm
                .evaluate_expression(
                    r#"
window.parserShadowJsAppendHost.remove();
window.parserShadowParserAppendHost.remove();
window.parserShadowJsBeforeHost.remove();
window.parserShadowParserBeforeHost.remove();
window.parserShadowInsertionEvents.length = 0;

window.parserShadowJsAppendParent.appendChild(window.parserShadowJsAppendHost);
window.parserShadowJsAppendEvents = window.parserShadowInsertionEvents.slice();
window.parserShadowInsertionEvents.length = 0;

window.parserShadowJsBeforeParent.insertBefore(
  window.parserShadowJsBeforeHost,
  window.parserShadowJsBeforeReference
);
window.parserShadowJsBeforeEvents = window.parserShadowInsertionEvents.slice();
window.parserShadowInsertionEvents.length = 0;
"#,
                )
                .expect("shadow-including JS baseline should evaluate");

            let append_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_append_parent,
                    child: parser_append_host,
                },
                "parser shadow-including append should apply",
            );
            assert!(
                !append_reaction_roots.is_empty(),
                "parser shadow-including append should defer connected reactions for shadow descendants"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(append_reaction_roots)
                .expect("parser shadow-including append reactions should dispatch");
            page_vm
                .evaluate_expression(
                    r#"
window.parserShadowParserAppendEvents = window.parserShadowInsertionEvents.slice();
window.parserShadowInsertionEvents.length = 0;
"#,
                )
                .expect("shadow-including parser append events should snapshot");

            let before_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_before_parent,
                    child: parser_before_host,
                    reference_child: Some(parser_before_reference),
                },
                "parser shadow-including insertBefore should apply",
            );
            assert!(
                !before_reaction_roots.is_empty(),
                "parser shadow-including insertBefore should defer connected reactions for shadow descendants"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(before_reaction_roots)
                .expect("parser shadow-including insertBefore reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"(() => {
  const parserBeforeEvents = window.parserShadowInsertionEvents.slice();
  const normalize = (events, prefix) =>
    events.map(event => event.replaceAll(prefix, 'target'));
  return JSON.stringify({
    jsAppendEvents: window.parserShadowJsAppendEvents,
    parserAppendEvents: window.parserShadowParserAppendEvents,
    appendSame: JSON.stringify(normalize(window.parserShadowJsAppendEvents, 'js-shadow-append')) ===
      JSON.stringify(normalize(window.parserShadowParserAppendEvents, 'parser-shadow-append')),
    jsBeforeEvents: window.parserShadowJsBeforeEvents,
    parserBeforeEvents,
    beforeSame: JSON.stringify(normalize(window.parserShadowJsBeforeEvents, 'js-shadow-before')) ===
      JSON.stringify(normalize(parserBeforeEvents, 'parser-shadow-before')),
    parserBeforeReferencePrevious: window.parserShadowParserBeforeReference.previousSibling &&
      window.parserShadowParserBeforeReference.previousSibling.id,
    parserBeforeHostNext: window.parserShadowParserBeforeHost.nextSibling &&
      window.parserShadowParserBeforeHost.nextSibling.id
  });
})()"#,
                )
                .expect("shadow-including insertion result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppendEvents":["js-shadow-append-shadow-child:connected:true:true:js-shadow-append-host"],"parserAppendEvents":["parser-shadow-append-shadow-child:connected:true:true:parser-shadow-append-host"],"appendSame":true,"jsBeforeEvents":["js-shadow-before-shadow-child:connected:true:true:js-shadow-before-host"],"parserBeforeEvents":["parser-shadow-before-shadow-child:connected:true:true:parser-shadow-before-host"],"beforeSame":true,"parserBeforeReferencePrevious":"parser-shadow-before-host","parserBeforeHostNext":"parser-shadow-before-reference"}"#
                ),
                "parser connected insertion should dispatch connectedCallback for upgraded shadow-including descendants like JS insertion"
            );
        }));
}
#[test]
fn parser_shadow_including_remove_reparent_dispatches_disconnected_reactions_like_js() {
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
window.parserShadowDisconnectEvents = [];
class ParserShadowDisconnectedElement extends HTMLElement {
  connectedCallback() {
    const root = this.getRootNode();
    window.parserShadowDisconnectEvents.push(
      `${this.id}:connected:${this.isConnected}:${root instanceof ShadowRoot}:${root.host && root.host.id}`
    );
  }
  disconnectedCallback() {
    const root = this.getRootNode();
    window.parserShadowDisconnectEvents.push(
      `${this.id}:disconnected:${this.isConnected}:${root instanceof ShadowRoot}:${root.host && root.host.id}`
    );
  }
}
customElements.define('parser-shadow-disconnected-element', ParserShadowDisconnectedElement);

function makeConnectedParent(prefix) {
  const parent = document.createElement('section');
  parent.id = `${prefix}-parent`;
  document.body.appendChild(parent);
  return parent;
}

function makeShadowHost(prefix) {
  const host = document.createElement('div');
  host.id = `${prefix}-host`;
  const shadow = host.attachShadow({ mode: 'open' });
  const shadowChild = document.createElement('parser-shadow-disconnected-element');
  shadowChild.id = `${prefix}-shadow-child`;
  shadow.appendChild(shadowChild);
  return host;
}

window.parserShadowJsRemoveParent = makeConnectedParent('js-shadow-remove');
window.parserShadowParserRemoveParent = makeConnectedParent('parser-shadow-remove');
window.parserShadowJsAppendParent = makeConnectedParent('js-shadow-append-disconnect');
window.parserShadowParserAppendParent = makeConnectedParent('parser-shadow-append-disconnect');
window.parserShadowJsBeforeParent = makeConnectedParent('js-shadow-before-disconnect');
window.parserShadowParserBeforeParent = makeConnectedParent('parser-shadow-before-disconnect');

window.parserShadowJsRemoveHost = makeShadowHost('js-shadow-remove');
window.parserShadowParserRemoveHost = makeShadowHost('parser-shadow-remove');
window.parserShadowJsAppendHost = makeShadowHost('js-shadow-append-disconnect');
window.parserShadowParserAppendHost = makeShadowHost('parser-shadow-append-disconnect');
window.parserShadowJsBeforeHost = makeShadowHost('js-shadow-before-disconnect');
window.parserShadowParserBeforeHost = makeShadowHost('parser-shadow-before-disconnect');

window.parserShadowJsRemoveParent.appendChild(window.parserShadowJsRemoveHost);
window.parserShadowParserRemoveParent.appendChild(window.parserShadowParserRemoveHost);
window.parserShadowJsAppendParent.appendChild(window.parserShadowJsAppendHost);
window.parserShadowParserAppendParent.appendChild(window.parserShadowParserAppendHost);
window.parserShadowJsBeforeParent.appendChild(window.parserShadowJsBeforeHost);
window.parserShadowParserBeforeParent.appendChild(window.parserShadowParserBeforeHost);

window.parserShadowJsAppendDetachedParent = document.createElement('div');
window.parserShadowJsBeforeDetachedParent = document.createElement('div');
window.parserShadowJsBeforeReference = document.createElement('span');
window.parserShadowJsBeforeReference.id = 'js-shadow-before-disconnect-reference';
window.parserShadowJsBeforeDetachedParent.appendChild(window.parserShadowJsBeforeReference);

window.parserShadowDisconnectEvents.length = 0;
window.parserShadowJsRemoveParent.removeChild(window.parserShadowJsRemoveHost);
window.parserShadowJsRemoveEvents = window.parserShadowDisconnectEvents.slice();
window.parserShadowDisconnectEvents.length = 0;

window.parserShadowJsAppendDetachedParent.appendChild(window.parserShadowJsAppendHost);
window.parserShadowJsAppendEvents = window.parserShadowDisconnectEvents.slice();
window.parserShadowDisconnectEvents.length = 0;

window.parserShadowJsBeforeDetachedParent.insertBefore(
  window.parserShadowJsBeforeHost,
  window.parserShadowJsBeforeReference
);
window.parserShadowJsBeforeEvents = window.parserShadowDisconnectEvents.slice();
window.parserShadowDisconnectEvents.length = 0;
"#,
                )
                .expect("shadow-including disconnected setup should evaluate");

            let (
                parser_remove_parent,
                parser_remove_host,
                parser_append_host,
                parser_before_host,
                append_detached_parent,
                before_detached_parent,
                before_reference,
            ) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let parser_remove_parent = dom_host
                    .element_handle_by_id("parser-shadow-remove-parent")
                    .expect("parser shadow remove parent should exist");
                let parser_remove_host = dom_host
                    .element_handle_by_id("parser-shadow-remove-host")
                    .expect("parser shadow remove host should exist");
                let parser_append_host = dom_host
                    .element_handle_by_id("parser-shadow-append-disconnect-host")
                    .expect("parser shadow append host should exist");
                let parser_before_host = dom_host
                    .element_handle_by_id("parser-shadow-before-disconnect-host")
                    .expect("parser shadow before host should exist");
                let append_detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
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
                assert!(dom_host.set_attribute(
                    before_reference,
                    "id",
                    "parser-shadow-before-disconnect-reference"
                ));
                assert!(dom_host.append_child(before_detached_parent, before_reference));
                (
                    parser_remove_parent,
                    parser_remove_host,
                    parser_append_host,
                    parser_before_host,
                    append_detached_parent,
                    before_detached_parent,
                    before_reference,
                )
            };

            let reaction_roots = ParserPostStepRuntimeWorkForTest::merge_for_test([
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::RemoveChild {
                        parent: parser_remove_parent,
                        child: parser_remove_host,
                    },
                    "parser shadow-including remove should apply",
                ),
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: append_detached_parent,
                        child: parser_append_host,
                    },
                    "parser shadow-including append to detached parent should apply",
                ),
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: before_detached_parent,
                        child: parser_before_host,
                        reference_child: Some(before_reference),
                    },
                    "parser shadow-including insertBefore into detached parent should apply",
                ),
            ]);
            assert!(
                !reaction_roots.is_empty(),
                "parser shadow-including remove/reparent should defer disconnected reactions for shadow descendants"
            );

            {
                let dom_host = page_vm.vm().document_runtime.dom_host();
                assert_eq!(
                    dom_host
                        .child_handles(before_detached_parent)
                        .collect::<Vec<_>>(),
                    vec![parser_before_host, before_reference],
                    "parser shadow-including insertBefore should move the host before the native reference"
                );
                assert_eq!(
                    dom_host
                        .child_handles(append_detached_parent)
                        .collect::<Vec<_>>(),
                    vec![parser_append_host],
                    "parser shadow-including AppendChild should move the host under the native detached parent"
                );
            }

            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(reaction_roots)
                .expect("parser shadow-including disconnected reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"(() => {
  const normalize = (events) =>
    events.map(event => event.replaceAll('js-shadow-', '').replaceAll('parser-shadow-', ''));
  const parserEvents = window.parserShadowDisconnectEvents.slice();
  return JSON.stringify({
    jsEvents: [
      ...window.parserShadowJsRemoveEvents,
      ...window.parserShadowJsAppendEvents,
      ...window.parserShadowJsBeforeEvents
    ],
    parserEvents,
    same: JSON.stringify(normalize([
      ...window.parserShadowJsRemoveEvents,
      ...window.parserShadowJsAppendEvents,
      ...window.parserShadowJsBeforeEvents
    ])) === JSON.stringify(normalize(parserEvents)),
    parserRemoveParent: window.parserShadowParserRemoveHost.parentNode && window.parserShadowParserRemoveHost.parentNode.id,
    parserAppendParentConnected: window.parserShadowParserAppendHost.parentNode &&
      window.parserShadowParserAppendHost.parentNode.isConnected,
    parserBeforeParentConnected: window.parserShadowParserBeforeHost.parentNode &&
      window.parserShadowParserBeforeHost.parentNode.isConnected,
    parserBeforeHostNext: window.parserShadowParserBeforeHost.nextSibling &&
      window.parserShadowParserBeforeHost.nextSibling.id
  });
})()"#,
                )
                .expect("shadow-including disconnected result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-shadow-remove-shadow-child:disconnected:false:true:js-shadow-remove-host","js-shadow-append-disconnect-shadow-child:disconnected:false:true:js-shadow-append-disconnect-host","js-shadow-before-disconnect-shadow-child:disconnected:false:true:js-shadow-before-disconnect-host"],"parserEvents":["parser-shadow-remove-shadow-child:disconnected:false:true:parser-shadow-remove-host","parser-shadow-append-disconnect-shadow-child:disconnected:false:true:parser-shadow-append-disconnect-host","parser-shadow-before-disconnect-shadow-child:disconnected:false:true:parser-shadow-before-disconnect-host"],"same":true,"parserRemoveParent":null,"parserAppendParentConnected":false,"parserBeforeParentConnected":false,"parserBeforeHostNext":"parser-shadow-before-disconnect-reference"}"#
                ),
                "parser remove/reparent to a disconnected parent should dispatch disconnectedCallback for upgraded shadow-including descendants like JS mutation"
            );
        }));
}
#[test]
fn parser_reparent_selected_option_preserves_selectedness_like_js() {
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
window.parserOptionSelectedStates = {};
function optionMoveState(select, fallback, chosen) {
  return {
    selectedIndex: select.selectedIndex,
    value: select.value,
    fallbackSelected: fallback.selected,
    chosenSelected: chosen.selected,
    chosenParent: chosen.parentNode && chosen.parentNode.nodeName
  };
}
function optionInsertBeforeState(select, fallback, chosen, reference) {
  const state = optionMoveState(select, fallback, chosen);
  state.chosenNextIsReference = chosen.nextSibling === reference;
  return state;
}

const jsSelect = document.createElement('select');
jsSelect.id = 'js-option-move-select';
const jsFallback = new Option('fallback', 'fallback');
jsFallback.id = 'js-option-move-fallback';
const jsChosen = new Option('chosen', 'chosen');
jsChosen.id = 'js-option-move-chosen';
jsSelect.append(jsFallback, jsChosen);

const parserSelect = document.createElement('select');
parserSelect.id = 'parser-option-move-select';
const parserFallback = new Option('fallback', 'fallback');
parserFallback.id = 'parser-option-move-fallback';
const parserChosen = new Option('chosen', 'chosen');
parserChosen.id = 'parser-option-move-chosen';
parserSelect.append(parserFallback, parserChosen);

const jsBeforeSelect = document.createElement('select');
jsBeforeSelect.id = 'js-option-before-select';
const jsBeforeFallback = new Option('fallback', 'fallback');
jsBeforeFallback.id = 'js-option-before-fallback';
const jsBeforeChosen = new Option('chosen', 'chosen');
jsBeforeChosen.id = 'js-option-before-chosen';
jsBeforeSelect.append(jsBeforeFallback, jsBeforeChosen);
const jsBeforeReference = document.createElement('span');
jsBeforeReference.id = 'js-option-before-reference';

const parserBeforeSelect = document.createElement('select');
parserBeforeSelect.id = 'parser-option-before-select';
const parserBeforeFallback = new Option('fallback', 'fallback');
parserBeforeFallback.id = 'parser-option-before-fallback';
const parserBeforeChosen = new Option('chosen', 'chosen');
parserBeforeChosen.id = 'parser-option-before-chosen';
parserBeforeSelect.append(parserBeforeFallback, parserBeforeChosen);
const parserBeforeReference = document.createElement('span');
parserBeforeReference.id = 'parser-option-before-reference';

document.body.append(
  jsSelect,
  parserSelect,
  jsBeforeSelect,
  jsBeforeReference,
  parserBeforeSelect,
  parserBeforeReference
);
jsSelect.selectedIndex = 1;
parserSelect.selectedIndex = 1;
jsBeforeSelect.selectedIndex = 1;
parserBeforeSelect.selectedIndex = 1;
window.parserOptionMoveSelect = parserSelect;
window.parserOptionMoveFallback = parserFallback;
window.parserOptionMoveChosen = parserChosen;
window.parserOptionBeforeSelect = parserBeforeSelect;
window.parserOptionBeforeFallback = parserBeforeFallback;
window.parserOptionBeforeChosen = parserBeforeChosen;
window.parserOptionBeforeReference = parserBeforeReference;

document.body.appendChild(jsChosen);
document.body.insertBefore(jsBeforeChosen, jsBeforeReference);
window.parserOptionSelectedStates.jsAppend = optionMoveState(jsSelect, jsFallback, jsChosen);
window.parserOptionSelectedStates.jsInsertBefore =
  optionInsertBeforeState(jsBeforeSelect, jsBeforeFallback, jsBeforeChosen, jsBeforeReference);
"#,
                )
                .expect("option selectedness setup should evaluate");

            let (parser_chosen, parser_before_chosen, parser_before_reference) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-option-move-chosen")
                        .expect("parser selected option should exist"),
                    runtime
                        .get_element_by_id("parser-option-before-chosen")
                        .expect("parser insertBefore selected option should exist"),
                    runtime
                        .get_element_by_id("parser-option-before-reference")
                        .expect("parser insertBefore reference should exist"),
                )
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: parser_chosen,
                    },
                    "parser selected option reparent should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain option reparent should not queue custom element reactions"
            );
            let insert_before_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: parser_before_chosen,
                        reference_child: Some(parser_before_reference),
                    },
                    "parser selected option insertBefore reparent should apply",
                )
            };
            assert!(
                insert_before_reaction_roots.is_empty(),
                "plain option insertBefore reparent should not queue custom element reactions"
            );

            let result = page_vm
                .evaluate_expression(
                    r#"
window.parserOptionSelectedStates.parserAppend = optionMoveState(
  window.parserOptionMoveSelect,
  window.parserOptionMoveFallback,
  window.parserOptionMoveChosen
);
window.parserOptionSelectedStates.parserInsertBefore = optionInsertBeforeState(
  window.parserOptionBeforeSelect,
  window.parserOptionBeforeFallback,
  window.parserOptionBeforeChosen,
  window.parserOptionBeforeReference
);
JSON.stringify({
  jsAppend: window.parserOptionSelectedStates.jsAppend,
  parserAppend: window.parserOptionSelectedStates.parserAppend,
  appendSame: JSON.stringify(window.parserOptionSelectedStates.jsAppend) ===
    JSON.stringify(window.parserOptionSelectedStates.parserAppend),
  jsInsertBefore: window.parserOptionSelectedStates.jsInsertBefore,
  parserInsertBefore: window.parserOptionSelectedStates.parserInsertBefore,
  insertBeforeSame: JSON.stringify(window.parserOptionSelectedStates.jsInsertBefore) ===
    JSON.stringify(window.parserOptionSelectedStates.parserInsertBefore)
})
"#,
                )
                .expect("option selectedness comparison should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppend":{"selectedIndex":0,"value":"fallback","fallbackSelected":true,"chosenSelected":true,"chosenParent":"BODY"},"parserAppend":{"selectedIndex":0,"value":"fallback","fallbackSelected":true,"chosenSelected":true,"chosenParent":"BODY"},"appendSame":true,"jsInsertBefore":{"selectedIndex":0,"value":"fallback","fallbackSelected":true,"chosenSelected":true,"chosenParent":"BODY","chosenNextIsReference":true},"parserInsertBefore":{"selectedIndex":0,"value":"fallback","fallbackSelected":true,"chosenSelected":true,"chosenParent":"BODY","chosenNextIsReference":true},"insertBeforeSame":true}"#
                ),
                "parser reparent should preserve selected option state like JS appendChild and insertBefore"
            );
        }));
}
#[test]
fn parser_connected_head_document_write_keeps_later_head_tokens_in_head() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = Box::leak(Box::new(
                ResourceRequestClient::new(&FetchConfig::default()).expect("default loader"),
            ));
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

            let html = "<!doctype html><html><head><script>document.write('<style>.runtime-style{color:red}</style>');document.write('<script>window.__docWriteHeadMutation=true;<\\/script>');</script><meta charset='utf-8'><title>x</title></head><body><main>late</main></body></html>";
            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(html);
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at inline script handoff");
            };

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
                    root_frame_id: None,
                    main_document_commit: None,
                    top_level_storage_key: None,
                    document_start_scripts: vec![],
                    runtime_bindings: vec![],
                    runtime_inspector_session_restore_snapshots: vec![],
                    runtime_isolated_worlds: vec![],
                    permission_overrides: vec![],
                    extra_http_headers: Default::default(),
                    navigator_identity: Default::default(),
                    document_policy_container: Default::default(),
                    document_default_language: None,
                    document_last_modified: None,
                    document_settings: Default::default(),
                    network_offline: false,
                    blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
            storage_bucket_store: None,
                    fetch_subresource_interception_enabled: false,
                    fetch_subresource_interception_resource_type: None,
                    layout_configuration: moli_page_types::LayoutConfiguration {
                        policy: moli_page_types::LayoutPolicy::default(),
                        scrollbars_hidden: false,
                    },
                    wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
},
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let handoff = handoff.clone();
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one document.write handoff local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .handle_parse_time_script_handoff(page_vm, *handoff, None)
                        .await
                },
            )
            .await
            .expect("inline document.write handoff should execute");
            assert!(
                matches!(outcome, ScriptHandoffOutcome::NoNavigation),
                "inline document.write fixture should not navigate"
            );

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one document.write continuation local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, "", None).await
                },
            )
            .await
            .expect("parser should continue after the inline script");
            assert!(
                matches!(outcome, ParserStepAdvanceOutcome::Continue),
                "parser should finish the remaining buffered html after the script"
            );

            let snapshot = page_vm.vm().snapshot_live_document();
            let serialized = snapshot.serialize_document();
            assert!(
                serialized.to_ascii_lowercase().contains("<!doctype html>"),
                "doctype should survive parser-connected document.write execution: {serialized}"
            );

            let inserted_script = snapshot
                .script_handles()
                .into_iter()
                .find(|handle| {
                    snapshot
                        .direct_text_content(*handle)
                        .is_some_and(|source| {
                            source
                                .trim_start()
                                .starts_with("window.__docWriteHeadMutation")
                        })
                })
                .expect("document.write-inserted script should remain in the live document");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .parser_script_start_position(inserted_script),
                Some(crate::document_runtime::ParserScriptStartPosition {
                    line: 0,
                    column: 0,
                }),
                "document.write-generated script source positions are intentionally unknown"
            );

            let head = snapshot.document_head_handle().expect("head should exist");
            let body = snapshot.document_body_handle().expect("body should exist");
            let head_children = snapshot.child_ids(head).collect::<Vec<_>>();
            let body_children = snapshot.child_ids(body).collect::<Vec<_>>();

            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("meta"))
                }),
                "later <meta> should stay under <head>: {serialized}"
            );
            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("title"))
                }),
                "later <title> should stay under <head>: {serialized}"
            );
            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("style"))
                }),
                "document.write-inserted <style> should stay under <head>: {serialized}"
            );
            assert!(
                body_children.iter().all(|handle| {
                    !snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| {
                            element.is_html_element("meta") || element.is_html_element("title")
                        })
                }),
                "<body> should not receive later head-only tokens: {serialized}"
            );
        }));
}
#[test]
fn document_write_fostered_text_updates_live_range_boundaries() {
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

            let html = r##"<!doctype html><html><body><table id="t"><script>
window.fosterRange = document.createRange();
window.fosterRange.setStart(document.body, document.body.childNodes.length);
window.fosterRange.setEnd(document.body, document.body.childNodes.length);
window.fosterBefore = document.body.childNodes.length;
document.write("hello");
</script></table><script>
document.body.setAttribute("data-range", [
  window.fosterBefore,
  window.fosterRange.startOffset,
  window.fosterRange.endOffset,
  Array.from(document.body.childNodes).map(node => node.nodeName + (node.id ? "#" + node.id : "")).join(",")
].join("|"));
</script></body></html>"##;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one document.write fostered text live range local task channel closed",
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
                body_element.attribute("data-range"),
                Some("1|2|2|#text,TABLE#t,SCRIPT"),
                "parser-stream document.write foster parenting must update live Range boundaries like browser DOM mutation"
            );
        }));
}

    #[test]
    fn parser_option_finish_and_select_value_sync_selectedcontent_clones() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><body>
<select id="select">
  <button><selectedcontent id="selectedcontent">default</selectedcontent></button>
  <div><option id="one"><span id="source-span">one</span></option></div>
  <div><option id="two"><strong>two</strong></option></div>
</select>
<script>
const select = document.getElementById('select');
const selectedcontent = document.getElementById('selectedcontent');
const sourceSpan = document.querySelector('#one > span');
window.selectedcontentState = [
  selectedcontent.textContent.trim(),
  selectedcontent.firstElementChild !== sourceSpan,
];
select.value = 'two';
window.selectedcontentState.push(
  selectedcontent.textContent.trim(),
  selectedcontent.firstElementChild.tagName,
);
</script>
</body></html>"#,
            )
            .await;

            let result = page_vm
                .evaluate_expression("JSON.stringify(window.selectedcontentState)")
                .expect("selectedcontent parser state should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"["one",true,"two","STRONG"]"#),
                "parser option completion and select.value must synchronously clone the selected option children"
            );
        }));
    }

    #[test]
    fn parser_eof_option_finish_syncs_selectedcontent_clones() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut text_page_vm = parse_finished_phase_one_html_into_page_vm_for_test(
                r#"<select><button><selectedcontent></button><option>X"#,
            )
            .await;
            let text_result = text_page_vm
                .evaluate_expression(
                    r#"
(() => {
  const selectedcontent = document.querySelector('selectedcontent');
  const source = document.querySelector('option');
  return [
    selectedcontent.textContent,
    selectedcontent.firstChild !== source.firstChild
  ].join('|');
})()
"#,
                )
                .expect("EOF-closed text option selectedcontent state should evaluate");
            assert_eq!(
                text_result.get("value").and_then(serde_json::Value::as_str),
                Some("X|true"),
                "EOF-closing an option must clone its text into selectedcontent"
            );

            let mut nested_page_vm = parse_finished_phase_one_html_into_page_vm_for_test(
                r#"<select><button><selectedcontent></button><option>x<i>i<b>ib</i>b"#,
            )
            .await;
            let nested_result = nested_page_vm
                .evaluate_expression(
                    r#"
(() => {
  const selectedcontent = document.querySelector('selectedcontent');
  const source = document.querySelector('option');
  return [
    selectedcontent.textContent,
    selectedcontent.innerHTML === source.innerHTML,
    selectedcontent.firstChild !== source.firstChild,
    selectedcontent.querySelector('i') !== source.querySelector('i'),
    selectedcontent.querySelectorAll('b').length
  ].join('|');
})()
"#,
                )
                .expect("EOF-closed nested option selectedcontent state should evaluate");
            assert_eq!(
                nested_result
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some("xiibb|true|true|true|2"),
                "EOF-closing an option must deep-clone its parsed children into selectedcontent"
            );
        }));
    }
