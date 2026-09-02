use super::*;

#[test]
fn child_content_document_body_set_html_unsafe_preserves_declarative_shadow_roots() {
    let mut vm = new_storage_test_vm("https://child-window-set-html-unsafe.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const doc = frame.contentDocument;
  doc.body.setHTMLUnsafe(
    "<div id='host'>" +
      "<template shadowrootmode='open' " +
        "shadowrootadoptedstylesheets='https://child-window-set-html-unsafe.test/styles.css'>" +
        "<span id='target'>child</span>" +
      "</template>" +
    "</div>"
  );
  const host = doc.getElementById('host');
  const root = host && host.shadowRoot;
  const sheet = root && root.adoptedStyleSheets[0];
  return [
    typeof doc.body.setHTMLUnsafe,
    host && host.id,
    root && root.querySelector('#target').textContent,
    root && root.adoptedStyleSheets.length,
    sheet && sheet.cssRules.length
  ].join('|');
})()
"#,
        )
        .expect("child content document setHTMLUnsafe should preserve declarative shadow roots");

    assert_eq!(result, "function|host|child|1|0");
}
#[test]
fn detached_iframe_shadow_events_retarget_related_target_for_outer_listener() {
    let mut vm = new_storage_test_vm("https://detached-shadow-event-retargeting.test/");

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
  document.body.appendChild(frame);
  const doc = frame.contentWindow.document;
  doc.body.innerHTML = '<div id="host"></div><div id="outside"></div>';
  const host = doc.querySelector('#host');
  const outside = doc.querySelector('#outside');
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<div id="inside"></div>';
  const inside = shadow.querySelector('#inside');

  const seen = [];
  const label = node => node === doc.defaultView ? 'Window' : (node.id || node.nodeName);
  doc.body.addEventListener('mouseover', event => {
    seen.push([
      label(event.target),
      event.relatedTarget && label(event.relatedTarget),
      event.composedPath().map(label).join(',')
    ].join(':'));
  });

  const event = document.createEvent('MouseEvents');
  event.initMouseEvent('mouseover', true, false, window,
    0, 10, 10, 10, 10, false, false, false, false, 0, inside);
  outside.dispatchEvent(event);

  return seen.join('|');
})()
"#,
        )
        .expect("detached iframe shadow event relatedTarget should retarget");

    assert_eq!(result, "outside:host:outside,BODY,HTML,#document,Window");
}
#[test]
fn detached_iframe_shadow_events_cross_host_for_composed_dispatch() {
    let mut vm = new_storage_test_vm("https://detached-shadow-composed-events.test/");

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
  document.body.appendChild(frame);
  const doc = frame.contentWindow.document;
  doc.body.innerHTML = '<div id="host"></div>';
  const host = doc.querySelector('#host');
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<button id="button"></button>';
  const button = shadow.querySelector('#button');

  const seen = [];
  const label = node => {
    if (node === button) return 'button';
    if (node === shadow) return 'shadow';
    if (node === host) return 'host';
    return node.id || node.nodeName;
  };
  for (const target of [button, shadow, host, doc.body]) {
    target.addEventListener('x-detached', event => {
      seen.push(`${label(target)}:${label(event.target)}:${event.composedPath().map(label).join(',')}`);
    });
  }
  button.dispatchEvent(new Event('x-detached', { bubbles: true, composed: true }));

  return seen.join('|');
})()
"#,
        )
        .expect("detached iframe shadow composed events should cross host");

    assert_eq!(
        result,
        "button:button:button,shadow,host,BODY,HTML,#document,|shadow:button:button,shadow,host,BODY,HTML,#document,|host:host:button,shadow,host,BODY,HTML,#document,|BODY:host:button,shadow,host,BODY,HTML,#document,"
    );
}
#[test]
fn detached_iframe_open_shadow_host_receives_related_target_events() {
    let mut vm = new_storage_test_vm("https://detached-shadow-related-host.test/");

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
  document.body.appendChild(frame);
  const doc = frame.contentWindow.document;
  doc.body.innerHTML = '<div id="host"></div>';
  const host = doc.querySelector('#host');
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<button id="one"></button><button id="two"></button>';
  const one = shadow.querySelector('#one');
  const two = shadow.querySelector('#two');

  const seen = [];
  const label = node => {
    if (node === one) return 'one';
    if (node === two) return 'two';
    if (node === shadow) return 'shadow';
    if (node === host) return 'host';
    return node && (node.id || node.nodeName);
  };
  for (const target of [one, shadow, host, doc.body]) {
    target.addEventListener('x-related', event => {
      seen.push(`${label(target)}:${label(event.target)}:${label(event.relatedTarget)}:${event.composedPath().map(label).join(',')}`);
    });
  }

  const event = new Event('x-related', {
    bubbles: true,
    composed: true
  });
  Object.defineProperty(event, 'relatedTarget', {
    value: two,
    writable: true,
    configurable: true
  });
  one.dispatchEvent(event);

  return seen.join('|');
})()
"#,
        )
        .expect("detached open shadow host should observe relatedTarget events");

    assert_eq!(result, "one:one:two:one,shadow|shadow:one:two:one,shadow");
}
#[test]
fn detached_iframe_shadow_focus_events_retarget_and_trim_paths() {
    let mut vm = new_storage_test_vm("https://detached-shadow-focus-events.test/");

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
  document.body.appendChild(frame);
  const win = frame.contentWindow;
  const doc = win.document;
  doc.body.innerHTML = '<input id="outside"><div id="host"></div>';
  const outside = doc.querySelector('#outside');
  const host = doc.querySelector('#host');
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.id = 'shadow';
  shadow.innerHTML = '<input id="input1"><input id="input2">';
  const input1 = shadow.querySelector('#input1');
  const input2 = shadow.querySelector('#input2');

  const label = node => {
    if (node === shadow) return 'shadow';
    if (node === win) return 'window';
    return node && (node.id || node.nodeName);
  };
  const seen = [];
  input1.addEventListener('focusin', event => {
    seen.push(`input1-in:${event.composed}:${event.composedPath().map(label).join(',')}`);
  });
  input1.addEventListener('focusout', event => {
    seen.push(`input1-out:${event.composed}:${label(event.relatedTarget)}`);
  });
  input2.addEventListener('focusin', event => {
    seen.push(`input2-in:${event.composedPath().map(label).join(',')}`);
  });
  input1.addEventListener('blur', event => {
    seen.push(`input1-blur:${label(event.target)}:${label(event.relatedTarget)}`);
  });

  input1.focus();
  input2.focus();
  outside.focus();

  return seen.join('|');
})()
"#,
        )
        .expect("detached iframe shadow focus events should retarget and trim paths");

    assert_eq!(
        result,
        "input1-in:true:input1,shadow,host,BODY,HTML,#document,window|input1-blur:input1:input2|input1-out:true:input2|input2-in:input2,shadow"
    );
}
#[test]
fn detached_iframe_shadow_focus_blur_uses_canonical_query_wrappers() {
    let mut vm = new_storage_test_vm("https://detached-shadow-focus-blur-wpt.test/");

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
  document.body.appendChild(frame);
  const doc = frame.contentWindow.document;
  doc.body.innerHTML = '<div id="player"><input type="checkbox" id="outside-control"><div id="player-shadow-host"></div></div>';
  const playerShadowRoot = doc.querySelector('#player-shadow-host').attachShadow({ mode: 'open' });
  playerShadowRoot.innerHTML = '<div id="controls"><div tabindex="0" class="volume-slider" id="volume-slider"></div></div>';

  const first = playerShadowRoot.querySelector('.volume-slider');
  first.focus();

  const seen = [];
  const second = playerShadowRoot.querySelector('.volume-slider');
  second.addEventListener('blur', event => {
    seen.push([
      first === second,
      event.target && event.target.getAttribute('id'),
      event.relatedTarget && event.relatedTarget.getAttribute('id')
    ].join(':'));
  });

  doc.querySelector('#outside-control').focus();
  return seen.join('|') || `none:${first === second}:${doc.activeElement && doc.activeElement.id}`;
})()
"#,
        )
        .expect("detached iframe shadow blur should use canonical query wrappers");

    assert_eq!(result, "true:volume-slider:outside-control");
}
#[test]
fn node_move_before_dispatches_shadow_slotchange_events() {
    let mut vm = new_storage_test_vm("https://node-move-before-slotchange.test/");

    let setup = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const host = document.createElement('div');
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const slot = document.createElement('slot');
  slot.name = 'content';
  shadow.appendChild(slot);
  const slottable = document.createElement('p');
  slottable.slot = 'content';
  document.body.appendChild(slottable);
  const fallback = document.createElement('span');
  shadow.appendChild(fallback);
  globalThis.__moveBeforeSlot = slot;
  globalThis.__moveBeforeSlottable = slottable;
  globalThis.__moveBeforeFallback = fallback;
  globalThis.__moveBeforeShadow = shadow;
  globalThis.__moveBeforeSlotchangeCount = 0;
  slot.addEventListener('slotchange', () => globalThis.__moveBeforeSlotchangeCount++);
  host.moveBefore(slottable, null);
  return [
    globalThis.__moveBeforeSlotchangeCount,
    slot.assignedNodes().includes(slottable)
  ].join(':');
})()
"#,
        )
        .expect("moveBefore slotchange setup should evaluate");
    let after_move_in = vm
        .eval(
            "`${globalThis.__moveBeforeSlotchangeCount}:${globalThis.__moveBeforeSlot.assignedNodes().includes(globalThis.__moveBeforeSlottable)}`",
        )
        .expect("moveBefore slottable-in slotchange should run");
    let after_move_out = vm
        .eval(
            r#"
(() => {
  document.body.moveBefore(globalThis.__moveBeforeSlottable, null);
  return `${globalThis.__moveBeforeSlotchangeCount}:${globalThis.__moveBeforeSlot.assignedNodes().length}`;
})()
"#,
        )
        .expect("moveBefore slottable-out should evaluate");
    let after_move_out_microtask = vm
        .eval(
            "`${globalThis.__moveBeforeSlotchangeCount}:${globalThis.__moveBeforeSlot.assignedNodes().length}`",
        )
        .expect("moveBefore slottable-out slotchange should run");
    let after_fallback_in = vm
        .eval(
            r#"
(() => {
  globalThis.__moveBeforeSlot.moveBefore(globalThis.__moveBeforeFallback, null);
  return globalThis.__moveBeforeSlotchangeCount;
})()
"#,
        )
        .expect("moveBefore fallback-in should evaluate");
    let after_fallback_in_microtask = vm
        .eval("globalThis.__moveBeforeSlotchangeCount")
        .expect("moveBefore fallback-in slotchange should run");
    let after_fallback_out = vm
        .eval(
            r#"
(() => {
  globalThis.__moveBeforeShadow.moveBefore(globalThis.__moveBeforeFallback, null);
  return globalThis.__moveBeforeSlotchangeCount;
})()
"#,
        )
        .expect("moveBefore fallback-out should evaluate");
    let after_fallback_out_microtask = vm
        .eval("globalThis.__moveBeforeSlotchangeCount")
        .expect("moveBefore fallback-out slotchange should run");

    assert_eq!(setup, "0:true");
    assert_eq!(after_move_in, "1:true");
    assert_eq!(after_move_out, "1:0");
    assert_eq!(after_move_out_microtask, "2:0");
    assert_eq!(after_fallback_in, "2");
    assert_eq!(after_fallback_in_microtask, "3");
    assert_eq!(after_fallback_out, "3");
    assert_eq!(after_fallback_out_microtask, "4");
}
#[test]
fn detached_shadow_root_selectors_do_not_cross_nested_shadow_boundaries() {
    let mut vm = new_storage_test_vm("https://detached-shadow-nested-boundaries.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument('Test');
  function setupBlock(prefix, root) {
    const div1 = doc.createElement('div');
    div1.id = `${prefix}_id1`;
    div1.className = 'cls';
    const p1 = doc.createElement('p');
    p1.className = 'cls';
    p1.setAttribute('test', 'A_04_01_09');
    const div2 = doc.createElement('div');
    div2.id = `${prefix}_id2`;
    div2.className = 'cls';
    div2.setAttribute('test', 'A_04_01_09');
    root.appendChild(div1);
    div1.appendChild(p1);
    p1.appendChild(div2);
    return { div1, p1, div2 };
  }

  const d = setupBlock('d', doc.body);
  const s1 = d.p1.attachShadow({ mode: 'open' });
  const s1Nodes = setupBlock('s1', s1);
  const s2 = s1Nodes.p1.attachShadow({ mode: 'open' });
  const s2Nodes = setupBlock('s2', s2);

  const ids = list => Array.from(list).map(node => node.id || node.localName).join(',');
  return JSON.stringify({
    documentS1Hidden: doc.querySelector('#s1_id1') === null,
    documentS2Hidden: doc.querySelector('#s2_id1') === null,
    s1SeesOwnDivs: ids(s1.querySelectorAll('div')),
    s2SeesOwnDivs: ids(s2.querySelectorAll('div')),
    s1S2Hidden: s1.querySelector('#s2_id1') === null,
    s2S1Hidden: s2.querySelector('#s1_id1') === null,
    s1All: ids(s1.querySelectorAll('*')),
    s1Attr: ids(s1.querySelectorAll('[test=A_04_01_09]')),
    s1FirstChild: ids(s1.querySelectorAll('div:first-child')),
    s1LastChild: ids(s1.querySelectorAll('div:last-child')),
    s1OnlyChild: ids(s1.querySelectorAll('p:only-child')),
    s1Empty: ids(s1.querySelectorAll('div:empty')),
    s1Descendant: ids(s1.querySelectorAll('p div')),
    s1Child: ids(s1.querySelectorAll('p > div')),
    s2Own: s2.querySelector('#s2_id1') === s2Nodes.div1
  });
})()
"#,
        )
        .expect("detached nested ShadowRoot selectors should evaluate");

    assert_eq!(
        result,
        r#"{"documentS1Hidden":true,"documentS2Hidden":true,"s1SeesOwnDivs":"s1_id1,s1_id2","s2SeesOwnDivs":"s2_id1,s2_id2","s1S2Hidden":true,"s2S1Hidden":true,"s1All":"s1_id1,p,s1_id2","s1Attr":"p,s1_id2","s1FirstChild":"s1_id1,s1_id2","s1LastChild":"s1_id1,s1_id2","s1OnlyChild":"p","s1Empty":"s1_id2","s1Descendant":"s1_id2","s1Child":"s1_id2","s2Own":true}"#
    );
}
#[test]
fn detached_selectors_handle_target_inside_not_pseudo_class() {
    let mut vm = new_storage_test_vm("https://detached-selector-target.test/#target");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(`
    <html><body>
      <section id="target" class="hit"></section>
      <section id="other" class="hit"></section>
    </body></html>
  `, 'text/html');
  const target = doc.getElementById('target');
  const other = doc.getElementById('other');
  return [
    target.matches(':target'),
    target.matches(':not(:target)'),
    other.matches(':not(:target)'),
    target.matches('#target:target.hit'),
    doc.body.querySelector('section:not(:target)') === other
  ].join('|');
})()
"#,
        )
        .expect("detached target selector fallback should evaluate nested pseudo-classes");

    assert_eq!(result, "true|false|true|true|true");
}
#[test]
fn dom_parser_document_stylesheets_match_chromium_owner_activation() {
    let mut vm = new_storage_test_vm("https://dom-parser-stylesheets-rel.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(`
    <html><head>
      <link id="plain">
      <link id="preload" rel="preload">
      <link id="sheet" rel="stylesheet">
      <link id="untitled-alternate" rel="alternate stylesheet">
      <link id="alternate" rel="alternate stylesheet" title="alternate">
      <style id="inline"></style>
    </head><body></body></html>
  `, 'text/html');
  const inline = doc.getElementById('inline');
  const inlineSheet = inline.sheet;
  inlineSheet.insertRule('.detached { color: red; }', 0);
  return [
    Array.from(doc.styleSheets).map(sheet => sheet.ownerNode.id).join(','),
    doc.getElementById('sheet').sheet === null,
    doc.getElementById('alternate').sheet === null,
    inlineSheet instanceof CSSStyleSheet,
    inlineSheet.ownerNode === inline,
    inline.sheet === inlineSheet,
    inlineSheet.cssRules.length,
  ].join('|');
})()
"#,
        )
        .expect("DOMParser document stylesheets should use detached owner activation");

    assert_eq!(result, "inline|true|true|true|true|true|1");
}

#[test]
fn created_html_document_activates_inserted_inline_stylesheet() {
    let mut vm = new_storage_test_vm("https://created-document-stylesheet.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument('detached');
  const style = doc.createElement('style');
  style.textContent = '.created { color: red; }';
  doc.head.appendChild(style);
  const listedBeforeDirectGetter = Array.from(doc.styleSheets)
    .map(candidate => candidate.ownerNode === style)
    .join(',');
  const sheet = style.sheet;
  sheet.insertRule('.inserted { color: blue; }', 1);
  return [
    style.isConnected,
    sheet instanceof CSSStyleSheet,
    sheet.ownerNode === style,
    style.sheet === sheet,
    listedBeforeDirectGetter,
    sheet.cssRules.length,
  ].join('|');
})()
"#,
        )
        .expect("createHTMLDocument should activate an inserted inline stylesheet");

    assert_eq!(result, "true|true|true|true|true|2");
}

#[test]
fn html_slot_element_assigned_node_methods_surface_current_assignments() {
    let mut vm = new_storage_test_vm("https://html-slot-assignment-surface.test/");

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
  const host = document.createElement('div');
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<slot name="named"></slot>';

  const slot = shadow.querySelector('slot');
  const assigned = document.createElement('span');
  assigned.id = 'assigned';
  assigned.slot = 'named';
  host.appendChild(assigned);

  return [
    typeof slot.assignedNodes,
    typeof slot.assignedElements,
    slot.assignedNodes === HTMLSlotElement.prototype.assignedNodes,
    slot.assignedElements === HTMLSlotElement.prototype.assignedElements,
    slot.assign === HTMLSlotElement.prototype.assign,
    Object.prototype.hasOwnProperty.call(slot, 'assignedNodes'),
    Object.prototype.hasOwnProperty.call(slot, 'assignedElements'),
    Object.prototype.hasOwnProperty.call(slot, 'assign'),
    Object.getOwnPropertyDescriptor(HTMLSlotElement.prototype, 'assignedNodes').enumerable,
    Object.getOwnPropertyDescriptor(HTMLSlotElement.prototype, 'assignedElements').enumerable,
    Object.getOwnPropertyDescriptor(HTMLSlotElement.prototype, 'assign').enumerable,
    HTMLSlotElement.prototype.assignedNodes.length,
    HTMLSlotElement.prototype.assignedElements.length,
    HTMLSlotElement.prototype.assign.length,
    HTMLSlotElement.prototype.assignedNodes.name,
    HTMLSlotElement.prototype.assignedElements.name,
    HTMLSlotElement.prototype.assign.name,
    Element.prototype.attachShadow.length,
    Element.prototype.attachShadow.name,
    slot.assignedNodes().length,
    slot.assignedNodes()[0] === assigned,
    slot.assignedElements().length,
    slot.assignedElements()[0] === assigned,
    slot.assignedElements({ flatten: true })[0] === assigned
  ].join('|');
})()
"#,
        )
        .expect("slot assignment methods should surface assigned nodes and elements");

    assert_eq!(
        result,
        "function|function|true|true|true|false|false|false|false|false|false|0|0|0|assignedNodes|assignedElements|assign|1|attachShadow|1|true|1|true|true"
    );
}
#[test]
fn html_slot_element_assign_tracks_manual_assignments() {
    let mut vm = new_storage_test_vm("https://html-slot-manual-assignment.test/");

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
  const host = document.createElement('div');
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open', slotAssignment: 'manual' });
  shadow.innerHTML = '<slot id="a"></slot><slot id="b"></slot>';
  const slotA = shadow.querySelector('#a');
  const slotB = shadow.querySelector('#b');
  const one = document.createElement('span');
  const two = document.createElement('strong');
  const outside = document.createElement('em');
  host.append(one, two);

  slotA.assign(one, outside, two, one);
  const first = [
    typeof slotA.assign,
    slotA.assignedNodes().length,
    slotA.assignedNodes()[0] === one,
    slotA.assignedNodes()[1] === two,
    one.assignedSlot === slotA,
    outside.assignedSlot === null
  ].join(',');

  host.append(outside);
  const revealed = slotA.assignedNodes()[1] === outside;

  slotB.assign(outside);
  const moved = [
    slotA.assignedNodes().length,
    slotA.assignedNodes()[0] === one,
    slotA.assignedNodes()[1] === two,
    slotB.assignedNodes()[0] === outside,
    outside.assignedSlot === slotB
  ].join(',');

  let invalid = 'no-throw';
  try {
    slotA.assign(document.createComment('x'));
  } catch (error) {
    invalid = error.name;
  }

  return [first, revealed, moved, invalid].join('|');
})()
"#,
        )
        .expect("manual slot assignment should track assigned slottables");

    assert_eq!(
        result,
        "function,2,true,true,true,true|true|2,true,true,true,true|TypeError"
    );
}
#[test]
fn html_slot_element_assign_dispatches_slotchange_as_microtask() {
    let mut vm = new_storage_test_vm("https://html-slot-manual-slotchange.test/");

    let sync_count = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const host = document.createElement('div');
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open', slotAssignment: 'manual' });
  shadow.innerHTML = '<slot></slot>';
  globalThis.__manualSlotchangeSlot = shadow.querySelector('slot');
  globalThis.__manualSlotchangeNode = document.createElement('span');
  host.append(globalThis.__manualSlotchangeNode);
  globalThis.__manualSlotchangeCount = 0;
  globalThis.__manualSlotchangeBubbledCount = 0;
  globalThis.__manualSlotchangeBubbles = null;
  shadow.addEventListener('slotchange', () => {
    globalThis.__manualSlotchangeBubbledCount++;
  });
  globalThis.__manualSlotchangeSlot.addEventListener('slotchange', (event) => {
    globalThis.__manualSlotchangeCount++;
    globalThis.__manualSlotchangeBubbles = event.bubbles;
  });
  globalThis.__manualSlotchangeSlot.assign(globalThis.__manualSlotchangeNode);
  return [
    globalThis.__manualSlotchangeCount,
    globalThis.__manualSlotchangeBubbledCount,
    globalThis.__manualSlotchangeBubbles
  ].join('|');
})()
"#,
        )
        .expect("manual slotchange setup should evaluate");
    let async_count = vm
        .eval(
            r#"
[
  globalThis.__manualSlotchangeCount,
  globalThis.__manualSlotchangeBubbledCount,
  globalThis.__manualSlotchangeBubbles
].join('|')
"#,
        )
        .expect("manual slotchange microtask should have run");

    let unchanged_sync_count = vm
        .eval(
            r#"
(() => {
  globalThis.__manualSlotchangeSlot.assign(globalThis.__manualSlotchangeNode);
  globalThis.__manualSlotchangeSlot.assign(globalThis.__manualSlotchangeNode, globalThis.__manualSlotchangeNode);
  return [
    globalThis.__manualSlotchangeCount,
    globalThis.__manualSlotchangeBubbledCount
  ].join('|');
})()
"#,
        )
        .expect("unchanged manual assignment should evaluate");
    let unchanged_async_count = vm
        .eval(
            r#"
[
  globalThis.__manualSlotchangeCount,
  globalThis.__manualSlotchangeBubbledCount
].join('|')
"#,
        )
        .expect("unchanged manual assignment should not queue slotchange");
    let unassigned_child_sync_count = vm
        .eval(
            r#"
(() => {
  globalThis.__manualSlotchangeUnassigned = document.createElement('b');
  globalThis.__manualSlotchangeSlot.parentNode.host.appendChild(globalThis.__manualSlotchangeUnassigned);
  return [
    globalThis.__manualSlotchangeCount,
    globalThis.__manualSlotchangeBubbledCount
  ].join('|');
})()
"#,
        )
        .expect("unassigned manual host child insertion should evaluate");
    let unassigned_child_async_count = vm
        .eval(
            r#"
[
  globalThis.__manualSlotchangeCount,
  globalThis.__manualSlotchangeBubbledCount
].join('|')
"#,
        )
        .expect("unassigned manual host child insertion should not queue slotchange");
    let assigned_child_remove_sync_count = vm
        .eval(
            r#"
(() => {
  globalThis.__manualSlotchangeNode.remove();
  return [
    globalThis.__manualSlotchangeCount,
    globalThis.__manualSlotchangeBubbledCount
  ].join('|');
})()
"#,
        )
        .expect("assigned manual host child removal should evaluate");
    let assigned_child_remove_async_count = vm
        .eval(
            r#"
[
  globalThis.__manualSlotchangeCount,
  globalThis.__manualSlotchangeBubbledCount
].join('|')
"#,
        )
        .expect("assigned manual host child removal should queue slotchange");

    assert_eq!(sync_count, "0|0|");
    assert_eq!(async_count, "1|1|true");
    assert_eq!(unchanged_sync_count, "1|1");
    assert_eq!(unchanged_async_count, "1|1");
    assert_eq!(unassigned_child_sync_count, "1|1");
    assert_eq!(unassigned_child_async_count, "1|1");
    assert_eq!(assigned_child_remove_sync_count, "1|1");
    assert_eq!(assigned_child_remove_async_count, "2|2");
}
#[test]
fn html_slot_element_named_assignment_mutations_dispatch_slotchange_as_microtask() {
    let mut vm = new_storage_test_vm("https://html-slot-named-slotchange.test/");

    let append_sync_count = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const host = document.createElement('div');
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<slot name="a"></slot>';
  globalThis.__namedSlotchangeSlot = shadow.querySelector('slot');
  globalThis.__namedSlotchangeNode = document.createElement('span');
  globalThis.__namedSlotchangeNode.slot = 'a';
  globalThis.__namedSlotchangeCount = 0;
  globalThis.__namedSlotchangeBubbles = null;
  globalThis.__namedSlotchangeSlot.addEventListener('slotchange', (event) => {
    globalThis.__namedSlotchangeCount++;
    globalThis.__namedSlotchangeBubbles = event.bubbles;
  });
  host.appendChild(globalThis.__namedSlotchangeNode);
  return [
    globalThis.__namedSlotchangeCount,
    globalThis.__namedSlotchangeBubbles
  ].join('|');
})()
"#,
        )
        .expect("named slotchange append setup should evaluate");
    let append_async_count = vm
        .eval(
            r#"
[
  globalThis.__namedSlotchangeCount,
  globalThis.__namedSlotchangeBubbles
].join('|')
"#,
        )
        .expect("named slotchange append microtask should run");

    let slot_attr_sync_count = vm
        .eval(
            r#"
(() => {
  globalThis.__namedSlotchangeNode.slot = 'b';
  return globalThis.__namedSlotchangeCount;
})()
"#,
        )
        .expect("slot attribute change should evaluate");
    let slot_attr_async_count = vm
        .eval("globalThis.__namedSlotchangeCount")
        .expect("slot attribute change microtask should run");

    let remove_sync_count = vm
        .eval(
            r#"
(() => {
  globalThis.__namedSlotchangeNode.slot = 'a';
  return globalThis.__namedSlotchangeCount;
})()
"#,
        )
        .expect("slot attribute restore should evaluate");
    let restored_async_count = vm
        .eval("globalThis.__namedSlotchangeCount")
        .expect("slot attribute restore microtask should run");
    let remove_after_restore_sync_count = vm
        .eval(
            r#"
(() => {
  globalThis.__namedSlotchangeNode.remove();
  return globalThis.__namedSlotchangeCount;
})()
"#,
        )
        .expect("slottable removal should evaluate");
    let remove_async_count = vm
        .eval("globalThis.__namedSlotchangeCount")
        .expect("slottable removal microtask should run");

    assert_eq!(append_sync_count, "0|");
    assert_eq!(append_async_count, "1|true");
    assert_eq!(slot_attr_sync_count, "1");
    assert_eq!(slot_attr_async_count, "2");
    assert_eq!(remove_sync_count, "2");
    assert_eq!(restored_async_count, "3");
    assert_eq!(remove_after_restore_sync_count, "3");
    assert_eq!(remove_async_count, "4");
}
#[test]
fn shadow_root_onslotchange_runs_for_bubbled_slotchange() {
    let mut vm = new_storage_test_vm("https://shadow-root-onslotchange.test/");

    let sync_count = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const host = document.createElement('div');
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<slot name="slot1"></slot>';
  const slottable = document.createElement('span');
  slottable.slot = 'slot1';
  globalThis.__shadowRootOnslotchangeCount = 0;
  globalThis.__shadowRootOnslotchangeTarget = '';
  shadow.onslotchange = event => {
    globalThis.__shadowRootOnslotchangeCount++;
    globalThis.__shadowRootOnslotchangeTarget = event.target.localName;
  };
  host.appendChild(slottable);
  return [
    globalThis.__shadowRootOnslotchangeCount,
    globalThis.__shadowRootOnslotchangeTarget
  ].join('|');
})()
"#,
        )
        .expect("shadow root onslotchange setup should evaluate");
    let async_count = vm
        .eval(
            r#"
[
  globalThis.__shadowRootOnslotchangeCount,
  globalThis.__shadowRootOnslotchangeTarget
].join('|')
"#,
        )
        .expect("shadow root onslotchange should run after slotchange microtask");

    assert_eq!(sync_count, "0|");
    assert_eq!(async_count, "1|slot");
}
#[test]
fn detached_shadow_root_onslotchange_runs_for_bubbled_slotchange() {
    let mut vm = new_storage_test_vm("https://detached-shadow-root-onslotchange.test/");

    let sync_count = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const source = document.createElement('div');
  source.innerHTML = '<div id="host"><template><slot name="slot1"></slot></template><span id="existing" slot="slot1"></span></div>';
  const root = source.cloneNode(true);
  const host = root.querySelector('#host');
  const template = host.querySelector('template');
  template.remove();
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.appendChild(document.importNode(template.content, true));
  const slot = shadow.querySelector('slot');
  const slottable = document.createElement('span');
  slottable.slot = 'slot1';
  globalThis.__detachedShadowRootOnslotchangeCount = 0;
  globalThis.__detachedShadowRootOnslotchangeAssigned = -1;
  shadow.onslotchange = () => {
    globalThis.__detachedShadowRootOnslotchangeCount++;
    globalThis.__detachedShadowRootOnslotchangeAssigned = slot.assignedNodes().length;
  };
  host.appendChild(slottable);
  return [
    globalThis.__detachedShadowRootOnslotchangeCount,
    globalThis.__detachedShadowRootOnslotchangeAssigned
  ].join('|');
})()
"#,
        )
        .expect("detached shadow root onslotchange setup should evaluate");
    let async_count = vm
        .eval(
            r#"
[
  globalThis.__detachedShadowRootOnslotchangeCount,
  globalThis.__detachedShadowRootOnslotchangeAssigned
].join('|')
"#,
        )
        .expect("detached shadow root onslotchange should run after slotchange microtask");

    assert_eq!(sync_count, "0|-1");
    assert_eq!(async_count, "1|2");
}
#[test]
fn html_slot_element_shadow_tree_slot_mutations_dispatch_slotchange_as_microtask() {
    let mut vm = new_storage_test_vm("https://html-slot-shadow-tree-slotchange.test/");

    let insert_sync_count = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const host = document.createElement('div');
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  globalThis.__shadowTreeSlotchangeNode = document.createElement('span');
  globalThis.__shadowTreeSlotchangeNode.slot = 'dynamic';
  host.appendChild(globalThis.__shadowTreeSlotchangeNode);
  globalThis.__shadowTreeSlotchangeSlot = document.createElement('slot');
  globalThis.__shadowTreeSlotchangeSlot.name = 'dynamic';
  globalThis.__shadowTreeSlotchangeCount = 0;
  globalThis.__shadowTreeSlotchangeSlot.addEventListener('slotchange', () => {
    globalThis.__shadowTreeSlotchangeCount++;
  });
  shadow.appendChild(globalThis.__shadowTreeSlotchangeSlot);
  return globalThis.__shadowTreeSlotchangeCount;
})()
"#,
        )
        .expect("shadow tree slot insertion should evaluate");
    let insert_async_count = vm
        .eval("globalThis.__shadowTreeSlotchangeCount")
        .expect("shadow tree slot insertion microtask should run");

    let name_sync_count = vm
        .eval(
            r#"
(() => {
  globalThis.__shadowTreeSlotchangeSlot.name = 'other';
  return globalThis.__shadowTreeSlotchangeCount;
})()
"#,
        )
        .expect("shadow tree slot name mutation should evaluate");
    let name_async_count = vm
        .eval("globalThis.__shadowTreeSlotchangeCount")
        .expect("shadow tree slot name mutation microtask should run");

    let remove_sync_count = vm
        .eval(
            r#"
(() => {
  globalThis.__shadowTreeSlotchangeNode.slot = 'other';
  return globalThis.__shadowTreeSlotchangeCount;
})()
"#,
        )
        .expect("slottable should move to renamed slot");
    let moved_async_count = vm
        .eval("globalThis.__shadowTreeSlotchangeCount")
        .expect("slottable move microtask should run");
    let removed_slot_sync_count = vm
        .eval(
            r#"
(() => {
  globalThis.__shadowTreeSlotchangeSlot.remove();
  return globalThis.__shadowTreeSlotchangeCount;
})()
"#,
        )
        .expect("shadow tree slot removal should evaluate");
    let removed_slot_async_count = vm
        .eval("globalThis.__shadowTreeSlotchangeCount")
        .expect("shadow tree slot removal microtask should run");

    assert_eq!(insert_sync_count, "0");
    assert_eq!(insert_async_count, "1");
    assert_eq!(name_sync_count, "1");
    assert_eq!(name_async_count, "2");
    assert_eq!(remove_sync_count, "2");
    assert_eq!(moved_async_count, "3");
    assert_eq!(removed_slot_sync_count, "3");
    assert_eq!(removed_slot_async_count, "4");
}
#[test]
fn html_slot_element_empty_shadow_tree_slot_mutations_do_not_dispatch_slotchange() {
    let mut vm = new_storage_test_vm("https://html-slot-empty-slotchange.test/");

    let insert_sync_count = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const host = document.createElement('div');
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const slot = document.createElement('slot');
  globalThis.__emptySlotchangeSlot = slot;
  globalThis.__emptySlotchangeCount = 0;
  slot.addEventListener('slotchange', () => globalThis.__emptySlotchangeCount++);

  shadow.appendChild(slot);
  return globalThis.__emptySlotchangeCount;
})()
"#,
        )
        .expect("empty slot insertion should evaluate");
    let insert_async_count = vm
        .eval("globalThis.__emptySlotchangeCount")
        .expect("empty slot insertion microtask should run");

    let name_sync_count = vm
        .eval(
            r#"
(() => {
  globalThis.__emptySlotchangeSlot.name = 'unused';
  return globalThis.__emptySlotchangeCount;
})()
"#,
        )
        .expect("empty slot name mutation should evaluate");
    let name_async_count = vm
        .eval("globalThis.__emptySlotchangeCount")
        .expect("empty slot name mutation microtask should run");

    let remove_sync_count = vm
        .eval(
            r#"
(() => {
  globalThis.__emptySlotchangeSlot.remove();
  return globalThis.__emptySlotchangeCount;
})()
"#,
        )
        .expect("empty slot removal should evaluate");
    let remove_async_count = vm
        .eval("globalThis.__emptySlotchangeCount")
        .expect("empty slot removal microtask should run");

    assert_eq!(insert_sync_count, "0");
    assert_eq!(insert_async_count, "0");
    assert_eq!(name_sync_count, "0");
    assert_eq!(name_async_count, "0");
    assert_eq!(remove_sync_count, "0");
    assert_eq!(remove_async_count, "0");
}
#[test]
fn html_slot_element_inner_html_fragment_marks_actual_slottables() {
    let mut vm = new_storage_test_vm("https://html-slotchange-inner-html.test/");

    let setup_sync_counts = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const host = document.createElement('div');
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const namedSlot = document.createElement('slot');
  namedSlot.name = 'someSlot';
  const defaultSlot = document.createElement('slot');
  shadow.append(namedSlot, defaultSlot);
  globalThis.__innerHtmlDefaultCount = 0;
  globalThis.__innerHtmlNamedCount = 0;
  defaultSlot.addEventListener('slotchange', () => globalThis.__innerHtmlDefaultCount++);
  namedSlot.addEventListener('slotchange', () => globalThis.__innerHtmlNamedCount++);
  host.innerHTML = 'foo <b>bar</b>';
  return [globalThis.__innerHtmlDefaultCount, globalThis.__innerHtmlNamedCount].join('|');
})()
"#,
        )
        .expect("innerHTML default setup should evaluate");
    let setup_async_counts = vm
        .eval("[globalThis.__innerHtmlDefaultCount, globalThis.__innerHtmlNamedCount].join('|')")
        .expect("innerHTML default slotchange should run");
    let clear_sync_counts = vm
        .eval(
            r#"
(() => {
  document.body.firstChild.innerHTML = '';
  return [globalThis.__innerHtmlDefaultCount, globalThis.__innerHtmlNamedCount].join('|');
})()
"#,
        )
        .expect("innerHTML clear should evaluate");
    let clear_async_counts = vm
        .eval("[globalThis.__innerHtmlDefaultCount, globalThis.__innerHtmlNamedCount].join('|')")
        .expect("innerHTML clear slotchange should run");
    let named_sync_counts = vm
        .eval(
            r#"
(() => {
  document.body.firstChild.innerHTML = '<b slot="someSlot">content</b>';
  return [globalThis.__innerHtmlDefaultCount, globalThis.__innerHtmlNamedCount].join('|');
})()
"#,
        )
        .expect("innerHTML named slot insertion should evaluate");
    let named_async_counts = vm
        .eval("[globalThis.__innerHtmlDefaultCount, globalThis.__innerHtmlNamedCount].join('|')")
        .expect("innerHTML named slotchange should run");

    assert_eq!(setup_sync_counts, "0|0");
    assert_eq!(setup_async_counts, "1|0");
    assert_eq!(clear_sync_counts, "1|0");
    assert_eq!(clear_async_counts, "2|0");
    assert_eq!(named_sync_counts, "2|0");
    assert_eq!(named_async_counts, "2|1");
}
#[test]
fn html_slot_element_slotchange_mutation_observer_delivery_keeps_later_signal() {
    let mut vm = new_storage_test_vm("https://html-slotchange-mo-order.test/");

    let sync_log = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const host = document.createElement('div');
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const slot = document.createElement('slot');
  shadow.appendChild(slot);

  const element = document.createElement('div');
  globalThis.__slotchangeMoEventCount = 0;
  globalThis.__slotchangeMoLog = [];

  slot.addEventListener('slotchange', () => {
    globalThis.__slotchangeMoEventCount++;
    globalThis.__slotchangeMoLog.push(`slot:${globalThis.__slotchangeMoEventCount}`);
  });

  new MutationObserver(() => {
    globalThis.__slotchangeMoLog.push(`mo1:${globalThis.__slotchangeMoEventCount}`);
    host.appendChild(document.createElement('span'));
    element.setAttribute('title', 'bar');
  }).observe(element, { attributes: true, attributeFilter: ['id'] });

  new MutationObserver(() => {
    globalThis.__slotchangeMoLog.push(`mo2:${globalThis.__slotchangeMoEventCount}`);
  }).observe(element, { attributes: true, attributeFilter: ['title'] });

  element.setAttribute('id', 'foo');
  host.appendChild(document.createElement('div'));
  return globalThis.__slotchangeMoLog.join('|');
})()
"#,
        )
        .expect("slotchange mutation observer setup should evaluate");
    let async_log = vm
        .eval("globalThis.__slotchangeMoLog.join('|')")
        .expect("slotchange and mutation observers should flush");

    assert_eq!(sync_log, "");
    assert_eq!(async_log, "mo1:0|slot:1|mo2:1|slot:2");
}
#[test]
fn html_slot_element_slotchange_preserves_reassignment_signal_order() {
    let mut vm = new_storage_test_vm("https://html-slotchange-reassignment-order.test/");

    let sync_log = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const host = document.createElement('div');
  const shadow = host.attachShadow({ mode: 'open' });
  const named = document.createElement('slot');
  named.name = 'title';
  const fallback = document.createElement('slot');
  shadow.append(named, fallback);
  const body = document.createElement('p');
  host.append(body);
  document.body.append(host);
  globalThis.__slotReassignmentLog = [];
  globalThis.__slotReassignmentBody = body;
  fallback.addEventListener('slotchange', () => __slotReassignmentLog.push('default'));
  named.addEventListener('slotchange', () => __slotReassignmentLog.push('title'));
  return __slotReassignmentLog.join('|');
})()
"#,
        )
        .expect("slot reassignment order setup should evaluate");
    let mutation_sync_log = vm
        .eval(
            r#"
(() => {
  __slotReassignmentLog.length = 0;
  __slotReassignmentBody.slot = 'title';
  return __slotReassignmentLog.join('|');
})()
"#,
        )
        .expect("slot reassignment should evaluate");
    let async_log = vm
        .eval("__slotReassignmentLog.join('|')")
        .expect("slot reassignment events should flush");

    assert_eq!(sync_log, "");
    assert_eq!(mutation_sync_log, "");
    assert_eq!(async_log, "default|title");
}
#[test]
fn html_slot_element_slotchange_bubbles_through_nested_assigned_slot() {
    let mut vm = new_storage_test_vm("https://html-slotchange-nested-path.test/");

    let sync_counts = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const outerHost = document.createElement('div');
  document.body.appendChild(outerHost);
  const outerShadow = outerHost.attachShadow({ mode: 'open' });
  const outerSlot = document.createElement('slot');
  outerShadow.appendChild(document.createElement('span'));

  const innerHost = document.createElement('div');
  innerHost.appendChild(outerSlot);
  outerShadow.appendChild(innerHost);

  const innerShadow = innerHost.attachShadow({ mode: 'open' });
  innerShadow.appendChild(document.createElement('span'));
  const innerSlot = document.createElement('slot');

  globalThis.__nestedSlotchangeOuterCount = 0;
  globalThis.__nestedSlotchangeInnerTargets = [];
  outerSlot.addEventListener('slotchange', () => {
    globalThis.__nestedSlotchangeOuterCount++;
  });
  innerSlot.addEventListener('slotchange', (event) => {
    if (event.target === innerSlot) {
      globalThis.__nestedSlotchangeInnerTargets.push('inner');
    } else if (event.target === outerSlot) {
      globalThis.__nestedSlotchangeInnerTargets.push('outer');
    } else if (event.target === innerHost) {
      globalThis.__nestedSlotchangeInnerTargets.push('host');
    } else {
      globalThis.__nestedSlotchangeInnerTargets.push('other');
    }
  });

  innerShadow.appendChild(innerSlot);
  outerHost.appendChild(document.createElement('span'));
  return [
    globalThis.__nestedSlotchangeOuterCount,
    globalThis.__nestedSlotchangeInnerTargets.join(',')
  ].join('|');
})()
"#,
        )
        .expect("nested slotchange setup should evaluate");
    let async_counts = vm
        .eval(
            r#"
[
  globalThis.__nestedSlotchangeOuterCount,
  globalThis.__nestedSlotchangeInnerTargets.join(',')
].join('|')
"#,
        )
        .expect("nested slotchange microtask should run");

    assert_eq!(sync_counts, "0|");
    assert_eq!(async_counts, "1|inner,outer");
}
#[test]
fn shadow_dom_composed_path_includes_window_and_filters_closed_shadow_internals() {
    let mut vm = new_storage_test_vm("https://shadow-composed-path.test/");

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
  const plainRoot = document.createElement('div');
  const plainParent = document.createElement('section');
  const plainTarget = document.createElement('span');
  document.body.appendChild(plainRoot);
  plainRoot.appendChild(plainParent);
  plainParent.appendChild(plainTarget);

  let plainPath = [];
  plainTarget.addEventListener('x-plain', event => {
    plainPath = event.composedPath().map(node => {
      if (node === plainTarget) return 'target';
      if (node === plainParent) return 'parent';
      if (node === plainRoot) return 'root';
      if (node === window) return 'window';
      return 'other';
    });
  });
  plainTarget.dispatchEvent(new Event('x-plain', { bubbles: true, composed: true }));

  const host = document.createElement('div');
  const slottable = document.createElement('span');
  document.body.appendChild(host);
  host.appendChild(slottable);
  const shadow = host.attachShadow({ mode: 'closed' });
  const slot = document.createElement('slot');
  shadow.appendChild(slot);

  let targetPath = [];
  let slotPath = [];
  slottable.addEventListener('x-slot', event => {
    targetPath = event.composedPath().map(node => {
      if (node === slottable) return 'target';
      if (node === slot) return 'slot';
      if (node === shadow) return 'shadow';
      if (node === host) return 'host';
      if (node === window) return 'window';
      return 'other';
    });
  });
  slot.addEventListener('x-slot', event => {
    slotPath = event.composedPath().map(node => {
      if (node === slottable) return 'target';
      if (node === slot) return 'slot';
      if (node === shadow) return 'shadow';
      if (node === host) return 'host';
      if (node === window) return 'window';
      return 'other';
    });
  });
  slottable.dispatchEvent(new Event('x-slot', { bubbles: true, composed: true }));

  return [
    plainPath.join(','),
    targetPath.join(','),
    slotPath.join(',')
  ].join('|');
})()
"#,
        )
        .expect("composed path should evaluate");

    assert_eq!(
        result,
        "target,parent,root,other,other,other,window|target,host,other,other,other,window|target,slot,shadow,host,other,other,other,window"
    );
}
#[test]
fn detached_shadow_dom_composed_path_does_not_include_window() {
    let mut vm = new_storage_test_vm("https://detached-shadow-composed-path.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('div');
  const shadow = host.attachShadow({ mode: 'open' });
  const target = document.createElement('span');
  shadow.appendChild(target);

  let observed = [];
  target.addEventListener('x-detached', event => {
    observed = event.composedPath().map(node => {
      if (node === target) return 'target';
      if (node === shadow) return 'shadow';
      if (node === host) return 'host';
      if (node === window) return 'window';
      return 'other';
    });
  });

  target.dispatchEvent(new Event('x-detached', { bubbles: true, composed: true }));
  return observed.join(',');
})()
"#,
        )
        .expect("detached composed path should evaluate");

    assert_eq!(result, "target,shadow,host");
}
#[test]
fn shadow_dom_related_target_retargets_and_trims_dispatch_path() {
    let mut vm = new_storage_test_vm("https://shadow-related-target.test/");

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

  const root = document.createElement('main');
  const host1 = document.createElement('div');
  const host2 = document.createElement('div');
  root.append(host1, host2);
  document.body.appendChild(root);

  const shadow1 = host1.attachShadow({ mode: 'open' });
  const target = document.createElement('button');
  shadow1.appendChild(target);
  const shadow2 = host2.attachShadow({ mode: 'open' });
  const related = document.createElement('button');
  shadow2.appendChild(related);

  const log = [];
  const label = node => {
    if (node === target) return 'target';
    if (node === related) return 'related';
    if (node === host1) return 'host1';
    if (node === host2) return 'host2';
    if (node === shadow1) return 'shadow1';
    if (node === shadow2) return 'shadow2';
    if (node === root) return 'root';
    if (node === window) return 'window';
    return 'other';
  };
  for (const node of [target, shadow1, host1, root]) {
    node.addEventListener('x-related', event => {
      log.push([
        label(node),
        label(event.target),
        label(event.relatedTarget),
        event.composedPath().map(label).join(',')
      ].join(':'));
    });
  }
  target.dispatchEvent(new FocusEvent('x-related', {
    bubbles: true,
    composed: true,
    relatedTarget: related
  }));

  const ancestorHost = document.createElement('section');
  const ancestorShadow = ancestorHost.attachShadow({ mode: 'open' });
  const internal = document.createElement('span');
  ancestorShadow.appendChild(internal);
  document.body.appendChild(ancestorHost);
  let ancestorCount = 0;
  ancestorHost.addEventListener('x-ancestor', () => ancestorCount++);
  ancestorHost.dispatchEvent(new FocusEvent('x-ancestor', {
    bubbles: true,
    composed: true,
    relatedTarget: internal
  }));

  return `${log.join('|')}|ancestor:${ancestorCount}`;
})()
"#,
        )
        .expect("relatedTarget retargeting should evaluate");

    assert_eq!(
        result,
        "target:target:host2:target,shadow1,host1,root,other,other,other,window|shadow1:target:host2:target,shadow1,host1,root,other,other,other,window|host1:host1:host2:target,shadow1,host1,root,other,other,other,window|root:host1:host2:target,shadow1,host1,root,other,other,other,window|ancestor:0"
    );
}
#[test]
fn shadow_dom_retargeted_shadow_host_listeners_run_at_target_phase() {
    let mut vm = new_storage_test_vm("https://shadow-retargeted-host-phase.test/");

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

  const host = document.createElement('div');
  const shadow = host.attachShadow({ mode: 'closed' });
  const target = document.createElement('button');
  shadow.appendChild(target);
  document.body.appendChild(host);

  const log = [];
  const record = (name, mode) => event => {
    log.push(`${name}:${mode}:${event.eventPhase}:${event.target === host ? 'host' : 'target'}`);
  };
  host.addEventListener('x-phase', record('host', 'capture'), true);
  host.addEventListener('x-phase', record('host', 'bubble'));
  shadow.addEventListener('x-phase', record('shadow', 'capture'), true);
  shadow.addEventListener('x-phase', record('shadow', 'bubble'));
  target.addEventListener('x-phase', record('target', 'capture'), true);
  target.addEventListener('x-phase', record('target', 'bubble'));

  target.dispatchEvent(new Event('x-phase', { bubbles: true, composed: true }));
  return log.join('|');
})()
"#,
        )
        .expect("retargeted host event phases should evaluate");

    assert_eq!(
        result,
        "host:capture:2:host|shadow:capture:1:target|target:capture:2:target|target:bubble:2:target|shadow:bubble:3:target|host:bubble:2:host"
    );
}
#[test]
fn shadow_dom_non_composed_slotted_event_returns_to_original_light_tree() {
    let mut vm = new_storage_test_vm("https://shadow-non-composed-slotted-path.test/");

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

  const outerHost = document.createElement('div');
  const outerShadow = outerHost.attachShadow({ mode: 'open' });
  const innerHost = document.createElement('section');
  const target = document.createElement('button');
  const innerShadow = innerHost.attachShadow({ mode: 'open' });
  const slot = document.createElement('slot');

  target.id = 'target';
  slot.id = 'slot';
  innerShadow.id = 'inner-shadow';
  innerHost.id = 'inner-host';
  outerShadow.id = 'outer-shadow';
  outerHost.id = 'outer-host';

  innerHost.appendChild(target);
  innerShadow.appendChild(slot);
  outerShadow.appendChild(innerHost);
  document.body.appendChild(outerHost);

  const log = [];
  for (const node of [target, slot, innerShadow, innerHost, outerShadow, outerHost, document.body]) {
    node.addEventListener('x-path', () => log.push(node.id || node.localName));
  }

  target.dispatchEvent(new Event('x-path', { bubbles: true }));
  return log.join('|');
})()
"#,
        )
        .expect("non-composed slotted event path should evaluate");

    assert_eq!(result, "target|slot|inner-shadow|inner-host|outer-shadow");
}
#[test]
fn shadow_dom_event_post_dispatch_uses_initial_path_boundaries() {
    let mut vm = new_storage_test_vm("https://shadow-post-dispatch.test/");

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

  const composedHost = document.createElement('div');
  const composedShadow = composedHost.attachShadow({ mode: 'open' });
  const composedTarget = document.createElement('span');
  composedShadow.appendChild(composedTarget);
  document.body.appendChild(composedHost);
  const composedEvent = new Event('x-unheard-composed', { bubbles: true, composed: true });
  composedTarget.dispatchEvent(composedEvent);

  const scopedHost = document.createElement('div');
  const scopedShadow = scopedHost.attachShadow({ mode: 'open' });
  const scopedTarget = document.createElement('span');
  scopedShadow.appendChild(scopedTarget);
  document.body.appendChild(scopedHost);
  const scopedEvent = new Event('x-unheard-scoped', { bubbles: true, composed: false });
  scopedTarget.dispatchEvent(scopedEvent);

  const movedOutHost = document.createElement('div');
  const movedOutShadow = movedOutHost.attachShadow({ mode: 'open' });
  const movedOutTarget = document.createElement('span');
  movedOutShadow.appendChild(movedOutTarget);
  document.body.appendChild(movedOutHost);
  let scopedWindowEventHidden = false;
  movedOutTarget.addEventListener('x-moved-out', event => {
    scopedWindowEventHidden = window.event === undefined;
    movedOutHost.appendChild(movedOutTarget);
  });
  const movedOutEvent = new MouseEvent('x-moved-out', {
    bubbles: true,
    composed: false,
    relatedTarget: movedOutHost
  });
  movedOutTarget.dispatchEvent(movedOutEvent);

  const movedInHost = document.createElement('div');
  const movedInShadow = movedInHost.attachShadow({ mode: 'open' });
  const movedInTarget = document.createElement('span');
  movedInHost.appendChild(movedInTarget);
  document.body.appendChild(movedInHost);
  let documentWindowEventVisible = false;
  movedInTarget.addEventListener('x-moved-in', event => {
    documentWindowEventVisible = window.event === event;
    movedInShadow.appendChild(movedInTarget);
  });
  const movedInEvent = new MouseEvent('x-moved-in', {
    bubbles: true,
    composed: false,
    relatedTarget: movedInHost
  });
  movedInTarget.dispatchEvent(movedInEvent);

  const detachedMovedInHost = document.createElement('div');
  const detachedMovedInShadow = detachedMovedInHost.attachShadow({ mode: 'open' });
  const detachedMovedInTarget = document.createElement('span');
  detachedMovedInHost.appendChild(detachedMovedInTarget);
  let detachedWindowEventVisible = false;
  detachedMovedInTarget.addEventListener('x-detached-moved-in', event => {
    detachedWindowEventVisible = window.event === event;
    detachedMovedInShadow.appendChild(detachedMovedInTarget);
  });
  detachedMovedInTarget.dispatchEvent(new MouseEvent('x-detached-moved-in', {
    bubbles: true,
    composed: false,
    relatedTarget: detachedMovedInHost
  }));

  return [
    composedEvent.target === composedHost,
    composedEvent.currentTarget === null,
    composedEvent.composedPath().length,
    scopedEvent.target === null,
    movedOutEvent.target === null,
    movedOutEvent.relatedTarget === null,
    scopedWindowEventHidden,
    movedInEvent.target === movedInTarget,
    movedInEvent.relatedTarget === movedInHost,
    documentWindowEventVisible,
    detachedWindowEventVisible
  ].join('|');
})()
"#,
        )
        .expect("post-dispatch event state should evaluate");

    assert_eq!(
        result,
        "true|true|0|true|true|true|true|true|true|true|true"
    );
}

#[test]
fn window_event_uses_initial_invocation_target_shadow_scope() {
    let mut vm = new_storage_test_vm("https://shadow-window-event.test/");

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

  const host = document.createElement('div');
  const root = host.attachShadow({ mode: 'closed' });
  const target = document.createElement('span');
  root.appendChild(target);
  document.body.appendChild(host);

  const log = [];
  target.addEventListener('scoped', event => {
    log.push(`target-before:${window.event === undefined}`);
    host.appendChild(target);
  });
  target.addEventListener('scoped', () => {
    log.push(`target-after:${window.event === undefined}`);
  });
  root.addEventListener('scoped', () => {
    log.push(`root:${window.event === undefined}`);
  });
  host.addEventListener('scoped', event => {
    log.push(`host:${window.event === event}`);
  });
  window.addEventListener('scoped', event => {
    log.push(`window:${window.event === event}`);
  }, { once: true });

  target.dispatchEvent(new Event('scoped', { bubbles: true, composed: true }));
  return log.join('|');
})()
"#,
        )
        .expect("shadow-scoped window.event probe should evaluate");

    assert_eq!(
        result,
        "target-before:true|target-after:true|root:true|host:true|window:true"
    );
}

#[test]
fn shadow_root_active_element_tracks_focused_descendant() {
    let mut vm = new_storage_test_vm("https://shadow-active-element.test/");

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
  const host = document.createElement('div');
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<input><button></button>';
  const input = shadow.querySelector('input');
  const button = shadow.querySelector('button');

  const before = shadow.activeElement === null;
  input.focus();
  const afterInput = shadow.activeElement === input;
  button.focus();
  const afterButton = shadow.activeElement === button;
  host.remove();

  return [before, afterInput, afterButton, shadow.activeElement === null].join('|');
})()
"#,
        )
        .expect("ShadowRoot.activeElement should reflect focused descendants");

    assert_eq!(result, "true|true|true|true");
}
#[test]
fn detached_child_document_active_element_retargets_shadow_descendant_to_host() {
    let mut vm = new_storage_test_vm("https://detached-child-active-element.test/");

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
  document.body.appendChild(frame);
  const d = frame.contentWindow.document;
  const host = d.createElement('div');
  host.id = 'shRoot';
  d.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const input = d.createElement('input');
  input.id = 'inpId';
  shadow.appendChild(input);

  input.focus();

  return [
    d.activeElement === host,
    d.activeElement && d.activeElement.getAttribute('id'),
    shadow.activeElement === input,
    shadow.activeElement && shadow.activeElement.getAttribute('id')
  ].join('|');
})()
"#,
        )
        .expect("detached child activeElement should retarget through shadow hosts");

    assert_eq!(result, "true|shRoot|true|inpId");
}
#[test]
fn detached_child_shadow_root_does_not_inherit_content_editable_from_host() {
    let mut vm = new_storage_test_vm("https://detached-child-contenteditable-shadow.test/");

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
  document.body.appendChild(frame);
  const d = frame.contentWindow.document;

  const hostTrue = d.createElement('div');
  hostTrue.contentEditable = 'true';
  d.body.appendChild(hostTrue);
  const trueRoot = hostTrue.attachShadow({ mode: 'open' });

  const hostFalse = d.createElement('div');
  hostFalse.contentEditable = 'false';
  d.body.appendChild(hostFalse);
  const falseRoot = hostFalse.attachShadow({ mode: 'open' });

  const inheritedHost = d.createElement('div');
  d.body.appendChild(inheritedHost);
  d.body.contentEditable = 'true';
  const inheritedRoot = inheritedHost.attachShadow({ mode: 'open' });

  return [
    hostTrue.contentEditable,
    trueRoot.contentEditable === undefined,
    hostFalse.contentEditable,
    falseRoot.contentEditable === undefined,
    inheritedHost.contentEditable,
    inheritedRoot.contentEditable === undefined
  ].join('|');
})()
"#,
        )
        .expect("detached child contentEditable should evaluate");

    assert_eq!(result, "true|true|false|true|inherit|true");
}

#[test]
fn html_element_content_editable_and_is_content_editable_reflect_state() {
    let mut vm = new_storage_test_vm("https://contenteditable-reflection.test/");

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
    <div id="host" contenteditable="true"><span id="child"></span></div>
    <div id="plain"></div>
    <div id="off" contenteditable="false"><span id="off-child"></span></div>
    <div id="text" contenteditable="plaintext-only"></div>
    <div id="invalid" contenteditable="wat"></div>
  `;
  const host = document.getElementById('host');
  const child = document.getElementById('child');
  const plain = document.getElementById('plain');
  const off = document.getElementById('off');
  const offChild = document.getElementById('off-child');
  const text = document.getElementById('text');
  const invalid = document.getElementById('invalid');
  plain.contentEditable = 'true';
  return [
    host.contentEditable,
    host.isContentEditable,
    child.contentEditable,
    child.isContentEditable,
    plain.getAttribute('contenteditable'),
    plain.isContentEditable,
    off.contentEditable,
    off.isContentEditable,
    offChild.isContentEditable,
    text.contentEditable,
    text.isContentEditable,
    invalid.contentEditable,
    invalid.isContentEditable
  ].join('|');
})()
"#,
        )
        .expect("contenteditable reflection should evaluate");

    assert_eq!(
        result,
        "true|true|inherit|true|true|true|false|false|false|plaintext-only|true|inherit|false"
    );
}

