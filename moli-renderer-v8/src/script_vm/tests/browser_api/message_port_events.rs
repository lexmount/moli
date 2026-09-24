use super::service_worker_drain::drain_service_worker_test_turn;
use super::*;

const DISPATCH_PROBE: &str = include_str!("message_port_dispatch.js");

fn assert_dispatch_rows(value: &serde_json::Value) {
    let rows = value.as_array().expect("MessagePort dispatch rows");
    assert_eq!(rows.len(), 46);
    for row in &rows[..32] {
        let mut labels = vec!["capture", "bubble"];
        if matches!(row["type"].as_str(), Some("message" | "messageerror")) {
            labels.push("handler");
        }
        labels.push("late");
        let trace: Vec<_> = labels
            .into_iter()
            .map(|label| serde_json::json!([label, true, true, true, 2, false, 1, true]))
            .collect();
        assert_eq!(row["trace"], serde_json::json!(trace), "{row}");
        assert_eq!(row["returned"], true, "{row}");
        assert_eq!(row["error"], serde_json::Value::Null, "{row}");
        assert_eq!(row["after"], serde_json::json!([true, true, 0, 0]), "{row}");
    }
    for row in &rows[32..] {
        let action = row["action"].as_str().unwrap();
        let trace: &[&str] = match action {
            "once" => &["first", "handler", "later", "handler", "later"],
            "remove" | "abort" => &["first", "handler", "first", "handler"],
            "replace-handler" => &[
                "first",
                "replacement",
                "later",
                "first",
                "replacement",
                "later",
            ],
            "reactivate-handler" => &["first", "later", "first", "later"],
            "capture-add" => &[
                "first", "handler", "later", "added", "first", "handler", "later", "added", "added",
            ],
            "stop" | "stopImmediate" => &["first", "first"],
            "recursive" => &[
                "first",
                "InvalidStateError",
                "handler",
                "later",
                "first",
                "handler",
                "later",
            ],
            "prevent" | "passive" | "return-false" | "close" | "transfer" => {
                &["first", "handler", "later", "first", "handler", "later"]
            }
            _ => panic!("unexpected action: {row}"),
        };
        let canceled = matches!(action, "prevent" | "return-false");
        assert_eq!(row["trace"], serde_json::json!(trace), "{row}");
        assert_eq!(
            row["returns"],
            serde_json::json!([!canceled, !canceled]),
            "{row}"
        );
        assert_eq!(row["error"], serde_json::Value::Null, "{row}");
        assert_eq!(
            row["after"],
            serde_json::json!([canceled, false, 0, true]),
            "{row}"
        );
    }
}

#[test]
fn message_port_dispatch_preserves_object_listeners_across_close_and_transfer() {
    let mut vm = new_parsed_test_vm("https://message-port-events.test/", "<!doctype html><body>");
    let value = vm
        .eval(&format!(
            "{DISPATCH_PROBE}\nJSON.stringify(portDispatchProbe())"
        ))
        .unwrap();
    assert_dispatch_rows(&serde_json::from_str(&value).unwrap());
}

