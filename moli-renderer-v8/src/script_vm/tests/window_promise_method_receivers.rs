use super::*;

#[test]
fn window_promise_methods_accept_native_bridge_receivers_without_inheriting_their_brand() {
    let mut vm = new_storage_test_vm("https://window-promise-receivers.test/");
    vm.exec(
        r#"
globalThis.__nativePromiseReceiversResult = 'pending';
(async () => {
  const native = __moliNativeBridge.window;
  const marker = {};
  let conversions = 0, traps = 0;
  const input = {toString() { conversions++; throw marker; }};
  const options = {get imageOrientation() { conversions++; throw marker; }};
  const image = new ImageData(1, 1);
  const scrollOptions = {get behavior() { conversions++; throw marker; }};
  const handler = new Proxy({}, {get() { traps++; throw marker; }});
  const revoked = Proxy.revocable(native, {});
  revoked.revoke();
  const receivers = [new Proxy(native, handler), Object.create(native), {__moliNativeBridge}, revoked.proxy];
  for (const [name, args] of [['fetch', [input]], ['createImageBitmap', [image, options]],
    ...['scroll', 'scrollTo', 'scrollBy'].map(name => [name, [scrollOptions]])]) {
    for (const receiver of [...receivers, native]) {
      let promise;
      try { promise = window[name].apply(receiver, args); }
      catch (error) { return name + ' threw synchronously'; }
      if (Object.getPrototypeOf(promise) !== Promise.prototype) return name + ' wrong Promise realm';
      try { await promise; return name + ' unexpectedly fulfilled'; }
      catch (error) {
        if (receiver === native ? error !== marker : !(error instanceof TypeError)) {
          return name + ' wrong rejection';
        }
      }
    }
  }
  return conversions === 5 && traps === 0 ? 'ok' : 'brand checks ran author code';
})().then(
  result => { __nativePromiseReceiversResult = result; },
  error => { __nativePromiseReceiversResult = String(error); }
);
"#,
        None,
    )
    .unwrap();
    vm.eval("0").unwrap();
    assert_eq!(vm.eval("__nativePromiseReceiversResult").unwrap(), "ok");
}

#[tokio::test(flavor = "current_thread")]
async fn window_promise_methods_reject_in_the_callee_realm_before_argument_conversion() {
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/window-promise-method-receivers.js"
    ));
    let server = StaticHttpServer::spawn_with_bodies(vec![
        "<!doctype html><body>local".to_owned(),
        "<!doctype html><body>remote".to_owned(),
    ])
    .await;
    let loader = static_http_loader([
        server.resolve_entry("localhost"),
        server.resolve_entry("127.0.0.1"),
    ]);
    let parent_url = server.url_for_host("localhost", "/page.html");
    let same_url = server.url_for_host("localhost", "/local.html");
    let cross_url = server.url_for_host("127.0.0.1", "/remote.html");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.exec(
        &format!(
            r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__promiseMethodReceiversResult = null;
({script})({{sameURL: {same_url:?}, crossURL: {cross_url:?}}}).then(
  result => {{ __promiseMethodReceiversResult = result; }},
  error => {{ __promiseMethodReceiversResult = {{error: String(error)}}; }}
);
"#,
            same_url = same_url.as_str(),
            cross_url = cross_url.as_str(),
        ),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__promiseMethodReceiversResult !== null)",
        "true",
        "Window Promise method receiver regression should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__promiseMethodReceiversResult)")
            .expect("Window Promise method receiver observations"),
    )
    .unwrap();
    assert_eq!(result["checks"], 530, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(
        server.finish_targets().await,
        ["/local.html", "/remote.html"]
    );
}
