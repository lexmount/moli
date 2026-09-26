use super::*;

#[test]
fn editing_host_focus_initializes_the_first_visible_dom_caret() {
    let mut vm = new_storage_test_vm("https://editing-focus-selection.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/editing-host-focus-selection.js"
        ))
        .expect("editing focus should initialize a native caret"),
        ""
    );
}

#[test]
fn editing_host_focus_preserves_selection_identity_and_native_host_semantics() {
    let mut vm = new_storage_test_vm("https://editing-focus-retention.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/editing-host-focus-retention.js"
        ))
        .expect("editing focus should preserve existing host selections"),
        ""
    );
}

#[test]
fn editing_host_focus_selection_uses_owner_realm_and_shadow_boundaries() {
    let mut vm = new_storage_test_vm("https://editing-focus-realms.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/editing-host-focus-realms.js"
        ))
        .expect("editing focus should respect realms and shadow trees"),
        ""
    );
}

#[test]
fn editing_host_initial_caret_is_used_by_native_text_input() {
    let mut vm = new_streamed_parser_test_vm(
        "https://editing-focus-input.test/",
        r#"<!doctype html><body><div id="editor" contenteditable><p> abc</p></div>"#,
    );
    vm.eval("document.getElementById('editor').focus()")
        .expect("editor should focus");
    assert!(
        vm.insert_text_into_active_control("X")
            .expect("insert text")
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify([document.getElementById('editor').innerHTML, getSelection().anchorOffset])"
        )
        .expect("text should be inserted at initial caret"),
        r#"["<p> Xabc</p>",2]"#
    );
}

#[tokio::test]
async fn editing_host_focus_selectionchange_is_queued_and_coalesced() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://editing-focus-events.test/",
        &loader,
    );
    assert_eq!(
        vm.eval(r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = '<button id="reset"></button><div id="editor" contenteditable>abc</div><div id="other" contenteditable>def</div>';
  globalThis.editingFocusEvents = [];
  document.getElementById('editor').focus();
  document.getElementById('other').focus();
  document.getElementById('reset').focus();
  document.getElementById('other').focus();
  document.addEventListener('selectionchange', event => {
    editingFocusEvents.push([event.target === document, event.bubbles, event.composed].join(':'));
  });
  return editingFocusEvents.length;
})()
"#).expect("selectionchange should not dispatch synchronously"),
        "0"
    );
    for _ in 0..16 {
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("selection tasks should run")
        {
            break;
        }
    }
    assert_eq!(
        vm.eval("editingFocusEvents.join('|')").expect("event log"),
        "true:false:false"
    );
    vm.eval(
        "editingFocusEvents.length = 0; document.getElementById('reset').focus(); document.getElementById('other').focus();",
    )
    .expect("refocus should retain the range");
    for _ in 0..16 {
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("refocus tasks should run")
        {
            break;
        }
    }
    assert_eq!(
        vm.eval("editingFocusEvents.length").expect("event log"),
        "0"
    );
}

#[test]
fn text_control_focus_projects_selection_without_replacing_retained_ranges() {
    let mut vm = new_storage_test_vm("https://focus-selection.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/text-control-focus-selection.js"
        ))
        .expect("text control focus should update Selection natively"),
        ""
    );
}

#[test]
fn text_control_focus_selection_uses_owner_realm_and_rescopes_shadow_boundaries() {
    let mut vm = new_storage_test_vm("https://focus-selection-realms.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/text-control-focus-selection-realms.js"
        ))
        .expect("focus selection should respect owner realms and shadow roots"),
        ""
    );
}

