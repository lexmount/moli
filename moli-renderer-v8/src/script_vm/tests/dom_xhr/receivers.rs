use super::*;

const RECEIVER_FIXTURE: &str = include_str!("receivers.js");

#[test]
fn xhr_generated_receivers_reject_invalid_objects_before_conversion_in_window() {
    let mut vm = new_storage_test_vm("https://xhr-receivers.test/");
    assert_eq!(
        vm.eval(&format!(
            "{RECEIVER_FIXTURE}\nxhrInvalidReceiverProbe(globalThis, globalThis)"
        ))
        .unwrap(),
        "ok"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
async fn xhr_generated_receivers_reject_invalid_objects_before_conversion_in_worker() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader("https://xhr-receivers.test/", &loader);
    let script = format!(
        "{RECEIVER_FIXTURE}\nPromise.resolve().then(() => {{ xhrGenuineReceiverProbe(self, self); return xhrInvalidReceiverProbe(self, self); }}).then(value => {{ postMessage(value); close(); }}, error => {{ postMessage(String(error.stack || error)); close(); }});"
    );
    vm.eval(&format!(
        r#"
globalThis.xhrReceiverResult = 'pending';
const worker = new Worker(URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}})));
worker.onmessage = event => {{ xhrReceiverResult = event.data; }};
worker.onerror = event => {{ xhrReceiverResult = event.message; event.preventDefault(); }};
"#,
        serde_json::to_string(&script).unwrap()
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "xhrReceiverResult !== 'pending'",
        "true",
        "Worker XHR receiver validation",
    )
    .await;
    assert_eq!(vm.eval("xhrReceiverResult").unwrap(), "ok");
}

#[test]
fn xhr_generated_receivers_preserve_cross_realm_native_objects() {
    let mut vm = new_parsed_test_vm(
        "https://xhr-receivers.test/",
        "<!doctype html><body><iframe></iframe></body>",
    );
    materialize_single_child_default_realm_for_test(&mut vm, "XHR receiver realm");
    assert_eq!(
        vm.eval(&format!(
            r#"{RECEIVER_FIXTURE}
(() => {{
  const other = document.querySelector('iframe').contentWindow;
  for (const [callee, owner] of [[window, other], [other, window]]) {{
    xhrGenuineReceiverProbe(callee, owner);
    xhrInvalidReceiverProbe(callee, owner);
  }}
  return 'ok';
}})()"#
        ))
        .unwrap(),
        "ok"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 32)]
async fn xhr_generated_receivers_preserve_registered_native_proxy_state_and_loads() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://xhr-proxy.test/", &loader);
    vm.eval("document.body.innerHTML = '<iframe></iframe>';")
        .unwrap();
    vm.eval(
        "globalThis.xhrOwner = document.querySelector('iframe').contentWindow; globalThis.realXhr = new xhrOwner.XMLHttpRequest(); globalThis.realUpload = realXhr.upload;",
    ).unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, alias) in [("realXhr", "xhrProxy"), ("realUpload", "uploadProxy")] {
            let key = crate::util::v8str(scope, name);
            let target =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert!(proxy.is_proxy());
            assert!(
                crate::web_api_interfaces::XMLHttpRequestEventTarget::is_instance(
                    scope,
                    proxy.into()
                )
            );
            let alias = crate::util::v8str(scope, alias);
            assert_eq!(
                global.create_data_property(scope, alias.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    vm.eval(r#"
globalThis.xhrProxyResult = 'pending';
(async () => {
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  for (const realm of [window, xhrOwner]) {
    const prototype = realm.XMLHttpRequest.prototype;
    const property = name => Object.getOwnPropertyDescriptor(prototype, name);
    property('timeout').set.call(xhrProxy, 12345);
    assert(property('timeout').get.call(realXhr) === 12345, 'Proxy writes the native timeout');
    property('timeout').set.call(realXhr, 0);
    assert(property('timeout').get.call(xhrProxy) === 0, 'Proxy reads the native timeout');
    property('withCredentials').set.call(xhrProxy, true);
    assert(property('withCredentials').get.call(realXhr), 'Proxy writes native credentials');
    property('responseType').set.call(xhrProxy, '');
    assert(property('upload').get.call(xhrProxy) === realUpload, 'native upload identity');
    const onload = Object.getOwnPropertyDescriptor(realm.XMLHttpRequestEventTarget.prototype, 'onload');
    for (const [proxy, target] of [[xhrProxy, realXhr], [uploadProxy, realUpload]]) {
      let calls = 0;
      const listener = function(event) {
        assert(this === target && event.target === target, 'native event target identity');
        calls++;
      };
      onload.set.call(proxy, listener);
      assert(onload.get.call(target) === listener && onload.get.call(proxy) === listener, 'shared handler state');
      realm.EventTarget.prototype.dispatchEvent.call(target, new realm.Event('load'));
      assert(calls === 1, 'registered handler reaches the native listener registry');
      onload.set.call(target, null);
      assert(onload.get.call(proxy) === null, 'native handler removal reaches Proxy');
    }
    let traps = 0, conversions = 0;
    const author = new Proxy(xhrProxy, {get() { traps++; throw 42; }, getPrototypeOf() { traps++; throw 42; }});
    try { property('timeout').set.call(author, {valueOf() { conversions++; return 1; }}); throw Error('accepted author Proxy'); }
    catch (error) { assert(Object.getPrototypeOf(error) === realm.TypeError.prototype, 'author wrapper has no native identity'); }
    assert(traps === 0 && conversions === 0, 'reject author wrapper before author code');
  }
  Object.setPrototypeOf(realXhr, null);
  Object.setPrototypeOf(realUpload, null);
  for (const realm of [window, xhrOwner]) {
    const prototype = realm.XMLHttpRequest.prototype;
    const property = name => Object.getOwnPropertyDescriptor(prototype, name);
    for (const [opener, sender] of [[xhrProxy, realXhr], [realXhr, xhrProxy]]) {
      const url = URL.createObjectURL(new Blob(['native reply'], {type: 'text/plain'}));
      prototype.open.call(opener, 'GET', url);
      const onloadend = Object.getOwnPropertyDescriptor(realm.XMLHttpRequestEventTarget.prototype, 'onloadend');
      const done = new Promise(resolve => onloadend.set.call(xhrProxy, resolve));
      prototype.send.call(sender);
      await done;
      URL.revokeObjectURL(url);
      assert(property('readyState').get.call(xhrProxy) === 4, 'native DONE state');
      assert(property('status').get.call(xhrProxy) === 200, 'registered Proxy loads the same Blob response');
      assert(property('responseText').get.call(xhrProxy) === 'native reply', 'shared response text');
      assert(property('response').get.call(realXhr) === 'native reply', 'shared response value');
      assert(prototype.getResponseHeader.call(xhrProxy, 'content-type') === 'text/plain', 'shared response headers');
      assert(prototype.getAllResponseHeaders.call(xhrProxy).includes('content-type: text/plain\r\n'), 'shared serialized headers');
      prototype.abort.call(xhrProxy);
      assert(property('readyState').get.call(realXhr) === 0, 'native abort resets the same state');
    }
  }
  xhrProxyResult = 'ok';
})().catch(error => { xhrProxyResult = String(error.stack || error); });
"#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "xhrProxyResult !== 'pending'",
        "true",
        "registered native Proxy XHR loading",
    )
    .await;
    assert_eq!(vm.eval("xhrProxyResult").unwrap(), "ok");
}
