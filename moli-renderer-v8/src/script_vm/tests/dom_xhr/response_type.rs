use super::*;

const CONVERSION_PROBE: &str = r#"
(() => {
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const worker = typeof document === 'undefined';
  const descriptor = Object.getOwnPropertyDescriptor(XMLHttpRequest.prototype, 'responseType');
  const xhr = new XMLHttpRequest();
  const invalidValues = [
    'JSON', 'arrayBuffer', 'nosuchtype', ' text ', 'text/html', 'json\0', '\ud800',
    'moz-blob', 'moz-chunked-text', 'moz-chunked-arraybuffer',
    undefined, null, false, 0, NaN, Infinity, 1n, {}, ['JSON']
  ];
  for (const type of ['', 'arraybuffer', 'blob', 'document', 'json', 'text']) {
    xhr.responseType = 'text';
    xhr.responseType = type;
    const expected = worker && type === 'document' ? 'text' : type;
    assert(xhr.responseType === expected, 'valid enum value: ' + type);
    for (const invalid of invalidValues) {
      xhr.responseType = invalid;
      assert(xhr.responseType === expected, 'invalid enum assignment preserves ' + expected);
    }
    assert(descriptor.set.call(xhr) === undefined, 'omitted setter value is ignored');
    assert(xhr.responseType === expected, 'omitted value preserves the response type');
  }
  let conversions = 0;
  xhr.responseType = {
    [Symbol.toPrimitive](hint) {
      assert(hint === 'string', 'enum conversion uses the string hint');
      ++conversions;
      return 'json';
    },
    toString() { throw new Error('unexpected second conversion'); }
  };
  assert(conversions === 1 && xhr.responseType === 'json', 'convert once');
  xhr.responseType = { toString() { xhr.responseType = 'blob'; return 'invalid'; } };
  assert(xhr.responseType === 'blob', 'ignored outer assignment preserves conversion side effects');
  const marker = {};
  let caught;
  try { xhr.responseType = { toString() { throw marker; } }; } catch (error) { caught = error; }
  assert(caught === marker && xhr.responseType === 'blob', 'preserve the original conversion exception');
  try { xhr.responseType = Symbol('type'); } catch (error) { caught = error; }
  assert(caught instanceof TypeError && xhr.responseType === 'blob', 'Symbols still throw');

  const sync = new XMLHttpRequest();
  sync.open('GET', 'http://example.test/response', false);
  for (const invalid of invalidValues) sync.responseType = invalid;
  assert(sync.responseType === '', 'invalid values bypass synchronous Window restrictions');
  caught = undefined;
  try { sync.responseType = 'json'; } catch (error) { caught = error; }
  if (worker) assert(!caught && sync.responseType === 'json', 'sync Worker accepts json');
  else assert(caught instanceof DOMException && caught.name === 'InvalidAccessError', 'sync Window rejects valid values');
  return 'ok';
})()
"#;

fn start_response_type_probe(
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
globalThis.__responseTypeResult = 'pending';
const worker = new Worker(URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}})));
worker.onmessage = event => {{ globalThis.__responseTypeResult = event.data; }};
worker.onerror = event => {{ globalThis.__responseTypeResult = event.message; event.preventDefault(); }};
'started'
"#,
            serde_json::to_string(&script).unwrap()
        )
    } else {
        format!(
            r#"
globalThis.__responseTypeResult = 'pending';
Promise.resolve().then(() => {probe}).then(
  value => {{ globalThis.__responseTypeResult = value; }},
  error => {{ globalThis.__responseTypeResult = String(error.stack || error); }}
);
'started'
"#
        )
    };
    assert_eq!(
        vm.eval(&source).expect("start responseType probe"),
        "started"
    );
}

#[test]
fn window_xhr_response_type_ignores_invalid_enum_assignments() {
    let mut vm = new_storage_test_vm("https://xhr-response-type.test/");
    assert_eq!(vm.eval(CONVERSION_PROBE).unwrap(), "ok");
}