#[tokio::test]
async fn text_control_focus_selectionchange_is_queued_and_coalesced_for_native_targets() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://focus-selection-events.test/",
        &loader,
    );
    assert_eq!(
        vm.eval(r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = '<input id="a"><input id="b"><button id="reset">reset</button><div id="host"></div>';
  const a = document.getElementById('a');
  const b = document.getElementById('b');
  const c = document.getElementById('host').attachShadow({mode: 'closed'})
    .appendChild(document.createElement('input'));
  globalThis.control = c;
  globalThis.focusSelectionEvents = [];
  a.focus();
  b.focus();
  b.focus();
  c.focus();
  c.blur();
  c.focus();
  // Register after focus to check that event scheduling does not depend on
  // whether a listener existed when the selection changed.
  document.addEventListener('selectionchange', event => {
    focusSelectionEvents.push([
      event.target === document ? 'document' : event.target.id,
      event.bubbles, event.composed
    ].join(':'));
  });
  return focusSelectionEvents.length;
})()
"#).expect("focus should queue selection changes"),
        "0"
    );
    for _ in 0..16 {
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("focus selection events should dispatch")
        {
            break;
        }
    }
    assert_eq!(
        vm.eval("focusSelectionEvents.join('|')")
            .expect("focus selection event targets should be observable"),
        "a:true:false|b:true:false|document:false:false"
    );
    vm.eval(
        r#"
focusSelectionEvents.length = 0;
document.getElementById('reset').focus();
control.focus();
control.focus();
"#,
    )
    .expect("refocusing should retain selection");
    for _ in 0..16 {
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("refocus tasks should run")
        {
            break;
        }
    }
    assert_eq!(
        vm.eval("focusSelectionEvents.length")
            .expect("unchanged selection should not dispatch an event"),
        "0"
    );
}

#[test]
fn element_focusability_uses_parsed_tabindex_and_native_defaults() {
    let mut vm = new_storage_test_vm("https://focusability.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/element-focusability.js"
        ))
        .expect("element focusability should be observable"),
        ""
    );
}

#[test]
fn foreign_element_focus_methods_validate_native_receivers_before_options() {
    let mut vm = new_storage_test_vm("https://foreign-focus-receivers.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/foreign-focus-receivers.js"
        ))
        .expect("foreign focus methods should validate receivers"),
        ""
    );
}

#[test]
fn tab_navigation_uses_native_defaults_and_shared_integer_parsing() {
    let mut vm = new_streamed_parser_test_vm(
        "https://focusability-navigation.test/",
        r##"<!doctype html><body>
<input id="start"><a id="nonlink"></a><a id="link" href="#" tabindex="invalid"></a>
<div tabindex="&#xA0;0"></div><div tabindex="&#xB;0"></div><div tabindex="2147483648"></div>
<details open><summary id="summary"></summary><summary></summary></details>
<div id="editing" contenteditable></div>
<svg><a id="svg-link" href="#"></a><defs><rect tabindex="0"></rect></defs><rect id="rect" tabindex="0"></rect></svg>
<math><a id="math-link" href="#"></a><mrow id="row" tabindex="0"></mrow></math>
<button id="end"></button>
</body>"##,
    );
    assert_eq!(
        vm.eval(r#"
(() => {
  const ids = ['start', 'link', 'summary', 'editing', 'svg-link', 'rect', 'math-link', 'row', 'end'];
  const failures = [];
  for (const reverse of [false, true]) {
    const order = reverse ? ids.slice().reverse() : ids;
    document.getElementById(order[0]).focus();
    for (const id of order.slice(1)) {
      __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, reverse);
      if (document.activeElement.id !== id) failures.push(`${reverse}: ${document.activeElement.id} != ${id}`);
    }
  }
  return failures.join(',');
})()
"#).expect("Tab navigation should share programmatic focus eligibility"),
        ""
    );
}

#[test]
fn post_parse_autofocus_uses_connection_order_after_reinsert() {
    let mut vm = new_storage_test_vm("https://autofocus-reinsert.test/");
    vm.eval(
        r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = '<input id="first" autofocus><input id="second" autofocus>';
  const first = document.getElementById('first');
  const second = document.getElementById('second');
  first.remove();
  body.insertBefore(first, second);
})()
"#,
    )
    .expect("autofocus candidates should be reinserted");

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, runtime_ptr| {
        assert!(crate::native_bridge::element::process_post_parse_autofocus(
            scope,
            runtime_ptr,
            unsafe { &*runtime_ptr }.document_handle(),
        ));
        Ok(())
    })
    .expect("autofocus should select a candidate");

    assert_eq!(
        vm.eval("document.activeElement.id")
            .expect("autofocus result should be observable"),
        "second"
    );
}

