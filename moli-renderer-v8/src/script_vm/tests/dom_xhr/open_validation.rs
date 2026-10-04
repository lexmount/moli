use super::*;

const OPEN_METHOD_PROBE: &str = r#"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const url = 'http://example.test/resource';
  const expectError = (action, name, code) => {
    let caught;
    try { action(); } catch (error) { caught = error; }
    assert(caught && caught.name === name, `${name}: got ${caught}`);
    if (code !== undefined)
      assert(caught instanceof DOMException && caught.code === code, 'DOMException identity and code');
    else
      assert(caught instanceof TypeError, 'WebIDL conversion throws TypeError');
  };
  const tokenCharacters = "!#$%&'*+-.^_`|~0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
  for (let byte = 0; byte < 256; ++byte) {
    const method = String.fromCharCode(byte);
    const xhr = new XMLHttpRequest();
    let events = 0;
    xhr.onreadystatechange = () => ++events;
    if (tokenCharacters.includes(method)) {
      xhr.open(method, url);
      assert(xhr.readyState === 1 && events === 1, 'valid token opens the request');
    } else {
      expectError(() => xhr.open(method, url), 'SyntaxError', 12);
      assert(xhr.readyState === 0 && events === 0, 'invalid token preserves UNSENT');
    }
  }
  for (const method of ['', 'GET ', 'G ET', 'GET\n', 'GET\0', 'TRACE '])
    expectError(() => new XMLHttpRequest().open(method, url), 'SyntaxError', 12);
  for (const method of ['connect', 'CONNECT', 'cOnNeCt', 'trace', 'TRACE', 'TrAcE', 'track', 'TRACK', 'tRaCk'])
    expectError(() => new XMLHttpRequest().open(method, 'http://['), 'SecurityError', 18);

  for (const [method, name, code] of [['G ET', 'SyntaxError', 12], ['TRACE', 'SecurityError', 18]]) {
    const order = [];
    const convert = (label, value) => ({ toString() { order.push(label); return value; } });
    expectError(() => new XMLHttpRequest().open(
      convert('method', method), convert('url', 'http://['),
      { valueOf() { throw new Error('ToBoolean must not call valueOf'); } },
      convert('username', 'user'), convert('password', 'pass')
    ), name, code);
    assert(order.join(',') === 'method,url,username,password', 'convert every argument before validation');
  }
  for (const method of ['\u0100', '\ud800', Symbol('method')]) {
    let convertedUrl = false;
    expectError(() => new XMLHttpRequest().open(method, {
      toString() { convertedUrl = true; return url; }
    }), 'TypeError');
    assert(!convertedUrl, 'failed ByteString conversion stops before URL conversion');
  }
  for (const failingArgument of ['method', 'url', 'username', 'password']) {
    const marker = {};
    const order = [];
    const convert = (label, value) => ({ toString() {
      order.push(label);
      if (label === failingArgument) throw marker;
      return value;
    } });
    let caught;
    try {
      new XMLHttpRequest().open(convert('method', 'TRACE'), convert('url', url), true,
        convert('username', 'user'), convert('password', 'pass'));
    } catch (error) { caught = error; }
    assert(caught === marker, 'argument conversion exception is propagated unchanged');
    assert(order.at(-1) === failingArgument, 'conversion stops at the throwing argument');
  }
  const open = XMLHttpRequest.prototype.open;
  for (const receiver of [
    {}, XMLHttpRequest.prototype, Object.create(XMLHttpRequest.prototype),
    Object.create(new XMLHttpRequest()), new Proxy(new XMLHttpRequest(), {}),
    {__lmMethod: 'GET', __lmXhrReadyState: 0}
  ]) {
    const order = [];
    expectError(() => open.call(receiver, {
      toString() { order.push('method'); return 'GET'; }
    }, url), 'TypeError');
    assert(order.length === 0, 'receiver validation precedes argument conversion');
  }
  class DerivedRequest extends XMLHttpRequest {}
  const derived = new DerivedRequest();
  open.call(derived, 'GET', url);
  assert(derived.readyState === 1, 'native subtypes are accepted');
  const changedPrototype = new XMLHttpRequest();
  Object.setPrototypeOf(changedPrototype, null);
  open.call(changedPrototype, 'GET', url);
  const readyState = Object.getOwnPropertyDescriptor(XMLHttpRequest.prototype, 'readyState').get;
  assert(readyState.call(changedPrototype) === 1, 'native identity survives prototype changes');
  expectError(() => new XMLHttpRequest().open(), 'TypeError');
  expectError(() => new XMLHttpRequest().open('GET'), 'TypeError');
  for (const method of ['', 'G ET', 'TRACE'])
    expectError(() => new Request(url, {method}), 'TypeError');
  return 'ok';
})()
"#;

fn start_open_validation_probe(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    probe: &str,
    worker: bool,
) {
    let source = if worker {
        let script = format!(
            "Promise.resolve().then(() => {probe}).then(value => {{ postMessage(value); close(); }}, error => {{ postMessage(String(error.stack || error)); close(); }});"
        );
        format!(
            r#"
globalThis.__xhrOpenValidation = 'pending';
const worker = new Worker(URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}})));
worker.onmessage = event => {{ globalThis.__xhrOpenValidation = event.data; }};
worker.onerror = event => {{ globalThis.__xhrOpenValidation = event.message; event.preventDefault(); }};
'started'
"#,
            serde_json::to_string(&script).unwrap()
        )
    } else {
        format!(
            r#"
globalThis.__xhrOpenValidation = 'pending';
Promise.resolve().then(() => {probe}).then(
  value => {{ globalThis.__xhrOpenValidation = value; }},
  error => {{ globalThis.__xhrOpenValidation = String(error.stack || error); }}
);
'started'
"#
        )
    };
    assert_eq!(vm.eval(&source).expect("start XHR open probe"), "started");
}

