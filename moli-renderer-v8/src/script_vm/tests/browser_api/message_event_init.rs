use super::service_worker_drain::drain_service_worker_test_turn;
use super::*;

const NATIVE_PROBE: &str = include_str!("native_message_event.js");

#[test]
fn message_event_initializers_validate_interfaces_sequences_and_preserve_dom_strings() {
    let mut vm = new_storage_html_test_vm("https://message-event-init.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>';")
        .unwrap();
    assert_eq!(
        vm.eval(include_str!("message_event_init.js")).unwrap(),
        "true"
    );
}

fn assert_native_message_events(value: &serde_json::Value, rows: usize) {
    assert_eq!(value["constructorReads"], 0, "{value}");
    assert_eq!(value["dictionaryReads"], 0, "{value}");
    assert_eq!(value["iteratorReads"], 0, "{value}");
    let events = value["rows"].as_array().expect("native MessageEvent rows");
    assert_eq!(events.len(), rows, "{value}");
    for event in events {
        for name in [
            "branded",
            "prototype",
            "eventType",
            "flags",
            "payload",
            "eventOrigin",
            "eventId",
            "eventSource",
            "frozen",
            "arrayRealm",
            "portCount",
            "portBrand",
            "trusted",
        ] {
            assert_eq!(event[name], true, "{event}: {name}");
        }
    }
}

#[tokio::test]
async fn native_message_events_bypass_author_constructors_dictionaries_and_iterators() {
    for mode in ["page", "dedicated", "shared"] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, browser_context_runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                "https://native-message-events.test/",
                &loader,
            );
        let source = if mode == "page" {
            format!(
                "{NATIVE_PROBE}\nnativeMessageEventProbe().then(value=>globalThis.nativeResult=value,error=>globalThis.nativeResult=String(error));"
            )
        } else {
            let input_probe = r#"
const InputMessageEvent = MessageEvent;
const inputConstructor = Object.getOwnPropertyDescriptor(globalThis, 'MessageEvent');
const inputIterator = Object.getOwnPropertyDescriptor(Array.prototype, Symbol.iterator);
const snapshot = event => ({
  branded:event instanceof InputMessageEvent,
  prototype:Object.getPrototypeOf(event) === InputMessageEvent.prototype,
  flags:!event.bubbles && !event.cancelable && !event.composed,
  eventType:event.type,
  payload:event.type === 'connect' ? event.data === '' : event.data.marker === 'native-message-event-probe',
  eventSource:event.type === 'connect' ? event.source === event.ports[0] : event.source === null,
  frozen:Array.isArray(event.ports) && Object.isFrozen(event.ports),
  arrayRealm:Object.getPrototypeOf(event.ports) === Array.prototype,
  portCount:event.ports.length,
  trusted:event.isTrusted,
});
const restoreInput = () => {
  Object.defineProperty(globalThis, 'MessageEvent', inputConstructor);
  Object.defineProperty(Array.prototype, Symbol.iterator, inputIterator);
};
Object.defineProperty(globalThis,'MessageEvent',{configurable:true,get(){throw new Error('native worker input read author constructor');}});
Object.defineProperty(Array.prototype,Symbol.iterator,{configurable:true,get(){throw new Error('native worker input read author iterator');}});
"#;
            let (constructor, endpoint, cleanup, source) = if mode == "shared" {
                (
                    "SharedWorker",
                    "worker.port",
                    "port.close()",
                    format!(
                        "{NATIVE_PROBE}\n{input_probe}\nonconnect=e=>{{const input=snapshot(e),port=e.ports[0];restoreInput();nativeMessageEventProbe().then(probe=>port.postMessage({{input,probe}}),error=>port.postMessage(String(error)));}};"
                    ),
                )
            } else {
                (
                    "Worker",
                    "worker",
                    "worker.terminate()",
                    format!(
                        "{NATIVE_PROBE}\n{input_probe}\nonmessage=e=>{{const input=snapshot(e);restoreInput();nativeMessageEventProbe().then(probe=>postMessage({{input,probe}}),error=>postMessage(String(error)));}};postMessage('input-ready');"
                    ),
                )
            };
            let source = serde_json::to_string(&source).unwrap();
            format!(
                r#"
const OriginalMessageEvent = MessageEvent;
const url=URL.createObjectURL(new Blob([{source}],{{type:'text/javascript'}}));
const worker=new {constructor}(url),port={endpoint};
port.onmessage=e=>{{
  if(e.data === 'input-ready'){{port.postMessage({{marker:'native-message-event-probe'}});return;}}
  globalThis.nativeResult={{result:e.data,hostEvent:e instanceof OriginalMessageEvent && Object.isFrozen(e.ports)}};
  {cleanup};URL.revokeObjectURL(url);
}};
worker.onerror=e=>{{globalThis.nativeResult=String(e.message);}};
Object.defineProperty(globalThis,'MessageEvent',{{configurable:true,get(){{throw new Error('native worker host read author constructor');}}}});
Object.defineProperty(Array.prototype,Symbol.iterator,{{configurable:true,get(){{throw new Error('native worker host read author iterator');}}}});
"#
            )
        };
        vm.eval(&source).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while vm.eval("globalThis.nativeResult !== undefined").unwrap() != "true" {
                browser_context_runtime.drain_shared_worker_service_lane();
                drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{mode} native MessageEvent probe did not settle"));
        let value = vm.eval("JSON.stringify(globalThis.nativeResult)").unwrap();
        let value: serde_json::Value = serde_json::from_str(&value).unwrap();
        if mode == "page" {
            assert_native_message_events(&value, 4);
        } else {
            assert_eq!(value["hostEvent"], true, "{mode}: {value}");
            let input = &value["result"]["input"];
            for field in [
                "branded",
                "prototype",
                "flags",
                "payload",
                "eventSource",
                "frozen",
                "arrayRealm",
            ] {
                assert_eq!(input[field], true, "{mode}: {value}");
            }
            assert_eq!(
                input["eventType"],
                if mode == "shared" {
                    "connect"
                } else {
                    "message"
                }
            );
            assert_eq!(input["portCount"], if mode == "shared" { 1 } else { 0 });
            assert_native_message_events(&value["result"]["probe"], 3);
        }
    }
}