#[test]
fn image_map_area_focusability_supports_autofocus() {
    let mut vm = new_storage_test_vm("https://area-autofocus.test/");

    let setup = vm
        .eval(
            r##"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = `
    <area id="outside" href="#" autofocus>
    <map name="unused"><area id="unreferenced" href="#" autofocus></map>
    <map name="active">
      <area id="no-href" autofocus>
      <area id="target" href="#" autofocus>
    </map>
    <img usemap="#active">`;

  const rejected = ['outside', 'unreferenced', 'no-href'].map(id => {
    const candidate = document.getElementById(id);
    candidate.focus();
    return document.activeElement !== candidate;
  });
  return rejected.every(Boolean) && document.activeElement === body;
})()
"##,
        )
        .expect("image-map focusability fixture should initialize");
    assert_eq!(setup, "true");

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, runtime_ptr| {
        assert!(
            crate::native_bridge::element::post_parse_autofocus_is_pending(
                unsafe { &*runtime_ptr },
                unsafe { &*runtime_ptr }.document_handle()
            )
        );
        assert!(crate::native_bridge::element::process_post_parse_autofocus(
            scope,
            runtime_ptr,
            unsafe { &*runtime_ptr }.document_handle(),
        ));
        Ok(())
    })
    .expect("image-map area autofocus should run");

    assert_eq!(
        vm.eval("document.activeElement === document.getElementById('target')")
            .expect("image-map autofocus result should remain observable"),
        "true"
    );
}

#[test]
fn focus_prevent_scroll_controls_real_nested_scroll_container_reveal() {
    let mut vm = new_storage_test_vm("https://focus-prevent-scroll.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = `
    <button id="first">first</button>
    <div id="scroller" style="width:100px;height:100px;overflow:auto">
      <div style="width:500px;height:400px"></div>
      <button id="target" style="margin-left:400px">target</button>
    </div>`;
  return 'installed';
})()
"#,
    )
    .expect("focus scroll fixture should initialize");
    refresh_layout_for_test(&mut vm);

    vm.eval(
        r#"
(() => {
  const first = document.getElementById('first');
  const scroller = document.getElementById('scroller');
  const target = document.getElementById('target');

  target.focus({ preventScroll: true });
  const prevented = scroller.scrollLeft === 0 && scroller.scrollTop === 0;
  const focused = document.activeElement === target;
  first.focus();
  target.focus();
  window.beforeReveal = [prevented, focused];
})()
"#,
    )
    .expect("focus preventScroll probe should evaluate");

    publish_layout_for_test(&mut vm);
    let result = vm
        .eval("beforeReveal.concat(scroller.scrollLeft > 0, scroller.scrollTop > 0).join('|')")
        .unwrap();
    assert_eq!(result, "true|true|true|true");
}

#[test]
fn focusing_contenteditable_in_child_frame_reveals_authored_frame_position() {
    let mut vm = new_storage_test_vm("https://focus-scroll.test/");

    vm.eval(
        r#"
  const root = document.documentElement ||
    document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    iframe { position: absolute; left: 250vw; }
    .spacer { width: 100vw; height: 250vh; }
  `;
  head.appendChild(style);

  const first = document.createElement('div');
  first.contentEditable = 'true';
  const spacer = document.createElement('div');
  spacer.className = 'spacer';
  const frame = document.createElement('iframe');
  body.append(first, spacer, frame);

  const childDocument = frame.contentDocument;
  childDocument.open();
  childDocument.write('<div id="target" contenteditable="true">target</div>');
  childDocument.close();
  const target = childDocument.getElementById('target');
"#,
    )
    .expect("prepare child focus fixture");
    publish_layout_for_test(&mut vm);
    let result = vm
        .eval(
            r#"(() => {
  first.focus();
  target.focus();
  const firstX = window.scrollX;
  const firstY = window.scrollY;

  window.scroll(0, 0);
  first.focus();
  target.focus();
  return JSON.stringify({
    beyondViewport: firstX > window.innerWidth && firstY > window.innerHeight,
    repeated: firstX === window.scrollX && firstY === window.scrollY,
    parentRetargeted: document.activeElement === frame,
    childFocused: childDocument.activeElement === target
  });
})()"#,
        )
        .expect("child contenteditable focus scroll probe should evaluate");

    assert_eq!(
        result,
        r#"{"beyondViewport":true,"repeated":true,"parentRetargeted":true,"childFocused":true}"#
    );
}