#[test]
fn document_design_mode_controls_content_editable_without_crossing_shadow_roots() {
    let mut vm = new_storage_test_vm("https://document-design-mode.test/");

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
  const inherited = document.createElement('div');
  const disabled = document.createElement('div');
  disabled.contentEditable = 'false';
  const root = inherited.attachShadow({ mode: 'open' });
  const inside = document.createElement('span');
  root.appendChild(inside);
  document.body.append(inherited, disabled);
  const before = [document.designMode, inherited.isContentEditable].join(':');
  document.designMode = 'ON';
  const enabled = [
    document.designMode,
    inherited.isContentEditable,
    disabled.isContentEditable,
    inside.isContentEditable
  ].join(':');
  document.designMode = 'invalid';
  const afterInvalid = document.designMode;
  document.designMode = 'off';
  const disabledAgain = [document.designMode, inherited.isContentEditable].join(':');
  return [before, enabled, afterInvalid, disabledAgain].join('|');
})()
"#,
        )
        .expect("Document designMode contenteditable probe should evaluate");

    assert_eq!(result, "off:false|on:true:false:false|on|off:false");
}

#[test]
fn delegates_focus_host_focuses_shadow_descendant_and_retargets_document_active_element() {
    let mut vm = new_storage_test_vm("https://shadow-delegates-focus-active-element.test/");

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
  const host = document.createElement('div');
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open', delegatesFocus: true });
  const input = document.createElement('input');
  shadow.appendChild(input);

  host.focus();
  const afterHostFocus = [
    document.activeElement === host,
    shadow.activeElement === input
  ].join(',');

  host.setAttribute('tabindex', '0');
  const afterTabIndex = [
    document.activeElement === host,
    shadow.activeElement === input
  ].join(',');

  input.blur();
  const afterBlur = [
    document.activeElement === document.body,
    shadow.activeElement === null
  ].join(',');

  input.focus();
  const afterDirectFocus = [
    document.activeElement === host,
    shadow.activeElement === input
  ].join(',');

  return [afterHostFocus, afterTabIndex, afterBlur, afterDirectFocus].join('|');
})()
"#,
        )
        .expect(
            "delegatesFocus should focus the inner control and retarget document.activeElement",
        );

    assert_eq!(result, "true,true|true,true|true,true|true,true");
}
#[test]
fn delegates_focus_programmatic_focus_preserves_active_and_requires_inner_target() {
    let mut vm = new_storage_test_vm("https://shadow-delegates-programmatic-focus.test/");

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

  const emptyHost = document.createElement('div');
  emptyHost.tabIndex = 0;
  document.body.appendChild(emptyHost);
  const emptyRoot = emptyHost.attachShadow({ mode: 'open', delegatesFocus: true });
  emptyRoot.appendChild(document.createElement('span'));
  emptyHost.focus();
  const emptyState = [
    document.activeElement === document.body,
    emptyRoot.activeElement === null
  ].join(',');

  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = host.attachShadow({ mode: 'open', delegatesFocus: true });
  const first = document.createElement('div');
  first.tabIndex = 0;
  const second = document.createElement('div');
  second.tabIndex = 0;
  root.append(first, second);
  second.focus();
  host.focus();
  const preservedState = [
    document.activeElement === host,
    root.activeElement === second
  ].join(',');

  second.blur();
  host.click();
  const syntheticClickState = [
    document.activeElement === document.body,
    root.activeElement === null
  ].join(',');

  return [emptyState, preservedState, syntheticClickState].join('|');
})()
"#,
        )
        .expect("delegatesFocus programmatic focus should evaluate");

    assert_eq!(result, "true,true|true,true|true,true");
}
#[test]
fn focus_pseudo_matches_shadow_hosts_and_drives_style_rules() {
    let mut vm = new_storage_test_vm("https://shadow-focus-pseudo.test/");

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
  const style = document.createElement('style');
  style.textContent = '#host:focus { display: none; }';
  document.head.appendChild(style);

  const host = document.createElement('div');
  host.id = 'host';
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open', delegatesFocus: true });
  const nestedHost = document.createElement('div');
  const nested = nestedHost.attachShadow({ mode: 'open' });
  const input = document.createElement('input');
  nested.appendChild(input);
  shadow.appendChild(nestedHost);

  input.focus();
  const nestedFocus = [
    input.matches(':focus'),
    nestedHost.matches(':focus'),
    host.matches(':focus'),
    host.matches(':focus-within'),
    getComputedStyle(host).display
  ].join(',');

  const slottedHost = document.createElement('div');
  document.body.appendChild(slottedHost);
  const slottedRoot = slottedHost.attachShadow({ mode: 'open' });
  slottedRoot.appendChild(document.createElement('slot'));
  const slotted = document.createElement('input');
  slottedHost.appendChild(slotted);
  slotted.focus();
  const slottedFocus = [
    slotted.matches(':focus'),
    slottedHost.matches(':focus'),
    slottedHost.matches(':focus-within')
  ].join(',');

  input.blur();
  const afterBlur = [
    input.matches(':focus'),
    nestedHost.matches(':focus'),
    host.matches(':focus'),
    getComputedStyle(host).display
  ].join(',');

  return [nestedFocus, slottedFocus, afterBlur].join('|');
})()
"#,
        )
        .expect("shadow host focus pseudo matching should evaluate");

    assert_eq!(
        result,
        "true,true,true,true,none|true,false,true|false,false,false,block"
    );
}
#[test]
fn focus_pseudo_matches_wpt_shadow_host_matrix() {
    let mut vm = new_storage_test_vm("https://shadow-focus-pseudo-matrix.test/");

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
  const failures = [];
  const modes = [
    ['open', false],
    ['open', true],
    ['closed', false],
    ['closed', true]
  ];
  function check(condition, label) {
    if (!condition) failures.push(label);
  }
  for (const [mode, delegatesFocus] of modes) {
    const modeString = `${mode}:${delegatesFocus}`;
    const defaultFocus = document.body.appendChild(document.createElement('input'));
    defaultFocus.focus();

    let host = document.body.appendChild(document.createElement('div'));
    let shadowRoot = host.attachShadow({ mode, delegatesFocus });
    shadowRoot.innerHTML = '<input>';
    check(document.activeElement === defaultFocus, `default-active:${modeString}`);
    check(shadowRoot.activeElement === null, `default-shadow-active:${modeString}`);
    check(!host.matches(':focus'), `default-host-focus:${modeString}`);
    host.remove();

    host = document.body.appendChild(document.createElement('div'));
    shadowRoot = host.attachShadow({ mode, delegatesFocus });
    shadowRoot.innerHTML = '<input>';
    shadowRoot.firstChild.focus();
    check(document.activeElement === host, `inner-document-active:${modeString}`);
    check(shadowRoot.activeElement === shadowRoot.firstChild, `inner-shadow-active:${modeString}`);
    check(host.matches(':focus'), `inner-host-focus:${modeString}`);
    host.remove();

    host = document.body.appendChild(document.createElement('div'));
    shadowRoot = host.attachShadow({ mode, delegatesFocus });
    shadowRoot.innerHTML = '<slot>';
    host.innerHTML = '<input>';
    host.firstChild.focus();
    check(document.activeElement === host.firstChild, `slot-document-active:${modeString}`);
    check(shadowRoot.activeElement === null, `slot-shadow-active:${modeString}`);
    check(!host.matches(':focus'), `slot-host-focus:${modeString}`);
    host.remove();

    const host1 = document.body.appendChild(document.createElement('div'));
    const shadowRoot1 = host1.attachShadow({ mode, delegatesFocus });
    const host2 = shadowRoot1.appendChild(document.createElement('div'));
    const shadowRoot2 = host2.attachShadow({ mode, delegatesFocus });
    shadowRoot2.innerHTML = '<input>';
    shadowRoot2.firstChild.focus();
    check(document.activeElement === host1, `nested-document-active:${modeString}`);
    check(shadowRoot1.activeElement === host2, `nested-shadow1-active:${modeString}`);
    check(shadowRoot2.activeElement === shadowRoot2.firstChild, `nested-shadow2-active:${modeString}`);
    check(host1.matches(':focus'), `nested-host1-focus:${modeString}`);
    check(host2.matches(':focus'), `nested-host2-focus:${modeString}`);
    host1.remove();

    host = document.body.appendChild(document.createElement('div'));
    shadowRoot = host.attachShadow({ mode, delegatesFocus });
    shadowRoot.innerHTML = '<input>';
    const input = shadowRoot.firstChild;
    const outer = document.body.appendChild(document.createElement('div'));
    check(!host.matches(':focus'), `move-before-focus:${modeString}`);
    input.focus();
    check(host.matches(':focus'), `move-focused:${modeString}`);
    outer.appendChild(input);
    check(!host.matches(':focus'), `move-after:${modeString}`);
    host.remove();
    outer.remove();
    defaultFocus.remove();
  }
  return failures.join('|');
})()
"#,
        )
        .expect("shadow host focus pseudo WPT matrix should evaluate");

    assert_eq!(result, "");
}
#[tokio::test]
async fn focused_display_none_shadow_host_blurs_on_timer_task() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://shadow-focus-display-none-blur.test/", &loader);

    let plain_sync_state = vm
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
  const style = document.createElement('style');
  style.textContent = '#plainHost:focus, #host:focus { display: none; }';
  document.head.appendChild(style);

  const plainHost = document.createElement('div');
  plainHost.id = 'plainHost';
  document.body.appendChild(plainHost);
  plainHost.tabIndex = 0;
  globalThis.__plainDisplayNoneFocus = { host: plainHost, blurCount: 0 };
  plainHost.focus();
  const syncState = [
    getComputedStyle(plainHost).display,
    document.activeElement === plainHost,
    globalThis.__plainDisplayNoneFocus.blurCount
  ].join('|');
  plainHost.addEventListener('blur', () => globalThis.__plainDisplayNoneFocus.blurCount++);
  return syncState;
})()
"#,
        )
        .expect("plain display:none focus setup should evaluate");

    assert!(
        vm.run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("plain display:none focus blur timer should run")
    );

    let plain_async_state = vm
        .eval(
            r#"
[
  getComputedStyle(globalThis.__plainDisplayNoneFocus.host).display,
  document.activeElement === document.body,
  globalThis.__plainDisplayNoneFocus.blurCount
].join('|')
"#,
        )
        .expect("plain display:none focus task result should evaluate");

    let delegated_sync_state = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('div');
  host.id = 'host';
  document.body.appendChild(host);
  const root = host.attachShadow({ mode: 'open', delegatesFocus: true });
  const input = document.createElement('input');
  root.appendChild(input);
  globalThis.__displayNoneFocus = { host, root, input, blurCount: 0 };
  input.focus();
  const syncState = [
    getComputedStyle(host).display,
    document.activeElement === host,
    root.activeElement === input,
    globalThis.__displayNoneFocus.blurCount
  ].join('|');
  input.addEventListener('blur', () => globalThis.__displayNoneFocus.blurCount++);
  return syncState;
})()
"#,
        )
        .expect("delegated display:none focus setup should evaluate");

    assert!(
        vm.run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("delegated display:none focus blur timer should run")
    );

    let delegated_async_state = vm
        .eval(
            r#"
[
  getComputedStyle(globalThis.__displayNoneFocus.host).display,
  document.activeElement === document.body,
  globalThis.__displayNoneFocus.root.activeElement === null,
  globalThis.__displayNoneFocus.blurCount
].join('|')
"#,
        )
        .expect("delegated display:none focus task result should evaluate");

    assert_eq!(plain_sync_state, "none|true|0");
    assert_eq!(plain_async_state, "block|true|1");
    assert_eq!(delegated_sync_state, "none|true|true|0");
    assert_eq!(delegated_async_state, "block|true|true|1");
}
#[tokio::test]
async fn focused_display_none_blur_listener_sees_updated_focus_state() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://shadow-focus-blur-order.test/", &loader);

    let sync_state = vm
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
  const style = document.createElement('style');
  style.textContent = '#host:focus { display: none; }';
  document.head.appendChild(style);

  const host = document.createElement('div');
  host.id = 'host';
  document.body.appendChild(host);
  const root = host.attachShadow({ mode: 'open', delegatesFocus: true });
  const input = document.createElement('input');
  root.appendChild(input);
  globalThis.__focusBlurOrder = { host, root, seen: [] };
  input.addEventListener('blur', () => {
    globalThis.__focusBlurOrder.seen.push([
      getComputedStyle(host).display,
      document.activeElement === document.body,
      root.activeElement === null
    ].join(','));
  });
  input.focus();
  return [
    getComputedStyle(host).display,
    document.activeElement === host,
    root.activeElement === input
  ].join('|');
})()
"#,
        )
        .expect("display none focus setup should evaluate");

    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "the inline setup stylesheet queues its own event body before focus-update work"
    );
    assert!(
        vm.run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("exact display-none focus blur timer should run")
    );

    let blur_state = vm
        .eval(
            r#"
[
  globalThis.__focusBlurOrder.seen.join('|'),
  getComputedStyle(globalThis.__focusBlurOrder.host).display
].join('|')
"#,
        )
        .expect("display none blur state should evaluate");

    assert_eq!(sync_state, "none|true|true");
    assert_eq!(blur_state, "block,true,true|block");
}
#[test]
fn scroll_container_focusability_tracks_current_layout() {
    let mut vm = new_storage_test_vm("https://scroll-focusability.test/");
    vm.force_fresh_layout_reads_for_test();

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
    <button id="button">Button</button>
    <div id="scroller" style="overflow:scroll; width:200px; height:50px;">
      <div style="height:100px"></div>
    </div>
    <div id="horizontal" style="overflow-x:scroll; width:50px; height:50px;">
      <div style="width:100px; height:10px"></div>
    </div>`;
  const button = document.getElementById('button');
  const scroller = document.getElementById('scroller');
  const horizontal = document.getElementById('horizontal');
  scroller.focus();
  const initiallyScrollable = document.activeElement === scroller;
  button.focus();
  horizontal.focus();
  const horizontallyScrollable = document.activeElement === horizontal;
  button.focus();
  scroller.style.height = '200px';
  scroller.focus();
  return [initiallyScrollable, horizontallyScrollable, document.activeElement === button].join('|');
})()
"#,
        )
        .expect("scroll container focusability should evaluate");

    assert_eq!(result, "true|true|true");
}
#[test]
fn tab_key_keeps_focused_scroller_until_focus_leaves_interactive_child() {
    let mut vm = new_storage_test_vm("https://scroll-tab-focusability.test/");

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
    <button id="start">START</button>
    <div id="scroller" style="overflow:scroll; width:50px; height:50px;">
      <div style="height:100px"></div>
      <button id="submit" disabled>submit</button>
    </div>
    <button id="end">END</button>`;
  function pressTab(shiftKey = false) {
    __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, shiftKey);
  }
  const start = document.getElementById('start');
  const submit = document.getElementById('submit');
  start.focus();
  pressTab();
  const first = document.activeElement.id;
  submit.disabled = false;
  pressTab();
  const second = document.activeElement.id;
  pressTab();
  const third = document.activeElement.id;
  pressTab(true);
  const fourth = document.activeElement.id;
  pressTab(true);
  const fifth = document.activeElement.id;
  return [first, second, third, fourth, fifth].join('|');
})()
"#,
        )
        .expect("Tab default action should drop scroller after focus leaves it");

    assert_eq!(result, "scroller|submit|end|submit|start");
}

