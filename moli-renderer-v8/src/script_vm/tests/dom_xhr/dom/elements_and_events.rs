use super::*;

#[test]
fn detached_html_elements_reflect_common_attributes() {
    let mut vm = new_storage_test_vm("https://detached-reflected-attrs.test/path/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body>' +
      '<input id="input" name="token" value="seed" checked disabled>' +
      '<img id="img" name="hero" src="img/a.png">' +
      '<script id="script" src="/app.js"></script>' +
      '<iframe id="frame" name="child" src="child.html"></iframe>' +
      '<button id="button" name="go" value="yes" disabled></button>' +
      '<textarea id="textarea" name="bio"></textarea>' +
      '<select id="select" name="choice"><option id="option" value="a" disabled>A</option></select>' +
      '<section id="section"></section>' +
    '</body></html>',
    'text/html'
  );
  const input = doc.getElementById('input');
  const img = doc.getElementById('img');
  const script = doc.getElementById('script');
  const frame = doc.getElementById('frame');
  const button = doc.getElementById('button');
  const textarea = doc.getElementById('textarea');
  const select = doc.getElementById('select');
  const option = doc.getElementById('option');
  const section = doc.getElementById('section');
  const created = doc.createElement('input');
  created.name = 'created-name';
  created.value = 42;
  created.checked = true;
  created.disabled = true;
  input.checked = false;
  input.disabled = false;
  img.src = '/asset.png';
  button.disabled = false;
  textarea.value = 'typed';
  select.value = 'b';
  option.disabled = false;
  return [
    input.name,
    input.value,
    input.checked,
    input.disabled,
    input.getAttribute('checked') === null,
    input.getAttribute('disabled') === null,
    created.name,
    created.value,
    created.checked,
    created.disabled,
    created.getAttribute('checked'),
    created.getAttribute('disabled'),
    img.name,
    img.getAttribute('src'),
    img.src,
    script.getAttribute('src'),
    script.src,
    frame.name,
    frame.getAttribute('src'),
    frame.src,
    button.name,
    button.value,
    button.disabled,
    button.getAttribute('disabled') === null,
    textarea.name,
    textarea.value,
    textarea.getAttribute('value'),
    select.name,
    select.value,
    select.getAttribute('value'),
    option.value,
    option.disabled,
    option.getAttribute('disabled') === null,
    'name' in section,
    typeof section.value,
    typeof section.checked,
    typeof section.src
  ].join('|');
})()
"#,
        )
        .expect("detached HTML elements should reflect common attributes");

    assert_eq!(
        result,
        "token|seed|false|false|false|true|created-name|42|true|true|||hero|/asset.png|https://detached-reflected-attrs.test/asset.png|/app.js|https://detached-reflected-attrs.test/app.js|child|child.html|https://detached-reflected-attrs.test/path/child.html|go|yes|false|true|bio|typed||choice|||a|false|true|false|undefined|undefined|undefined"
    );
}

#[test]
fn live_element_name_accessor_is_writable() {
    let mut vm = new_storage_test_vm("https://live-name-attr.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const input = document.createElement('input');
  input.name = 'live-token';
  return [input.name, input.getAttribute('name')].join('|');
})()
"#,
        )
        .expect("live element name accessor should evaluate");

    assert_eq!(result, "live-token|live-token");
}

#[test]
fn option_name_assignment_is_an_expando_and_does_not_reflect_the_content_attribute() {
    let mut vm = new_storage_test_vm("https://option-name.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const option = document.createElement('option');
  option.setAttribute('name', 'content-name');
  const before = option.name;
  option.name = 'expando-name';
  const descriptor = Object.getOwnPropertyDescriptor(option, 'name');
  return JSON.stringify({
    prototypeHasName: Object.prototype.hasOwnProperty.call(HTMLOptionElement.prototype, 'name'),
    before: before === undefined ? 'undefined' : before,
    expando: option.name,
    attribute: option.getAttribute('name'),
    descriptor: [
      descriptor.value,
      descriptor.enumerable,
      descriptor.writable,
      descriptor.configurable
    ]
  });
})()
"#,
        )
        .expect("option name expando probe should evaluate");

    assert_eq!(
        result,
        r#"{"prototypeHasName":false,"before":"undefined","expando":"expando-name","attribute":"content-name","descriptor":["expando-name",true,true,true]}"#
    );
}