#[test]
fn focusing_frame_owner_then_input_dispatches_child_window_focus_and_blur() {
    let mut vm = new_storage_test_vm("https://focus-frame-window.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const input = document.createElement('input');
  const frame = document.createElement('iframe');
  body.append(input, frame);
  const log = [];
  window.onblur = () => log.push('top-window-blur');
  window.onfocus = () => log.push('top-window-focus');
  frame.onfocus = () => log.push('frame-focus');
  frame.onblur = () => log.push('frame-blur');
  frame.contentWindow.onfocus = () => log.push('child-window-focus');
  frame.contentWindow.onblur = () => log.push('child-window-blur');
  input.onfocus = () => log.push('input-focus');

  frame.focus();
  input.focus();
  return `${log.join(',')}|${document.activeElement === input}`;
})()
"#,
        )
        .expect("frame window focus transition probe should evaluate");

    assert_eq!(
        result,
        "top-window-blur,frame-focus,child-window-focus,frame-blur,child-window-blur,input-focus,top-window-focus|true"
    );
}

#[test]
fn focus_transitions_publish_state_before_each_event_phase() {
    let mut vm = new_storage_test_vm("https://focus-state.test/");
    let result = vm.eval(r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = '<style>input { color: red } input:focus { color: blue }</style><input id=a><input id=b>';
  const a = document.getElementById('a'), b = document.getElementById('b');
  a.focus();
  const events = [];
  for (const node of [a, b]) for (const type of ['blur', 'focusout', 'focus', 'focusin']) {
    node.addEventListener(type, e => events.push([
      e.type, e.target.id, document.activeElement.id || 'body',
      e.relatedTarget.id, e.target.matches(':focus'), getComputedStyle(e.target).color
    ].join(':')));
  }
  b.focus();
  return events.join('|');
})()
"#).expect("focus event state fixture");
    assert_eq!(
        result,
        "blur:a:body:b:false:rgb(255, 0, 0)|focusout:a:body:b:false:rgb(255, 0, 0)|focus:b:b:a:true:rgb(0, 0, 255)|focusin:b:b:a:true:rgb(0, 0, 255)"
    );
}