#[test]
fn message_event_sources_preserve_registered_native_proxy_identity() {
    let mut vm = new_storage_html_test_vm("https://message-event-native-source.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'; globalThis.sourcePort = new MessageChannel().port1;")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "sourcePort");
        let port =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        assert!(crate::web_api_interfaces::MessagePort::is_instance(
            scope, port
        ));
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, port, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeSourceProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      let traps = 0;
      const handler = {
        get() { traps++; throw Error('get trap'); },
        getPrototypeOf() { traps++; throw Error('prototype trap'); }
      };
      const revoked = Proxy.revocable(nativeSourceProxy, {}); revoked.revoke();
      const invalid = [new Proxy(nativeSourceProxy, handler), revoked.proxy, Object.create(nativeSourceProxy)];
      const realms = [globalThis, document.getElementById('child').contentWindow];
      for (const realm of realms) {
        const event = new realm.MessageEvent('x', {source: nativeSourceProxy, ports: [nativeSourceProxy]});
        if (event.source !== nativeSourceProxy || event.ports[0] !== nativeSourceProxy) throw Error('constructor identity');
        realm.MessageEvent.prototype.initMessageEvent.call(event, 'x', false, false, null, '', '', nativeSourceProxy, [nativeSourceProxy]);
        if (event.source !== nativeSourceProxy || event.ports[0] !== nativeSourceProxy) throw Error('legacy identity');
        for (const source of invalid) {
          for (const run of [
            () => new realm.MessageEvent('x', {source}),
            () => realm.MessageEvent.prototype.initMessageEvent.call(event, 'x', false, false, null, '', '', source)
          ]) {
            let caught; try { run(); } catch (error) { caught = error; }
            if (!(caught instanceof realm.TypeError) || traps !== 0) throw Error('invalid source');
            if (event.source !== nativeSourceProxy || event.ports[0] !== nativeSourceProxy) throw Error('failed conversion mutated event');
          }
        }
      }
      sourcePort.close();
      return true;
    })()"#).unwrap(), "true");
}