#[test]
fn caret_position_get_client_rect_declared_prototype_method() {
    let mut vm = new_storage_test_vm("https://caret-position-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptor = Object.getOwnPropertyDescriptor(CaretPosition.prototype, "getClientRect");
  const value = descriptor && descriptor.value;
  const rect = value.call({});
  return [
    typeof value,
    value && value.name,
    value && value.length,
    descriptor && descriptor.enumerable,
    descriptor && descriptor.configurable,
    descriptor && descriptor.writable,
    /\[native code\]/.test(String(value)),
    rect instanceof DOMRect,
    Number.isNaN(rect.x),
    Number.isNaN(rect.y),
    Number.isNaN(rect.width),
    Number.isNaN(rect.height)
  ].join("|");
})()
"#,
        )
        .expect("CaretPosition prototype method shape should evaluate");

    assert_eq!(
        result,
        "function|getClientRect|0|true|true|true|true|true|true|true|true|true"
    );
}

#[test]
fn caret_position_backing_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://caret-position-private-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = '<span id="target">hello</span>';
  const span = document.getElementById('target');
  const rect = span.getBoundingClientRect();
  const charWidth = rect.width / span.textContent.length;
  const pos = document.caretPositionFromPoint(
    rect.left + charWidth * 2,
    rect.top + rect.height / 2
  );
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith('__moliCaretPosition'))
    .sort()
    .join(',');
  const initialOwnNames = internalNames(pos);
  const beforeRect = pos.getClientRect();
  const slotNames = [
    '__moliCaretPositionOffsetNode',
    '__moliCaretPositionOffset',
    '__moliCaretPositionRectX',
    '__moliCaretPositionRectY',
    '__moliCaretPositionRectWidth',
    '__moliCaretPositionRectHeight'
  ];
  for (const name of slotNames) {
    CaretPosition.prototype[name] = name.endsWith('OffsetNode') ? body : 999;
    pos[name] = name.endsWith('OffsetNode') ? body : 999;
  }
  const afterRect = pos.getClientRect();
  const same = (actual, expected) => Math.abs(actual - expected) < 0.000001;
  return JSON.stringify({
    initialOwnNames,
    spoofedOwnNames: internalNames(pos),
    offsetNodeStable: pos.offsetNode === span.firstChild,
    offsetStable: pos.offset === 2,
    rectStable: [
      same(beforeRect.x, afterRect.x),
      same(beforeRect.y, afterRect.y),
      same(beforeRect.width, afterRect.width),
      same(beforeRect.height, afterRect.height)
    ].join(',')
  });
})()
"#,
        )
        .expect("CaretPosition private backing slots should ignore public spoofing");

    assert_eq!(
        result,
        r#"{"initialOwnNames":"","spoofedOwnNames":"__moliCaretPositionOffset,__moliCaretPositionOffsetNode,__moliCaretPositionRectHeight,__moliCaretPositionRectWidth,__moliCaretPositionRectX,__moliCaretPositionRectY","offsetNodeStable":true,"offsetStable":true,"rectStable":"true,true,true,true"}"#
    );
}

