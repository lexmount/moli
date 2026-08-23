use super::*;

#[test]
fn child_window_exposes_core_ecmascript_global_functions() {
    let mut vm = new_storage_test_vm("https://child-window-global-functions.test/");

    let first_result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  if (!document.head) {
    document.documentElement.insertBefore(document.createElement('head'), document.body);
  }
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__childCoreFunctionWindow = frame.contentWindow;
  return typeof globalThis.__childCoreFunctionWindow.Function;
})()
"#,
        )
        .expect("child window Function request should evaluate");
    assert_eq!(first_result, "function");

    let result = vm
        .eval(
            r#"
(() => {
  const win = globalThis.__childCoreFunctionWindow;
  return [
    typeof isNaN,
    typeof win.isNaN,
    win.isNaN(NaN),
    typeof win.Function,
    typeof new win.Function('return 1'),
    new win.Function('return 1')(),
    win.Function === Function,
    typeof win.Error,
    new win.Error('child').message,
    win.Error === Error,
    typeof win.isFinite,
    win.isFinite(1),
    typeof win.parseInt,
    win.parseInt('10', 10),
    typeof win.decodeURIComponent,
    win.decodeURIComponent('%E6%B5%8B%E8%AF%95')
  ].join('|');
})()
"#,
        )
        .expect("child window global aliases should be readable");

    assert_eq!(
        result,
        "function|function|true|function|function|1|false|function|child|false|function|true|function|10|function|测试"
    );
}
#[test]
fn child_window_exposes_browser_surface_aliases() {
    let mut vm = new_storage_test_vm("https://child-window-browser-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const win = frame.contentWindow;
  const xhrOpenBefore = win.XMLHttpRequest.prototype.open;
  win.XMLHttpRequest.prototype.__lmChildProbe = 'patched';
  return [
    typeof win.XMLHttpRequest,
    typeof win.XMLHttpRequest.prototype,
    typeof xhrOpenBefore,
    win.XMLHttpRequest.prototype.__lmChildProbe,
    typeof win.fetch,
    typeof win.Headers,
    typeof win.Headers.prototype,
    typeof win.Request,
    typeof win.Response,
    typeof win.FormData,
    typeof win.WebSocket,
    typeof win.CloseEvent,
    typeof win.ReadableStream,
    typeof win.TextEncoderStream,
    typeof win.TextDecoderStream,
    typeof win.MutationObserver,
    typeof win.navigator,
    typeof win.performance,
    typeof win.localStorage,
    typeof win.indexedDB,
    typeof win.DOMParser,
    typeof win.URLSearchParams,
    typeof win.DOMException,
    win.DOMException.name,
    typeof win.NavigationHistoryEntry,
    typeof win.NavigationActivation,
    win.XMLHttpRequest === XMLHttpRequest,
    win.Headers === Headers,
    win.CloseEvent === CloseEvent,
    win.MutationObserver === MutationObserver,
    win.DOMException === DOMException,
    win.NavigationHistoryEntry === NavigationHistoryEntry,
    win.NavigationActivation === NavigationActivation
  ].join('|');
})()
"#,
        )
        .expect("child window browser aliases should be readable");

    assert_eq!(
        result,
        "function|object|function|patched|function|function|object|function|function|function|function|function|function|function|function|function|object|object|object|object|function|function|function|DOMException|function|function|false|false|false|false|false|false|false"
    );
}
#[test]
fn current_script_is_null_while_script_remains_in_shadow_tree() {
    let mut vm = new_storage_test_vm("https://shadow-current-script.test/");
    vm.exec(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  window.__shadowCurrentScriptProbe = {};
  const host = document.createElement('div');
  host.id = 'shadow-host';
  const root = host.attachShadow({ mode: 'open' });
  const script = document.createElement('script');
  script.id = 'shadow-script';
  window.__shadowCurrentScriptProbe.script = script;
  root.appendChild(script);
  document.body.appendChild(host);
})()
"#,
        None,
    )
    .expect("shadow script setup should run");

    vm.exec(
        r#"
(() => {
  const outer = document.createElement('script');
  outer.id = 'outer-script';
  document.body.appendChild(outer);
})()
"#,
        None,
    )
    .expect("outer script placeholder should be inserted");
    let outer = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("outer-script")
        .expect("outer script handle");
    vm.document_runtime
        .set_current_script_context(CurrentScriptContextSpec {
            handle: Some(outer),
            parser_write_insertion_point_active: false,
            parser_bridge: None,
        });
    vm.exec(
        r#"
(() => {
  const host = document.createElement('div');
  const root = host.attachShadow({ mode: 'open' });
  const script = document.createElement('script');
  script.textContent = `
    window.__inlineShadowCurrentScript =
      document.currentScript === null ? 'null' : document.currentScript.id;
  `;
  root.appendChild(script);
  document.body.appendChild(host);
  window.__outerCurrentScriptAfterShadowAppend = document.currentScript && document.currentScript.id;
})()
"#,
        None,
    )
    .expect("inline shadow script should execute");
    vm.document_runtime.clear_current_script_handle();
    assert_eq!(
        vm.eval(
            "[window.__inlineShadowCurrentScript, window.__outerCurrentScriptAfterShadowAppend].join('|')"
        )
        .expect("inline shadow currentScript result should be readable"),
        "null|outer-script"
    );

    let host = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("shadow-host")
        .expect("shadow host handle");
    let shadow_root = vm
        .document_runtime
        .dom_host()
        .shadow_root_handle(host)
        .expect("shadow root handle");
    let script = vm
        .document_runtime
        .dom_host()
        .node(shadow_root)
        .and_then(Node::first_child)
        .expect("shadow script handle");
    vm.document_runtime
        .set_current_script_context(CurrentScriptContextSpec {
            handle: Some(script),
            parser_write_insertion_point_active: false,
            parser_bridge: None,
        });
    assert_eq!(
        vm.eval("String(document.currentScript)")
            .expect("currentScript should be readable"),
        "null"
    );
    vm.document_runtime.clear_current_script_handle();

    vm.exec("window.__shadowCurrentScriptProbe.script.remove()", None)
        .expect("shadow script should be removable");
    vm.document_runtime
        .set_current_script_context(CurrentScriptContextSpec {
            handle: Some(script),
            parser_write_insertion_point_active: false,
            parser_bridge: None,
        });
    assert_eq!(
        vm.eval("document.currentScript && document.currentScript.id")
            .expect("removed currentScript should be visible"),
        "shadow-script"
    );
    vm.document_runtime.clear_current_script_handle();
}
#[test]
fn child_window_same_realm_alias_boundary_is_explicit() {
    let mut vm = new_storage_test_vm("https://child-window-same-realm-boundary.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const win = frame.contentWindow;
  const doc = frame.contentDocument;
  const child = doc.createElement('div');
  doc.body.appendChild(child);
  const owns = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const frameSlot = '__moliWindowFrameElement';
  const frameSlotNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name === frameSlot)
    .join(',');
  const frameSlotNamesBefore = frameSlotNames(win);
  const deleteDefaultView = delete doc.defaultView;
  const deleteParentWindow = delete doc.parentWindow;
  win.HTMLDivElement.prototype.__lmSameRealmProbe = 'shared';
  Object.defineProperty(win, frameSlot, {
    value: { spoof: true },
    configurable: true,
    writable: true
  });
  const frameSlotNamesAfterSpoof = frameSlotNames(win);
  const frameSlotPublicSpoof = win[frameSlot].spoof === true;
  const deleteFrameSlotPublicSpoof = delete win[frameSlot];
  const output = [
    win.self === win,
    win.window === win,
    win.globalThis === win,
    win.frames === win,
    win.parent === globalThis,
    win.top === globalThis,
    win.frameElement === frame,
    frameSlotNamesBefore,
    frameSlotNamesAfterSpoof,
    frameSlotPublicSpoof,
    deleteFrameSlotPublicSpoof,
    frameSlotNames(win),
    win.frameElement === frame,
    !owns(doc, 'defaultView'),
    !owns(doc, 'parentWindow'),
    deleteDefaultView,
    deleteParentWindow,
    doc.defaultView === win,
    typeof doc.parentWindow === 'undefined',
    win.Node === Node,
    win.Element === Element,
    win.HTMLElement !== HTMLElement,
    win.HTMLDivElement !== HTMLDivElement,
    win.Event === Event,
    win.DOMParser === DOMParser,
    Object.getPrototypeOf(child) === win.HTMLDivElement.prototype,
    Object.getPrototypeOf(child) === HTMLDivElement.prototype,
    Object.getPrototypeOf(child).constructor && Object.getPrototypeOf(child).constructor.name,
    child instanceof win.HTMLDivElement,
    child instanceof HTMLDivElement,
    win.HTMLDivElement.prototype.__lmSameRealmProbe,
    HTMLDivElement.prototype.__lmSameRealmProbe === undefined
  ].join('|');
  delete win.HTMLDivElement.prototype.__lmSameRealmProbe;
  return output;
})()
"#,
        )
        .expect("child window same-realm alias boundary should be explicit");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true||__moliWindowFrameElement|true|true||true|true|true|true|true|true|true|false|false|true|true|false|false|true|false|HTMLDivElement|true|false|shared|true"
    );
}