#[tokio::test(flavor = "current_thread")]
async fn worker_xhr_response_type_ignores_invalid_enum_assignments() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://xhr-response-type.test/", &loader);
    start_response_type_probe(&mut vm, CONVERSION_PROBE, true);
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "globalThis.__responseTypeResult",
        "ok",
        "Worker enum setter",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn xhr_response_type_checks_state_after_conversion_and_preserves_delivery() {
    for worker in [false, true] {
        let server =
            StaticHttpServer::spawn_with_bodies(vec![r#"{"value":7}"#.to_owned(); 3]).await;
        let url = server.base_url();
        let loader = static_http_loader(std::iter::empty::<String>());
        let mut vm = new_page_task_executor_test_vm_with_loader(url.as_str(), &loader);
        let probe = r#"
(async () => {
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const worker = typeof document === 'undefined';
  const xhr = new XMLHttpRequest();
  const states = [];
  xhr.responseType = 'json';
  const check = () => {
    const before = xhr.response;
    const type = xhr.responseType;
    const count = states.length;
    for (const value of ['JSON', 'moz-blob', 'moz-chunked-text', 'moz-chunked-arraybuffer', undefined]) {
      xhr.responseType = value;
      assert(xhr.responseType === type && xhr.response === before, 'ignore without resetting the response');
    }
    if (worker) {
      xhr.responseType = 'document';
      assert(xhr.responseType === type && xhr.response === before, 'Worker ignores document in every state');
    }
    assert(states.length === count, 'ignored assignments dispatch no state events');
    if (xhr.readyState >= 3) {
      let conversions = 0, caught;
      try { xhr.responseType = { toString() { ++conversions; return 'text'; } }; }
      catch (error) { caught = error; }
      assert(conversions === 1 && caught instanceof DOMException && caught.name === 'InvalidStateError',
             'convert before rejecting a valid value in LOADING or DONE');
      caught = undefined;
      try { xhr.responseType = Symbol(); } catch (error) { caught = error; }
      assert(caught instanceof TypeError, 'conversion TypeError precedes the state error');
      const marker = {};
      try { xhr.responseType = { toString() { throw marker; } }; } catch (error) { caught = error; }
      assert(caught === marker, 'conversion exceptions precede the state error');
    }
  };
  const complete = new Promise((resolve, reject) => {
    xhr.onreadystatechange = () => {
      try {
        states.push(xhr.readyState);
        check();
        if (xhr.readyState === 4) resolve();
      } catch (error) { reject(error); }
    };
    xhr.onerror = () => reject(new Error('request failed'));
  });
  xhr.open('GET', REQUEST_URL + 'async');
  xhr.send();
  await complete;
  assert(states.join(',') === '1,2,3,4', 'observe each request state');
  assert(xhr.status === 200 && xhr.response.value === 7, 'keep the parsed JSON response');
  xhr.onreadystatechange = null;
  xhr.responseType = { toString() { xhr.open('GET', REQUEST_URL + 'reopened'); return 'text'; } };
  assert(xhr.readyState === 1 && xhr.responseType === 'text', 'reopening during conversion permits the assignment');

  for (const value of ['invalid', 'text']) {
    const sync = new XMLHttpRequest();
    sync.open('GET', REQUEST_URL + value, false);
    let conversions = 0, caught;
    try {
      sync.responseType = { toString() { ++conversions; sync.send(); return value; } };
    } catch (error) { caught = error; }
    assert(conversions === 1 && sync.readyState === 4, 'conversion may finish a synchronous request');
    if (value === 'invalid') assert(!caught, 'ignore invalid enum before checking the new state');
    else assert(caught instanceof DOMException && caught.name === 'InvalidStateError', 'check the state after conversion');
    assert(sync.responseType === '' && sync.responseText === '{"value":7}', 'preserve the completed response');
  }
  return 'ok';
})()
"#.replace("REQUEST_URL", &serde_json::to_string(url.as_str()).unwrap());
        start_response_type_probe(&mut vm, &probe, worker);
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "globalThis.__responseTypeResult",
            "ok",
            "responseType delivery and reentrancy",
        )
        .await;
        let requests = server.finish().await;
        assert_eq!(
            requests
                .iter()
                .map(|request| request.target.as_str())
                .collect::<Vec<_>>(),
            ["/async", "/invalid", "/text"]
        );
    }
}

#[test]
fn xhr_response_type_accessors_validate_native_receivers_in_the_accessor_realm() {
    let mut vm = new_parsed_test_vm(
        "https://xhr-response-type-realms.test/",
        "<!doctype html><html><body></body></html>",
    );
    assert_eq!(vm.eval(r#"
(() => {
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const frame = document.body.appendChild(document.createElement('iframe'));
  const owners = [window, frame.contentWindow];
  for (const owner of owners) {
    const {get, set} = Object.getOwnPropertyDescriptor(owner.XMLHttpRequest.prototype, 'responseType');
    for (const receiver of [null, undefined, {}, owner.XMLHttpRequest.prototype,
      Object.create(owner.XMLHttpRequest.prototype), Object.create(new owner.XMLHttpRequest()),
      new Proxy(new owner.XMLHttpRequest(), {}), {__lmXhrResponseType: 'text'}]) {
      let conversions = 0, caught;
      try { set.call(receiver, {toString() { ++conversions; return 'invalid'; }}); }
      catch (error) { caught = error; }
      assert(caught instanceof owner.TypeError && conversions === 0, 'validate receiver before conversion in setter realm');
      caught = undefined;
      try { get.call(receiver); } catch (error) { caught = error; }
      assert(caught instanceof owner.TypeError, 'getter rejects forged receivers in its realm');
    }
    for (const receiverOwner of owners) {
      const xhr = new receiverOwner.XMLHttpRequest();
      set.call(xhr, 'json');
      assert(get.call(xhr) === 'json', 'native receivers work across realms');
      let caught;
      try { set.call(xhr, Symbol()); } catch (error) { caught = error; }
      assert(caught instanceof owner.TypeError, 'conversion exception uses the setter realm');
    }
    class DerivedRequest extends owner.XMLHttpRequest {}
    const derived = new DerivedRequest();
    Object.setPrototypeOf(derived, null);
    set.call(derived, 'blob');
    assert(get.call(derived) === 'blob', 'native subclass identity survives prototype changes');
  }
  return 'ok';
})()
"#).unwrap(), "ok");
}
