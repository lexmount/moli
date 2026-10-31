use super::service_worker_drain::drain_service_worker_test_turn;
use super::*;

async fn assert_opaque_origin_wasm_message_port_delivery(shared_origin: bool, attachments: bool) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://opaque-wasm-message.test/",
        &loader,
    );
    let registry = vm._context_host.borrow().message_port_registry();
    vm.eval(&format!("globalThis.sharedOpaqueOrigin = {shared_origin}; globalThis.transferAttachments = {attachments};"))
        .unwrap();
    vm.eval(
        r#"
globalThis.opaqueWasmResult = null;
const frames = [];
const count = sharedOpaqueOrigin ? 1 : 2;
const childScript = `
  let sendingPort = null;
  const retainedPorts = [];
  onmessage = event => {
    if (event.data.kind === 'close') {
      sendingPort.close();
      for (const port of retainedPorts) port.close();
    } else if (event.data.kind === 'send') {
      sendingPort = event.data.port;
      const module = new WebAssembly.Module(new Uint8Array([0,97,115,109,1,0,0,0]));
      if (event.data.attachments) {
        const extra = new MessageChannel(), nested = new MessageChannel();
        retainedPorts.push(extra.port1, nested.port1);
        // The transferred endpoint also owns a queued transferred attachment.
        extra.port1.postMessage(nested.port2, [nested.port2]);
        sendingPort.postMessage({module, extra: extra.port2}, [extra.port2]);
      } else {
        sendingPort.postMessage(module);
      }
    } else if (event.data.kind === 'receive') {
      const port = event.data.port;
      port.onmessage = event => {
        const module = event.data instanceof WebAssembly.Module ? event.data : event.data.module;
        parent.postMessage({result: ['message',
          module instanceof WebAssembly.Module,
          Object.keys(new WebAssembly.Instance(module).exports).length === 0]}, '*');
        for (const attachment of event.ports) attachment.close();
        port.close();
      };
      port.onmessageerror = event => {
        parent.postMessage({result: ['messageerror', event instanceof MessageEvent]}, '*');
        port.close();
      };
      port.start();
    }
  };
  parent.postMessage({ready: true}, '*');
`;
let ready = 0;
addEventListener('message', event => {
  if (event.data.ready && ++ready === count) {
    const channel = new MessageChannel();
    frames[0].contentWindow.postMessage({kind: 'send', port: channel.port1, attachments: transferAttachments}, '*', [channel.port1]);
    frames[count - 1].contentWindow.postMessage({kind: 'receive', port: channel.port2}, '*', [channel.port2]);
  } else if (event.data.result) {
    opaqueWasmResult = event.data.result;
    frames[0].contentWindow.postMessage({kind: 'close'}, '*');
  }
});
for (let index = 0; index < count; index++) {
  const frame = document.createElement('iframe');
  frame.setAttribute('sandbox', 'allow-scripts');
  frame.srcdoc = '<!doctype html><script>' + childScript + '<\/script>';
  frames.push(frame);
  (document.body || document.documentElement || document).appendChild(frame);
}
"#,
    )
    .unwrap();
    let expected = if shared_origin {
        r#"["message",true,true]"#
    } else {
        r#"["messageerror",true]"#
    };
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "JSON.stringify(opaqueWasmResult)",
        expected,
        "Wasm delivery must compare the native opaque origin, not its serialization",
    )
    .await;
    for _ in 0..32 {
        if registry.endpoint_count() == 0
            && vm
                ._context_host
                .borrow()
                .message_port_execution_context_owners_for_test()
                .is_empty()
        {
            break;
        }
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .unwrap();
    }
    assert_eq!(registry.endpoint_count(), 0);
    assert!(
        vm._context_host
            .borrow()
            .message_port_execution_context_owners_for_test()
            .is_empty()
    );
}

#[tokio::test]
async fn message_ports_reject_wasm_between_independent_opaque_origins() {
    assert_opaque_origin_wasm_message_port_delivery(false, false).await;
}

#[tokio::test]
async fn message_ports_allow_wasm_within_the_same_opaque_origin() {
    assert_opaque_origin_wasm_message_port_delivery(true, false).await;
}

#[tokio::test]
async fn rejected_opaque_wasm_messages_release_transferred_ports_and_nested_attachments() {
    assert_opaque_origin_wasm_message_port_delivery(false, true).await;
}

#[tokio::test]
async fn accepted_opaque_wasm_messages_keep_transferred_attachments_closable() {
    assert_opaque_origin_wasm_message_port_delivery(true, true).await;
}