#[test]
fn window_global_accessors_use_the_borrowed_window_receiver() {
    let mut vm = new_storage_test_vm("https://window-accessor-receiver.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const child = frame.contentWindow;
  const parentDescriptor =
    Object.getOwnPropertyDescriptor(window, "frameElement");
  const childDescriptor =
    Object.getOwnPropertyDescriptor(child, "frameElement");
  let incompatibleReceiver;
  try {
    parentDescriptor.get.call({});
    incompatibleReceiver = "no-throw";
  } catch (error) {
    incompatibleReceiver = error && error.name;
  }
  const unexpectedPrototypeMembers =
    Object.getOwnPropertyNames(Window.prototype)
      .filter(name => !["TEMPORARY", "PERSISTENT", "constructor"].includes(name));
  return [
    Object.hasOwn(window, "frameElement"),
    Object.hasOwn(child, "frameElement"),
    Object.hasOwn(Window.prototype, "frameElement"),
    unexpectedPrototypeMembers.join(","),
    parentDescriptor.get.name,
    parentDescriptor.get.length,
    parentDescriptor.get.call(window) === null,
    parentDescriptor.get.call(child) === frame,
    childDescriptor.get.call(window) === null,
    childDescriptor.get.call(child) === frame,
    incompatibleReceiver
  ].join("|");
})()
"#,
        )
        .expect("Window accessor receiver probe should evaluate");

    assert_eq!(
        result,
        "true|true|false||get frameElement|0|true|true|true|true|TypeError"
    );
}