#[test]
fn reentrant_focus_handlers_preserve_the_completed_transition() {
    // These event traces also match Chromium. In particular, a temporary focus
    // that is blurred before the handler returns does not cancel the request.
    for (source, event_type, action, expected) in [
        (
            "a",
            "blur",
            "c.focus()",
            "blur:a:body:b|focus:c:c:null|focusin:c:c:null|focusout:a:c:null;c",
        ),
        (
            "a",
            "blur",
            "b.focus()",
            "blur:a:body:b|focus:b:b:null|focusin:b:b:null|focusout:a:b:null;b",
        ),
        (
            "a",
            "blur",
            "a.focus()",
            "blur:a:body:b|focus:a:a:null|focusin:a:a:null|focusout:a:a:null;a",
        ),
        (
            "a",
            "blur",
            "c.focus(); c.blur()",
            "blur:a:body:b|focus:c:c:null|focusin:c:c:null|blur:c:body:null|focusout:c:body:null|focusout:a:body:b|focus:b:b:a|focusin:b:b:a;b",
        ),
        (
            "a",
            "blur",
            "c.focus(); a.addEventListener('focusout', () => c.blur(), {once:true})",
            "blur:a:body:b|focus:c:c:null|focusin:c:c:null|focusout:a:c:null|blur:c:body:null|focusout:c:body:null;body",
        ),
        (
            "a",
            "focusout",
            "c.focus()",
            "blur:a:body:b|focusout:a:body:b|focus:c:c:null|focusin:c:c:null;c",
        ),
        (
            "a",
            "focusout",
            "c.focus(); c.blur()",
            "blur:a:body:b|focusout:a:body:b|focus:c:c:null|focusin:c:c:null|blur:c:body:null|focusout:c:body:null|focus:b:b:a|focusin:b:b:a;b",
        ),
        (
            "b",
            "focus",
            "c.focus()",
            "blur:a:body:b|focusout:a:body:b|focus:b:b:a|blur:b:body:c|focusout:b:body:c|focus:c:c:b|focusin:c:c:b;c",
        ),
        (
            "b",
            "focus",
            "b.blur()",
            "blur:a:body:b|focusout:a:body:b|focus:b:b:a|blur:b:body:null|focusout:b:body:null;body",
        ),
        (
            "b",
            "focus",
            "b.remove()",
            "blur:a:body:b|focusout:a:body:b|focus:b:b:a|blur:b:body:null|focusout:b:body:null;body",
        ),
    ] {
        let mut vm = new_storage_test_vm("https://focus-reentry.test/");
        let script = r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = '<input id=a><input id=b><input id=c>';
  const a = document.getElementById('a'), b = document.getElementById('b'), c = document.getElementById('c');
  a.focus();
  const label = node => node === body ? 'body' : node?.id ?? 'null';
  const events = [];
  for (const node of [a, b, c]) for (const type of ['blur', 'focusout', 'focus', 'focusin'])
    node.addEventListener(type, e => events.push([e.type, e.target.id, label(document.activeElement), label(e.relatedTarget)].join(':')));
  /*SOURCE*/.addEventListener(/*TYPE*/, () => { /*ACTION*/ }, {once:true});
  b.focus();
  return events.join('|') + ';' + label(document.activeElement);
})()
"#
            .replace("/*SOURCE*/", source)
            .replace("/*TYPE*/", &serde_json::to_string(event_type).unwrap())
            .replace("/*ACTION*/", action);
        assert_eq!(
            vm.eval(&script).expect("reentrant focus fixture"),
            expected,
            "{source} {event_type}: {action}"
        );
    }
}

#[test]
fn focus_transitions_revalidate_the_target_after_blur_handlers() {
    for mutation in [
        "b.remove()",
        "b.disabled = true",
        "b.hidden = true",
        "b.inert = true",
        "document.implementation.createHTMLDocument().body.append(b)",
    ] {
        let mut vm = new_storage_test_vm("https://focus-target-revalidation.test/");
        let script = r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = '<input id=a><input id=b>';
  const a = document.getElementById('a'), b = document.getElementById('b');
  a.focus();
  let focused = 0;
  b.addEventListener('focus', () => focused++);
  b.addEventListener('focusin', () => focused++);
  a.addEventListener('blur', () => { /*MUTATION*/ });
  b.focus();
  return [document.activeElement === body, focused].join(':');
})()
"#
        .replace("/*MUTATION*/", mutation);
        assert_eq!(
            vm.eval(&script).expect("target retirement fixture"),
            "true:0",
            "{mutation}"
        );
    }
}

#[test]
fn shadow_root_active_element_tracks_focus_event_phases() {
    let mut vm = new_storage_test_vm("https://shadow-focus-phases.test/");
    let result = vm.eval(r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = '<div id=host></div>';
  const host = document.getElementById('host');
  const shadow = host.attachShadow({mode:'closed'});
  shadow.innerHTML = '<input id=a><input id=b>';
  const a = shadow.querySelector('#a'), b = shadow.querySelector('#b');
  a.focus();
  const events = [];
  for (const node of [a, b]) for (const type of ['blur', 'focusout', 'focus', 'focusin'])
    node.addEventListener(type, e => events.push([
      type, document.activeElement === host, shadow.activeElement?.id ?? 'null', host.matches(':focus-within')
    ].join(':')));
  b.focus();
  return events.join('|');
})()
"#).expect("shadow focus state fixture");
    assert_eq!(
        result,
        "blur:false:null:true|focusout:false:null:true|focus:true:b:true|focusin:true:b:true"
    );
}