#[test]
fn detached_child_document_elements_expose_focus_and_blur() {
    let mut vm = new_storage_test_vm("https://child-window-detached-focus.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<body></body>';
  (document.body || document.documentElement || document).appendChild(frame);
  const input = frame.contentDocument.createElement('input');
  let status = 'ok';
  try {
    input.focus();
    input.blur();
  } catch (error) {
    status = error && error.message;
  }
  return [
    input instanceof HTMLElement,
    input instanceof frame.contentWindow.HTMLElement,
    input instanceof frame.contentWindow.HTMLInputElement,
    Object.getPrototypeOf(input) === frame.contentWindow.HTMLInputElement.prototype,
    typeof input.focus,
    typeof input.blur,
    status
  ].join('|');
})()
"#,
        )
        .expect("detached child document elements should expose focus/blur");

    assert_eq!(result, "false|true|true|true|function|function|ok");
}

#[test]
fn dom_parser_detached_elements_dispatch_local_events() {
    let mut vm = new_storage_test_vm("https://dom-parser-detached-events.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><div id="parent"><button id="button"></button></div></body></html>',
    'text/html'
  );
  const parent = doc.getElementById('parent');
  const button = doc.getElementById('button');
  const order = [];
  parent.addEventListener('click', (event) => {
    order.push('capture:' + (event.target === button) + ':' + (event.currentTarget === parent));
  }, true);
  button.addEventListener('click', (event) => {
    order.push('target:' + (event.target === button) + ':' + (event.currentTarget === button) + ':' + (event.composedPath()[0] === button));
  });
  parent.addEventListener('click', () => order.push('bubble'));
  button.click();
  return order.join('|');
})()
"#,
        )
        .expect("DOMParser detached elements should dispatch local click events");

    assert_eq!(result, "capture:true:true|target:true:true:true|bubble");
}

#[test]
fn dom_parser_detached_iframe_load_ignores_tampered_non_elements() {
    let mut vm = new_storage_test_vm("https://dom-parser-detached-iframe-load-guard.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const text = doc.createTextNode('not an iframe');
  Object.defineProperty(text, 'localName', { value: 'iframe' });
  let textLoads = 0;
  text.addEventListener('load', () => ++textLoads);
  doc.body.appendChild(text);

  const iframe = doc.createElement('iframe');
  let iframeLoads = 0;
  iframe.addEventListener('load', () => ++iframeLoads);
  doc.body.appendChild(iframe);
  return `${textLoads}|${iframeLoads}`;
})()
"#,
        )
        .expect("DOMParser detached iframe load should guard node type");

    assert_eq!(result, "0|1");
}

#[test]
fn dom_parser_detached_iframe_window_declares_own_methods() {
    let mut vm = new_storage_test_vm("https://dom-parser-detached-window-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const iframe = doc.createElement('iframe');
  doc.body.appendChild(iframe);
  const win = iframe.contentWindow;
  const shape = name => {
    const descriptor = Object.getOwnPropertyDescriptor(win, name);
    const value = descriptor && descriptor.value;
    return [
      typeof value,
      value && value.name,
      value && value.length,
      descriptor && descriptor.enumerable,
      descriptor && descriptor.configurable,
      descriptor && descriptor.writable,
      /\[native code\]/.test(String(value))
    ].join(':');
  };
  return [
    Object.prototype.toString.call(win),
    shape('postMessage'),
    shape('open'),
    shape('blur'),
    shape('find'),
    shape('stop'),
    shape('print'),
    win.find('needle'),
    String(win.blur()),
    String(win.print())
  ].join('|');
})()
"#,
        )
        .expect("DOMParser detached iframe window methods should evaluate");

    assert_eq!(
        result,
        "[object Window]|function:postMessage:1:false:true:true:true|function:open:0:false:true:true:true|function:blur:0:false:true:true:true|function:find:0:false:true:true:true|function:stop:0:false:true:true:true|function:print:0:false:true:true:true|false|undefined|undefined"
    );
}