#[test]
fn message_port_eventtarget_uses_native_brands_and_validates_events() {
    let mut vm = new_parsed_test_vm("https://message-port-brands.test/", "<!doctype html><body>");
    let value = vm.eval(r#"
(() => {
  const failures = [];
  const check = (value, label) => { if (!value) failures.push(label); };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const channel = new MessageChannel(), port = channel.port1;
  const other = frame.contentWindow;
  for (const realm of [window, other]) {
    const target = realm.EventTarget.prototype, prototype = realm.MessagePort.prototype;
    for (const method of ['addEventListener', 'removeEventListener', 'dispatchEvent']) {
      check(!Object.hasOwn(prototype, method), 'inherited ' + method);
    }
    const revoked = Proxy.revocable(port, {}); revoked.revoke();
    const generic = new EventTarget();
    const receivers = [null, {}, Object.create(port), new Proxy(port, {}), revoked.proxy, generic];
    for (const receiver of receivers) {
      for (const [name, method] of [
        ['postMessage', prototype.postMessage], ['start', prototype.start], ['close', prototype.close],
        ...['onmessage','onmessageerror'].flatMap(name => {
          const d = Object.getOwnPropertyDescriptor(prototype, name);
          return [[name + ':get', d.get], [name + ':set', d.set]];
        })
      ]) {
        try { method.call(receiver, {}); failures.push('accepted ' + name); }
        catch (error) { check(error instanceof realm.TypeError, name + ' error realm'); }
      }
      // Null this on EventTarget methods selects the global Window.
      if (receiver === generic || receiver === null) continue;
      for (const method of ['addEventListener', 'removeEventListener', 'dispatchEvent']) {
        let converted = false;
        const type = {toString() { converted = true; return 'probe'; }};
        try { target[method].call(receiver, type, () => {}); failures.push('accepted ' + method); }
        catch (error) { check(error instanceof realm.TypeError, method + ' error realm'); }
        check(!converted, method + ' receiver before conversion');
      }
    }
    let calls = 0;
    const callback = e => { check(e.currentTarget === port, 'borrowed target'); calls++; };
    target.addEventListener.call(port, 'probe', callback);
    check(target.dispatchEvent.call(port, new realm.Event('probe')), 'borrowed dispatch');
    target.removeEventListener.call(port, 'probe', callback);
    target.dispatchEvent.call(port, new realm.Event('probe'));
    check(calls === 1, 'borrowed add/remove');
    const real = new realm.Event('probe');
    const eventProxy = Proxy.revocable(real, {}); eventProxy.revoke();
    for (const event of [undefined, null, {}, Object.create(real), new Proxy(real, {}), eventProxy.proxy]) {
      try { target.dispatchEvent.call(port, event); failures.push('accepted invalid Event'); }
      catch (error) { check(error instanceof realm.TypeError, 'Event error realm'); }
    }
    const uninitialized = document.createEvent('Event');
    try { target.dispatchEvent.call(port, uninitialized); failures.push('uninitialized'); }
    catch (error) { check(error.name === 'InvalidStateError', 'uninitialized name'); }
  }
  const trace = [];
  port.onmessage = () => trace.push('old');
  port.addEventListener('message', () => trace.push('listener'));
  const noncallable = {get handleEvent() { throw new Error('EventHandler operation lookup'); }};
  port.onmessage = noncallable;
  check(port.onmessage === noncallable, 'noncallable handler identity');
  port.dispatchEvent(new Event('message'));
  check(trace.join() === 'listener', 'noncallable handler replaces old function');
  trace.length = 0;
  port.onmessage = () => trace.push('replacement');
  port.dispatchEvent(new Event('message'));
  check(trace.join() === 'replacement,listener', 'noncallable handler keeps registration position');
  for (const name of ['addEventListener', 'removeEventListener']) {
    let conversions = 0;
    try {
      port[name]({toString() { ++conversions; return 'boundary'; }});
      failures.push(name + ' accepted missing listener');
    } catch (error) { check(error instanceof TypeError, name + ' missing listener TypeError'); }
    check(conversions === 0, name + ' arity before conversion');
    let optionReads = 0;
    try {
      port[name]('boundary', 1, new Proxy({}, {get() { ++optionReads; }}));
      failures.push(name + ' accepted primitive listener');
    } catch (error) { check(error instanceof TypeError, name + ' primitive listener TypeError'); }
    check(optionReads === 0, name + ' callback before options');
  }
  const sentinel = new Error('capture conversion');
  let invoked = false;
  try {
    port.addEventListener('failed-conversion', () => { invoked = true; }, {
      get capture() { throw sentinel; },
      get once() { failures.push('continued after capture threw'); return false; },
    });
    failures.push('accepted throwing options');
  } catch (error) { check(error === sentinel, 'original options exception'); }
  port.dispatchEvent(new Event('failed-conversion'));
  check(!invoked, 'no registration after options exception');
  port.close(); channel.port2.close(); frame.remove();
  return JSON.stringify(failures);
})()
"#).unwrap();
    assert_eq!(value, "[]");
}

const QUEUE_PROBE: &str = r#"
async function portQueueProbe() {
  const rows = [];
  for (const activation of ['start', 'null', 'undefined', 'number', 'object', 'function-then-null']) {
    const channel = new MessageChannel(), port = channel.port1, trace = [];
    let settle;
    const delivered = new Promise(resolve => { settle = resolve; });
    port.addEventListener('message', function(e) {
      trace.push([e.data, e.isTrusted, this === port, e.target === port, e.currentTarget === port, e.eventPhase]);
      if (e.isTrusted) settle();
    });
    channel.port2.postMessage('queued');
    await new Promise(resolve => setTimeout(resolve, 0));
    const before = trace.length;
    port.dispatchEvent(new MessageEvent('message', {data:'manual'}));
    if (activation === 'start') port.start();
    else if (activation === 'null') port.onmessage = null;
    else if (activation === 'undefined') port.onmessage = undefined;
    else if (activation === 'number') port.onmessage = 123;
    else if (activation === 'object') port.onmessage = {};
    else { port.onmessage = () => { throw new Error('cleared handler'); }; port.onmessage = null; }
    await delivered;
    rows.push({activation, before, trace});
    port.close(); channel.port2.close();
  }
  return rows;
}
"#;

fn assert_queue_rows(value: &serde_json::Value) {
    let rows = value.as_array().expect("queue probe rows");
    assert_eq!(rows.len(), 6);
    for row in rows {
        assert_eq!(row["before"], 0, "{row}");
        assert_eq!(
            row["trace"],
            serde_json::json!([
                ["manual", false, true, true, true, 2],
                ["queued", true, true, true, true, 2],
            ]),
            "{row}"
        );
    }
}

#[tokio::test]
async fn message_port_queue_activation_and_worker_manual_dispatch_share_eventtarget() {
    for mode in ["page", "dedicated", "shared"] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, browser_context_runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                "https://message-port-queue.test/",
                &loader,
            );
        let probe = format!(
            "{DISPATCH_PROBE}\n{QUEUE_PROBE}\nasync function run() {{ return {{dispatch:portDispatchProbe(),queue:await portQueueProbe()}}; }}"
        );
        let source = if mode == "page" {
            format!(
                "{probe}\nrun().then(value=>globalThis.portResult=value,error=>globalThis.portResult=String(error));"
            )
        } else {
            let (constructor, endpoint, cleanup, source) = if mode == "shared" {
                (
                    "SharedWorker",
                    "worker.port",
                    "port.close()",
                    format!("{probe}\nonconnect=async e=>e.ports[0].postMessage(await run());"),
                )
            } else {
                (
                    "Worker",
                    "worker",
                    "worker.terminate()",
                    format!("{probe}\nrun().then(value=>postMessage(value));"),
                )
            };
            let source = serde_json::to_string(&source).unwrap();
            format!(
                r#"
const url=URL.createObjectURL(new Blob([{source}],{{type:'text/javascript'}}));
const worker=new {constructor}(url),port={endpoint};
port.onmessage=e=>{{globalThis.portResult=e.data;{cleanup};URL.revokeObjectURL(url);}};
worker.onerror=e=>{{globalThis.portResult=String(e.message);}};
"#
            )
        };
        vm.eval(&source).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while vm.eval("globalThis.portResult !== undefined").unwrap() != "true" {
                browser_context_runtime.drain_shared_worker_service_lane();
                drain_service_worker_test_turn(&mut vm, &browser_context_runtime).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{mode} port probe did not settle"));
        let value = vm.eval("JSON.stringify(globalThis.portResult)").unwrap();
        let value: serde_json::Value = serde_json::from_str(&value).unwrap();
        assert_dispatch_rows(&value["dispatch"]);
        assert_queue_rows(&value["queue"]);
    }
}