#[test]
fn child_focus_transitions_preserve_the_viewport_and_scope_related_targets() {
    for (scenario, expected) in [
        (
            "same-child",
            "blur:a:one:body:body:b|focusout:a:one:body:body:b|focus:b:one:b:body:a|focusin:b:one:b:body:a",
        ),
        (
            "enter-child",
            "blur:main:body:body:body:null|focusout:main:body:body:body:null|focus:b:one:b:body:null|focusin:b:one:b:body:null",
        ),
        (
            "leave-child",
            "blur:a:one:body:body:null|focusout:a:one:body:body:null|focus:main:main:body:body:null|focusin:main:main:body:body:null",
        ),
        (
            "sibling-child",
            "blur:a:one:body:body:null|focusout:a:one:body:body:null|focus:c:two:body:c:null|focusin:c:two:body:c:null",
        ),
    ] {
        let mut vm = new_storage_test_vm("https://child-focus-phases.test/");
        let script = r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = '<input id=main><iframe id=one></iframe><iframe id=two></iframe>';
  const scenario = /*SCENARIO*/;
  const d1 = document.getElementById('one').contentDocument;
  const d2 = document.getElementById('two').contentDocument;
  d1.body.innerHTML = '<input id=a><input id=b>';
  d2.body.innerHTML = '<input id=c>';
  const a = scenario === 'enter-child' ? document.getElementById('main') : d1.getElementById('a');
  const b = scenario === 'leave-child' ? document.getElementById('main') :
    scenario === 'sibling-child' ? d2.getElementById('c') : d1.getElementById('b');
  const label = node => node?.id || node?.localName || 'null';
  a.focus();
  const events = [];
  for (const node of [a, b]) for (const type of ['blur', 'focusout', 'focus', 'focusin'])
    node.addEventListener(type, e => events.push([
      type, label(e.target), label(document.activeElement), label(d1.activeElement),
      label(d2.activeElement), label(e.relatedTarget)
    ].join(':')));
  b.focus();
  return events.join('|');
})()
"#
        .replace("/*SCENARIO*/", &serde_json::to_string(scenario).unwrap());
        assert_eq!(
            vm.eval(&script).expect("child focus transition fixture"),
            expected,
            "{scenario}"
        );
    }
}

#[test]
fn text_change_commit_can_redirect_a_pending_focus_transition() {
    for (action, expected) in [
        (
            "",
            "change:a:body:null|blur:a:body:b|focusout:a:body:b|focus:b:b:a|focusin:b:b:a;b",
        ),
        (
            "c.focus()",
            "change:a:body:null|focus:c:c:null|focusin:c:c:null|blur:a:c:b|focusout:a:c:null;c",
        ),
    ] {
        let mut vm = new_storage_test_vm("https://focus-change-commit.test/");
        let script = r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = '<input id=a><input id=b><input id=c>';
  const a = document.getElementById('a'), b = document.getElementById('b'), c = document.getElementById('c');
  a.focus();
  document.execCommand('insertText', false, 'edited');
  const label = node => node?.id || node?.localName || 'null';
  const events = [];
  for (const node of [a, b, c]) for (const type of ['change', 'blur', 'focusout', 'focus', 'focusin'])
    node.addEventListener(type, e => events.push([type, label(e.target), label(document.activeElement), label(e.relatedTarget)].join(':')));
  a.addEventListener('change', () => { /*ACTION*/ }, {once:true});
  b.focus();
  return events.join('|') + ';' + label(document.activeElement);
})()
"#.replace("/*ACTION*/", action);
        assert_eq!(
            vm.eval(&script).expect("text commit focus fixture"),
            expected,
            "{action}"
        );
    }
}

#[test]
fn child_viewport_focus_is_retired_when_its_container_is_removed() {
    let mut vm = new_storage_test_vm("https://focus-viewport-retirement.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = '<iframe id=frame></iframe>';
  const frame = document.getElementById('frame'), child = frame.contentDocument;
  child.body.innerHTML = '<input id=a>';
  const input = child.getElementById('a');
  input.focus(); input.blur();
  const viewport = document.activeElement === frame && child.activeElement === child.body;
  frame.remove();
  const removed = document.activeElement === body;
  body.append(frame);
  return [viewport, removed, document.activeElement === body].join(':');
})()
"#,
        )
        .expect("child viewport retirement fixture");
    assert_eq!(result, "true:true:true");
}