#[test]
fn dom_parser_detached_iframe_adopted_node_loads_after_insert() {
    let mut vm = new_storage_test_vm("https://dom-parser-detached-iframe-import-sync.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sourceDoc = new DOMParser().parseFromString(
    '<html><body><iframe></iframe></body></html>',
    'text/html'
  );
  const targetDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const source = sourceDoc.querySelector('iframe');
  let sourceListenerLoads = 0;
  const handlerTargets = [];
  source.onload = function() {
    handlerTargets.push(this.ownerDocument === targetDoc ? 'imported' : 'source');
  };
  source.addEventListener('load', () => ++sourceListenerLoads);
  targetDoc.body.appendChild(source);
  const imported = targetDoc.querySelector('iframe');

  return [
    !!imported,
    sourceListenerLoads,
    handlerTargets.join(','),
    source.contentDocument === imported.contentDocument,
    source.contentWindow === imported.contentWindow
  ].join('|');
})()
"#,
        )
        .expect("DOMParser detached iframe import source sync should evaluate");

    assert_eq!(result, "true|1|imported|true|true");
}

#[test]
fn dom_parser_detached_event_target_handles_remove_once_prevent_default_and_exceptions() {
    let mut vm = new_storage_test_vm("https://dom-parser-detached-events-edge.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body><div id="target"></div></body></html>', 'text/html');
  const target = doc.getElementById('target');
  const out = [];
  function removed() { out.push('removed'); }
  target.addEventListener('custom', removed);
  target.removeEventListener('custom', removed);
  target.addEventListener('custom', { handleEvent() { out.push('object-once'); } }, { once: true });
  target.dispatchEvent(new Event('custom'));
  target.dispatchEvent(new Event('custom'));
  target.addEventListener('cancelable', (event) => event.preventDefault());
  const allowed = target.dispatchEvent(new Event('cancelable', { cancelable: true }));
  target.addEventListener('boom', () => { throw new Error('detached listener boom'); });
  target.addEventListener('boom', () => out.push('after-throw'));
  const boomAllowed = target.dispatchEvent(new Event('boom'));
  return [out.join(','), allowed, boomAllowed].join('|');
})()
"#,
        )
        .expect("DOMParser detached event dispatch should match EventTarget edge behavior");

    assert_eq!(result, "object-once,after-throw|false|true");
}

#[test]
fn constructed_event_target_exposes_composed_path_during_dispatch() {
    let mut vm = new_storage_test_vm("https://event-target-constructible.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = new EventTarget();
  const event = new Event('custom');
  const observed = [];
  target.addEventListener('custom', (event) => {
    observed.push(event.target === target);
    observed.push(event.currentTarget === target);
    observed.push(event.composedPath().length);
    observed.push(event.composedPath()[0] === target);
    event.initEvent('mutated', true, true);
    observed.push(event.type);
    observed.push(event.bubbles);
    observed.push(event.cancelable);
  }, { once: true });
  const allowed = target.dispatchEvent(event);
  observed.push(allowed);
  observed.push(event.currentTarget === null);
  observed.push(event.composedPath().length);
  target.dispatchEvent(event);
  return observed.join('|');
})()
"#,
        )
        .expect("constructed EventTarget should expose composedPath while dispatching");

    assert_eq!(result, "true|true|1|true|custom|false|false|true|true|0");
}

#[test]
fn dom_parser_detached_event_handler_properties_dispatch_through_local_events() {
    let mut vm = new_storage_test_vm("https://dom-parser-detached-handler-properties.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body><button id="target"></button></body></html>', 'text/html');
  const target = doc.getElementById('target');
  const out = [];
  out.push(target.onclick === null);
  target.onclick = function(event) {
    out.push('first:' + (this === target) + ':' + event.type + ':' + (event.currentTarget === target));
  };
  out.push(typeof target.onclick);
  target.click();
  target.onclick = function() { out.push('second'); };
  target.addEventListener('click', () => out.push('listener'));
  target.click();
  target.onclick = null;
  out.push(target.onclick === null);
  target.click();
  return out.join('|');
})()
"#,
        )
        .expect("DOMParser detached event handler properties should dispatch local events");

    assert_eq!(
        result,
        "true|function|first:true:click:true|second|listener|true|listener"
    );
}