#[test]
fn window_xhr_open_validates_methods_after_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://xhr-open-validation.test/");
    assert_eq!(
        vm.eval(OPEN_METHOD_PROBE)
            .expect("window XHR method validation"),
        "ok"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn worker_xhr_open_validates_methods_after_webidl_conversion() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://xhr-open-validation.test/", &loader);
    start_open_validation_probe(&mut vm, OPEN_METHOD_PROBE, true);
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "globalThis.__xhrOpenValidation",
        "ok",
        "worker XHR method validation",
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn xhr_invalid_open_preserves_headers_pending_send_and_completed_response() {
    for worker in [false, true] {
        let server = StaticHttpServer::spawn_with_bodies(vec!["kept".to_owned()]).await;
        let base_url = server.base_url();
        let loader = static_http_loader(std::iter::empty::<String>());
        let mut vm = new_page_task_executor_test_vm_with_loader(base_url.as_str(), &loader);
        let probe = r#"
(async () => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const xhr = new XMLHttpRequest();
  const events = [];
  for (const type of ['readystatechange', 'abort', 'error', 'load', 'loadend'])
    xhr.addEventListener(type, () => events.push(type));
  const snapshot = () => JSON.stringify([
    xhr.readyState, xhr.status, xhr.statusText, xhr.responseText, xhr.response,
    xhr.responseURL, xhr.getAllResponseHeaders(), xhr.responseType, xhr.timeout
  ]);
  const invalidOpen = () => {
    const before = snapshot();
    const count = events.length;
    for (const [method, name] of [['', 'SyntaxError'], ['G ET', 'SyntaxError'], ['trace', 'SecurityError']]) {
      let caught;
      try { xhr.open(method, REQUEST_URL + '/replacement', false); }
      catch (error) { caught = error; }
      assert(caught instanceof DOMException && caught.name === name, 'reject invalid open: ' + method);
      assert(snapshot() === before && events.length === count, 'failed open preserves response and events');
    }
  };
  xhr.open('post', REQUEST_URL + '/original');
  xhr.responseType = 'text';
  xhr.timeout = 2000;
  xhr.setRequestHeader('X-Preserved', 'before');
  invalidOpen();
  const done = new Promise((resolve, reject) => {
    xhr.onloadend = resolve;
    xhr.onerror = () => reject(new Error('original request failed'));
    xhr.ontimeout = () => reject(new Error('original request timed out'));
  });
  xhr.send('original body');
  invalidOpen();
  await done;
  assert(xhr.status === 200 && xhr.responseText === 'kept', 'original request completes');
  assert(xhr.responseURL === REQUEST_URL + '/original', 'original URL is retained');
  invalidOpen();
  assert(!events.includes('abort') && !events.includes('error'), 'no cancellation events');
  return 'ok';
})()
"#
        .replace(
            "REQUEST_URL",
            &serde_json::to_string(base_url.as_str().trim_end_matches('/')).unwrap(),
        );
        start_open_validation_probe(&mut vm, &probe, worker);
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "globalThis.__xhrOpenValidation",
            "ok",
            "failed XHR open preserves the request",
        )
        .await;
        let requests = server.finish().await;
        assert_eq!(requests.len(), 1, "worker={worker}");
        assert_eq!(requests[0].method, "POST", "worker={worker}");
        assert_eq!(requests[0].target, "/original", "worker={worker}");
        assert_eq!(requests[0].header_value("x-preserved"), Some("before"));
    }
}

#[test]
fn xhr_open_checks_document_activity_before_methods_in_the_method_realm() {
    let mut vm = new_parsed_test_vm(
        "https://xhr-open-realms.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm.eval(r#"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const frame = document.body.appendChild(document.createElement('iframe'));
  const child = frame.contentWindow;
  const owners = [window, child];
  const methods = owners.map(owner => ({
    open: owner.XMLHttpRequest.prototype.open,
    exception: owner.DOMException,
    typeError: owner.TypeError
  }));
  const check = (method, xhr, name, value, url) => {
    let caught;
    try { method.open.call(xhr, value, url); } catch (error) { caught = error; }
    assert(caught && caught.name === name, name + ': ' + caught);
    const ctor = name === 'TypeError' ? method.typeError : method.exception;
    assert(Object.getPrototypeOf(caught) === ctor.prototype, 'exception uses the invoked method realm');
  };
  for (const owner of owners) {
    const xhr = new owner.XMLHttpRequest();
    for (const method of methods) {
      check(method, xhr, 'SyntaxError', 'G ET', 'http://[');
      check(method, xhr, 'SecurityError', 'TRACE', 'http://[');
      check(method, xhr, 'SyntaxError', 'GET', 'http://[');
    }
  }
  const inactive = new child.XMLHttpRequest();
  frame.remove();
  for (const method of methods) {
    for (const value of ['GET', '', 'G ET', 'TRACE'])
      check(method, inactive, 'InvalidStateError', value, 'http://[');
    check(method, inactive, 'TypeError', '\u0100', 'http://[');
  }
  for (const borrowParent of [false, true]) {
    const frame = document.body.appendChild(document.createElement('iframe'));
    const child = frame.contentWindow;
    const method = borrowParent ? methods[0] : {
      open: child.XMLHttpRequest.prototype.open, exception: child.DOMException
    };
    const xhr = new child.XMLHttpRequest();
    check(method, xhr, 'InvalidStateError', 'TRACE', {
      toString() { frame.remove(); return 'http://['; }
    });
  }
  return 'ok';
})()
"#).expect("XHR open validation order and exception realm");
    assert_eq!(result, "ok");
}
