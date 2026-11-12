use super::*;

#[test]
fn image_map_area_focusability_uses_the_first_map_in_the_same_tree() {
    let mut vm = new_parsed_test_vm(
        "https://area-map-scope.test/",
        "<!doctype html><body></body>",
    );
    let result = vm
        .eval(
            r##"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  document.body.innerHTML = `<img usemap="#shared">
    <map name="shared"><area id="first" href="#"></map>
    <map name="shared"><area id="second" href="#"></map>`;
  const first = document.getElementById("first");
  const second = document.getElementById("second");
  first.focus();
  assert(document.activeElement === first, "first matching map is focusable");
  second.focus();
  assert(document.activeElement === first, "duplicate map is not associated");
  first.parentNode.remove();
  second.focus();
  assert(document.activeElement === second, "association follows tree mutations");

  const host = document.body.appendChild(document.createElement("div"));
  const root = host.attachShadow({mode: "open"});
  root.innerHTML = `<map name="shared"><area id="shadow" href="#"></map>`;
  const shadow = root.getElementById("shadow");
  shadow.focus();
  assert(document.activeElement === second, "document image cannot reference shadow map");
  const image = document.createElement("img");
  image.useMap = "#shared";
  root.appendChild(image);
  shadow.focus();
  assert(root.activeElement === shadow, "shadow image references its own map");

  const frame = document.body.appendChild(document.createElement("iframe"));
  const child = frame.contentDocument;
  child.body.innerHTML = `<map name="shared"><area id="child" href="#"></map>`;
  const area = child.getElementById("child");
  area.focus();
  assert(child.activeElement !== area, "parent image cannot reference child map");
  const childImage = child.createElement("img");
  childImage.useMap = "#shared";
  child.body.appendChild(childImage);
  area.focus();
  assert(child.activeElement === area, "child image references its own map");
  return "ok";
})()
"##,
        )
        .expect("image-map focus should use native tree-scoped associations");
    assert_eq!(result, "ok");
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
            crate::native_bridge::element::post_parse_autofocus_is_pending(unsafe {
                &*runtime_ptr
            })
        );
        assert!(crate::native_bridge::element::process_post_parse_autofocus(
            scope,
            runtime_ptr
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
            runtime_ptr
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