const RECEIVER_PROBE: &str = r#"
function messageChannelReceiverProbe(realms = [globalThis]) {
  const failures = [], records = [];
  const check = (value, label) => { if (!value) failures.push(label); };
  for (const realm of realms) {
    class Subchannel extends realm.MessageChannel {}
    for (const channel of [new realm.MessageChannel(), new Subchannel(),
                          Reflect.construct(realm.MessageChannel, [], function Custom() {})]) {
      const getters = ['port1', 'port2'].map(name =>
        Object.getOwnPropertyDescriptor(realm.MessageChannel.prototype, name).get);
      const ports = getters.map(getter => getter.call(channel));
      check(ports[0] !== ports[1], 'distinct ports');
      check(ports.every(port => port instanceof realm.MessagePort), 'port creation realm');
      records.push({channel, ports});
    }
  }
  const real = records[0].channel;
  const revoked = Proxy.revocable(real, {}); revoked.revoke();
  let traps = 0, invalid = 0, valid = 0;
  const trap = () => { traps++; throw new Error('receiver inspection reached author code'); };
  const receivers = [undefined, null, false, 0, 0n, '', Symbol(), {},
    MessageChannel.prototype, Object.create(MessageChannel.prototype), Object.create(real),
    new Proxy(real, {}), revoked.proxy,
    new Proxy(real, {get: trap, getPrototypeOf: trap, has: trap}),
    records[0].ports[0], new EventTarget()];
  for (const [realmIndex, realm] of realms.entries()) {
    for (const name of ['port1', 'port2']) {
      const getter = Object.getOwnPropertyDescriptor(realm.MessageChannel.prototype, name).get;
      for (const [index, receiver] of receivers.entries()) {
        invalid++;
        const label = realmIndex + ':' + name + ':' + index;
        try { getter.call(receiver); failures.push('accepted ' + label); }
        catch (error) {
          check(Object.getPrototypeOf(error) === realm.TypeError.prototype, 'error realm ' + label);
        }
      }
    }
  }
  for (const phase of ['original', 'null-prototype', 'foreign-prototype']) {
    for (const record of records) {
      if (phase === 'null-prototype') Object.setPrototypeOf(record.channel, null);
      if (phase === 'foreign-prototype') {
        Object.setPrototypeOf(record.channel, realms[realms.length - 1].Object.prototype);
        Object.freeze(record.channel);
      }
      for (const realm of realms) {
        for (const [index, name] of ['port1', 'port2'].entries()) {
          const getter = Object.getOwnPropertyDescriptor(realm.MessageChannel.prototype, name).get;
          valid++;
          check(getter.call(record.channel) === record.ports[index], phase + ' ' + name);
          check(getter.call(record.channel) === record.ports[index], 'same object ' + name);
        }
      }
    }
  }
  for (const record of records) for (const port of record.ports) port.close();
  return {failures, invalid, valid, traps};
}
"#;

fn assert_receiver_probe(value: &str, realms: usize) {
    let value: serde_json::Value = serde_json::from_str(value).unwrap();
    assert_eq!(value["failures"], serde_json::json!([]), "{value}");
    assert_eq!(value["invalid"], 32 * realms, "{value}");
    assert_eq!(value["valid"], 18 * realms * realms, "{value}");
    assert_eq!(value["traps"], 0, "{value}");
}

#[test]
fn message_channel_getters_use_native_brands_and_callee_realm_errors() {
    let mut vm = new_parsed_test_vm(
        "https://message-channel-brands.test/",
        "<!doctype html><body>",
    );
    let value = vm
        .eval(&format!(
            r#"{RECEIVER_PROBE}
const frame = document.createElement('iframe');
document.body.appendChild(frame);
const result = messageChannelReceiverProbe([window, frame.contentWindow]);
frame.remove();
JSON.stringify(result)
"#
        ))
        .unwrap();
    assert_receiver_probe(&value, 2);
}

#[tokio::test]
async fn message_channel_worker_getters_reject_unbranded_receivers() {
    for shared in [false, true] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, browser_context_runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                "https://message-channel-worker.test/",
                &loader,
            );
        let (constructor, endpoint, cleanup, source) = if shared {
            (
                "SharedWorker",
                "worker.port",
                "port.close()",
                format!(
                    "{RECEIVER_PROBE}\nonconnect=e=>e.ports[0].postMessage(messageChannelReceiverProbe());"
                ),
            )
        } else {
            (
                "Worker",
                "worker",
                "worker.terminate()",
                format!("{RECEIVER_PROBE}\npostMessage(messageChannelReceiverProbe());"),
            )
        };
        let source = serde_json::to_string(&source).unwrap();
        vm.eval(&format!(
            r#"
const url = URL.createObjectURL(new Blob([{source}], {{type:'text/javascript'}}));
const worker = new {constructor}(url), port = {endpoint};
port.onmessage = e => {{ globalThis.channelResult=e.data; {cleanup}; URL.revokeObjectURL(url); }};
worker.onerror = e => {{ globalThis.channelResult=String(e.message); }};
"#
        ))
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while vm.eval("globalThis.channelResult !== undefined").unwrap() != "true" {
                browser_context_runtime.drain_shared_worker_service_lane();
                drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("shared={shared}: MessageChannel probe did not settle"));
        let value = vm.eval("JSON.stringify(globalThis.channelResult)").unwrap();
        assert_receiver_probe(&value, 1);
    }
}
