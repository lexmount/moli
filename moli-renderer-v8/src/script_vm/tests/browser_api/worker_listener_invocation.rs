use super::service_worker_drain::drain_service_worker_test_turn;
use super::*;

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