#[test]
fn child_document_body_with_scope_exposes_insert_before() {
    let mut vm = new_storage_test_vm("https://child-window-with-body.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<body><span id="first"></span></body>';
  (document.body || document.documentElement || document).appendChild(frame);
  const doc = frame.contentDocument;
  let scriptTagName = 'missing';
  let withResult = 'ok';
  try {
    with (doc) with (body) with (insertBefore(createElement('script'), firstChild)) {
      scriptTagName = tagName;
    }
  } catch (error) {
    withResult = error && error.message;
  }
  return [
    typeof doc.body.insertBefore,
    doc.body.firstChild && doc.body.firstChild.tagName,
    scriptTagName,
    withResult
  ].join('|');
})()
"#,
        )
        .expect("child document body should expose insertBefore under with");

    assert_eq!(result, "function|SCRIPT|SCRIPT|ok");
}
#[test]
fn child_content_document_body_inner_html_setter_updates_query_surface() {
    let mut vm = new_storage_test_vm("https://child-window-inner-html-setter.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const doc = frame.contentWindow.document;
  doc.body.innerHTML = '<div id="player-shadow-host"><span class="leaf">leaf</span></div>';
  const host = doc.querySelector('#player-shadow-host');
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<button id="shadow-button">shadow</button>';
  const button = shadow.querySelector('#shadow-button');
  return [
    frame.contentDocument === doc,
    host && host.id,
    host && host.querySelector('.leaf').textContent,
    shadow.childNodes.length,
    button && button.textContent,
    doc.body.innerHTML
  ].join('|');
})()
"#,
        )
        .expect("child content document innerHTML setter should update query surface");

    assert_eq!(
        result,
        "true|player-shadow-host|leaf|1|shadow|<div id=\"player-shadow-host\"><span class=\"leaf\">leaf</span></div>"
    );
}
#[test]
fn detached_document_shadow_root_inner_html_setter_updates_query_surface() {
    let mut vm = new_storage_test_vm("https://detached-shadow-inner-html-setter.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument('');
  const host = doc.createElement('div');
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<button id="shadow-button">shadow</button>';
  const button = shadow.querySelector('#shadow-button');
  return [
    shadow.childNodes.length,
    button && button.textContent
  ].join('|');
})()
"#,
        )
        .expect("detached document shadow root innerHTML setter should update query surface");

    assert_eq!(result, "1|shadow");
}
#[test]
fn child_document_content_type_uses_resource_mime() {
    let mut vm = new_storage_test_vm("https://child-content-type.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parsed = new DOMParser().parseFromString(
    '<iframe id="html" src="data:text/html;charset=utf-8,<p>ok</p>"></iframe>' +
    '<iframe id="image" src="data:image/png;base64,AA=="></iframe>',
    'text/html'
  );
  const htmlFrame = parsed.getElementById('html');
  const imageFrame = parsed.getElementById('image');

  return [
    htmlFrame.contentDocument && htmlFrame.contentDocument.contentType,
    imageFrame.contentDocument && imageFrame.contentDocument.contentType
  ].join('|');
})()
"#,
        )
        .expect("child document contentType should evaluate");

    assert_eq!(result, "text/html|image/png");
}
#[test]
fn node_move_before_parent_node_surface_and_validation() {
    let mut vm = new_storage_test_vm("https://node-move-before.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.createElement('div');
  const first = document.createElement('span');
  const second = document.createElement('em');
  parent.append(first, second);
  const liveReturn = parent.moveBefore(second, first);

  const detachedDocument = document.implementation.createHTMLDocument('title');
  const detachedFirst = detachedDocument.createElement('span');
  const detachedSecond = detachedDocument.createElement('em');
  detachedDocument.body.append(detachedFirst, detachedSecond);
  const detachedReturn = detachedDocument.body.moveBefore(detachedSecond, detachedFirst);

  let missingArg = '';
  try {
    parent.moveBefore(first);
  } catch (error) {
    missingArg = error.name;
  }

  let disconnectedRoots = '';
  try {
    document.createElement('div').moveBefore(document.createElement('p'), null);
  } catch (error) {
    disconnectedRoots = error.name + ':' + error.code;
  }

  return [
    liveReturn === undefined,
    parent.firstChild === second,
    detachedReturn === undefined,
    detachedDocument.body.firstChild === detachedSecond,
    'moveBefore' in document.createTextNode('text'),
    'moveBefore' in new Comment('comment'),
    missingArg,
    disconnectedRoots,
    parent.moveBefore.length,
    detachedDocument.body.moveBefore.length
  ].join('|');
})()
"#,
        )
        .expect("Node.moveBefore behavior should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|false|false|TypeError|HierarchyRequestError:3|2|2"
    );
}
#[test]
fn node_move_before_preserves_nonce_content_attribute() {
    let mut vm = new_storage_test_vm("https://node-move-before-nonce.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.createElement('section');
  const target = document.createElement('div');
  const body = document.body || document.documentElement || document;
  body.appendChild(parent);
  body.appendChild(target);
  target.setAttribute('nonce', 'abc123');
  const beforeMove = target.getAttribute('nonce');
  parent.moveBefore(target, null);
  const afterMove = target.getAttribute('nonce');
  parent.insertBefore(target, null);
  const afterInsert = target.getAttribute('nonce');

  const script = document.createElement('script');
  script.setAttribute('nonce', 'script-nonce');
  body.appendChild(script);
  return [beforeMove, afterMove, afterInsert, script.getAttribute('nonce'), script.nonce].join('|');
})()
"#,
        )
        .expect("moveBefore should preserve nonce content attributes");

    assert_eq!(result, "abc123|abc123|||script-nonce");
}
#[test]
fn node_move_before_updates_name_and_window_maps() {
    let mut vm = new_storage_test_vm("https://node-move-before-name-map.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const body = document.body;
  const container = document.createElement('div');
  const windowContainer = document.createElement('div');
  body.appendChild(container);
  body.appendChild(windowContainer);

  const target = document.createElement('img');
  target.name = 'target';
  body.appendChild(target);
  const shadowParent = document.createElement('div');
  container.attachShadow({ mode: 'open' }).appendChild(shadowParent);
  const names = document.getElementsByName('target');
  const beforeName = `${names.length}:${names[0] === target}`;
  shadowParent.moveBefore(target, null);
  const afterShadowMove = names.length;

  const target2 = document.createElement('img');
  target2.name = 'target2';
  body.appendChild(target2);
  const windowBefore = window.target2 === target2;
  const windowShadowParent = document.createElement('div');
  windowContainer.attachShadow({ mode: 'open' }).appendChild(windowShadowParent);
  windowShadowParent.moveBefore(target2, null);
  const windowAfter = window.target2 === undefined;

  const a = document.createElement('img');
  const b = document.createElement('img');
  a.name = 'duplicate';
  b.name = 'duplicate';
  body.appendChild(a);
  body.appendChild(b);
  const duplicates = document.getElementsByName('duplicate');
  const orderBefore = duplicates[0] === a;
  body.moveBefore(b, a);
  const orderAfter = duplicates.length + ':' + (duplicates[0] === b);
  return [beforeName, afterShadowMove, windowBefore, windowAfter, orderBefore, orderAfter].join('|');
})()
"#,
        )
        .expect("moveBefore should update name and window maps");

    assert_eq!(result, "1:true|0|true|true|true|2:true");
}
#[test]
fn node_move_before_preserves_option_selectedness() {
    let mut vm = new_storage_test_vm("https://node-move-before-option-selectedness.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  document.body.innerHTML = `
    <select>
      <option value="A">A</option>
      <optgroup label="Optgroup"><option value="B">B</option></optgroup>
      <option value="C">C</option>
    </select>`;
  const select = document.querySelector('select');
  const optionA = document.querySelector('option[value=A]');
  const optionB = document.querySelector('option[value=B]');
  const optionC = document.querySelector('option[value=C]');
  const state = () => [optionA.selected, optionB.selected, optionC.selected].join(',');
  const before = state();
  document.body.moveBefore(optionA, null);
  const afterAOut = state();
  document.body.moveBefore(optionB, null);
  const afterBOut = state();
  select.moveBefore(optionA, optionC);
  const afterAIn = state();
  return [before, afterAOut, afterBOut, afterAIn].join('|');
})()
"#,
        )
        .expect("moveBefore should preserve option selectedness");

    assert_eq!(
        result,
        "true,false,false|true,true,false|true,true,true|true,true,false"
    );
}
#[test]
fn node_move_before_dispatches_custom_element_move_reactions() {
    let mut vm = new_storage_test_vm("https://node-move-before-custom-elements.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const section = document.createElement('section');
  document.body.appendChild(section);
  let index = 0;
  const uniqueName = () => `ce-move-${++index}`;

  const firstReactions = [];
  const firstName = uniqueName();
  customElements.define(firstName, class extends HTMLElement {
    connectedCallback() { firstReactions.push(`connected:${this.isConnected}`); }
    disconnectedCallback() { firstReactions.push(`disconnected:${this.isConnected}`); }
  });
  const first = document.createElement(firstName);
  document.body.appendChild(first);
  firstReactions.length = 0;
  section.moveBefore(first, null);

  const moveReactions = [];
  const moveName = uniqueName();
  customElements.define(moveName, class extends HTMLElement {
    connectedMoveCallback() { moveReactions.push('move'); }
    connectedCallback() { moveReactions.push('connected'); }
    disconnectedCallback() { moveReactions.push('disconnected'); }
  });
  const moved = document.createElement(moveName);
  document.body.appendChild(moved);
  moveReactions.length = 0;
  section.moveBefore(moved, null);

  const nestedQueueReactions = [];
  const nestedQueueName = uniqueName();
  customElements.define(nestedQueueName, class extends HTMLElement {
    static observedAttributes = ['data-moved'];
    connectedMoveCallback() {
      nestedQueueReactions.push('move:start');
      this.setAttribute('data-moved', 'yes');
      nestedQueueReactions.push('move:end');
    }
    attributeChangedCallback() {
      nestedQueueReactions.push(`attr:${this.getAttribute('data-moved')}`);
    }
  });
  const nestedQueued = document.createElement(nestedQueueName);
  document.body.appendChild(nestedQueued);
  nestedQueueReactions.length = 0;
  section.moveBefore(nestedQueued, null);

  const nestedReactions = [];
  const outerName = uniqueName();
  customElements.define(outerName, class extends HTMLElement {
    connectedCallback() { nestedReactions.push('outer connected'); }
    disconnectedCallback() { nestedReactions.push('outer disconnected'); }
  });
  const innerName = uniqueName();
  customElements.define(innerName, class extends HTMLElement {
    connectedCallback() { nestedReactions.push('inner connected'); }
    disconnectedCallback() { nestedReactions.push('inner disconnected'); }
  });
  const outer = document.createElement(outerName);
  const inner = document.createElement(innerName);
  outer.appendChild(inner);
  document.body.appendChild(outer);
  nestedReactions.length = 0;
  section.moveBefore(outer, null);

  return [
    firstReactions.join(','),
    moveReactions.join(','),
    nestedQueueReactions.join(','),
    nestedReactions.join(',')
  ].join('|');
})()
"#,
        )
        .expect("moveBefore should dispatch custom element move reactions");

    assert_eq!(
        result,
        "disconnected:true,connected:true|move|move:start,attr:yes,move:end|outer disconnected,outer connected,inner disconnected,inner connected"
    );
}
#[test]
fn node_move_before_updates_shadow_flat_tree_geometry() {
    let mut vm = new_storage_test_vm("https://node-move-before-shadow-geometry.test/");
    vm.force_fresh_layout_reads_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.head) {
    document.documentElement.appendChild(document.createElement('head'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }

  const host = document.createElement('div');
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<div><div style="display: none;"><slot></slot></div></div>';
  const span = document.createElement('span');
  span.textContent = 'Text in light DOM';
  document.body.append(host, span);
  const beforeSlotWidth = span.getBoundingClientRect().width;
  host.moveBefore(span, null);
  const afterSlotWidth = span.getBoundingClientRect().width;

  const style = document.createElement('style');
  style.textContent = `
    section { position: relative; }
    #item { position: absolute; inset: 0; }
    #section1 { width: 200px; height: 200px; }
    #section2 { width: 300px; height: 300px; }
  `;
  document.head.appendChild(style);
  const section1 = document.createElement('section');
  section1.id = 'section1';
  const section2 = document.createElement('section');
  section2.id = 'section2';
  const item = document.createElement('div');
  item.id = 'item';
  section1.appendChild(item);
  document.body.append(section1, section2);
  const itemShadow = item.attachShadow({ mode: 'open' });
  itemShadow.innerHTML = `
    <style>div { position: absolute; inset: 0; }</style>
    <div></div>
  `;
  section2.moveBefore(item, null);
  const innerWidth = itemShadow.querySelector('div').getBoundingClientRect().width;
  return [
    beforeSlotWidth > 0,
    afterSlotWidth,
    section2.getBoundingClientRect().width,
    item.getBoundingClientRect().width,
    innerWidth
  ].join('|');
})()
"#,
        )
        .expect("moveBefore shadow flat-tree geometry should evaluate");

    assert_eq!(result, "true|0|300|300|300");
}
#[test]
fn node_move_before_preserves_modal_dialog_state() {
    let mut vm = new_storage_test_vm("https://node-move-before-modal-dialog.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const oldParent = document.createElement('section');
  const newParent = document.createElement('section');
  const modal = document.createElement('dialog');
  oldParent.appendChild(modal);
  document.body.append(oldParent, newParent);
  modal.showModal();
  const beforeMove = modal.matches(':modal');
  newParent.moveBefore(modal, null);
  const afterMove = modal.matches(':modal');
  modal.close();
  const afterClose = modal.matches(':modal');

  const nonModal = document.createElement('dialog');
  document.body.appendChild(nonModal);
  nonModal.show();
  const showIsModal = nonModal.matches(':modal');
  nonModal.open = true;
  const openSetterIsModal = nonModal.matches(':modal');

  return [
    beforeMove,
    afterMove,
    afterClose,
    showIsModal,
    openSetterIsModal
  ].join('|');
})()
"#,
        )
        .expect("moveBefore should preserve modal dialog state");

    assert_eq!(result, "true|true|false|false|false");
}
#[test]
fn node_move_before_preserves_focus_without_synchronous_focusout() {
    let mut vm = new_storage_test_vm("https://node-move-before-focus.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const oldParent = document.createElement('section');
  const newParent = document.createElement('section');
  const button = document.createElement('button');
  button.textContent = 'Button';
  oldParent.appendChild(button);
  document.body.append(oldParent, newParent);
  const seen = [];
  document.addEventListener('focusin', event => seen.push(`${event.target.localName}.in`));
  document.addEventListener('focusout', event => seen.push(`${event.target.localName}.out`));
  button.focus();
  newParent.moveBefore(button, null);
  const movePreserved = [
    seen.join(','),
    document.activeElement === button,
    button.matches(':focus'),
    newParent.matches(':focus-within')
  ].join('/');
  oldParent.append(button);
  const appendBlurred = document.activeElement === document.body;
  button.focus();
  return [
    movePreserved,
    appendBlurred,
    seen.join(',')
  ].join('|');
})()
"#,
        )
        .expect("moveBefore should preserve focus without synchronous focusout");

    assert_eq!(
        result,
        "button.in/true/true/true|true|button.in,button.out,button.in"
    );
}
#[test]
fn html_canvas_webgl_context_is_cached_and_kind_locked() {
    let mut vm = new_storage_test_vm("https://webgl-canvas-kind-lock.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  const first = canvas.getContext('webgl');
  const second = canvas.getContext('webgl');
  const twoD = canvas.getContext('2d');
  return [
    first instanceof WebGLRenderingContext,
    first === second,
    twoD === null,
    first && first.constructor && first.constructor.name,
    Object.prototype.hasOwnProperty.call(first, 'getContextAttributes'),
    typeof WebGLRenderingContext.prototype.getContextAttributes,
    typeof WebGLRenderingContext.prototype.isContextLost,
  ].join('|');
})()
"#,
        )
        .expect("html canvas webgl surface should be readable");

    assert_eq!(
        result,
        "true|true|true|WebGLRenderingContext|false|function|function"
    );
}
#[test]
fn webgl_context_attributes_have_expected_shape() {
    let mut vm = new_storage_test_vm("https://webgl-context-attributes-shape.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const gl = document.createElement('canvas').getContext('webgl');
  const attrs = gl.getContextAttributes();
  const keys = Object.keys(attrs).sort().join(',');
  const clone = gl.getContextAttributes();
  return [
    keys,
    attrs.alpha,
    attrs.antialias,
    attrs.depth,
    attrs.desynchronized,
    attrs.failIfMajorPerformanceCaveat,
    attrs.premultipliedAlpha,
    attrs.preserveDrawingBuffer,
    attrs.stencil,
    attrs.xrCompatible,
    attrs.powerPreference,
    attrs === clone,
  ].join('|');
})()
"#,
        )
        .expect("webgl context attributes should expose a stable shape");

    assert_eq!(
        result,
        "alpha,antialias,depth,desynchronized,failIfMajorPerformanceCaveat,powerPreference,premultipliedAlpha,preserveDrawingBuffer,stencil,xrCompatible|true|true|true|false|false|true|false|false|false|default|false"
    );
}
#[test]
fn webgl_supported_extensions_have_expected_shape() {
    let mut vm = new_storage_test_vm("https://webgl-supported-extensions-shape.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const gl = document.createElement('canvas').getContext('webgl');
  const extensions = gl.getSupportedExtensions();
  const clone = gl.getSupportedExtensions();
  return [
    Array.isArray(extensions),
    extensions.join(','),
    extensions === clone
  ].join('|');
})()
"#,
        )
        .expect("webgl supported extensions should expose a stable array");

    assert_eq!(
        result,
        "true|ANGLE_instanced_arrays,EXT_blend_minmax,EXT_clip_control,EXT_color_buffer_half_float,EXT_depth_clamp,EXT_disjoint_timer_query,EXT_float_blend,EXT_frag_depth,EXT_polygon_offset_clamp,EXT_shader_texture_lod,EXT_texture_compression_bptc,EXT_texture_compression_rgtc,EXT_texture_filter_anisotropic,EXT_texture_mirror_clamp_to_edge,EXT_sRGB,KHR_parallel_shader_compile,OES_element_index_uint,OES_fbo_render_mipmap,OES_standard_derivatives,OES_texture_float,OES_texture_float_linear,OES_texture_half_float,OES_texture_half_float_linear,OES_vertex_array_object,WEBGL_blend_func_extended,WEBGL_color_buffer_float,WEBGL_compressed_texture_astc,WEBGL_compressed_texture_etc,WEBGL_compressed_texture_etc1,WEBGL_compressed_texture_pvrtc,WEBGL_compressed_texture_s3tc,WEBGL_compressed_texture_s3tc_srgb,WEBGL_debug_renderer_info,WEBGL_debug_shaders,WEBGL_depth_texture,WEBGL_draw_buffers,WEBGL_lose_context,WEBGL_multi_draw,WEBGL_polygon_mode|false"
    );
}
#[test]
fn webgl_extension_and_parameter_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://webgl-method-webidl-boundaries.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === null ? "null" : String(value);
    } catch (error) {
      return error && error.name;
    }
  };

  const gl = document.createElement('canvas').getContext('webgl');
  const debugInfo = gl.getExtension({
    toString() { return 'WEBGL_debug_renderer_info'; }
  });
  const descriptor = (object, name) => {
    const desc = Object.getOwnPropertyDescriptor(object, name);
    return `${desc && desc.value}:${desc && desc.enumerable}`;
  };

  return JSON.stringify({
    extensionObject: debugInfo instanceof WEBGL_debug_renderer_info,
    vendorConstantOwn: descriptor(debugInfo, 'UNMASKED_VENDOR_WEBGL'),
    rendererConstantOwn: descriptor(debugInfo, 'UNMASKED_RENDERER_WEBGL'),
    extensionMissing: probe(() => gl.getExtension()),
    extensionSymbol: probe(() => gl.getExtension(Symbol('extension'))),
    extensionNull: probe(() => gl.getExtension(null)),
    extensionThrow: probe(() => gl.getExtension({ toString() { throw new RangeError('extension'); } })),
    parameterObject: probe(() => gl.getParameter({ valueOf() { return debugInfo.UNMASKED_VENDOR_WEBGL; } })),
    parameterMissing: probe(() => gl.getParameter()),
    parameterSymbol: probe(() => gl.getParameter(Symbol('pname'))),
    parameterNegative: probe(() => gl.getParameter(-1)),
    parameterWrap: probe(() => gl.getParameter(4294967296)),
    parameterFraction: probe(() => gl.getParameter(37445.9))
  });
})()
"#,
        )
        .expect("WebGL method WebIDL boundaries should evaluate");

    assert_eq!(
        result,
        r#"{"extensionObject":true,"vendorConstantOwn":"37445:false","rendererConstantOwn":"37446:false","extensionMissing":"TypeError","extensionSymbol":"TypeError","extensionNull":"null","extensionThrow":"RangeError","parameterObject":"","parameterMissing":"TypeError","parameterSymbol":"TypeError","parameterNegative":"null","parameterWrap":"null","parameterFraction":""}"#
    );
}
#[test]
fn offscreen_canvas_webgl_surface_matches_html_canvas_surface() {
    let mut vm = new_storage_test_vm("https://offscreen-webgl-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = new OffscreenCanvas(32, 18);
  const gl = canvas.getContext('webgl');
  return [
    gl instanceof WebGLRenderingContext,
    typeof gl.getContextAttributes,
    typeof gl.isContextLost,
    gl.getContextAttributes().powerPreference,
    gl.isContextLost(),
  ].join('|');
})()
"#,
        )
        .expect("offscreen canvas webgl surface should be readable");

    assert_eq!(result, "true|function|function|default|false");
}
#[test]
fn offset_parent_and_offsets_cross_shadow_scopes_like_chromium() {
    let mut vm = new_parsed_test_vm(
        "https://shadow-offset-parent.test/",
        r#"<html><body><div id="container" style="position: relative"></div></body></html>"#,
    );
    vm.force_fresh_layout_reads_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const container = document.getElementById('container');

  const internalHost = document.createElement('div');
  container.appendChild(internalHost);
  const internalShadow = internalHost.attachShadow({ mode: 'open' });
  internalShadow.innerHTML =
    '<div id="relativeParent" style="position: relative; padding-left: 100px; padding-top: 70px;">' +
      '<div id="target"></div>' +
    '</div>';
  const relativeParent = internalShadow.getElementById('relativeParent');
  const internalTarget = internalShadow.getElementById('target');
  const internalResult = [
    internalTarget.offsetParent === relativeParent,
    internalTarget.offsetLeft,
    internalTarget.offsetTop
  ];

  container.innerHTML = '';
  const slottedHost = document.createElement('div');
  slottedHost.innerHTML = '<div id="target"></div>';
  container.appendChild(slottedHost);
  const slottedShadow = slottedHost.attachShadow({ mode: 'open' });
  slottedShadow.innerHTML =
    '<div style="position: relative; padding-left: 85px; padding-top: 45px;">' +
      '<slot></slot>' +
    '</div>';
  const slottedTarget = slottedHost.querySelector('#target');
  const slottedResult = [
    slottedTarget.offsetParent === container,
    slottedTarget.offsetLeft,
    slottedTarget.offsetTop
  ];

  container.innerHTML = '';
  const fixedHost = document.createElement('div');
  fixedHost.innerHTML = '<div id="target"></div>';
  container.appendChild(fixedHost);
  const fixedShadow = fixedHost.attachShadow({ mode: 'open' });
  fixedShadow.innerHTML =
    '<div id="fixed" style="position: fixed; padding-left: 85px; padding-top: 45px;">' +
      '<slot></slot>' +
    '</div>';
  const fixedTarget = fixedHost.querySelector('#target');
  const fixed = fixedShadow.getElementById('fixed');
  const fixedResult = [
    fixedTarget.offsetParent === null,
    fixedTarget.offsetLeft,
    85 + fixed.offsetLeft,
    fixedTarget.offsetTop,
    45 + fixed.offsetTop
  ];

  container.innerHTML = '';
  const transformedFixedHost = document.createElement('div');
  transformedFixedHost.innerHTML = '<div id="target" style="position: fixed;"></div>';
  container.appendChild(transformedFixedHost);
  const transformedFixedShadow = transformedFixedHost.attachShadow({ mode: 'open' });
  transformedFixedShadow.innerHTML =
    '<div style="transform: translate(10px, 10px);" id="wrapper">' +
      '<div style="position: fixed; padding-left: 85px; padding-top: 45px;">' +
        '<slot></slot>' +
      '</div>' +
    '</div>';
  const transformedFixedTarget = transformedFixedHost.querySelector('#target');
  const transformedFixedResult = [
    transformedFixedTarget.offsetParent === container,
    transformedFixedTarget.offsetLeft,
    transformedFixedTarget.offsetTop
  ];

  container.innerHTML = '';
  const previousBlock = document.createElement('div');
  previousBlock.style.height = '12px';
  const nestedHost = document.createElement('div');
  nestedHost.innerHTML = '<div id="target" style="border: solid 1px blue;">hi</div>';
  container.append(previousBlock, nestedHost);
  const nestedShadow = nestedHost.attachShadow({ mode: 'open' });
  nestedShadow.innerHTML =
    '<section style="position: relative; margin-left: 20px; margin-top: 100px;">' +
      '<div style="position: absolute; top: 10px; left: 10px;"><slot></slot></div>' +
    '</section>';
  const nestedTarget = nestedHost.querySelector('#target');
  const nestedResult = [
    nestedTarget.offsetParent === container,
    nestedTarget.offsetLeft,
    nestedTarget.offsetTop
  ];

  container.innerHTML = '';
  const unassignedHost = document.createElement('section');
  unassignedHost.innerHTML = '<div id="target"></div>';
  container.appendChild(unassignedHost);
  unassignedHost.attachShadow({ mode: 'open' }).innerHTML =
    '<section style="position: absolute; top: 50px; left: 50px;">content</section>';
  const unassignedTarget = unassignedHost.querySelector('#target');
  const unassignedResult = [
    unassignedTarget.offsetParent === null,
    unassignedTarget.offsetLeft,
    unassignedTarget.offsetTop
  ];

  return [
    ...internalResult,
    ...slottedResult,
    ...fixedResult,
    ...transformedFixedResult,
    ...nestedResult,
    ...unassignedResult
  ].join('|');
})()
"#,
        )
        .expect("shadow-scoped offset geometry should evaluate");

    assert_eq!(
        result,
        "true|100|70|true|85|45|true|93|93|53|53|true|85|45|true|30|122|true|0|0"
    );
}

#[test]
fn shadow_host_unassigned_children_are_empty_and_slotted_text_has_layout() {
    let mut vm = new_storage_test_vm("https://shadow-unassigned-geometry.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }

  const previous = document.createElement('div');
  const host = document.createElement('div');
  const hidden = document.createElement('span');
  hidden.textContent = 'hidden';
  host.appendChild(hidden);
  document.body.append(previous, host);
  host.attachShadow({mode: 'open'}).innerHTML = '<span>shadow content</span>';

  const slottedHost = document.createElement('div');
  const slotted = document.createElement('span');
  slotted.textContent = 'slotted';
  slottedHost.appendChild(slotted);
  document.body.appendChild(slottedHost);
  slottedHost.attachShadow({mode: 'open'}).innerHTML = '<slot></slot>';
  const hiddenRect = hidden.getBoundingClientRect();
  const slottedRect = slotted.getBoundingClientRect();

  return JSON.stringify({
    hiddenOffsetTop: hidden.offsetTop,
    hiddenWidth: hiddenRect.width,
    hiddenHeight: hiddenRect.height,
    slottedWidth: slottedRect.width,
    slottedHeight: slottedRect.height
  });
})()
"#,
        )
        .expect("shadow unassigned light geometry should evaluate");

    let result: serde_json::Value =
        serde_json::from_str(&result).expect("shadow geometry should be JSON");
    assert_eq!(result["hiddenOffsetTop"], 0);
    assert_eq!(result["hiddenWidth"], 0.0);
    assert_eq!(result["hiddenHeight"], 0.0);
    assert!(
        result["slottedWidth"]
            .as_f64()
            .is_some_and(|width| width > 0.0)
    );
    assert!(
        result["slottedHeight"]
            .as_f64()
            .is_some_and(|height| height > 0.0)
    );
}

#[test]
fn shadow_flat_tree_offsets_match_chromium_layout_order() {
    let mut vm = new_storage_test_vm("https://shadow-offset-wpt.test/");

    let result = vm
        .eval(
            r#"
if (!document.documentElement) {
  document.appendChild(document.createElement('html'));
}
if (!document.body) {
  document.documentElement.appendChild(document.createElement('body'));
}
const body = document.body;
body.setHTMLUnsafe(`
<style>
.box {
  width: 10px;
  height: 10px;
}
</style>

<div>
  <template shadowrootmode=open>
    <style>
    .box {
      width: 10px;
      height: 10px;
    }
    </style>
    <div class=box></div>
    <div style="position: relative">
      <div class=box></div>
      <slot></slot>
    </div>
  </template>
  <div class=box></div>
  <div id=target1 style="position: absolute" class=box></div>
</div>

<span>
  <template shadowrootmode=open>
    <style>
    .box {
      width: 10px;
      height: 10px;
    }
    </style>
    <span class=box></span>
    <span style="position: relative">
      <span class=box></span>
      <slot></slot>
    </span>
  </template>
  <span class=box></span>
  <span id=target2 style="position: absolute" class=box></span>
</span>

<div>
  <template shadowrootmode=open>
    <style>
    .box {
      width: 10px;
      height: 10px;
    }
    </style>
    <div class=box></div>
    <div style="position: relative">
      <div class=box></div>
      <div>
        <template shadowrootmode=open>
          <style>
          .box {
            width: 10px;
            height: 10px;
          }
          </style>
          <div class=box></div>
          <div style="position: relative">
            <div class=box></div>
            <slot></slot>
          </div>
        </template>
        <slot></slot>
      </div>
    </div>
  </template>
  <div class=box></div>
  <div id=target3 style="position: absolute" class=box></div>
</div>
`);
[
  target1.offsetTop,
  target2.offsetLeft,
  target3.offsetTop
].join('|')
"#,
        )
        .expect("headless mock shadow boundary offsets should evaluate");

    assert_eq!(result, "38|8|88");
}

#[test]
fn geometry_getters_reuse_latest_layout_across_nodes_and_mutation() {
    let mut vm = new_storage_test_vm("https://oneshot-layout-demand.test/");
    let passes_before = vm
        ._context_host
        .borrow()
        .layout_pass_observability_for_test()
        .1;
    let cache_before = vm.layout_snapshot_cache_observability_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const targets = [];
  for (let index = 0; index < 4; index++) {
    const target = document.createElement('div');
    target.style.height = '10px';
    document.body.appendChild(target);
    targets.push(target);
  }

  const initialLast = targets[3].offsetTop;
  const first = targets.map(target => target.offsetTop);
  const second = targets.map(target => target.offsetTop);

  const inserted = document.createElement('div');
  inserted.style.height = '5px';
  document.body.insertBefore(inserted, targets[0]);
  const afterInsertion = targets[3].offsetTop;
  return [
    initialLast,
    first[0],
    first[3],
    second[0],
    second[3],
    afterInsertion,
    inserted.offsetTop
  ].join('|');
})()
"#,
        )
        .expect("latest layout snapshot reads should evaluate");

    assert_eq!(result, "38|8|38|8|38|38|0");
    let passes = vm
        ._context_host
        .borrow()
        .layout_pass_observability_for_test()
        .1
        .saturating_sub(passes_before);
    assert_eq!(passes, 1, "only the cold getter may build layout");
    let cache_after = vm.layout_snapshot_cache_observability_for_test();
    assert_eq!(cache_after.0, cache_before.0 + 10);
    assert_eq!(cache_after.1, cache_before.1 + 1);
    assert_eq!(cache_after.2, cache_before.2 + 1);
    assert!(cache_after.3.is_some());
}

#[test]
fn switching_to_mock_geometry_drops_the_latest_real_layout_snapshot() {
    let mut vm = new_storage_test_vm("https://layout-policy-cache.test/");
    vm.eval(
        "document.appendChild(document.createElement('html')).appendChild(document.createElement('body')); 'installed'",
    )
    .expect("the geometry fixture should initialize");
    vm.eval("document.body.offsetTop")
        .expect("the real geometry read should populate the latest snapshot");
    assert!(
        vm.layout_snapshot_cache_observability_for_test()
            .3
            .is_some()
    );

    vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
    assert!(
        vm.layout_snapshot_cache_observability_for_test()
            .3
            .is_none(),
        "Mock policy must not retain a real frozen layout tree"
    );
    let passes_before_mock = vm.layout_pass_observability_for_test().1;
    vm.eval("document.body.offsetTop")
        .expect("mock geometry should remain available");
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        passes_before_mock
    );
    assert!(
        vm.layout_snapshot_cache_observability_for_test()
            .3
            .is_none()
    );
}

#[test]
fn block_flow_geometry_includes_body_margin_for_following_nodes() {
    let mut vm = new_storage_test_vm("https://mock-flow-height-hint.test/");

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const spacer = document.createElement('div');
  spacer.style.height = '3000px';
  const target = document.createElement('div');
  target.id = 'target';
  spacer.id = 'spacer';
  document.body.append(spacer, target);
  return 'installed';
})()
"#,
    )
    .expect("block-flow geometry fixture should initialize");
    refresh_layout_for_test(&mut vm);
    let initial = vm
        .eval("document.getElementById('target').offsetTop")
        .expect("initial block-flow geometry should evaluate");
    vm.eval("document.getElementById('spacer').style.height = '48px'; 'mutated'")
        .expect("block-flow height mutation should evaluate");
    refresh_layout_for_test(&mut vm);
    let mutated = vm
        .eval("document.getElementById('target').offsetTop")
        .expect("refreshed block-flow geometry should evaluate");

    assert_eq!(format!("{initial}|{mutated}"), "3008|56");
}

#[test]
fn body_default_geometry_tracks_each_css_margin_axis() {
    let mut vm = new_parsed_test_vm(
        "https://body-axis-margin-geometry.test/",
        r#"<html><head></head><body></body></html>"#,
    );

    vm.eval(
        r#"
(() => {
  const style = document.createElement('style');
  document.head.appendChild(style);
  const target = document.createElement('div');
  target.id = 'target';
  target.style.cssText = 'width: 10px; height: 10px;';
  document.body.appendChild(target);
  return 'installed';
})()
"#,
    )
    .expect("body margin geometry fixture should initialize");

    let mut rects = Vec::new();
    for css in [
        "",
        "body { margin-left: 0; }",
        "body { margin-top: 0; }",
        "body { margin: 0; }",
    ] {
        vm.eval(&format!(
            "document.querySelector('style').textContent = {}; 'styled'",
            serde_json::to_string(css).expect("CSS fixture should serialize")
        ))
        .expect("body margin mutation should evaluate");
        refresh_layout_for_test(&mut vm);
        rects.push(
            vm.eval(
                "(() => { const rect = document.getElementById('target').getBoundingClientRect(); return `${rect.x},${rect.y}`; })()",
            )
            .expect("refreshed body margin geometry should evaluate"),
        );
    }

    assert_eq!(rects.join("|"), "8,8|0,8|8,0|0,0");
}
#[test]
fn child_content_document_created_elements_expose_geometry_methods() {
    let mut vm = new_storage_test_vm("https://child-created-geometry.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const doc = frame.contentDocument;
  doc.head.appendChild(doc.createElement('style')).textContent = 'body { margin-left: 0; }';
  const item = doc.createElement('div');
  item.id = 'item';
  doc.body.append(item);
  const rect = item.getBoundingClientRect();
  return [
    typeof item.getBoundingClientRect,
    typeof item.getClientRects,
    rect.x,
    rect.width > 0
  ].join('|');
})()
"#,
        )
        .expect("child document created geometry should evaluate");

    assert_eq!(result, "function|function|0|true");
}
#[test]
fn geometry_rect_objects_expose_domrect_to_json() {
    let mut vm = new_parsed_test_vm(
        "https://geometry-domrect-shape.test/",
        "<!doctype html><body><div id='node'>text</div></body>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const node = document.getElementById('node');
  const rect = node.getBoundingClientRect();
  const rects = node.getClientRects();
  const range = document.createRange();
  range.selectNodeContents(node);
  const rangeRect = range.getBoundingClientRect();
  const rangeRects = range.getClientRects();
  const observer = new ResizeObserver(() => {});
  observer.observe(node);
  const entries = observer.takeRecords();
  const contentRect = entries[0].contentRect;
  const json = rect.toJSON();
  return [
    rect instanceof DOMRect,
    rect instanceof DOMRectReadOnly,
    Object.keys(rect).join(','),
    typeof rect.toJSON,
    Object.keys(json).join(','),
    [
      'x', 'y', 'width', 'height', 'top', 'right', 'bottom', 'left'
    ].every(name => json[name] === rect[name]),
    Array.isArray(rects),
    rects.length,
    rects[0] instanceof DOMRect,
    typeof rects[0].toJSON,
    rangeRect instanceof DOMRect,
    typeof rangeRect.toJSON,
    rangeRects[0] instanceof DOMRect,
    contentRect instanceof DOMRect,
    typeof contentRect.toJSON,
    Object.hasOwn(DOMRectReadOnly.prototype, 'toJSON'),
    Object.hasOwn(DOMRect.prototype, 'toJSON')
  ].join('|');
})()
"#,
        )
        .expect("geometry rect objects should expose DOMRect-compatible shapes");

    assert_eq!(
        result,
        "true|true||function|x,y,width,height,top,right,bottom,left|true|true|1|true|function|true|function|true|true|function|true|false"
    );
}

#[test]
fn geometry_domrect_private_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://geometry-domrect-private-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const rect = new DOMRect(10, 20, 30, 40);
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith('__moliDomRect'))
    .sort()
    .join(',');

  const initialOwnSlots = internalNames(rect);
  DOMRect.prototype.__moliDomRectX = -100;
  DOMRect.prototype.__moliDomRectY = -100;
  DOMRect.prototype.__moliDomRectBrand = true;
  DOMRectReadOnly.prototype.__moliDomRectWidth = -100;
  DOMRectReadOnly.prototype.__moliDomRectHeight = -100;
  DOMRectReadOnly.prototype.__moliDomRectBrand = true;
  rect.__moliDomRectX = -200;
  rect.__moliDomRectY = -200;
  rect.__moliDomRectWidth = -200;
  rect.__moliDomRectHeight = -200;
  rect.__moliDomRectBrand = true;

  const beforeSet = [
    rect.x, rect.y, rect.width, rect.height,
    rect.top, rect.right, rect.bottom, rect.left
  ].join(',');

  rect.x = 15;
  rect.width = -5;
  const afterSet = [
    rect.x, rect.y, rect.width, rect.height,
    rect.top, rect.right, rect.bottom, rect.left
  ].join(',');

  const readonlyX = Object.getOwnPropertyDescriptor(DOMRectReadOnly.prototype, 'x').get;
  const readonlyTop = Object.getOwnPropertyDescriptor(DOMRectReadOnly.prototype, 'top').get;
  const writableX = Object.getOwnPropertyDescriptor(DOMRect.prototype, 'x');
  const fake = Object.assign(Object.create(DOMRect.prototype), {
    __moliDomRectX: 1,
    __moliDomRectY: 2,
    __moliDomRectWidth: 3,
    __moliDomRectHeight: 4,
    __moliDomRectBrand: true
  });
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return error.constructor.name;
    }
  };
  const fakeResults = [
    probe(() => readonlyX.call(fake)),
    probe(() => readonlyTop.call(fake)),
    probe(() => writableX.get.call(fake)),
    probe(() => writableX.set.call(fake, 123)),
    probe(() => DOMRectReadOnly.prototype.toJSON.call(fake))
  ].join(',');
  function accessorSummary(prototype, name) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor.get,
      descriptor.get ? descriptor.get.name : "",
      descriptor.get ? descriptor.get.length : -1,
      typeof descriptor.set,
      descriptor.set ? descriptor.set.name : "",
      descriptor.set ? descriptor.set.length : -1,
      descriptor.enumerable,
      descriptor.configurable
    ].join(':');
  }
  function methodSummary(prototype, name) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor.value,
      descriptor.value ? descriptor.value.name : "",
      descriptor.value ? descriptor.value.length : -1,
      descriptor.enumerable,
      descriptor.writable,
      descriptor.configurable
    ].join(':');
  }
  const descriptors = [
    accessorSummary(DOMRectReadOnly.prototype, 'x'),
    accessorSummary(DOMRectReadOnly.prototype, 'top'),
    accessorSummary(DOMRect.prototype, 'x'),
    methodSummary(DOMRectReadOnly.prototype, 'toJSON'),
    Object.hasOwn(DOMRect.prototype, 'toJSON')
  ].join(';');

  return [
    initialOwnSlots,
    beforeSet,
    afterSet,
    JSON.stringify(rect.toJSON()),
    fakeResults,
    internalNames(fake),
    descriptors
  ].join('|');
})()
"#,
        )
        .expect("DOMRect backing slots should resist reflection and spoofing");

    assert_eq!(
        result,
        "|10,20,30,40,20,40,60,10|15,20,-5,40,20,15,60,10|{\"x\":15,\"y\":20,\"width\":-5,\"height\":40,\"top\":20,\"right\":15,\"bottom\":60,\"left\":10}|TypeError,TypeError,TypeError,TypeError,TypeError|__moliDomRectBrand,__moliDomRectHeight,__moliDomRectWidth,__moliDomRectX,__moliDomRectY|x:function:get x:0:undefined::-1:true:true;top:function:get top:0:undefined::-1:true:true;x:function:get x:0:function:set x:1:true:true;toJSON:function:toJSON:0:true:true:true;false"
    );
}

#[test]
fn fragment_anchor_scroll_ignores_shadow_tree_targets() {
    let mut vm = new_storage_test_vm("https://fragment-shadow-scroll.test/");
    vm.force_fresh_layout_reads_for_test();

    let result = vm
        .eval(
            r##"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const container = document.createElement('div');
  document.body.appendChild(container);
  function tall(id) {
    return '<div style="height: 2000px"><a href="#' + id + '">Go</a></div>';
  }

  container.innerHTML = tall('doc-target') + '<div id="doc-target">target</div>';
  container.querySelector('a').click();
  const documentTargetScrolls = window.pageYOffset > 0;

  window.scrollTo(0, 0);
  container.innerHTML = tall('shadow-target') + '<div id="host"></div>';
  container.querySelector('#host').attachShadow({ mode: 'open' }).innerHTML =
    '<div id="shadow-target">shadow</div>';
  container.querySelector('a').click();
  const shadowTargetIgnored = window.pageYOffset === 0;

  window.scrollTo(0, 0);
  container.innerHTML =
    tall('duplicate-target') +
    '<div id="host"></div>' +
    tall('duplicate-target') +
    '<a name="duplicate-target">document target</a>';
  const host = container.querySelector('#host');
  host.attachShadow({ mode: 'open' }).innerHTML =
    '<div id="duplicate-target">shadow</div>';
  container.querySelector('a').click();
  const duplicateUsesDocumentTree = window.pageYOffset > host.offsetTop;

  return [documentTargetScrolls, shadowTargetIgnored, duplicateUsesDocumentTree].join('|');
})()
"##,
        )
        .expect("fragment anchor scroll should evaluate");

    assert_eq!(result, "true|true|true");
}
