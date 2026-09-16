use super::service_worker_drain::drain_service_worker_test_turn;
use super::*;

const DISPATCH_PROBE: &str = r#"
(() => {
  const kind = __KIND__;
  const actions = ['replace', 'reactivate', 'listener', 'object-listener', 'abort-readd', 'once-nested', 'capture-add', 'capture-readd', 'realm-replace', 'retired-realm'];
  const rows = {};
  for (const action of actions) {
    let target, type = 'load', cleanup = () => {};
    switch (kind) {
      case 'reader': target = new FileReader(); break;
      case 'xhr': target = new XMLHttpRequest(); break;
      case 'xhr-upload': target = new XMLHttpRequest().upload; break;
      case 'broadcast': target = new BroadcastChannel('dispatch-snapshot'); type = 'message'; cleanup = () => target.close(); break;
      case 'performance': target = performance; type = 'resourcetimingbufferfull'; break;
      case 'fonts': target = document.fonts; type = 'loading'; break;
      case 'speech': target = new SpeechSynthesisUtterance(); type = 'start'; break;
      case 'worker-host': {
        const url = URL.createObjectURL(new Blob([''], {type:'text/javascript'}));
        target = new Worker(url); type = 'message';
        cleanup = () => { target.terminate(); URL.revokeObjectURL(url); }; break;
      }
      case 'shared-worker-host': {
        const url = URL.createObjectURL(new Blob(['onconnect = e => e.ports[0].start();'], {type:'text/javascript'}));
        target = new SharedWorker(url); type = 'error';
        cleanup = () => { target.port.close(); URL.revokeObjectURL(url); }; break;
      }
      default: throw new Error(kind);
    }
    const name = 'on' + type;
    const trace = [], registrations = [];
    const add = (callback, options) => { target.addEventListener(type, callback, options); registrations.push([callback, options]); };
    const remove = callback => target.removeEventListener(type, callback);
    const dispatch = () => target.dispatchEvent(new Event(type, {cancelable:true}));
    const original = () => trace.push('original');
    const replacement = () => { trace.push('replacement'); return false; };
    const after = () => trace.push('after');
    const controller = new AbortController();
    const objectListener = {get handleEvent() { trace.push('lookup'); return original; }};
    const frame = action.includes('realm') ? document.createElement('iframe') : null;
    if (frame) document.body.appendChild(frame);
    const child = frame?.contentWindow;
    if (child) child.trace = trace;
    const childHandler = child?.Function('event', 'trace.push("child:" + (window.event === event)); throw new TypeError("replacement");');
    const onError = event => { trace.push('error:' + (event.error instanceof child.TypeError)); event.preventDefault(); };
    if (child) child.addEventListener('error', onError);
    add(() => {
      trace.push('before');
      if (action === 'replace') target[name] = replacement;
      else if (action === 'reactivate') { target[name] = null; target[name] = replacement; }
      else if (action === 'listener' || action === 'capture-readd') { remove(original); add(original); }
      else if (action === 'object-listener') { remove(objectListener); add(objectListener); }
      else if (action === 'abort-readd') { controller.abort(); add(original); }
      else if (action === 'once-nested') { dispatch(); }
      else if (action === 'capture-add') { add(original); target[name] = replacement; }
      else if (action === 'realm-replace') target[name] = childHandler;
      else if (action === 'retired-realm') { target[name] = replacement; frame.remove(); }
    }, {once:true, capture:action.startsWith('capture-')});
    if (['listener','capture-readd','once-nested'].includes(action)) add(original, {once:action === 'once-nested'});
    else if (action === 'object-listener') add(objectListener);
    else if (action === 'abort-readd') add(original, {signal:controller.signal});
    else if (action !== 'capture-add') target[name] = action === 'retired-realm' ? childHandler : original;
    add(after);
    const firstResult = dispatch();
    const first = trace.splice(0);
    const secondResult = dispatch();
    const second = trace.splice(0);
    rows[action] = {first, second, firstResult, secondResult};
    target[name] = null;
    for (const [callback, options] of registrations) target.removeEventListener(type, callback, options);
    if (child) child.removeEventListener('error', onError);
    if (frame) frame.remove();
    cleanup();
  }
  return rows;
})()
"#;