#[test]
fn document_caret_position_from_point_handles_shadow_root_allowlist() {
    let mut vm = new_storage_test_vm("https://shadow-caret-position.test/");
    vm.force_fresh_layout_reads_for_test();

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
  if (!document.head) {
    document.documentElement.insertBefore(document.createElement('head'), document.body);
  }
  document.head.appendChild(document.createElement('style')).textContent =
    'textarea { font: 20px/1 Ahem; border: none; padding: 0; }';
  document.body.innerHTML = '<div id="container"></div>';
  const container = document.getElementById('container');
  const out = [];
  function labelNode(node) {
    if (!node) return 'null';
    return `${node.nodeName || '?'}#${node.id || ''}:${node.nodeType}`;
  }
  function check(name, condition, detail = '') {
    out.push(`${name}:${condition ? 'pass' : 'fail'}${condition || !detail ? '' : ':' + detail}`);
  }
  check('bad-options',
    (() => {
      try { document.caretPositionFromPoint(5, 5, 'foo'); return false; }
      catch (e) { return e instanceof TypeError; }
    })());

  container.setHTMLUnsafe('<span>hello, world</span>');
  {
    const span = container.firstChild;
    const rect = span.getBoundingClientRect();
    const pos = document.caretPositionFromPoint(rect.left + rect.width / span.textContent.length * 2, rect.top + rect.height / 2, {});
    const posRect = pos.getClientRect();
    check('light-text', pos instanceof CaretPosition && pos.offsetNode === span.firstChild && pos.offset === 2);
    check('light-text-rect', posRect.width === 0, `${posRect.width}`);
  }

  container.setHTMLUnsafe('<div id="textDiv" style="display:inline-block">aaa</div>');
  {
    const textDiv = document.getElementById('textDiv');
    const rect = textDiv.getBoundingClientRect();
    const characterWidth = rect.width / textDiv.textContent.length;
    const pos = document.caretPositionFromPoint(rect.left + characterWidth * 2, rect.top + rect.height / 2);
    const posRect = pos.getClientRect();
    const range = new Range();
    range.setStart(textDiv.firstChild, 2);
    const rangeRect = range.getBoundingClientRect();
    check('inline-block-text-rect', pos.offsetNode === textDiv.firstChild && posRect.width === 0 && rangeRect.x === posRect.x, `${labelNode(pos.offsetNode)},${posRect.width},${rangeRect.x},${posRect.x}`);
  }

  document.head.appendChild(document.createElement('style')).textContent = '#textDivFromSheet { display: inline-block; }';
  container.setHTMLUnsafe('<div id="textDivFromSheet">aaa</div>');
  {
    const textDiv = document.getElementById('textDivFromSheet');
    const rect = textDiv.getBoundingClientRect();
    const characterWidth = rect.width / textDiv.textContent.length;
    const pos = document.caretPositionFromPoint(rect.left + characterWidth * 2, rect.top + rect.height / 2);
    const posRect = pos.getClientRect();
    check('sheet-inline-block-text-rect', pos.offsetNode === textDiv.firstChild && posRect.width === 0, `${labelNode(pos.offsetNode)},${posRect.width},${rect.width}`);
  }

  container.setHTMLUnsafe('<script>const ignored = true;</script><svg width="100" height="100"><circle cx="50" cy="50" r="50"></circle></svg>');
  {
    const circle = container.querySelector('circle');
    const rect = circle.getBoundingClientRect();
    const pos = document.caretPositionFromPoint(50, 50);
    check('svg-circle', pos instanceof CaretPosition && pos.offsetNode === circle && pos.offset === 0, `${labelNode(pos && pos.offsetNode)},${circle.namespaceURI},${rect.left},${rect.top},${rect.width},${rect.height}`);
  }

  container.setHTMLUnsafe('<div id="textDivAfterSvg">aaa</div>');
  {
    const textDiv = document.getElementById('textDivAfterSvg');
    const rect = textDiv.getBoundingClientRect();
    const characterWidth = rect.width / textDiv.textContent.length;
    const pos = document.caretPositionFromPoint(rect.left + characterWidth * 2, rect.top + rect.height / 2);
    const posRect = pos.getClientRect();
    check('post-svg-text-rect', pos.offsetNode === textDiv.firstChild && posRect.width === 0, `${labelNode(pos.offsetNode)},${posRect.width},${rect.width}`);
  }

  container.setHTMLUnsafe("<input value='text inside input' />");
  {
    const input = container.firstChild;
    const rect = input.getBoundingClientRect();
    const pos = document.caretPositionFromPoint(rect.left + 1, rect.top + rect.height / 2);
    check('input', pos.offsetNode === input && pos.offset === 0);
  }

  container.setHTMLUnsafe('<textarea rows="3" cols="4">12345678\n901234567890</textarea>');
  {
    const textarea = container.firstChild;
    const rect = textarea.getBoundingClientRect();
    const fontMatch = getComputedStyle(textarea).fontSize.match(/\d+/);
    const fontSize = fontMatch ? parseInt(fontMatch[0]) : rect.width / 4;
    const pos = document.caretPositionFromPoint(rect.left + 1, rect.top + rect.height * 0.5);
    const pos2 = document.caretPositionFromPoint(rect.left + fontSize + 2, rect.bottom - fontSize / 2);
    check('textarea-wrap', pos.offsetNode === textarea && pos.offset === 5);
    check('textarea-forced', pos2.offsetNode === textarea && pos2.offset === 11 && pos2.getClientRect().left === rect.left + fontSize, `${pos2.offset},${pos2.getClientRect().left},${rect.left + fontSize},${rect.width},${rect.height}`);
  }

  container.setHTMLUnsafe('a<div id="host"></div>b');
  {
    const host = document.getElementById('host');
    const root = host.attachShadow({ mode: 'closed' });
    root.setHTMLUnsafe('<span>hello, world</span>');
    const span = root.firstChild;
    const rect = span.getBoundingClientRect();
    const pos = document.caretPositionFromPoint(rect.left + rect.width / span.textContent.length * 2, rect.top + rect.height / 2, { shadowRoots: [root] });
    check('closed-allowed', pos.offsetNode === span.firstChild && pos.offset === 2);
  }

  container.setHTMLUnsafe(`a<div id="host"><template shadowrootmode=open><span>hello, world</span></template></div>b`);
  {
    const root = document.getElementById('host').shadowRoot;
    const span = root.querySelector('span');
    const rect = span.getBoundingClientRect();
    const pos = document.caretPositionFromPoint(rect.left + rect.width / span.textContent.length * 2, rect.top + rect.height / 2);
    const posAllowed = document.caretPositionFromPoint(rect.left + rect.width / span.textContent.length * 2, rect.top + rect.height / 2, { shadowRoots: [root] });
    const hiddenRect = pos.getClientRect();
    const hostRect = document.getElementById('host').getBoundingClientRect();
    check('shadow-hidden', pos.offsetNode === container && pos.offset === 1);
    check('shadow-hidden-rect', hiddenRect.left === hostRect.left && hiddenRect.height === hostRect.height, `${hiddenRect.left},${hiddenRect.height},${hostRect.left},${hostRect.height}`);
    check('shadow-allowed', posAllowed.offsetNode === span.firstChild && posAllowed.offset === 2);
  }

  container.setHTMLUnsafe(`a<div id="outerHost"><template shadowrootmode=open><div id="innerHost"><template shadowrootmode=open><span>some text</span></template></div><div>world</div></template></div>b`);
  {
    const outerRoot = document.getElementById('outerHost').shadowRoot;
    const innerRoot = outerRoot.getElementById('innerHost').shadowRoot;
    const span = innerRoot.querySelector('span');
    const rect = span.getBoundingClientRect();
    const x = rect.left + rect.width / span.textContent.length * 2;
    const y = rect.top + rect.height / 2;
    const none = document.caretPositionFromPoint(x, y);
    const inner = document.caretPositionFromPoint(x, y, { shadowRoots: [innerRoot] });
    const outer = document.caretPositionFromPoint(x, y, { shadowRoots: [outerRoot] });
    const both = document.caretPositionFromPoint(x, y, { shadowRoots: [innerRoot, outerRoot] });
    check('nested-hidden', none.offsetNode === container && none.offset === 1);
    check('nested-inner', inner.offsetNode === span.firstChild && inner.offset === 2, `${labelNode(inner.offsetNode)},${inner.offset}`);
    check('nested-outer', outer.offsetNode === outerRoot && (outer.offset === 0 || outer.offset === 1), `${labelNode(outer.offsetNode)},${outer.offset}`);
    check('nested-both', both.offsetNode === span.firstChild && both.offset === 2, `${labelNode(both.offsetNode)},${both.offset}`);
  }
  return out.join('|');
})()
"#,
        )
        .expect("caret position shadow allowlist checks should evaluate");

    assert_eq!(
        result,
        "bad-options:pass|light-text:pass|light-text-rect:pass|inline-block-text-rect:pass|sheet-inline-block-text-rect:pass|svg-circle:fail:svg#:1,http://www.w3.org/2000/svg,0,0,0,0|post-svg-text-rect:pass|input:pass|textarea-wrap:fail|textarea-forced:fail:10,23.75,28,63,60|closed-allowed:pass|shadow-hidden:pass|shadow-hidden-rect:pass|shadow-allowed:pass|nested-hidden:pass|nested-inner:pass|nested-outer:pass|nested-both:pass"
    );
}
#[test]
fn document_caret_position_from_point_retargets_media_controls_to_parent() {
    let mut vm = new_storage_test_vm("https://caret-position-media.test/");
    vm.force_fresh_layout_reads_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = '<div id="container"></div>';
  const container = document.getElementById('container');
  const out = [];
  for (const tag of ['audio', 'video']) {
    container.innerHTML = `<${tag} controls></${tag}>`;
    const media = container.firstElementChild;
    const rect = media.getBoundingClientRect();
    const pos = document.caretPositionFromPoint(
      rect.left + rect.width / 2,
      rect.top + rect.height / 2
    );
    out.push(`${tag}:${pos.offsetNode === container}:${pos.offset}`);
  }
  return out.join('|');
})()
"#,
        )
        .expect("media caret position should evaluate");

    assert_eq!(result, "audio:true:0|video:true:0");
}
#[test]
fn child_document_caret_position_uses_srcdoc_geometry() {
    let mut vm = new_storage_test_vm("https://child-caret-position.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = '<iframe id="frame"></iframe>';
  const frame = document.getElementById('frame');
  frame.srcdoc = '<!doctype html><meta charset="utf-8"><h1>title</h1><p>paragraph</p>';
})()
"#,
    )
    .expect("child caret position setup should evaluate");

    assert!(
        vm.run_next_child_navigation_commit_body_for_test()
            .expect("typed child navigation-commit body should succeed")
            .is_some(),
        "the srcdoc Document must commit on its own owner turn"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById('frame');
  const frameDoc = frame.contentDocument;
  const h1 = frameDoc.querySelector('h1');
  const rect = h1.getBoundingClientRect();
  const pos = frameDoc.caretPositionFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
  return [
    rect.width > 0,
    rect.height > 0,
    h1.isConnected,
    pos instanceof frame.contentWindow.CaretPosition,
    !(pos instanceof CaretPosition),
    pos && pos.offsetNode === h1.firstChild,
    pos && pos.offset,
    frameDoc.nodeType,
    frameDoc.documentElement && frameDoc.documentElement.isConnected
  ].join('|');
})()
"#,
        )
        .expect("child caret position should evaluate");

    assert_eq!(result, "true|true|true|true|true|true|5|9|true");
}
#[test]
fn accesskey_default_action_finds_shadow_tree_buttons() {
    let mut vm = new_storage_test_vm("https://shadow-accesskey.test/");

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

  const openHost = document.createElement('div');
  document.body.appendChild(openHost);
  const openRoot = openHost.attachShadow({ mode: 'open' });
  openRoot.innerHTML = '<button accesskey="x g">open</button>';
  let openClicks = 0;
  openRoot.querySelector('button').addEventListener('click', () => ++openClicks);

  __moliDispatchTrustedKey('keydown', 'g', 'KeyG', true, false, false, true);
  openHost.remove();

  const closedHost = document.createElement('div');
  document.body.appendChild(closedHost);
  const closedRoot = closedHost.attachShadow({ mode: 'closed' });
  closedRoot.innerHTML = '<button accesskey="x g">closed</button>';
  let closedClicks = 0;
  closedRoot.querySelector('button').addEventListener('click', () => ++closedClicks);

  __moliDispatchTrustedKey('keydown', 'g', 'KeyG', true, false, false, true);

  return `${openClicks}|${closedClicks}`;
})()
"#,
        )
        .expect("accesskey default action should reach shadow tree buttons");

    assert_eq!(result, "1|1");
}
#[test]
fn untrusted_keydown_does_not_trigger_tab_or_accesskey_default_actions() {
    let mut vm = new_storage_test_vm("https://untrusted-key-default-action.test/");

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
    <button id="start">start</button>
    <button id="next">next</button>
    <button id="access" accesskey="x">access</button>
  `;
  let clicks = 0;
  document.getElementById('access').addEventListener('click', () => ++clicks);
  document.getElementById('start').focus();
  (document.activeElement || document.body).dispatchEvent(new KeyboardEvent('keydown', {
    key: 'Tab',
    bubbles: true,
    cancelable: true,
    composed: true
  }));
  const afterTab = document.activeElement && document.activeElement.id;
  document.body.dispatchEvent(new KeyboardEvent('keydown', {
    key: 'x',
    altKey: true,
    shiftKey: true,
    bubbles: true,
    cancelable: true,
    composed: true
  }));
  return `${afterTab}|${clicks}`;
})()
"#,
        )
        .expect("untrusted keydown should not run trusted input default actions");

    assert_eq!(result, "start|0");
}
#[test]
fn shadow_root_init_state_reflects_attach_shadow_options() {
    let mut vm = new_storage_test_vm("https://shadow-root-init-state.test/");

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
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = host.attachShadow({
    mode: 'open',
    delegatesFocus: true,
    slotAssignment: 'manual',
    clonable: true,
    serializable: true,
    referenceTarget: 'targetID'
  });

  const defaultHost = document.createElement('section');
  document.body.appendChild(defaultHost);
  const defaultRoot = defaultHost.attachShadow({ mode: 'open' });
  const nullReferenceHost = document.createElement('nav');
  document.body.appendChild(nullReferenceHost);
  const nullReferenceRoot = nullReferenceHost.attachShadow({
    mode: 'open',
    referenceTarget: null
  });
  const emptyReferenceHost = document.createElement('footer');
  document.body.appendChild(emptyReferenceHost);
  const emptyReferenceRoot = emptyReferenceHost.attachShadow({
    mode: 'open',
    referenceTarget: ''
  });
  const numericReferenceHost = document.createElement('main');
  document.body.appendChild(numericReferenceHost);
  const numericReferenceRoot = numericReferenceHost.attachShadow({
    mode: 'open',
    referenceTarget: 42
  });
  root.referenceTarget = null;
  const afterNullSetter = root.referenceTarget;
  root.referenceTarget = true;
  const afterBooleanSetter = root.referenceTarget;

  let invalidSlotAssignment = 'no-throw';
  try {
    document.createElement('article').attachShadow({
      mode: 'open',
      slotAssignment: 'invalid'
    });
  } catch (error) {
    invalidSlotAssignment = error.name;
  }
  let nullSlotAssignment = 'no-throw';
  try {
    document.createElement('aside').attachShadow({
      mode: 'open',
      slotAssignment: null
    });
  } catch (error) {
    nullSlotAssignment = error.name;
  }

  return [
    root.mode,
    root.delegatesFocus,
    root.slotAssignment,
    root.clonable,
    root.serializable,
    afterNullSetter === null,
    afterBooleanSetter,
    defaultRoot.delegatesFocus,
    defaultRoot.slotAssignment,
    defaultRoot.clonable,
    defaultRoot.serializable,
    defaultRoot.referenceTarget === null,
    nullReferenceRoot.referenceTarget === null,
    emptyReferenceRoot.referenceTarget,
    numericReferenceRoot.referenceTarget,
    invalidSlotAssignment,
    nullSlotAssignment
  ].join('|');
})()
"#,
        )
        .expect("ShadowRoot init state should reflect attachShadow options");

    assert_eq!(
        result,
        "open|true|manual|true|true|true|true|false|named|false|false|true|true||42|TypeError|TypeError"
    );
}
#[test]
fn clonable_shadow_roots_are_copied_by_host_clone_node() {
    let mut vm = new_storage_test_vm("https://shadow-root-clonable.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const rootCloneError = (() => {
    try {
      document.createElement('div')
        .attachShadow({ mode: 'open' })
        .cloneNode(true);
      return 'no-throw';
    } catch (error) {
      return error && error.name;
    }
  })();

  const host = document.createElement('div');
  const root = host.attachShadow({ mode: 'open', clonable: true });
  root.innerHTML = '<input><div><span></span></div>';

  const deep = host.cloneNode(true);
  const shallow = host.cloneNode(false);

  const plainHost = document.createElement('section');
  plainHost.attachShadow({ mode: 'open' }).innerHTML = '<input>';
  const plainClone = plainHost.cloneNode(true);

  const closedHost = document.createElement('article');
  closedHost.attachShadow({ mode: 'closed', clonable: true }).innerHTML = '<span></span>';
  const closedClone = closedHost.cloneNode(true);
  let closedCloneDuplicate = 'no-throw';
  try {
    closedClone.attachShadow({ mode: 'closed' });
  } catch (error) {
    closedCloneDuplicate = error && error.name;
  }

  const foreignElement = (localName) => ({
    nodeType: 1,
    localName,
    namespaceURI: 'http://www.w3.org/1999/xhtml',
    prefix: null,
    childNodes: [],
    getAttributeNames() { return []; },
    getAttribute() { return null; }
  });
  const implicitForeignShadowHost = foreignElement('div');
  implicitForeignShadowHost.shadowRoot = {
    mode: 'open',
    childNodes: [foreignElement('span')]
  };
  const implicitForeignShadowClone = document.importNode(implicitForeignShadowHost, true);
  const clonableForeignShadowHost = foreignElement('section');
  clonableForeignShadowHost.shadowRoot = {
    mode: 'open',
    clonable: true,
    childNodes: [foreignElement('strong')]
  };
  const clonableForeignShadowClone = document.importNode(clonableForeignShadowHost, true);

  const template = document.createElement('template');
  const svgNs = 'http://www.w3.org/2000/svg';
  const attrNs = 'urn:clone-attr';
  const group = document.createElementNS(svgNs, 'svg:g');
  group.setAttributeNS(attrNs, 'a:flag', 'yes');
  const input = document.createElement('input');
  input.type = 'checkbox';
  input.defaultValue = 'seed';
  input.value = 'dirty';
  input.defaultChecked = true;
  input.checked = false;
  const video = document.createElement('video');
  video.volume = 0.25;
  video.muted = true;
  video.playbackRate = 1.5;
  video.currentTime = 3;
  template.content.append(group, input, video);
  const templateClone = template.cloneNode(true);
  const clonedGroup = templateClone.content.firstElementChild;
  const clonedInput = clonedGroup.nextElementSibling;
  const clonedVideo = clonedInput.nextElementSibling;

  return [
    rootCloneError,
    root.clonable,
    deep.shadowRoot.clonable,
    deep.shadowRoot.children.length,
    deep.shadowRoot.children[0].localName,
    deep.shadowRoot.children[1].firstElementChild.localName,
    shallow.shadowRoot.clonable,
    shallow.shadowRoot.children.length,
    shallow.childNodes.length,
    plainClone.shadowRoot === null,
    closedClone.shadowRoot === null,
    closedCloneDuplicate,
    implicitForeignShadowClone.shadowRoot === null,
    clonableForeignShadowClone.shadowRoot instanceof ShadowRoot,
    clonableForeignShadowClone.shadowRoot.firstElementChild.localName,
    templateClone.content.childNodes.length,
    clonedGroup.namespaceURI,
    clonedGroup.localName,
    clonedGroup.getAttributeNS(attrNs, 'flag'),
    clonedInput.value,
    clonedInput.checked,
    clonedInput.defaultChecked,
    clonedVideo.volume,
    clonedVideo.muted,
    clonedVideo.playbackRate,
    clonedVideo.currentTime
  ].join('|');
})()
"#,
        )
        .expect("clonable shadow roots should clone with their hosts");

    assert_eq!(
        result,
        "NotSupportedError|true|true|2|input|span|true|2|0|true|true|NotSupportedError|true|true|strong|3|http://www.w3.org/2000/svg|g|yes|dirty|false|true|0.25|true|1.5|3"
    );
}
#[test]
fn set_html_unsafe_preserves_declarative_shadow_roots() {
    let mut vm = new_storage_test_vm("https://set-html-unsafe-shadow.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const wrapper = document.createElement('div');
  wrapper.setHTMLUnsafe(
    '<div id="host"><template shadowrootmode="open" shadowrootclonable>' +
    '<input><div><span></span></div></template><em>light</em></div>'
  );
  const host = wrapper.firstElementChild;
  const root = host.shadowRoot;
  const clone = host.cloneNode(true);

  const plain = document.createElement('section');
  plain.setHTMLUnsafe(
    '<div id="plain"><template shadowrootmode="open"><input></template></div>'
  );
  const plainClone = plain.firstElementChild.cloneNode(true);

  const shadowHost = document.createElement('article');
  const shadow = shadowHost.attachShadow({ mode: 'open' });
  shadow.setHTMLUnsafe(
    '<section id="nested"><template shadowrootmode="open" shadowrootclonable>' +
    '<slot></slot></template></section>'
  );
  const nested = shadow.getElementById('nested');

  const ordinary = document.createElement('div');
  ordinary.innerHTML =
    '<div id="ordinary"><template shadowrootmode="open"><span></span></template></div>';
  const ordinaryHost = ordinary.firstElementChild;

  return [
    typeof Element.prototype.setHTMLUnsafe,
    Element.prototype.setHTMLUnsafe.length,
    root instanceof ShadowRoot,
    root.clonable,
    root.children.length,
    root.children[0].localName,
    root.children[1].firstElementChild.localName,
    host.children.length,
    host.children[0].localName,
    clone.shadowRoot instanceof ShadowRoot,
    clone.shadowRoot.children.length,
    plain.firstElementChild.shadowRoot instanceof ShadowRoot,
    plain.firstElementChild.shadowRoot.clonable,
    plainClone.shadowRoot === null,
    nested.shadowRoot instanceof ShadowRoot,
    nested.shadowRoot.clonable,
    nested.shadowRoot.firstElementChild.localName,
    ordinaryHost.shadowRoot === null
  ].join('|');
})()
"#,
        )
        .expect("setHTMLUnsafe should preserve declarative shadow roots");

    assert_eq!(
        result,
        "function|1|true|true|2|input|span|1|em|true|2|true|false|true|true|true|slot|true"
    );
}
#[test]
fn get_html_serializes_shadow_roots_when_requested() {
    let mut vm = new_storage_test_vm("https://shadow-get-html.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const wrapper = document.createElement('section');
  const host = document.createElement('div');
  host.innerHTML = '<span>light &amp;</span>';
  wrapper.appendChild(host);
  const root = host.attachShadow({
    mode: 'open',
    delegatesFocus: true,
    serializable: true,
    slotAssignment: 'manual',
    clonable: true,
    referenceTarget: 'target&'
  });
  root.setHTMLUnsafe('<slot></slot><b data-x="&"></b>');

  const explicitHost = document.createElement('article');
  const explicitRoot = explicitHost.attachShadow({ mode: 'closed' });
  explicitRoot.setHTMLUnsafe('<em>closed</em>');

  const nestedHost = document.createElement('div');
  const nestedOuter = nestedHost.attachShadow({ mode: 'open' });
  nestedOuter.setHTMLUnsafe('<section id="inner-host"></section>');
  const innerHost = nestedOuter.querySelector('#inner-host');
  innerHost.attachShadow({ mode: 'open', serializable: true }).innerHTML = '<i>nested</i>';

  const rawWrapper = document.createElement('div');
  const title = document.createElement('title');
  title.textContent = '<title&>';
  const textarea = document.createElement('textarea');
  textarea.textContent = '<textarea&>';
  const noscript = document.createElement('noscript');
  noscript.textContent = '<p>noscript&</p>';
  rawWrapper.append(title, textarea, noscript);

  return [
    typeof Element.prototype.getHTML,
    Element.prototype.getHTML.length,
    typeof ShadowRoot.prototype.getHTML,
    ShadowRoot.prototype.getHTML.length,
    wrapper.getHTML() === wrapper.innerHTML,
    host.getHTML({ serializableShadowRoots: true }),
    root.getHTML({ serializableShadowRoots: true }),
    explicitHost.getHTML({ serializableShadowRoots: true }),
    explicitHost.getHTML({ shadowRoots: [explicitRoot] }),
    nestedHost.getHTML({ shadowRoots: [nestedOuter] }),
    nestedHost.getHTML({ serializableShadowRoots: true, shadowRoots: [nestedOuter] }),
    rawWrapper.getHTML() === rawWrapper.innerHTML,
    rawWrapper.getHTML()
  ].join('|');
})()
"#,
        )
        .expect("getHTML should serialize requested shadow roots");

    assert_eq!(
        result,
        concat!(
            "function|0|function|0|true|",
            "<template shadowrootmode=\"open\" shadowrootdelegatesfocus=\"\" ",
            "shadowrootserializable=\"\" shadowrootslotassignment=\"manual\" ",
            "shadowrootclonable=\"\" shadowrootreferencetarget=\"target&amp;\">",
            "<slot></slot><b data-x=\"&amp;\"></b></template><span>light &amp;</span>|",
            "<slot></slot><b data-x=\"&amp;\"></b>|",
            "|",
            "<template shadowrootmode=\"closed\"><em>closed</em></template>|",
            "<template shadowrootmode=\"open\"><section id=\"inner-host\"></section></template>|",
            "<template shadowrootmode=\"open\"><section id=\"inner-host\">",
            "<template shadowrootmode=\"open\" shadowrootserializable=\"\"><i>nested</i>",
            "</template></section></template>|",
            "true|<title>&lt;title&amp;&gt;</title><textarea>&lt;textarea&amp;&gt;</textarea>",
            "<noscript><p>noscript&</p></noscript>"
        )
    );
}
#[test]
fn get_html_round_trips_declarative_shadow_adopted_stylesheet_attribute() {
    let mut vm = new_storage_test_vm("https://shadow-get-html-adopted-stylesheets.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const wrapper = document.createElement('section');
  wrapper.setHTMLUnsafe(
    '<div id="basic"><template shadowrootmode="open" shadowrootserializable ' +
      'shadowrootadoptedstylesheets="foo"><span>basic</span></template></div>' +
    '<div id="space"><template shadowrootmode="open" shadowrootserializable ' +
      'shadowrootadoptedstylesheets="  foo   bar  "><span>space</span></template></div>' +
    '<div id="empty"><template shadowrootmode="open" shadowrootserializable ' +
      'shadowrootadoptedstylesheets=""><span>empty</span></template></div>' +
    '<div id="absent"><template shadowrootmode="open" shadowrootserializable>' +
      '<span>absent</span></template></div>'
  );

  const opts = { serializableShadowRoots: true };
  const basic = wrapper.querySelector('#basic');
  const empty = wrapper.querySelector('#empty');
  const constructed = new CSSStyleSheet();
  constructed.replaceSync('span { font-weight: bold; }');
  const original = Array.from(basic.shadowRoot.adoptedStyleSheets);
  basic.shadowRoot.adoptedStyleSheets = [...original, constructed];

  const basicHtml = basic.getHTML(opts);
  const emptyHtmlBefore = empty.getHTML(opts);
  empty.shadowRoot.adoptedStyleSheets = [constructed];
  const emptyHtmlAfter = empty.getHTML(opts);

  return [
    basicHtml,
    wrapper.querySelector('#space').getHTML(opts),
    emptyHtmlBefore,
    emptyHtmlAfter,
    wrapper.querySelector('#absent').getHTML(opts),
    basic.shadowRoot.getHTML().includes('shadowrootadoptedstylesheets'),
    basicHtml.includes('font-weight')
  ].join('|');
})()
"#,
        )
        .expect("getHTML should serialize authored shadowrootadoptedstylesheets");

    assert_eq!(
        result,
        concat!(
            "<template shadowrootmode=\"open\" shadowrootserializable=\"\" ",
            "shadowrootadoptedstylesheets=\"foo\"><span>basic</span></template>|",
            "<template shadowrootmode=\"open\" shadowrootserializable=\"\" ",
            "shadowrootadoptedstylesheets=\"  foo   bar  \"><span>space</span></template>|",
            "<template shadowrootmode=\"open\" shadowrootserializable=\"\" ",
            "shadowrootadoptedstylesheets=\"\"><span>empty</span></template>|",
            "<template shadowrootmode=\"open\" shadowrootserializable=\"\" ",
            "shadowrootadoptedstylesheets=\"\"><span>empty</span></template>|",
            "<template shadowrootmode=\"open\" shadowrootserializable=\"\"><span>absent</span></template>|",
            "false|false"
        )
    );
}
#[test]
fn declarative_shadow_adopted_stylesheets_resolve_data_css_specifiers() {
    let mut vm = new_storage_test_vm("https://shadow-adopted-css.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const cssA = 'data:text/css,span%20%7B%20color%3A%20blue%3B%20%7D';
  const cssB = 'data:text/css,span%20%7B%20text-decoration%3A%20underline%3B%20%7D';
	  const wrapper = document.createElement('section');
	  wrapper.setHTMLUnsafe(
	    `<div id="host"><template shadowrootmode="open" shadowrootclonable ` +
	      `shadowrootadoptedstylesheets="${cssA} ${cssB}">` +
	      `<span id="t">text</span></template></div>` +
	    `<div id="host2"><template shadowrootmode="open" ` +
	      `shadowrootadoptedstylesheets="${cssA}">` +
	      `<span id="t">text</span></template></div>`
	  );
	  (document.body || document.documentElement || document.appendChild(document.createElement('html'))).append(wrapper);
	  const host = wrapper.querySelector('#host');
	  const host2 = wrapper.querySelector('#host2');
	  const clone = host.cloneNode(true);
	  wrapper.append(clone);
  const sheets = host.shadowRoot.adoptedStyleSheets;
  const cloneSheets = clone.shadowRoot.adoptedStyleSheets;
  const cacheDescriptor = Object.getOwnPropertyDescriptor(
    globalThis,
    '__moliCssModuleSheetCache'
  );
  const cacheDeleteResult = delete globalThis.__moliCssModuleSheetCache;
  let cacheRedefineResult = 'no-throw';
  try {
    Object.defineProperty(globalThis, '__moliCssModuleSheetCache', { value: null });
  } catch (error) {
    cacheRedefineResult = error && error.name;
  }
  const postTamper = document.createElement('div');
  postTamper.setHTMLUnsafe(
    `<div id="host3"><template shadowrootmode="open" ` +
      `shadowrootadoptedstylesheets="${cssA}">` +
      `<span id="t">text</span></template></div>`
  );
  return [
    sheets.length,
    sheets[0].cssRules.length,
    sheets[0].cssRules[0].cssText,
    sheets[1].cssRules[0].cssText,
    getComputedStyle(host.shadowRoot.getElementById('t')).color,
    getComputedStyle(host.shadowRoot.getElementById('t')).textDecoration,
    sheets[0] === host2.shadowRoot.adoptedStyleSheets[0],
    cloneSheets.length,
    cloneSheets[0] === sheets[0],
    cacheDescriptor.enumerable,
    cacheDescriptor.configurable,
    cacheDescriptor.writable,
    cacheDeleteResult,
    cacheRedefineResult,
    postTamper.querySelector('#host3').shadowRoot.adoptedStyleSheets[0] === sheets[0],
    Object.prototype.hasOwnProperty.call(sheets[0], '__moliCssModuleSheetLoaded')
  ].join('|');
})()
"#,
        )
        .expect("declarative adoptedStyleSheets should resolve synchronous data CSS specifiers");

    assert_eq!(
        result,
        "2|1|span { color: blue; }|span { text-decoration: underline; }|rgb(0, 0, 255)|underline|true|2|true|false|false|false|false|TypeError|true|false"
    );
}
#[test]
fn declarative_shadow_adopted_stylesheets_are_scoped_per_nested_root() {
    let mut vm = new_storage_test_vm("https://shadow-adopted-css-nested.test/");

    let result = vm
        .eval(
            r#"
(() => {
	  const target = document.createElement('section');
	  target.setHTMLUnsafe(
	    "<div id='host'>" +
	      "<template shadowrootmode='open' " +
	        "shadowrootadoptedstylesheets='data:text/css,span%20%7B%20color%3A%20blue%3B%20%7D'>" +
        "<span id='outer_span'>Outer content</span>" +
        "<div id='inner_host'>" +
          "<template shadowrootmode='open' " +
            "shadowrootadoptedstylesheets='data:text/css,span%20%7B%20color%3A%20green%3B%20%7D'>" +
            "<span id='inner_span'>Inner content</span>" +
          "</template>" +
        "</div>" +
	      "</template>" +
	    "</div>"
	  );
	  (document.body || document.documentElement || document.appendChild(document.createElement('html'))).append(target);
	  const outerRoot = target.querySelector('#host').shadowRoot;
  const innerRoot = outerRoot.getElementById('inner_host').shadowRoot;
  return [
    outerRoot.adoptedStyleSheets.length,
    innerRoot.adoptedStyleSheets.length,
    outerRoot.adoptedStyleSheets[0].cssRules[0].cssText,
    innerRoot.adoptedStyleSheets[0].cssRules[0].cssText,
    getComputedStyle(outerRoot.getElementById('outer_span')).color,
    getComputedStyle(innerRoot.getElementById('inner_span')).color,
    outerRoot.adoptedStyleSheets[0] === innerRoot.adoptedStyleSheets[0]
  ].join('|');
})()
"#,
        )
        .expect("nested declarative adoptedStyleSheets should stay root-scoped");

    assert_eq!(
        result,
        concat!(
            "1|1|span { color: blue; }|span { color: green; }|",
            "rgb(0, 0, 255)|rgb(0, 128, 0)|false"
        )
    );
}
#[test]
fn declarative_shadow_adopted_stylesheets_style_without_explicit_getter() {
    let mut vm = new_storage_test_vm("https://shadow-adopted-css-lazy.test/");

    let result = vm
        .eval(
            r#"
(() => {
	  const target = document.createElement('section');
	  const css = 'data:text/css,span%20%7B%20color%3A%20blue%20%7D';
	  target.setHTMLUnsafe(
	    "<div id='host'>" +
	      "<template shadowrootmode='open' shadowrootclonable shadowrootadoptedstylesheets='" + css + "'>" +
        "<span id='t'>lazy</span>" +
      "</template>" +
    "</div>" +
    "<div id='invalid'>" +
      "<template shadowrootmode='open' shadowrootadoptedstylesheets='missing'>" +
        "<span id='u'>invalid</span>" +
	      "</template>" +
	    "</div>"
	  );
	  (document.body || document.documentElement || document.appendChild(document.createElement('html'))).append(target);
	  const clone = target.querySelector('#host').cloneNode(true);
	  target.append(clone);
  return [
    getComputedStyle(clone.shadowRoot.getElementById('t')).color,
    clone.shadowRoot.adoptedStyleSheets[0].cssRules[0].cssText,
    getComputedStyle(target.querySelector('#invalid').shadowRoot.getElementById('u')).color
  ].join('|');
})()
"#,
        )
        .expect("declarative adoptedStyleSheets should feed computed style lazily");

    assert_eq!(result, "rgb(0, 0, 255)|span { color: blue; }|rgb(0, 0, 0)");
}
#[test]
fn document_import_node_resolves_shadow_adopted_sheets_against_destination_document() {
    let mut vm = new_storage_test_vm("https://shadow-adopted-css-import.test/");

    let setup = vm
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
  const importMap = document.createElement('script');
  importMap.type = 'importmap';
  importMap.textContent = JSON.stringify({
    imports: {
      foo: 'data:text/css,span{color:blue}',
      bar: 'data:text/css,span{text-decoration:underline}'
    }
  });
  document.head.appendChild(importMap);

  const frame = document.createElement('iframe');
  frame.srcdoc = `
    <!doctype html>
    <script type="importmap">
    { "imports": { "iframe-only": "data:text/css,span{color:green}", "shared": "data:text/css,span{color:green}" } }
    <\/script>
    <body>
      <div id="iframe-only-host">
        <template shadowrootmode="open" shadowrootclonable shadowrootadoptedstylesheets="iframe-only">
          <span id="t">iframe</span>
        </template>
      </div>
      <div id="shared-host">
        <template shadowrootmode="open" shadowrootclonable shadowrootadoptedstylesheets="shared">
          <span id="t">shared</span>
        </template>
      </div>
      <div id="nonclonable-host">
        <template shadowrootmode="open" shadowrootadoptedstylesheets="iframe-only">
          <span id="t">nonclonable</span>
        </template>
      </div>
    </body>
  `;
  document.body.appendChild(frame);
  globalThis.__shadowAdoptedImportFrame = frame;
  return 'ready';
})()
"#,
        )
        .expect("shadow adoptedStyleSheets import setup should evaluate");
    assert_eq!(setup, "ready");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const doc = globalThis.__shadowAdoptedImportFrame.contentDocument;
  const iframeOnly = doc.getElementById('iframe-only-host');
  const shared = doc.getElementById('shared-host');
  const nonclonable = doc.getElementById('nonclonable-host');
  const importedIframeOnly = document.importNode(iframeOnly, true);
  document.body.appendChild(importedIframeOnly);
  const importedShared = document.importNode(shared, true);
  const importedNonclonable = document.importNode(nonclonable, true);
  const importedIframeOnlyRoot = importedIframeOnly.shadowRoot;
  const importedSharedRoot = importedShared.shadowRoot;
  return [
    iframeOnly.shadowRoot.adoptedStyleSheets.length,
    iframeOnly.shadowRoot.clonable,
    shared.shadowRoot.adoptedStyleSheets.length,
    shared.shadowRoot.clonable,
    nonclonable.shadowRoot.adoptedStyleSheets.length,
    nonclonable.shadowRoot.clonable,
    !!importedIframeOnlyRoot,
    importedIframeOnlyRoot && importedIframeOnlyRoot.adoptedStyleSheets.length,
    !!importedSharedRoot,
    importedSharedRoot && importedSharedRoot.adoptedStyleSheets.length,
    importedNonclonable.shadowRoot === null
  ].join('|');
})()
"#,
        )
        .expect("shadow adoptedStyleSheets import probe should evaluate");

    assert_eq!(result, "1|true|1|true|1|false|true|0|true|0|true");
}
#[test]
fn declarative_shadow_adopted_stylesheets_resolve_same_document_css_module() {
    let mut vm = new_storage_test_vm("https://shadow-adopted-css-module.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.setAttribute('type', 'module');
  style.setAttribute('specifier', 'foo');
  style.textContent = 'span { color: blue }';
  document.appendChild(style);

  const wrapper = document.createElement('section');
  wrapper.setHTMLUnsafe(
    "<div id='host'>" +
      "<template shadowrootmode='open' shadowrootadoptedstylesheets='foo'>" +
        "<span id='test_element'>Test content</span>" +
      "</template>" +
    "</div>"
  );
  document.appendChild(wrapper);

  const host = document.getElementById('host');
  const parsed = Document.parseHTMLUnsafe(
    "<div id='parsed_host'>" +
      "<template shadowrootmode='open' shadowrootadoptedstylesheets='foo'>" +
        "<span>Detached</span>" +
      "</template>" +
    "</div>"
  );
  const descriptor = Object.getOwnPropertyDescriptor(Document, 'parseHTMLUnsafe');
  const parsedHost = parsed.getElementById('parsed_host');
  return [
    typeof Document.parseHTMLUnsafe,
    Document.parseHTMLUnsafe.name,
    Document.parseHTMLUnsafe.length,
    !!descriptor,
    descriptor && descriptor.enumerable,
    descriptor && descriptor.writable,
    descriptor && descriptor.configurable,
    Object.keys(Document).includes('parseHTMLUnsafe'),
    host.shadowRoot.adoptedStyleSheets.length,
    host.shadowRoot.adoptedStyleSheets[0].cssRules[0].cssText,
    getComputedStyle(host.shadowRoot.getElementById('test_element')).color,
    parsedHost.shadowRoot.adoptedStyleSheets.length
  ].join('|');
})()
"#,
        )
        .expect("declarative adoptedStyleSheets should resolve same-document CSS modules");

    assert_eq!(
        result,
        "function|parseHTMLUnsafe|1|true|true|true|true|true|1|span { color: blue; }|rgb(0, 0, 255)|0"
    );
}
#[test]
fn detached_declarative_shadow_adopted_stylesheets_use_first_import_map_only() {
    let mut vm = new_storage_test_vm("https://shadow-adopted-css-import-map.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const blue = 'data:text/css,span%20%7B%20color%3A%20blue%3B%20%7D';
  const red = 'data:text/css,span%20%7B%20color%3A%20red%3B%20%7D';
  const green = 'data:text/css,span%20%7B%20color%3A%20green%3B%20%7D';
  const doc = Document.parseHTMLUnsafe(
    '<script type="importmap">' +
      JSON.stringify({ imports: { foo: blue } }) +
    '</script>' +
    '<script type="importmap">' +
      JSON.stringify({ imports: { foo: red, bar: green } }) +
    '</script>' +
    '<div id="host"><template shadowrootmode="open" ' +
      'shadowrootadoptedstylesheets="foo bar"><span>Detached</span></template></div>'
  );
  const sheets = doc.getElementById('host').shadowRoot.adoptedStyleSheets;
  return [
    sheets.length,
    sheets[0] && sheets[0].cssRules[0].cssText
  ].join('|');
})()
"#,
        )
        .expect("detached declarative adoptedStyleSheets should use only the first import map");

    assert_eq!(result, "1|span { color: blue; }");
}
#[test]
fn declarative_shadow_adopted_stylesheets_fill_css_module_placeholder_from_cache() {
    let mut vm = new_storage_test_vm("https://shadow-adopted-css-module.test/page.html");

    let before = vm
        .eval(
            r#"
(() => {
  const wrapper = document.createElement('section');
  wrapper.setHTMLUnsafe(
    "<div id='host'>" +
      "<template shadowrootmode='open' shadowrootadoptedstylesheets='./module.css'>" +
        "<span id='test_element'>Test content</span>" +
      "</template>" +
    "</div>"
  );
  document.appendChild(wrapper);

  const root = document.getElementById('host').shadowRoot;
  const sheet = root.adoptedStyleSheets[0];
  window.placeholderSheet = sheet;
  return [
    root.adoptedStyleSheets.length,
    sheet.cssRules.length,
    getComputedStyle(root.getElementById('test_element')).color
  ].join('|');
})()
"#,
        )
        .expect("declarative adoptedStyleSheets should create a placeholder");

    assert_eq!(before, "1|0|rgb(0, 0, 0)");

    let css_url = url::Url::parse("https://shadow-adopted-css-module.test/module.css").unwrap();
    vm._context_host
        .borrow_mut()
        .record_css_module_text_for_url(&css_url, "span { color: green }".to_owned());

    let after = vm
        .eval(
            r#"
(() => {
  const root = document.getElementById('host').shadowRoot;
  const sheet = root.adoptedStyleSheets[0];
  return [
    sheet === window.placeholderSheet,
    root.adoptedStyleSheets.length,
    sheet.cssRules.length,
    sheet.cssRules[0].cssText,
    getComputedStyle(root.getElementById('test_element')).color
  ].join('|');
})()
"#,
        )
        .expect("cached CSS module text should fill the existing placeholder");

    assert_eq!(after, "true|1|1|span { color: green; }|rgb(0, 128, 0)");
}
#[test]
fn declarative_shadow_adopted_stylesheet_placeholder_survives_later_css_module_failure() {
    let mut vm = new_storage_test_vm("https://shadow-adopted-css-module.test/page.html");

    let created = vm
        .eval(
            r#"
(() => {
  const wrapper = document.createElement('section');
  wrapper.setHTMLUnsafe(
    "<div id='host'>" +
      "<template shadowrootmode='open' shadowrootadoptedstylesheets='./missing.css'>" +
        "<span>Test content</span>" +
      "</template>" +
    "</div>"
  );
  document.appendChild(wrapper);
  window.pendingDeclarativeRoot = document.getElementById('host').shadowRoot;
  return window.pendingDeclarativeRoot instanceof ShadowRoot;
})()
"#,
        )
        .expect("declarative shadow root should be exposed before CSS module completion");

    assert_eq!(created, "true");

    let css_url = url::Url::parse("https://shadow-adopted-css-module.test/missing.css").unwrap();
    vm._context_host
        .borrow_mut()
        .record_css_module_failure_for_url(&css_url);

    let after = vm
        .eval(
            r#"
(() => {
  const root = window.pendingDeclarativeRoot;
  const sheet = root.adoptedStyleSheets[0];
  return [
    root.adoptedStyleSheets.length,
    sheet && sheet.cssRules.length
  ].join('|');
})()
"#,
        )
        .expect("first declarative consumer should retain its placeholder after failure");

    assert_eq!(after, "1|0");
}
#[test]
fn declarative_shadow_root_slot_assignment_reflects_template_attribute() {
    let mut vm = new_storage_test_vm("https://shadow-root-slot-assignment.test/");

    let result = vm
        .eval(
            r#"
(() => {
  customElements.define('x-closed', class extends HTMLElement {
    constructor() {
      super();
      this._internals = this.attachInternals();
    }
  });
  const template = document.createElement('template');
  const initial = template.shadowRootSlotAssignment;
  template.setAttribute('shadowrootslotassignment', 'MANUAL');
  const attrManual = template.shadowRootSlotAssignment;
  template.setAttribute('shadowrootslotassignment', 'invalid');
  const attrInvalid = template.shadowRootSlotAssignment;
  template.shadowRootSlotAssignment = 'manual';
  const setterManual = [
    template.getAttribute('shadowrootslotassignment'),
    template.shadowRootSlotAssignment
  ].join(':');
  template.shadowRootSlotAssignment = '';
  const setterEmpty = [
    template.getAttribute('shadowrootslotassignment'),
    template.shadowRootSlotAssignment
  ].join(':');

  const wrapper = document.createElement('div');
  wrapper.setHTMLUnsafe(
    '<div id="open"><template shadowrootmode="open" shadowrootslotassignment="MANUAL"></template></div>' +
    '<x-closed id="closed"><template shadowrootmode="closed" shadowrootslotassignment="manual"></template></x-closed>' +
    '<div id="default"><template shadowrootmode="open"></template></div>' +
    '<div id="invalid"><template shadowrootmode="open" shadowrootslotassignment="invalid"></template></div>'
  );
  const openHost = wrapper.querySelector('#open');
  const closedHost = wrapper.querySelector('#closed');
  const defaultHost = wrapper.querySelector('#default');
  const invalidHost = wrapper.querySelector('#invalid');
  const internals = closedHost._internals;

  return [
    initial,
    attrManual,
    attrInvalid,
    setterManual,
    setterEmpty,
    openHost.shadowRoot.slotAssignment,
    internals.shadowRoot instanceof ShadowRoot,
    defaultHost.shadowRoot.slotAssignment,
    invalidHost.shadowRoot.slotAssignment,
    closedHost.shadowRoot === null,
    typeof Element.prototype.attachInternals,
    Element.prototype.attachInternals.length
  ].join('|');
})()
"#,
        )
        .expect("declarative shadow root slotAssignment should reflect template state");

    assert_eq!(
        result,
        "named|manual|named|manual:manual|:named|manual|true|named|named|true|function|0"
    );
}
#[test]
fn declarative_shadow_root_init_accessors_reflect_template_attributes() {
    let mut vm = new_storage_test_vm("https://shadow-root-template-init.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const template = document.createElement('template');
  const featureDetect = [
    HTMLTemplateElement.prototype.hasOwnProperty('shadowRootMode'),
    HTMLTemplateElement.prototype.hasOwnProperty('shadowRootSlotAssignment'),
    HTMLTemplateElement.prototype.hasOwnProperty('shadowRootAdoptedStyleSheets')
  ].join(':');
  const adoptedDescriptor = Object.getOwnPropertyDescriptor(
    HTMLTemplateElement.prototype,
    'shadowRootAdoptedStyleSheets'
  );
  const adoptedDescriptorState = [
    typeof adoptedDescriptor.get,
    typeof adoptedDescriptor.set,
    adoptedDescriptor.enumerable,
    adoptedDescriptor.configurable
  ].join(':');

  template.setAttribute('shadowrootmode', 'OpEn');
  const attrMode = template.shadowRootMode;
  template.setAttribute('shadowrootmode', 'INVALID');
  const invalidMode = template.shadowRootMode;
  template.removeAttribute('shadowrootmode');
  const missingMode = template.shadowRootMode;
  template.shadowRootMode = 'CLOSED';
  const setterMode = [
    template.getAttribute('shadowrootmode'),
    template.shadowRootMode
  ].join(':');

  template.setAttribute('shadowrootdelegatesfocus', 'anything');
  const delegatesAttr = template.shadowRootDelegatesFocus;
  template.shadowRootDelegatesFocus = false;
  const delegatesRemoved = [
    template.hasAttribute('shadowrootdelegatesfocus'),
    template.shadowRootDelegatesFocus
  ].join(':');
  template.shadowRootDelegatesFocus = true;
  const delegatesSet = [
    template.getAttribute('shadowrootdelegatesfocus'),
    template.shadowRootDelegatesFocus
  ].join(':');

  template.shadowRootClonable = true;
  const clonableSet = [
    template.getAttribute('shadowrootclonable'),
    template.shadowRootClonable
  ].join(':');
  template.shadowRootClonable = false;
  const clonableRemoved = [
    template.hasAttribute('shadowrootclonable'),
    template.shadowRootClonable
  ].join(':');

  template.shadowRootSerializable = true;
  const serializableSet = [
    template.getAttribute('shadowrootserializable'),
    template.shadowRootSerializable
  ].join(':');
  template.shadowRootSerializable = false;
  const serializableRemoved = [
    template.hasAttribute('shadowrootserializable'),
    template.shadowRootSerializable
  ].join(':');

  const adoptedAbsent = template.shadowRootAdoptedStyleSheets;
  template.setAttribute('shadowrootadoptedstylesheets', 'style-a style-b');
  const adoptedAttr = template.shadowRootAdoptedStyleSheets;
  template.shadowRootAdoptedStyleSheets = 'sheet-1 sheet-2';
  const adoptedSet = [
    template.getAttribute('shadowrootadoptedstylesheets'),
    template.shadowRootAdoptedStyleSheets
  ].join(':');

  const wrapper = document.createElement('div');
  wrapper.setHTMLUnsafe(
    '<div id="multi">' +
    '<template shadowrootmode="open"></template>' +
    '<template shadowrootmode="closed"></template>' +
    '</div>'
  );
  const leftoverMode = wrapper.querySelector('#multi template').shadowRootMode;

  return [
    featureDetect,
    adoptedDescriptorState,
    attrMode,
    invalidMode,
    missingMode,
    setterMode,
    delegatesAttr,
    delegatesRemoved,
    delegatesSet,
    clonableSet,
    clonableRemoved,
    serializableSet,
    serializableRemoved,
    adoptedAbsent,
    adoptedAttr,
    adoptedSet,
    leftoverMode
  ].join('|');
})()
"#,
        )
        .expect("template declarative shadow init accessors should reflect attributes");

    assert_eq!(
        result,
        "true:true:true|function:function:true:true|open|||CLOSED:closed|true|false:false|:true|:true|false:false|:true|false:false||style-a style-b|sheet-1 sheet-2:sheet-1 sheet-2|closed"
    );
}
#[test]
fn attach_shadow_reuses_declarative_shadow_root() {
    let mut vm = new_storage_test_vm("https://shadow-root-declarative-reattach.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const wrapper = document.createElement('section');
  wrapper.setHTMLUnsafe(
    '<div id="closed-host"><template shadowrootmode="closed" ' +
    'shadowrootserializable=""><span>old</span></template>light</div>' +
    '<div id="open-host"><template shadowrootmode="open"><b>keep</b></template></div>'
  );
  const closedHost = wrapper.querySelector('#closed-host');
  const openHost = wrapper.querySelector('#open-host');
  const openRootBefore = openHost.shadowRoot;
  const closedRoot = closedHost.attachShadow({ mode: 'closed', serializable: true });
  closedRoot.append(document.createElement('slot'));
  const secondAttach = (() => {
    try {
      closedHost.attachShadow({ mode: 'closed' });
      return 'no-throw';
    } catch (error) {
      return `${error.name}:${error.code}`;
    }
  })();
  const mismatchedAttach = (() => {
    try {
      openHost.attachShadow({ mode: 'closed' });
      return 'no-throw';
    } catch (error) {
      return `${error.name}:${error.code}`;
    }
  })();

  return [
    closedHost.shadowRoot === null,
    closedRoot instanceof ShadowRoot,
    closedRoot.innerHTML,
    wrapper.getHTML({ serializableShadowRoots: true }),
    openHost.shadowRoot === openRootBefore,
    openRootBefore.innerHTML,
    secondAttach,
    mismatchedAttach
  ].join('|');
})()
"#,
        )
        .expect("attachShadow should reuse a declarative shadow root once");

    assert_eq!(
        result,
        concat!(
            "true|true|<slot></slot>|",
            "<div id=\"closed-host\"><template shadowrootmode=\"closed\" ",
            "shadowrootserializable=\"\"><slot></slot></template>light</div>",
            "<div id=\"open-host\"></div>|true|<b>keep</b>|",
            "NotSupportedError:9|NotSupportedError:9"
        )
    );
}
#[test]
fn child_document_snapshots_expose_declarative_shadow_roots() {
    let mut vm = new_storage_test_vm("https://shadow-root-child-documents.test/");

    vm.eval(
        r#"
(() => {
  const parent = document.body || document.documentElement || document;
  const srcdocFrame = document.createElement('iframe');
  parent.appendChild(srcdocFrame);
  srcdocFrame.srcdoc =
    '<div id="srcdoc-host"><template shadowrootmode="open">' +
    '<span class="content">srcdoc</span></template><p>light</p></div>';
  globalThis.__shadowSnapshotSrcdocFrame = srcdocFrame;

  const writeFrame = document.createElement('iframe');
  parent.appendChild(writeFrame);
  const writeDoc = writeFrame.contentDocument;
  writeDoc.open();
  writeDoc.write(
    '<div id="write-host"><template shadowrootmode="open">' +
    '<span class="content">write</span></template><p>Test</p></div>'
  );
  writeDoc.close();
  globalThis.__shadowSnapshotWriteHost = writeDoc.querySelector('#write-host');

  const noOpenFrame = document.createElement('iframe');
  parent.appendChild(noOpenFrame);
  noOpenFrame.contentDocument.write(
    '<div id="no-open-host"><template shadowrootmode="open"><slot></slot></template><p>NoOpen</p></div>'
  );
  globalThis.__shadowSnapshotNoOpenHost =
    noOpenFrame.contentDocument.querySelector('#no-open-host');

  const parserDoc = new DOMParser().parseFromString(
    '<div id="parser-host"><template shadowrootmode="open">' +
    '<span class="content">parser</span></template></div>',
    'text/html'
  );
  globalThis.__shadowSnapshotParserHost = parserDoc.querySelector('#parser-host');
})()
"#,
    )
    .expect("child document snapshot setup should evaluate");
    assert!(
        vm.run_next_child_navigation_commit_body_for_test()
            .expect("typed child navigation-commit body should succeed")
            .is_some(),
        "the srcdoc snapshot Document must commit on its own owner turn"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const srcdocHost = globalThis.__shadowSnapshotSrcdocFrame.contentDocument
    .querySelector('#srcdoc-host');
  const writeHost = globalThis.__shadowSnapshotWriteHost;
  const noOpenHost = globalThis.__shadowSnapshotNoOpenHost;
  const parserHost = globalThis.__shadowSnapshotParserHost;

  return [
    !!srcdocHost.shadowRoot,
    srcdocHost.shadowRoot.querySelector('.content').textContent,
    srcdocHost.querySelector('template') === null,
    srcdocHost.innerText,
    !!writeHost.shadowRoot,
    writeHost.shadowRoot.querySelector('.content').textContent,
    writeHost.querySelector('template') === null,
    writeHost.innerText,
    !!noOpenHost.shadowRoot,
    noOpenHost.innerText,
    !parserHost.shadowRoot,
    !!parserHost.querySelector('template'),
    parserHost.querySelector('template').content.querySelector('.content').textContent
  ].join('|');
})()
"#,
        )
        .expect("child document snapshots should expose declarative shadow roots");

    assert_eq!(
        result,
        "true|srcdoc|true||true|write|true||true|NoOpen|true|true|parser"
    );
}
#[test]
fn declarative_shadow_root_reference_target_reflects_template_attribute() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-root-reference-target-declarative.test/",
        r#"<!doctype html>
<div id="host1"><template shadowrootmode="open"></template></div>
<div id="host2"><template shadowrootmode="open" shadowrootreferencetarget></template></div>
<div id="host3"><template shadowRootMode="open" shadowRootReferenceTarget="targetID"></template></div>
"#,
    );

    let result = vm
        .eval(
            r#"
[
  document.querySelector('#host1').shadowRoot.referenceTarget === null,
  document.querySelector('#host2').shadowRoot.referenceTarget,
  document.querySelector('#host3').shadowRoot.referenceTarget
].join('|')
"#,
        )
        .expect("declarative ShadowRoot referenceTarget should reflect parser state");

    assert_eq!(result, "true||targetID");
}
#[test]
fn labels_follow_shadow_root_reference_target_chain() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-root-reference-target-labels.test/",
        r#"<!doctype html>
<label id="direct-label" for="x-direct">Direct</label>
<x-direct id="x-direct">
  <template shadowrootmode="open" shadowrootreferencetarget="direct-input">
    <input id="direct-input">
  </template>
</x-direct>
<label id="nested-label" for="x-outer">Nested</label>
<x-outer id="x-outer">
  <template shadowrootmode="open" shadowrootreferencetarget="x-inner">
    <x-inner id="x-inner">
      <template shadowrootmode="open" shadowrootreferencetarget="nested-input">
        <input id="nested-input">
      </template>
    </x-inner>
  </template>
</x-outer>
<label id="empty-label" for="x-empty">Empty</label>
<x-empty id="x-empty">
  <template shadowrootmode="open" shadowrootreferencetarget="">
    <input id="empty-input">
  </template>
</x-empty>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const directLabel = document.getElementById('direct-label');
  const directHost = document.getElementById('x-direct');
  const directInput = directHost.shadowRoot.getElementById('direct-input');
  const nestedLabel = document.getElementById('nested-label');
  const outer = document.getElementById('x-outer');
  const inner = outer.shadowRoot.getElementById('x-inner');
  const nestedInput = inner.shadowRoot.getElementById('nested-input');
  const emptyLabel = document.getElementById('empty-label');
  const emptyHost = document.getElementById('x-empty');
  const emptyInput = emptyHost.shadowRoot.getElementById('empty-input');

  const initial = [
    directLabel.control === directHost,
    Array.from(directInput.labels).map(label => label.id).join(','),
    nestedLabel.control === outer,
    Array.from(nestedInput.labels).map(label => label.id).join(','),
    emptyLabel.control === null,
    Array.from(emptyInput.labels).length
  ].join('|');

  directHost.shadowRoot.referenceTarget = null;
  const afterNull = [
    directLabel.control === null,
    Array.from(directInput.labels).length
  ].join('|');

  directHost.shadowRoot.referenceTarget = 'direct-input';
  const afterRestore = [
    directLabel.control === directHost,
    Array.from(directInput.labels).map(label => label.id).join(',')
  ].join('|');

  return [initial, afterNull, afterRestore].join(';');
})()
"#,
        )
        .expect("labels should resolve through ShadowRoot referenceTarget");

    assert_eq!(
        result,
        "true|direct-label|true|nested-label|true|0;true|0;true|direct-label"
    );
}
#[test]
fn labels_include_shadow_tree_and_implicit_reference_targets() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-root-reference-target-label-descendant.test/",
        r#"<!doctype html>
<label id="implicit-label">
  Implicit
  <x-implicit id="x-implicit">
    <template shadowrootmode="open" shadowrootreferencetarget="implicit-input">
      <input id="implicit-input">
    </template>
  </x-implicit>
</label>
<label id="outer-label" for="x-outer">Outer</label>
<x-outer id="x-outer">
  <template shadowrootmode="open" shadowrootreferencetarget="x-inner">
    <label id="middle-label" for="x-inner">Middle</label>
    <x-inner id="x-inner">
      <template shadowrootmode="open" shadowrootreferencetarget="deep-input">
        <label id="deep-label" for="deep-input">Deep</label>
        <input id="deep-input">
      </template>
    </x-inner>
  </template>
</x-outer>
<label id="tail-label" for="x-outer">Tail</label>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const implicitHost = document.getElementById('x-implicit');
  const implicitInput = implicitHost.shadowRoot.getElementById('implicit-input');
  const outer = document.getElementById('x-outer');
  const inner = outer.shadowRoot.getElementById('x-inner');
  const deepInput = inner.shadowRoot.getElementById('deep-input');
  const middleLabel = outer.shadowRoot.getElementById('middle-label');
  const container = document.createElement('div');
  const disconnectedLabel = document.createElement('label');
  disconnectedLabel.id = 'disconnected-label';
  disconnectedLabel.htmlFor = 'disconnected-input';
  const disconnectedInput = document.createElement('input');
  disconnectedInput.id = 'disconnected-input';

  const initial = [
    Array.from(implicitInput.labels).map(label => label.id).join(','),
    Array.from(deepInput.labels).map(label => label.id).join(','),
    middleLabel.control === inner
  ].join('|');

  implicitHost.shadowRoot.referenceTarget = null;
  const afterNull = Array.from(implicitInput.labels).length;
  const beforeDisconnectedInsert = Array.from(disconnectedInput.labels).length;
  container.append(disconnectedLabel, disconnectedInput);
  const afterDisconnectedInsert = Array.from(disconnectedInput.labels).map(label => label.id).join(',');

  return `${initial};${afterNull};${beforeDisconnectedInsert};${afterDisconnectedInsert}`;
})()
"#,
        )
        .expect("labels should include shadow tree labels and implicit reference targets");

    assert_eq!(
        result,
        "implicit-label|outer-label,middle-label,deep-label,tail-label|true;0;0;disconnected-label"
    );
}
#[test]
fn form_and_list_idrefs_follow_shadow_root_reference_target_chain() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-root-reference-target-form-list.test/",
        r#"<!doctype html>
<button id="reset-button" type="reset" form="x-form"></button>
<x-form id="x-form">
  <template shadowrootmode="open" shadowrootreferencetarget="real-form">
    <form id="real-form">
      <input id="owned-input" name="owned" value="default value">
    </form>
  </template>
</x-form>
<input id="external-input" name="external" form="x-form">
<select id="external-select" name="choice" form="x-form">
  <option id="external-option">one</option>
</select>
<input id="list-input" list="x-list">
<x-list id="x-list">
  <template shadowrootmode="open" shadowrootreferencetarget="real-list">
    <datalist id="real-list"><option value="one"></option></datalist>
  </template>
</x-list>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const formHost = document.getElementById('x-form');
  const realForm = formHost.shadowRoot.getElementById('real-form');
  const ownedInput = realForm.querySelector('#owned-input');
  const resetButton = document.getElementById('reset-button');
  const externalInput = document.getElementById('external-input');
  const externalSelect = document.getElementById('external-select');
  const externalOption = document.getElementById('external-option');
  const listHost = document.getElementById('x-list');
  const listInput = document.getElementById('list-input');
  const realList = listHost.shadowRoot.getElementById('real-list');

  ownedInput.value = 'changed value';
  const elements = Array.from(realForm.elements).map(element => element.id).join(',');
  const initial = [
    externalInput.form === formHost,
    externalSelect.form === formHost,
    externalOption.form === realForm,
    elements.includes('reset-button'),
    elements.includes('external-input'),
    elements.includes('external-select'),
    elements.includes('owned-input'),
    realForm.elements.length,
    listInput.list === listHost
  ].join('|');

  resetButton.click();
  const afterReset = ownedInput.value;

  formHost.shadowRoot.referenceTarget = null;
  listHost.shadowRoot.referenceTarget = null;
  const afterNull = [
    externalInput.form === null,
    externalSelect.form === null,
    externalOption.form === null,
    listInput.list === null
  ].join('|');

  formHost.shadowRoot.referenceTarget = 'real-form';
  listHost.shadowRoot.referenceTarget = 'real-list';
  listInput.list = document.createElement('datalist');
  const afterRestore = [
    externalInput.form === formHost,
    externalSelect.form === formHost,
    externalOption.form === realForm,
    listInput.list === listHost
  ].join('|');

  formHost.shadowRoot.referenceTarget = '';
  listHost.shadowRoot.referenceTarget = '';
  const afterEmpty = [
    externalInput.form === null,
    listInput.list === null
  ].join('|');

  return [initial, afterReset, afterNull, afterRestore, afterEmpty].join(';');
})()
"#,
        )
        .expect("form and list IDREFs should resolve through ShadowRoot referenceTarget");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true|4|true;default value;true|true|true|true;true|true|true|true;true|true"
    );
}
#[test]
fn form_elements_include_face_and_shadow_reference_targets_in_tree_order() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-root-reference-target-form-elements.test/",
        r#"<!doctype html>
<form-associated-custom-button id="custom-button" form="fancy-form"></form-associated-custom-button>
<fancy-form id="fancy-form">
  <template shadowrootmode="open" shadowrootreferencetarget="real-form">
    <form id="real-form">
      <input id="owned-input">
      <form-associated-custom-button id="custom-button-in-shadow"></form-associated-custom-button>
    </form>
  </template>
</fancy-form>
<button id="reset-button" type="reset" form="outer-form"></button>
<outer-form id="outer-form">
  <template shadowrootmode="open" shadowrootreferencetarget="nested-element">
    <nested-element id="nested-element">
      <template shadowrootmode="open" shadowrootreferencetarget="nested-real-form">
        <form id="nested-real-form">
          <input id="nested-input">
        </form>
      </template>
    </nested-element>
    <button id="button-in-shadow" form="nested-element"></button>
  </template>
</outer-form>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  class FormAssociatedCustomButton extends HTMLElement {
    static formAssociated = true;
    constructor() {
      super();
      this.attachInternals();
      this.attachShadow({ mode: 'open' }).innerHTML = '<button id="internal-button"></button>';
    }
  }
  customElements.define('form-associated-custom-button', FormAssociatedCustomButton);

  const fancyForm = document.getElementById('fancy-form');
  const realForm = fancyForm.shadowRoot.getElementById('real-form');
  const customOrder = Array.from(realForm.elements).map(element => element.id).join(',');

  const outerForm = document.getElementById('outer-form');
  const nestedElement = outerForm.shadowRoot.getElementById('nested-element');
  const nestedRealForm = nestedElement.shadowRoot.getElementById('nested-real-form');
  const initialNestedOrder = Array.from(nestedRealForm.elements).map(element => element.id).join(',');
  outerForm.shadowRoot.getElementById('button-in-shadow').remove();
  const newButton = document.createElement('button');
  newButton.id = 'new-button-in-shadow';
  newButton.setAttribute('form', 'nested-element');
  nestedElement.parentNode.insertBefore(newButton, nestedElement);
  const afterInsertOrder = Array.from(nestedRealForm.elements).map(element => element.id).join(',');

  return `${customOrder};${initialNestedOrder};${afterInsertOrder}`;
})()
"#,
        )
        .expect("form.elements should include FACE and reference-target controls in tree order");

    assert_eq!(
        result,
        "custom-button,owned-input,custom-button-in-shadow;reset-button,nested-input,button-in-shadow;reset-button,new-button-in-shadow,nested-input"
    );
}
#[test]
fn command_events_follow_reference_target_source_path() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-reference-target-command-events.test/",
        r#"<!doctype html>
<x-component id="outerA">
  <template shadowrootmode="open">
    <button id="button" command="--foo" commandfor="middleA">Do Foo</button>
    <x-component id="middleA">
      <template shadowrootmode="open" shadowrootreferencetarget="innerA">
        <x-component id="innerA">
          <template shadowrootmode="open" shadowrootreferencetarget="targetDiv">
            <div id="targetDiv"></div>
          </template>
        </x-component>
      </template>
    </x-component>
  </template>
</x-component>
<x-component id="outerB">
  <template shadowrootmode="open">
    <div id="targetDiv"></div>
    <x-component id="middleB">
      <template shadowrootmode="open" shadowrootreferencetarget="innerB">
        <x-component id="innerB">
          <template shadowrootmode="open" shadowrootreferencetarget="button">
            <button id="button" command="--foo">Do Foo</button>
          </template>
        </x-component>
      </template>
    </x-component>
  </template>
</x-component>
<x-component id="outerF">
  <template shadowrootmode="open">
    <button id="button">Do Foo</button>
    <x-component id="middleF">
      <template shadowrootmode="open">
        <x-component id="innerF">
          <template shadowrootmode="open">
            <div id="targetDiv">Target div</div>
          </template>
        </x-component>
      </template>
    </x-component>
  </template>
</x-component>
<x-component id="outerI">
  <template shadowrootmode="open">
    <button id="button" type="submit" form="middleI">Submit</button>
    <x-component id="middleI">
      <template shadowrootmode="open" shadowrootreferencetarget="innerI">
        <x-component id="innerI">
          <template shadowrootmode="open" shadowrootreferencetarget="targetForm">
            <form id="targetForm"></form>
          </template>
        </x-component>
      </template>
    </x-component>
  </template>
</x-component>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const label = (node) => {
    if (node === window) return 'window';
    if (node === document) return '#document';
    return node.id || node.nodeName;
  };

  const outerA = document.getElementById('outerA');
  const buttonA = outerA.shadowRoot.getElementById('button');
  const middleA = outerA.shadowRoot.getElementById('middleA');
  const innerA = middleA.shadowRoot.getElementById('innerA');
  const targetA = innerA.shadowRoot.getElementById('targetDiv');
  const seenA = [];
  targetA.addEventListener('command', (event) => {
    seenA.push(`target:${event instanceof CommandEvent}:${event.target === targetA}:${event.source === buttonA}:${event.composedPath().map(label).join(',')}`);
  });
  middleA.addEventListener('command', (event) => {
    seenA.push(`middle:${event.target === middleA}:${event.source === buttonA}`);
  });
  outerA.addEventListener('command', () => seenA.push('outer-unexpected'));
  innerA.addEventListener('command', () => seenA.push('inner-unexpected'));
  buttonA.click();

  const outerB = document.getElementById('outerB');
  const targetB = outerB.shadowRoot.getElementById('targetDiv');
  const middleB = outerB.shadowRoot.getElementById('middleB');
  const innerB = middleB.shadowRoot.getElementById('innerB');
  const buttonB = innerB.shadowRoot.getElementById('button');
  buttonB.commandForElement = targetB;
  const seenB = [];
  targetB.addEventListener('command', (event) => {
    seenB.push(`target:${event instanceof CommandEvent}:${event.target === targetB}:${event.source === middleB}:${event.composedPath().map(label).join(',')}`);
  });
  middleB.addEventListener('command', () => seenB.push('middle-unexpected'));
  innerB.addEventListener('command', () => seenB.push('inner-unexpected'));
  outerB.addEventListener('command', () => seenB.push('outer-unexpected'));
  buttonB.click();

  const outerF = document.getElementById('outerF');
  const buttonF = outerF.shadowRoot.getElementById('button');
  const middleF = outerF.shadowRoot.getElementById('middleF');
  const innerF = middleF.shadowRoot.getElementById('innerF');
  const targetF = innerF.shadowRoot.getElementById('targetDiv');
  const seenF = [];
  targetF.addEventListener('command', (event) => {
    seenF.push(`target:${event instanceof CommandEvent}:${event.target === targetF}:${event.source === buttonF}:${event.composedPath().map(label).join(',')}`);
  });
  middleF.addEventListener('command', (event) => {
    seenF.push(`middle:${event.target === middleF}:${event.source === buttonF}`);
  });
  targetF.dispatchEvent(new CommandEvent('command', { source: buttonF }));

  const outerI = document.getElementById('outerI');
  const buttonI = outerI.shadowRoot.getElementById('button');
  const middleI = outerI.shadowRoot.getElementById('middleI');
  const innerI = middleI.shadowRoot.getElementById('innerI');
  const targetForm = innerI.shadowRoot.getElementById('targetForm');
  const seenI = [];
  targetForm.addEventListener('submit', (event) => {
    event.preventDefault();
    seenI.push(`target:${event instanceof SubmitEvent}:${event.target === targetForm}:${event.submitter === buttonI}:${event.composedPath().map(label).join(',')}`);
  });
  middleI.addEventListener('submit', (event) => {
    seenI.push(`middle:${event.target === middleI}:${event.submitter === buttonI}`);
  });
  innerI.addEventListener('submit', () => seenI.push('inner-unexpected'));
  outerI.addEventListener('submit', () => seenI.push('outer-unexpected'));
  buttonI.click();

  return [seenA.join('|'), seenB.join('|'), seenF.join('|'), seenI.join('|')].join(';');
})()
"#,
        )
        .expect("command events should follow source scoped shadow paths");

    assert_eq!(
        result,
        "target:true:true:true:targetDiv,#document-fragment,middleA,#document-fragment|middle:true:true;\
target:true:true:true:targetDiv,#document-fragment;\
target:true:true:true:targetDiv,#document-fragment,middleF,#document-fragment|middle:true:true;\
target:true:true:true:targetForm,#document-fragment,middleI,#document-fragment|middle:true:true"
    );
}
#[tokio::test]
async fn toggle_and_interest_events_follow_reference_target_source_path() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_streamed_parser_page_task_executor_test_vm(
        "https://shadow-reference-target-toggle-interest-events.test/",
        r#"<!doctype html>
<x-component id="outerG">
  <template shadowrootmode="open">
    <button id="button" popovertarget="middleG">Toggle</button>
    <x-component id="middleG">
      <template shadowrootmode="open" shadowrootreferencetarget="innerG">
        <x-component id="innerG">
          <template shadowrootmode="open" shadowrootreferencetarget="targetDiv">
            <div id="targetDiv" popover></div>
          </template>
        </x-component>
      </template>
    </x-component>
  </template>
</x-component>
<x-component id="outerJ">
  <template shadowrootmode="open">
    <button id="button" interestfor="middleJ">Interest</button>
    <x-component id="middleJ">
      <template shadowrootmode="open" shadowrootreferencetarget="innerJ">
        <x-component id="innerJ">
          <template shadowrootmode="open" shadowrootreferencetarget="targetDiv">
            <div id="targetDiv"></div>
          </template>
        </x-component>
      </template>
    </x-component>
  </template>
</x-component>
<button id="otherFocusTarget">Other</button>
"#,
        &loader,
    );

    vm
        .eval(
            r#"
(() => {
  const label = (node) => {
    if (node === window) return 'window';
    if (node === document) return '#document';
    return node.id || node.nodeName;
  };

  const outerG = document.getElementById('outerG');
  const buttonG = outerG.shadowRoot.getElementById('button');
  const middleG = outerG.shadowRoot.getElementById('middleG');
  const innerG = middleG.shadowRoot.getElementById('innerG');
  const targetG = innerG.shadowRoot.getElementById('targetDiv');
  globalThis.__lmReferenceTargetSeenG = [];
  const seenG = globalThis.__lmReferenceTargetSeenG;
  globalThis.__lmReferenceTargetToggleSeenG = [];
  const toggleSeenG = globalThis.__lmReferenceTargetToggleSeenG;
  globalThis.__lmReferenceTargetToggleStatesG = [];
  const recordBeforeG = (entry) => {
    seenG.push(entry);
    toggleSeenG.push(entry);
  };
  targetG.addEventListener('beforetoggle', (event) => {
    if (event.newState === 'open') {
      recordBeforeG(`before-target:${event instanceof ToggleEvent}:${event.target === targetG}:${event.source === buttonG}:${event.oldState}:${event.newState}:${event.composedPath().map(label).join(',')}`);
    }
  });
  middleG.addEventListener('beforetoggle', (event) => {
    if (event.newState === 'open') {
      recordBeforeG(`before-middle:${event.target === middleG}:${event.source === buttonG}`);
    }
  });
  targetG.addEventListener('toggle', (event) => {
    globalThis.__lmReferenceTargetToggleStatesG.push(
      `${event.oldState}->${event.newState}`
    );
    if (event.newState === 'open') {
      toggleSeenG.push(`toggle-target:${event instanceof ToggleEvent}:${event.target === targetG}:${event.source === buttonG}:${event.oldState}:${event.newState}:${event.composedPath().map(label).join(',')}`);
    }
  });
  middleG.addEventListener('toggle', (event) => {
    if (event.newState === 'open') {
      toggleSeenG.push(`toggle-middle:${event.target === middleG}:${event.source === buttonG}`);
    }
  });
  outerG.addEventListener('beforetoggle', () => recordBeforeG('outer-before-unexpected'));
  innerG.addEventListener('beforetoggle', () => recordBeforeG('inner-before-unexpected'));
  outerG.addEventListener('toggle', () => toggleSeenG.push('outer-toggle-unexpected'));
  innerG.addEventListener('toggle', () => toggleSeenG.push('inner-toggle-unexpected'));
  globalThis.__lmReferenceTargetButtonG = buttonG;
  buttonG.click();

  const outerJ = document.getElementById('outerJ');
  const buttonJ = outerJ.shadowRoot.getElementById('button');
  const middleJ = outerJ.shadowRoot.getElementById('middleJ');
  const innerJ = middleJ.shadowRoot.getElementById('innerJ');
  const targetJ = innerJ.shadowRoot.getElementById('targetDiv');
  const otherFocusTarget = document.getElementById('otherFocusTarget');
  globalThis.__lmReferenceTargetSeenJ = [];
  const seenJ = globalThis.__lmReferenceTargetSeenJ;
  globalThis.__lmReferenceTargetToggleStatesJ = [];
  targetJ.addEventListener('toggle', (event) => {
    globalThis.__lmReferenceTargetToggleStatesJ.push(
      `${event.oldState}->${event.newState}`
    );
  });
  targetJ.addEventListener('interest', (event) => {
    seenJ.push(`interest-target:${event instanceof InterestEvent}:${event.target === targetJ}:${event.source === buttonJ}:${event.composedPath().map(label).join(',')}`);
  });
  middleJ.addEventListener('interest', (event) => {
    seenJ.push(`interest-middle:${event.target === middleJ}:${event.source === buttonJ}`);
  });
  targetJ.addEventListener('loseinterest', (event) => {
    seenJ.push(`lose-target:${event instanceof InterestEvent}:${event.target === targetJ}:${event.source === buttonJ}:${event.composedPath().map(label).join(',')}`);
  });
  middleJ.addEventListener('loseinterest', (event) => {
    seenJ.push(`lose-middle:${event.target === middleJ}:${event.source === buttonJ}`);
  });
  outerJ.addEventListener('interest', () => seenJ.push('outer-interest-unexpected'));
  innerJ.addEventListener('interest', () => seenJ.push('inner-interest-unexpected'));
  outerJ.addEventListener('loseinterest', () => seenJ.push('outer-lose-unexpected'));
  innerJ.addEventListener('loseinterest', () => seenJ.push('inner-lose-unexpected'));
  globalThis.__lmReferenceTargetButtonJ = buttonJ;
  globalThis.__lmReferenceTargetOtherFocusTarget = otherFocusTarget;

  return [seenG.join('|'), seenJ.join('|')].join(';');
})()
"#,
        )
        .expect("toggle and interest events should follow source scoped shadow paths");

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("first queued reference-target toggle task should run")
    );

    vm.eval(
        r#"
(() => {
  globalThis.__lmReferenceTargetButtonG.click();
  globalThis.__lmReferenceTargetButtonJ.focus();
  globalThis.__lmReferenceTargetOtherFocusTarget.focus();
})()
"#,
    )
    .expect("second reference-target interaction should evaluate");

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("second queued reference-target toggle task should run")
    );

    let result = vm
        .eval(
            r#"
(() => [
  globalThis.__lmReferenceTargetSeenG.join('|'),
  globalThis.__lmReferenceTargetSeenJ.join('|')
].join(';'))()
"#,
        )
        .expect("reference target event log should evaluate");

    assert_eq!(
        result,
        "before-target:true:true:true:closed:open:targetDiv,#document-fragment,middleG,#document-fragment|before-middle:true:true;\
interest-target:true:true:true:targetDiv,#document-fragment,middleJ,#document-fragment|interest-middle:true:true|\
lose-target:true:true:true:targetDiv,#document-fragment,middleJ,#document-fragment|lose-middle:true:true"
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("third queued reference-target toggle task should run")
    );
    assert!(
        !vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("reference-target toggle source should then be drained")
    );

    let after_toggle = vm
        .eval("globalThis.__lmReferenceTargetToggleSeenG.join('|')")
        .expect("queued reference-target popover toggle should be observable");
    assert_eq!(
        after_toggle,
        "before-target:true:true:true:closed:open:targetDiv,#document-fragment,middleG,#document-fragment|before-middle:true:true|\
toggle-target:true:true:true:closed:open:targetDiv,#document-fragment,middleG,#document-fragment|toggle-middle:true:true"
    );
    assert_eq!(
        vm.eval(
            "[globalThis.__lmReferenceTargetToggleStatesG.join('|'), \
              globalThis.__lmReferenceTargetToggleStatesJ.join('|')].join(';')"
        )
        .expect("reference-target toggle state log should evaluate"),
        "closed->open|open->closed;closed->open",
        "the three selected production tasks should belong to the command and interest popovers"
    );
}
#[test]
fn interest_for_element_focus_popover_respects_reference_target_states() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-interestfor-popover.test/",
        r#"<!doctype html>
<button id="button">Interest</button>
<button id="other">Other</button>
<x-popover id="host" popover>
  <template shadowrootmode="open" shadowrootreferencetarget="popover">
    <div id="popover" popover></div>
  </template>
</x-popover>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const button = document.getElementById('button');
  const other = document.getElementById('other');
  const host = document.getElementById('host');
  const popover = host.shadowRoot.getElementById('popover');
  button.interestForElement = host;

  let hostOpen = 0;
  let popoverOpen = 0;
  host.addEventListener('beforetoggle', (event) => {
    if (event.composedPath()[0] === host && event.newState === 'open') ++hostOpen;
  });
  popover.addEventListener('beforetoggle', (event) => {
    if (event.composedPath()[0] === popover && event.newState === 'open') ++popoverOpen;
  });

  button.focus();
  other.focus();
  button.focus();
  other.focus();
  host.shadowRoot.referenceTarget = null;
  button.focus();
  other.focus();
  host.shadowRoot.referenceTarget = '';
  button.focus();

  return `${button.interestForElement === host}|${popoverOpen}|${hostOpen}`;
})()
"#,
        )
        .expect("interestForElement focus popover should respect referenceTarget state");

    assert_eq!(result, "false|1|1");
}
#[test]
fn tab_key_default_action_follows_shadow_slot_focus_order() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-focus-navigation.test/",
        r#"<!doctype html>
<input id="i0" tabindex="0">
<div id="x-foo">
  <input id="i2" slot="s2" tabindex="2">
  <input id="i1" slot="s2" tabindex="1">
  <template shadowrootmode="open">
    <input id="j1" tabindex="1">
    <slot id="s1" name="s1">
      <input id="k0" tabindex="0">
      <input id="k1" tabindex="1">
      <slot id="s2" name="s2"></slot>
      <slot id="s3" name="s3">
        <input id="l1">
      </slot>
      <div id="x-bar" tabindex="2">
        <template shadowrootmode="open">
          <input id="m2" tabindex="2">
          <input id="m1" tabindex="1">
        </template>
      </div>
    </slot>
    <input id="j2" tabindex="2">
  </template>
</div>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  function innermostActive(element = document.activeElement) {
    while (element && element.shadowRoot && element.shadowRoot.activeElement) {
      element = element.shadowRoot.activeElement;
    }
    return element;
  }
  function pressTab(shiftKey = false) {
    __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, shiftKey);
  }

  document.getElementById('i0').focus();
  const seen = [];
  for (let i = 0; i < 10; i++) {
    pressTab();
    seen.push(innermostActive().id);
  }
  pressTab(true);
  const reverse = innermostActive().id;
  return `${seen.join(',')}|${reverse}`;
})()
"#,
        )
        .expect("Tab default action should navigate shadow slot focus order");

    assert_eq!(result, "j1,j2,k1,x-bar,m1,m2,k0,i1,i2,l1|i2");
}
#[test]
fn tab_key_uses_slot_tabindex_as_scope_without_focusing_slot() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-focus-slot-scope.test/",
        r#"<!doctype html>
<div id="host">
  <div id="i2" slot="slot5" tabindex="4"></div>
  <template shadowrootmode="open">
    <slot name="slot1">
      <div id="i5" tabindex="0"></div>
      <div id="i4" tabindex="2"></div>
    </slot>
    <slot name="slot2" id="x1" tabindex="3">
      <div id="i3" tabindex="10"></div>
    </slot>
    <slot name="slot3" id="x2" tabindex="0">
      <div id="i6" tabindex="1"></div>
      <div id="i7" tabindex="1"></div>
    </slot>
    <slot name="slot4" id="x3" tabindex="1">
      <div id="i1" tabindex="5"></div>
    </slot>
    <slot name="slot5" id="x5" tabindex="2">
      <div id="ignored" tabindex="1"></div>
    </slot>
    <slot name="slot7" id="x7" tabindex="-1">
      <div id="skipped" tabindex="1"></div>
    </slot>
  </template>
</div>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  function innermostActive(element = document.activeElement) {
    while (element && element.shadowRoot && element.shadowRoot.activeElement) {
      element = element.shadowRoot.activeElement;
    }
    return element;
  }
  function pressTab(shiftKey = false) {
    __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, shiftKey);
  }
  const seen = [];
  for (let i = 0; i < 7; i++) {
    pressTab();
    seen.push((innermostActive() && innermostActive().id) || '');
  }
  pressTab(true);
  return `${seen.join(',')}|${innermostActive() && innermostActive().id}`;
})()
"#,
        )
        .expect("slot tabindex should order its focus scope without focusing the slot itself");

    assert_eq!(result, "i1,i2,i3,i4,i5,i6,i7|i6");
}
#[test]
fn tab_key_focuses_slot_with_generated_box() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-focus-slot-box.test/",
        r#"<!doctype html>
<div id="host">
  <template shadowrootmode="open">
    <slot id="slot" tabindex="0" style="display: inline-block;"></slot>
  </template>
  Content
</div>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, false);
  const host = document.getElementById('host');
  return host.shadowRoot.activeElement === host.shadowRoot.getElementById('slot');
})()
"#,
        )
        .expect("slot with a generated box should be sequentially focusable");

    assert_eq!(result, "true");
}
#[test]
fn tab_key_focuses_stylesheet_styled_slot_with_generated_box() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-focus-slot-stylesheet-box.test/",
        r#"<!doctype html>
<div id="host">
  <template shadowrootmode="open">
    <style>.focusable-slot { display: flex; }</style>
    <slot id="slot" class="focusable-slot" tabindex="0"></slot>
  </template>
  Content
</div>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, false);
  const host = document.getElementById('host');
  return [
    getComputedStyle(host.shadowRoot.getElementById('slot')).display,
    host.shadowRoot.activeElement === host.shadowRoot.getElementById('slot')
  ].join('|');
})()
"#,
        )
        .expect("slot styled from a shadow stylesheet should generate a focusable box");

    assert_eq!(result, "flex|true");
}
#[test]
fn tab_key_follows_nested_shadow_focus_scope_order() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-focus-nested-scope.test/",
        r#"<!doctype html>
<input id="i0" tabindex="0">
<div id="x-foo">
  <input id="i2" slot="s1" tabindex="2">
  <input id="i1" slot="s1" tabindex="1">
  <template shadowrootmode="open">
    <div id="x-bar" tabindex="4">
      <input id="j1" slot="s2" tabindex="1">
      <slot id="s1" name="s1" slot="s2"></slot>
      <input id="j0" slot="s2" tabindex="0">
      <input id="j3" slot="s2" tabindex="2">
      <div id="j4" slot="s2" tabindex="3">
        <input id="j2" tabindex="1">
      </div>
      <template shadowrootmode="open">
        <input id="k0" tabindex="0">
        <slot id="s2" name="s2"></slot>
        <input id="k1" tabindex="1">
      </template>
    </div>
    <input id="j6" tabindex="4">
    <input id="j5" tabindex="3">
  </template>
</div>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  function innermostActive(element = document.activeElement) {
    while (element && element.shadowRoot && element.shadowRoot.activeElement) {
      element = element.shadowRoot.activeElement;
    }
    return element;
  }
  function pressTab(shiftKey = false) {
    __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, shiftKey);
  }
  document.getElementById('i0').focus();
  const seen = [];
  for (let i = 0; i < 12; i++) {
    pressTab();
    seen.push((innermostActive() && innermostActive().id) || '');
  }
  pressTab(true);
  return `${seen.join(',')}|${innermostActive() && innermostActive().id}`;
})()
"#,
        )
        .expect("Tab default action should navigate nested shadow focus scope order");

    assert_eq!(result, "j5,x-bar,k1,k0,j1,j2,j3,j4,i1,i2,j0,j6|j0");
}
#[test]
fn tab_key_skips_scroll_host_with_shadow_focusable_descendant() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-scroll-descendant.test/",
        r#"<!doctype html>
<div id="scroller" style="overflow: auto; width: 1px; height: 1px">
  <template shadowrootmode="open">
    <button id="inner">Inner</button>
  </template>
</div>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  function innermostActive(element = document.activeElement) {
    while (element && element.shadowRoot && element.shadowRoot.activeElement)
      element = element.shadowRoot.activeElement;
    return element;
  }
  __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, false);
  return innermostActive() && innermostActive().id;
})()
"#,
        )
        .expect("scroll host with shadow focusable descendant should not take Tab first");

    assert_eq!(result, "inner");
}
#[test]
fn blur_on_non_delegates_shadow_host_does_not_clear_inner_focus() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-host-blur.test/",
        r#"<!doctype html>
<div id="host">
  <template shadowrootmode="open">
    <input id="inner">
  </template>
</div>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.getElementById('host');
  const inner = host.shadowRoot.getElementById('inner');
  inner.focus();
  host.blur();
  return host.shadowRoot.activeElement && host.shadowRoot.activeElement.id;
})()
"#,
        )
        .expect("non-delegates host blur should not clear focused shadow child");

    assert_eq!(result, "inner");
}
#[test]
fn tab_key_follows_nested_shadow_focus_pairwise_navigation() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-focus-pairwise-scope.test/",
        r#"<!doctype html>
<input id="i0" tabindex="0">
<div id="x-foo">
  <input id="i2" slot="s1" tabindex="2">
  <input id="i1" slot="s1" tabindex="1">
  <template data-mode="open">
    <div id="x-bar" tabindex="4">
      <input id="j1" slot="s2" tabindex="1">
      <slot id="s1" name="s1" slot="s2"></slot>
      <input id="j0" slot="s2" tabindex="0">
      <input id="j3" slot="s2" tabindex="2">
      <div id="j4" slot="s2" tabindex="3">
        <input id="j2" tabindex="1">
      </div>
      <template data-mode="open">
        <input id="k0" tabindex="0">
        <slot id="s2" name="s2"></slot>
        <input id="k1" tabindex="1">
      </template>
    </div>
    <input id="j6" tabindex="4">
    <input id="j5" tabindex="3">
  </template>
</div>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  function convertTemplatesToShadowRootsWithin(node) {
    const nodes = node.querySelectorAll('template');
    for (let i = 0; i < nodes.length; ++i) {
      const template = nodes[i];
      const parent = template.parentNode;
      parent.removeChild(template);
      const shadowRoot = parent.attachShadow({ mode: template.getAttribute('data-mode') || 'open' });
      shadowRoot.appendChild(document.importNode(template.content, true));
      convertTemplatesToShadowRootsWithin(shadowRoot);
    }
  }
  convertTemplatesToShadowRootsWithin(document.getElementById('x-foo'));
  function node(path) {
    let current = document;
    for (const part of path.split('/')) {
      current = (current.shadowRoot || current).getElementById(part);
    }
    return current;
  }
  function innermostActive(element = document.activeElement) {
    while (element && element.shadowRoot && element.shadowRoot.activeElement) {
      element = element.shadowRoot.activeElement;
    }
    return element;
  }
  function pressTab(shiftKey = false) {
    __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, shiftKey);
  }
  const paths = [
    'i0',
    'x-foo/j5',
    'x-foo/x-bar',
    'x-foo/x-bar/k1',
    'x-foo/x-bar/k0',
    'x-foo/j1',
    'x-foo/j2',
    'x-foo/j3',
    'x-foo/j4',
    'i1',
    'i2',
    'x-foo/j0',
    'x-foo/j6'
  ];
  const seen = [];
  for (let i = 0; i + 1 < paths.length; i++) {
    node(paths[i]).focus();
    pressTab();
    seen.push(`${paths[i]}>${innermostActive() && innermostActive().id}`);
  }
  const reverse = [];
  for (let i = paths.length - 1; i > 0; i--) {
    node(paths[i]).focus();
    pressTab(true);
    reverse.push(`${paths[i]}<${innermostActive() && innermostActive().id}`);
  }
  return `${seen.join(',')}|${reverse.join(',')}`;
})()
"#,
        )
        .expect("Tab default action should navigate nested shadow scope pairwise");

    assert_eq!(
        result,
        "i0>j5,x-foo/j5>x-bar,x-foo/x-bar>k1,x-foo/x-bar/k1>k0,x-foo/x-bar/k0>j1,x-foo/j1>j2,x-foo/j2>j3,x-foo/j3>j4,x-foo/j4>i1,i1>i2,i2>j0,x-foo/j0>j6|x-foo/j6<j0,x-foo/j0<i2,i2<i1,i1<j4,x-foo/j4<j3,x-foo/j3<j2,x-foo/j2<j1,x-foo/j1<k0,x-foo/x-bar/k0<k1,x-foo/x-bar/k1<x-bar,x-foo/x-bar<j5,x-foo/j5<i0"
    );
}
#[test]
fn tab_key_handles_negative_shadow_focus_scopes() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-focus-negative-scope.test/",
        r#"<!doctype html>
<input id="i0" tabindex="0">
<div id="x-foo">
  <input id="i2" slot="s1" tabindex="2">
  <input id="i1" slot="s1" tabindex="1">
  <template data-mode="open">
    <div id="x-bar" tabindex="4">
      <input id="j1" slot="s2" tabindex="1">
      <slot id="s1" name="s1" slot="s2"></slot>
      <input id="j0" slot="s2" tabindex="0">
      <input id="j3" slot="s2" tabindex="2">
      <div id="j4" slot="s2" tabindex="3">
        <input id="j2" tabindex="1">
      </div>
      <template data-mode="open">
        <input id="k0" tabindex="0">
        <slot id="s2" name="s2" tabindex="-1"></slot>
        <input id="k1" tabindex="1">
      </template>
    </div>
    <div id="to-be-ignored-host" tabindex="-1">
      <template data-mode="open">
        <input id="ignored-input-in-shadow-host1" tabindex="1">
        <input id="ignored-input-in-shadow-host2" tabindex="2">
      </template>
    </div>
    <input id="j6" tabindex="4">
    <input id="j5" tabindex="3">
  </template>
</div>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  function convertTemplatesToShadowRootsWithin(node) {
    const nodes = node.querySelectorAll('template');
    for (let i = 0; i < nodes.length; ++i) {
      const template = nodes[i];
      const parent = template.parentNode;
      parent.removeChild(template);
      const shadowRoot = parent.attachShadow({ mode: template.getAttribute('data-mode') || 'open' });
      shadowRoot.appendChild(document.importNode(template.content, true));
      convertTemplatesToShadowRootsWithin(shadowRoot);
    }
  }
  convertTemplatesToShadowRootsWithin(document.getElementById('x-foo'));
  function node(path) {
    let current = document;
    for (const part of path.split('/')) {
      current = (current.shadowRoot || current).getElementById(part);
    }
    return current;
  }
  function innermostActive(element = document.activeElement) {
    while (element && element.shadowRoot && element.shadowRoot.activeElement) {
      element = element.shadowRoot.activeElement;
    }
    return element;
  }
  function pressTab(shiftKey = false) {
    __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, shiftKey);
  }
  const first = [];
  const firstPaths = ['i0', 'x-foo/j5', 'x-foo/x-bar', 'x-foo/x-bar/k1', 'x-foo/x-bar/k0'];
  for (const path of firstPaths) {
    node(path).focus();
    pressTab();
    first.push(`${path}>${innermostActive() && innermostActive().id}`);
  }
  const second = [];
  const secondForward = [
    'x-foo/to-be-ignored-host/ignored-input-in-shadow-host1',
    'x-foo/to-be-ignored-host/ignored-input-in-shadow-host2'
  ];
  for (const path of secondForward) {
    node(path).focus();
    pressTab();
    second.push(`${path}>${innermostActive() && innermostActive().id}`);
  }
  const secondBackward = [
    'x-foo/to-be-ignored-host/ignored-input-in-shadow-host2',
    'x-foo/to-be-ignored-host/ignored-input-in-shadow-host1'
  ];
  for (const path of secondBackward) {
    node(path).focus();
    pressTab(true);
    second.push(`${path}<${innermostActive() && innermostActive().id}`);
  }
  return `${first.join(',')}|${second.join(',')}`;
})()
"#,
        )
        .expect("Tab default action should handle negative shadow focus scopes");

    assert_eq!(
        result,
        "i0>j5,x-foo/j5>x-bar,x-foo/x-bar>k1,x-foo/x-bar/k1>k0,x-foo/x-bar/k0>j6|x-foo/to-be-ignored-host/ignored-input-in-shadow-host1>ignored-input-in-shadow-host2,x-foo/to-be-ignored-host/ignored-input-in-shadow-host2>j6,x-foo/to-be-ignored-host/ignored-input-in-shadow-host2<ignored-input-in-shadow-host1,x-foo/to-be-ignored-host/ignored-input-in-shadow-host1<k0"
    );
}
#[test]
fn tab_key_skips_delegates_focus_host_but_uses_host_tabindex_scope() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-focus-delegates-scope.test/",
        r#"<!doctype html>
<div id="sandbox"></div>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  function pressTab(shiftKey = false) {
    __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, shiftKey);
  }
  function innermostActive(element = document.activeElement) {
    while (element && element.shadowRoot && element.shadowRoot.activeElement) {
      element = element.shadowRoot.activeElement;
    }
    return element;
  }
  function setup(tabindex) {
    sandbox.innerHTML = `
      <input id="before">
      <div id="host"><input id="inner"></div>
      <input id="after">
    `;
    const host = document.getElementById('host');
    const inner = document.getElementById('inner');
    const root = host.attachShadow({ mode: 'open', delegatesFocus: true });
    root.appendChild(inner);
    if (tabindex !== null) {
      host.tabIndex = tabindex;
    }
    return { host, inner };
  }

  const zero = setup(0);
  document.getElementById('before').focus();
  pressTab();
  const zeroForward = `${document.activeElement.id}:${innermostActive().id}`;
  pressTab();
  const zeroAfter = innermostActive().id;
  pressTab(true);
  const zeroBackward = `${document.activeElement.id}:${innermostActive().id}`;

  const positive = setup(1);
  pressTab();
  const positiveFirst = `${document.activeElement.id}:${innermostActive().id}`;
  pressTab();
  const positiveNext = innermostActive().id;

  const negative = setup(-1);
  document.getElementById('before').focus();
  pressTab();
  const negativeSkip = innermostActive().id;

  return [
    zeroForward,
    zeroAfter,
    zeroBackward,
    positiveFirst,
    positiveNext,
    negativeSkip
  ].join('|');
})()
"#,
        )
        .expect("Tab default action should use delegatesFocus host as scope only");

    assert_eq!(
        result,
        "host:inner|after|host:inner|host:inner|before|after"
    );
}
#[test]
fn tab_key_keeps_shadow_host_focus_style_across_delegates_focus_siblings() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-focus-delegates-highlight.test/",
        r#"<!doctype html>
<section id="menus"></section>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  function pressTab(shiftKey = false) {
    __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, shiftKey);
  }
  function addMenu(id, delegatesFocus) {
    const host = document.createElement('x-menu');
    host.id = id;
    host.tabIndex = 0;
    if (delegatesFocus) {
      host.setAttribute('delegatesFocus', '');
    }
    menus.appendChild(host);
    const root = host.attachShadow({ mode: 'open', delegatesFocus });
    const style = document.createElement('style');
    style.textContent = `
      :host { background-color: #aaa; }
      :host(:focus) { background-color: #ccc; }
      li { display: inline-block; background-color: #eee; }
      li:focus { background-color: #fff; }
    `;
    root.appendChild(style);
    for (let i = 0; i < 3; i++) {
      const item = document.createElement('li');
      item.tabIndex = 0;
      item.textContent = `${id}-${i}`;
      root.appendChild(item);
    }
    return host;
  }
  const ids = ['XMenu1', 'XMenu2', 'XMenu3', 'XMenu4'];
  const hosts = [
    addMenu('XMenu1', false),
    addMenu('XMenu2', true),
    addMenu('XMenu3', true),
    addMenu('XMenu4', true)
  ];
  function colors() {
    return ids.map(id => getComputedStyle(document.getElementById(id)).backgroundColor).join(',');
  }
  const seen = [];
  hosts[0].focus();
  for (let group = 0; group < 4; group++) {
    for (let i = 0; i < 3; i++) {
      pressTab();
    }
    seen.push(`${document.activeElement.id}:${colors()}`);
  }
  for (let group = 0; group < 3; group++) {
    for (let i = 0; i < 3; i++) {
      pressTab(true);
    }
    seen.push(`${document.activeElement.id}:${colors()}`);
  }
  return seen.join('|');
})()
"#,
        )
        .expect("delegatesFocus sibling host focus styles should follow tab navigation");

    assert_eq!(
        result,
        "XMenu1:rgb(204, 204, 204),rgb(170, 170, 170),rgb(170, 170, 170),rgb(170, 170, 170)|\
XMenu2:rgb(170, 170, 170),rgb(204, 204, 204),rgb(170, 170, 170),rgb(170, 170, 170)|\
XMenu3:rgb(170, 170, 170),rgb(170, 170, 170),rgb(204, 204, 204),rgb(170, 170, 170)|\
XMenu4:rgb(170, 170, 170),rgb(170, 170, 170),rgb(170, 170, 170),rgb(204, 204, 204)|\
XMenu3:rgb(170, 170, 170),rgb(170, 170, 170),rgb(204, 204, 204),rgb(170, 170, 170)|\
XMenu2:rgb(170, 170, 170),rgb(204, 204, 204),rgb(170, 170, 170),rgb(170, 170, 170)|\
XMenu1:rgb(204, 204, 204),rgb(170, 170, 170),rgb(170, 170, 170),rgb(170, 170, 170)"
    );
}
#[test]
fn command_for_element_popover_respects_reference_target_states() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-commandfor-popover.test/",
        r#"<!doctype html>
<button id="button" command="toggle-popover">Command</button>
<x-popover id="host" popover>
  <template shadowrootmode="open" shadowrootreferencetarget="popover">
    <div id="popover" popover></div>
  </template>
</x-popover>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const button = document.getElementById('button');
  const host = document.getElementById('host');
  const popover = host.shadowRoot.getElementById('popover');
  button.commandForElement = host;
  const initialGetterIsHost = button.commandForElement === host;
  const initialReferenceTarget = host.shadowRoot.referenceTarget;

  let hostOpen = 0;
  let popoverOpen = 0;
  host.addEventListener('beforetoggle', (event) => {
    if (event.composedPath()[0] === host && event.newState === 'open') ++hostOpen;
  });
  popover.addEventListener('beforetoggle', (event) => {
    if (event.composedPath()[0] === popover && event.newState === 'open') ++popoverOpen;
  });

  button.click();
  popover.hidePopover();
  host.shadowRoot.referenceTarget = null;
  button.click();
  host.hidePopover();
  host.shadowRoot.referenceTarget = '';
  button.click();

  return `${initialGetterIsHost}|${initialReferenceTarget}|${button.commandForElement === host}|${host.shadowRoot.referenceTarget}|${popoverOpen}|${hostOpen}`;
})()
"#,
        )
        .expect("commandForElement popover should respect referenceTarget state");

    assert_eq!(result, "true|popover|false||1|1");
}
#[test]
fn radio_groups_are_scoped_to_shadow_root_tree() {
    let mut vm = new_storage_test_vm("https://shadow-radio-groups.test/");

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

  const lightRadio1 = document.createElement('input');
  lightRadio1.type = 'radio';
  lightRadio1.name = 'group';
  lightRadio1.id = 'lightRadio1';
  const lightRadio2 = document.createElement('input');
  lightRadio2.type = 'radio';
  lightRadio2.name = 'group';
  lightRadio2.id = 'lightRadio2';
  const host = document.createElement('div');
  document.body.append(lightRadio1, lightRadio2, host);

  const shadow = host.attachShadow({ mode: 'closed' });
  const shadowRadio1 = document.createElement('input');
  shadowRadio1.type = 'radio';
  shadowRadio1.name = 'group';
  shadowRadio1.id = 'shadowRadio1';
  const shadowRadio2 = document.createElement('input');
  shadowRadio2.type = 'radio';
  shadowRadio2.name = 'group';
  shadowRadio2.id = 'shadowRadio2';
  shadow.append(shadowRadio1, shadowRadio2);

  const snapshot = () => [
    lightRadio1.checked,
    lightRadio2.checked,
    shadowRadio1.checked,
    shadowRadio2.checked
  ].join(',');

  const states = [snapshot()];
  lightRadio1.click();
  states.push(snapshot());
  lightRadio2.click();
  states.push(snapshot());
  shadowRadio1.click();
  states.push(snapshot());
  shadowRadio2.click();
  states.push(snapshot());

  shadowRadio2.remove();
  shadow.appendChild(shadowRadio2);
  shadowRadio2.click();
  states.push(snapshot());
  shadowRadio1.click();
  states.push(snapshot());

  shadowRadio2.checked = true;
  states.push(snapshot());
  lightRadio1.checked = true;
  states.push(snapshot());

  return states.join('|');
})()
"#,
        )
        .expect("radio groups should be scoped to the containing tree root");

    assert_eq!(
        result,
        "false,false,false,false|true,false,false,false|false,true,false,false|false,true,true,false|false,true,false,true|false,true,false,true|false,true,true,false|false,true,false,true|true,false,false,true"
    );
}
#[test]
fn highlight_registry_hits_shadow_and_light_dom_text_ranges() {
    let mut vm = new_storage_test_vm("https://highlight-registry-shadow.test/");
    vm.force_fresh_layout_reads_for_test();

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
  const container = document.createElement('span');
  document.body.appendChild(container);
  container.setHTMLUnsafe(`
    <div id="host">
      <template shadowrootmode=open>
        <span>0123456789</span>
      </template>
    </div>`);
  const shadowRoot = container.querySelector('#host').shadowRoot;
  const spanInShadowDOM = shadowRoot.querySelector('span');
  let range = new Range();
  range.setStart(spanInShadowDOM.childNodes[0], 2);
  range.setEnd(spanInShadowDOM.childNodes[0], 10);
  const highlight = new Highlight(range);
  CSS.highlights.set('example-highlight', highlight);
  const rect = spanInShadowDOM.getBoundingClientRect();
  const x = rect.left + 3 * (rect.width / spanInShadowDOM.textContent.length);
  const y = rect.top + rect.height / 2;
  const shadowResults = CSS.highlights.highlightsFromPoint(x, y, { shadowRoots: [shadowRoot] });
  const secondRange = new Range();
  secondRange.setStart(spanInShadowDOM.childNodes[0], 5);
  secondRange.setEnd(spanInShadowDOM.childNodes[0], 10);
  CSS.highlights.set('example-highlight-2', new Highlight(secondRange));

  container.setHTMLUnsafe(
    `0123456789<div id=host>
      <template shadowrootmode=open>
        <span>0123456789</span>
      </template>
    </div>0123456789`);
  const nestedShadowRoot = container.querySelector('#host').shadowRoot;
  const nestedSpan = nestedShadowRoot.querySelector('span');
  const nestedRect = nestedSpan.getBoundingClientRect();
  const nestedCharacterWidth = nestedRect.width / nestedSpan.textContent.length;
  const nestedX = nestedRect.left + nestedCharacterWidth;
  const textPoint = (node) => {
    const textRange = new Range();
    textRange.selectNodeContents(node);
    const rect = textRange.getBoundingClientRect();
    return { x: rect.left + rect.width / node.data.length, y: rect.top + rect.height / 2 };
  };
  const firstPoint = textPoint(container.childNodes[0]);
  const thirdPoint = textPoint(container.childNodes[2]);
  range = new Range();
  range.setStart(container.childNodes[0], 0);
  range.setEnd(container.childNodes[2], 5);
  const lightHighlight = new Highlight(range);
  CSS.highlights.set('example-highlight', lightHighlight);
  const lightFirstResults = CSS.highlights.highlightsFromPoint(
    firstPoint.x,
    firstPoint.y
  );
  const lightShadowResults = CSS.highlights.highlightsFromPoint(
    nestedX,
    nestedRect.top + nestedRect.height / 2
  );
  const lightThirdResults = CSS.highlights.highlightsFromPoint(
    thirdPoint.x,
    thirdPoint.y
  );

  return JSON.stringify({
    shadowLength: shadowResults.length,
    shadowIdentity: shadowResults[0] && shadowResults[0].highlight === highlight,
    lightFirstLength: lightFirstResults.length,
    lightFirstIdentity: lightFirstResults[0] && lightFirstResults[0].highlight === lightHighlight,
    lightShadowLength: lightShadowResults.length,
    lightThirdLength: lightThirdResults.length,
    lightThirdIdentity: lightThirdResults[0] && lightThirdResults[0].highlight === lightHighlight
  });
})()
"#,
        )
        .expect("HighlightRegistry hit testing should evaluate");

    assert_eq!(
        result,
        r#"{"shadowLength":1,"shadowIdentity":true,"lightFirstLength":1,"lightFirstIdentity":true,"lightShadowLength":0,"lightThirdLength":1,"lightThirdIdentity":true}"#
    );
}

#[test]
fn highlight_registry_hits_cross_text_node_range_and_static_range() {
    let mut vm = new_storage_test_vm("https://highlight-registry-range-static-range.test/");

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
  CSS.highlights.clear();
  const span1 = document.createElement('span');
  span1.textContent = '0123456789';
  const span2 = document.createElement('span');
  span2.textContent = '0123456789';
  document.body.append(span1, document.createElement('br'), span2);
  const text1 = span1.firstChild;
  const text2 = span2.firstChild;

  const liveRange = new Range();
  liveRange.setStart(text1, 5);
  liveRange.setEnd(text1, 10);
  const staticRange = new StaticRange({
    startContainer: text1,
    startOffset: 2,
    endContainer: text2,
    endOffset: 8
  });
  const highlight = new Highlight(liveRange, staticRange);
  CSS.highlights.set('example-highlight', highlight);

  const rect = span1.getBoundingClientRect();
  const characterWidth = rect.width / span1.textContent.length;
  const characterHeight = rect.height;
  const names = new Map([[liveRange, 'live'], [staticRange, 'static']]);
  const probe = (x, y) => {
    const results = CSS.highlights.highlightsFromPoint(x, y);
    if (results.length === 0) return '0';
    return results.length + ':' + results[0].ranges.map(range => names.get(range)).join(',');
  };

  return [
    probe(rect.left + 3 * characterWidth, rect.top + characterHeight / 2),
    probe(rect.left + 7 * characterWidth, rect.top + characterHeight / 2),
    probe(rect.left + characterWidth, rect.top + 1.5 * characterHeight),
    probe(rect.left + 9 * characterWidth, rect.top + 1.5 * characterHeight)
  ].join('|');
})()
"#,
        )
        .expect("HighlightRegistry Range/StaticRange hit testing should evaluate");

    assert_eq!(result, "1:static|1:live,static|1:static|0");
}

#[test]
fn highlight_registry_highlights_from_point_validates_args_and_child_window_surface() {
    let mut vm = new_storage_test_vm("https://highlight-registry-child-window.test/");

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
  CSS.highlights.clear();
  const thrown = fn => {
    try {
      fn();
      return 'no';
    } catch (error) {
      return error && error.name;
    }
  };

  const frame = document.createElement('iframe');
  frame.style.display = 'none';
  document.body.appendChild(frame);
  const win = frame.contentWindow;
  const doc = win.document;
  const span = doc.createElement('span');
  span.textContent = '0123456789';
  doc.body.appendChild(span);
  const range = doc.createRange();
  range.setStart(span.firstChild, 0);
  range.setEnd(span.firstChild, 10);
  const highlight = new win.Highlight(range);
  win.CSS.highlights.clear();
  win.CSS.highlights.set('example-highlight', highlight);
  const results = win.CSS.highlights.highlightsFromPoint(5, 5);

  return [
    thrown(() => CSS.highlights.highlightsFromPoint('asdf', 10)),
    thrown(() => CSS.highlights.highlightsFromPoint(10)),
    thrown(() => CSS.highlights.highlightsFromPoint()),
    thrown(() => CSS.highlights.highlightsFromPoint(10, 10, 'asdf')),
    typeof win.Highlight,
    typeof win.CSS,
    typeof win.CSS.highlights,
    results.length
  ].join('|');
})()
"#,
        )
        .expect("HighlightRegistry child-window hit testing surface should evaluate");

    assert_eq!(
        result,
        "TypeError|TypeError|TypeError|TypeError|function|object|object|0"
    );
}

#[test]
fn highlight_and_registry_collection_surfaces_use_live_webidl_iteration() {
    let mut vm = new_storage_test_vm("https://highlight-registry-collection-surface.test/");

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
  CSS.highlights.clear();
  const range = document.createRange();
  range.setStart(document.body, 0);
  range.setEnd(document.body, 0);
  const staticRange = new StaticRange({
    startContainer: document.body,
    startOffset: 0,
    endContainer: document.body,
    endOffset: 0
  });
  const setIteratorPrototype = Reflect.getPrototypeOf(new Set().values());
  const mapIteratorPrototype = Reflect.getPrototypeOf(new Map().entries());

  const illegal = (() => {
    try {
      new HighlightRegistry();
      return 'no';
    } catch (error) {
      return error && error.name;
    }
  })();

  const highlight = new Highlight(staticRange);
  const highlightIterator = highlight[Symbol.iterator]();
  const firstHighlightValue = highlightIterator.next().value === staticRange;
  highlight.add(range);
  highlight.delete(staticRange);
  const secondHighlightValue = highlightIterator.next().value === range;
  const highlightEntry = highlight.entries().next().value;
  let highlightForEachCount = 0;
  highlight.forEach((value, key, owner) => {
    if (value === range && key === range && owner === highlight) {
      highlightForEachCount++;
    }
  });
  highlight.type = 'grammar-error';
  const acceptedType = highlight.type;
  highlight.type = 'Spelling-error';
  const rejectedType = highlight.type;

  const registry = CSS.highlights;
  const registryIterator = registry[Symbol.iterator]();
  registry.set('first', highlight);
  const firstRegistryValue = registryIterator.next().value;
  const secondHighlight = new Highlight(range);
  registry.set('second', secondHighlight);
  registry.delete('first');
  const secondRegistryValue = registryIterator.next().value;
  let registryForEachCount = 0;
  registry.forEach((value, key, owner) => {
    if (value === secondHighlight && key === 'second' && owner === registry) {
      registryForEachCount++;
    }
  });

  delete Set.prototype.size;
  Set.prototype.entries = null;
  Set.prototype.values = Symbol();
  Object.freeze(Set.prototype);
  delete Map.prototype.size;
  Map.prototype.entries = null;
  Map.prototype.values = 1;
  Object.freeze(Map.prototype);

  const iteratorShape = (iterator, parent, tagValue) => {
    const prototype = Reflect.getPrototypeOf(iterator);
    const next = Object.getOwnPropertyDescriptor(prototype, 'next');
    const tag = Object.getOwnPropertyDescriptor(prototype, Symbol.toStringTag);
    return (
      Reflect.getPrototypeOf(prototype) === parent &&
      iterator[Symbol.iterator]() === iterator &&
      !Object.hasOwn(iterator, 'next') &&
      !Object.hasOwn(iterator, Symbol.iterator) &&
      !Object.hasOwn(prototype, 'constructor') &&
      next?.enumerable === true &&
      next?.writable === true &&
      next?.configurable === true &&
      tag?.value === tagValue &&
      tag?.enumerable === false &&
      tag?.writable === false &&
      tag?.configurable === true
    );
  };

  return [
    typeof HighlightRegistry,
    illegal,
    Object.getPrototypeOf(registry) === HighlightRegistry.prototype,
    highlight.size,
    firstHighlightValue,
    secondHighlightValue,
    highlightEntry[0] === range && highlightEntry[1] === range,
    highlight.keys().next().value === range,
    highlightForEachCount,
    acceptedType,
    rejectedType,
    registry.size,
    firstRegistryValue[0] === 'first' && firstRegistryValue[1] === highlight,
    secondRegistryValue[0] === 'second' && secondRegistryValue[1] === secondHighlight,
    registry.keys().next().value,
    registry.values().next().value === secondHighlight,
    registryForEachCount,
    highlight.has(range),
    registry.has('second'),
    iteratorShape(highlightIterator, setIteratorPrototype, 'Highlight Iterator'),
    iteratorShape(registryIterator, mapIteratorPrototype, 'HighlightRegistry Iterator')
  ].join('|');
})()
"#,
        )
        .expect("Highlight and HighlightRegistry collection surfaces should evaluate");

    assert_eq!(
        result,
        "function|TypeError|true|1|true|true|true|true|1|grammar-error|grammar-error|1|true|true|second|true|1|true|true|true|true"
    );
}

#[test]
fn highlight_setlike_add_and_registry_reset_preserve_order() {
    let mut vm = new_storage_test_vm("https://highlight-setlike.test/");

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
  CSS.highlights.clear();
  const span = document.createElement('span');
  span.textContent = '0123456789';
  document.body.appendChild(span);
  const range = new Range();
  range.setStart(span.childNodes[0], 0);
  range.setEnd(span.childNodes[0], 10);

  const setlike = new Highlight();
  const returned = setlike.add(range, range).add(range);

  const first = new Highlight(range);
  const replacement = new Highlight(range);
  const third = new Highlight(range);
  const fourth = new Highlight(range);
  CSS.highlights.set('first', first);
  CSS.highlights.set('replace-me', new Highlight(range));
  CSS.highlights.set('third', third);
  CSS.highlights.set('fourth', fourth);
  CSS.highlights.set('replace-me', replacement);

  const rect = span.getBoundingClientRect();
  const names = new Map([
    [first, 'first'],
    [replacement, 'replacement'],
    [third, 'third'],
    [fourth, 'fourth']
  ]);
  const order = CSS.highlights
    .highlightsFromPoint(rect.left + 1, rect.top + rect.height / 2)
    .map(entry => names.get(entry.highlight))
    .join(',');

  return JSON.stringify({
    size: setlike.size,
    returned: returned === setlike,
    has: setlike.has(range),
    order
  });
})()
"#,
        )
        .expect("Highlight setlike and registry order probe should evaluate");

    assert_eq!(
        result,
        r#"{"size":1,"returned":true,"has":true,"order":"fourth,third,replacement,first"}"#
    );
}