const WORKER_PROBE: &str = r#"
(async () => {
  const mode = __MODE__;
  const rows = {};
  for (const action of ['replace','reactivate','listener','once-readd']) {
    const trace = [];
    let worker, port, url, timer;
    const result = await new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(mode + '/' + action + ' timeout')), 4000);
      const source = mode === 'worker-message' ? `
        const action = ${JSON.stringify(action)};
        const trace = [];
        const original = () => { trace.push('original'); if (action === 'once-readd') addEventListener('message', original, {once:true}); };
        const replacement = () => trace.push('replacement');
        addEventListener('message', () => {
          trace.push('before');
          if (action === 'replace') onmessage = replacement;
          else if (action === 'reactivate') { onmessage = null; onmessage = replacement; }
          else if (action === 'listener') { removeEventListener('message', original); addEventListener('message', original); }
        }, {once:true});
        if (action === 'listener' || action === 'once-readd') addEventListener('message', original, {once:action === 'once-readd'});
        else onmessage = original;
        addEventListener('message', () => { trace.push('after'); setTimeout(() => postMessage(trace.splice(0)), 0); });
      ` : mode === 'worker-error'
        ? 'onmessage = () => { throw new Error("worker failure"); };'
        : 'this is not valid JavaScript !!!';
      url = URL.createObjectURL(new Blob([source], {type:'text/javascript'}));
      worker = mode === 'shared-worker-error' ? new SharedWorker(url) : new Worker(url);
      port = mode === 'shared-worker-error' ? worker.port : worker;
      const samples = [];
      const sample = value => {
        samples.push(value);
        if (samples.length === 1) {
          if (mode === 'shared-worker-error') worker.dispatchEvent(new Event('error'));
          else port.postMessage('second');
        }
        else resolve(samples);
      };
      if (mode === 'worker-message') {
        worker.onmessage = event => sample(event.data);
        port.postMessage('first');
      } else {
        const original = () => { trace.push('original'); if (action === 'once-readd') worker.addEventListener('error', original, {once:true}); };
        const replacement = () => trace.push('replacement');
        worker.addEventListener('error', () => {
          trace.push('before');
          if (action === 'replace') worker.onerror = replacement;
          else if (action === 'reactivate') { worker.onerror = null; worker.onerror = replacement; }
          else if (action === 'listener') { worker.removeEventListener('error', original); worker.addEventListener('error', original); }
        }, {once:true});
        if (action === 'listener' || action === 'once-readd') worker.addEventListener('error', original, {once:action === 'once-readd'});
        else worker.onerror = original;
        worker.addEventListener('error', event => {
          event.preventDefault();
          trace.push('after');
          setTimeout(() => sample(trace.splice(0)), 0);
        });
        if (mode !== 'shared-worker-error') port.postMessage('first');
      }
    }).finally(() => {
      clearTimeout(timer);
      if (mode === 'shared-worker-error') port?.close();
      else worker?.terminate();
      URL.revokeObjectURL(url);
    });
    rows[action] = result;
  }
  return rows;
})()
"#;

#[test]
fn simple_event_dispatch_observes_registration_identity_and_current_handler() {
    let expected = [
        (
            "replace",
            &["before", "replacement", "after"][..],
            &["replacement", "after"][..],
            false,
            false,
        ),
        (
            "reactivate",
            &["before", "after"][..],
            &["after", "replacement"][..],
            true,
            false,
        ),
        (
            "listener",
            &["before", "after"][..],
            &["after", "original"][..],
            true,
            true,
        ),
        (
            "object-listener",
            &["before", "after"][..],
            &["after", "lookup", "original"][..],
            true,
            true,
        ),
        (
            "abort-readd",
            &["before", "after"][..],
            &["after", "original"][..],
            true,
            true,
        ),
        (
            "once-nested",
            &["before", "original", "after", "after"][..],
            &["after"][..],
            true,
            true,
        ),
        (
            "capture-add",
            &["before", "after"][..],
            &["after", "original", "replacement"][..],
            true,
            false,
        ),
        (
            "capture-readd",
            &["before", "after"][..],
            &["after", "original"][..],
            true,
            true,
        ),
        (
            "realm-replace",
            &["before", "child:true", "error:true", "after"][..],
            &["child:true", "error:true", "after"][..],
            true,
            true,
        ),
        (
            "retired-realm",
            &["before", "replacement", "after"][..],
            &["replacement", "after"][..],
            false,
            false,
        ),
    ];
    for kind in [
        "reader",
        "xhr",
        "xhr-upload",
        "broadcast",
        "performance",
        "fonts",
        "speech",
    ] {
        let mut vm = new_parsed_test_vm(
            "https://simple-event-dispatch.test/",
            "<!doctype html><body></body>",
        );
        let source = DISPATCH_PROBE.replace("__KIND__", &format!("{kind:?}"));
        let value = vm
            .eval(&format!("JSON.stringify({source})"))
            .expect("dispatch probe");
        let value: serde_json::Value = serde_json::from_str(&value).unwrap();
        for (action, first, second, first_result, second_result) in &expected {
            assert_eq!(
                value[action],
                serde_json::json!({
                    "first": first, "second": second,
                    "firstResult": first_result, "secondResult": second_result,
                }),
                "{kind}/{action}"
            );
        }
    }
}

#[tokio::test]
async fn simple_event_dispatch_worker_messages_and_host_errors_preserve_mutations() {
    let expected = serde_json::json!({
        "replace": [["before","replacement","after"], ["replacement","after"]],
        "reactivate": [["before","after"], ["after","replacement"]],
        "listener": [["before","after"], ["after","original"]],
        "once-readd": [["before","original","after"], ["after","original"]],
    });
    for mode in ["worker-message", "worker-error", "shared-worker-error"] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, browser_context_runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                "https://simple-event-dispatch.test/",
                &loader,
            );
        let source = WORKER_PROBE.replace("__MODE__", &format!("{mode:?}"));
        vm.eval(&format!(
            "({source}).then(value => globalThis.__simpleDispatchResult = value, error => globalThis.__simpleDispatchResult = String(error));"
        )).expect("worker dispatch probe should start");
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while vm
                .eval("globalThis.__simpleDispatchResult !== undefined")
                .unwrap()
                != "true"
            {
                // This fixture owns the browser-context services as well as
                // the page. Publish SharedWorker errors before selecting its
                // next production Page task.
                browser_context_runtime.drain_shared_worker_service_lane();
                drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
            }
        })
        .await
        .expect("worker dispatch probe should settle");
        let value = vm
            .eval("JSON.stringify(globalThis.__simpleDispatchResult)")
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&value).unwrap();
        assert_eq!(value, expected, "{mode}");
    }
}
