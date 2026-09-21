use super::*;

#[test]
fn window_scheduling_methods_accept_native_bridge_receivers_without_inheriting_their_brand() {
    let mut vm = new_storage_test_vm("https://window-scheduling-receivers.test/");
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const native = __moliNativeBridge.window;
  const cancellations = {setTimeout:'clearTimeout', setInterval:'clearInterval',
    requestAnimationFrame:'cancelAnimationFrame', requestIdleCallback:'cancelIdleCallback'};
  let traps = 0, conversions = 0;
  const proxy = new Proxy(native, {get() { traps++; throw new Error('unexpected trap'); }});
  const number = {valueOf() { conversions++; return 2147483647; }};
  globalThis.__nativeSchedulingCallbacks = 0;
  for (const name of [...Object.keys(cancellations), ...Object.values(cancellations), 'queueMicrotask']) {
    const args = cancellations[name] || name === 'queueMicrotask' ?
      [() => { __nativeSchedulingCallbacks++; }, number] : [number];
    for (const receiver of [proxy, Object.create(native), {__moliNativeBridge}]) {
      try {
        window[name].apply(receiver, args);
        return name + ' accepted forged receiver';
      } catch (error) {
        if (!(error instanceof TypeError)) return name + ' wrong receiver error';
      }
    }
    if (conversions !== 0) return name + ' converted before rejecting forged receiver';
    const id = window[name].apply(native, args);
    if (cancellations[name]) {
      if (!Number.isInteger(id) || id <= 0) return name + ' native timer handle';
      window[cancellations[name]].call(native, id);
    } else if (id !== undefined) {
      return name + ' native return value';
    }
    conversions = 0;
  }
  return traps === 0 ? 'ok' : 'receiver invoked traps';
})()
"#,
        )
        .unwrap(),
        "ok",
    );
    assert_eq!(vm.eval("__nativeSchedulingCallbacks").unwrap(), "1");
}

#[tokio::test(flavor = "current_thread")]
async fn window_scheduling_methods_validate_receivers_before_conversion_and_queuing() {
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/window-scheduling-receivers.js"
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
globalThis.__schedulingReceiversResult = null;
({script})({{sameURL: {same_url:?}, crossURL: {cross_url:?}}}).then(
  result => {{ __schedulingReceiversResult = result; }},
  error => {{ __schedulingReceiversResult = {{error: String(error)}}; }}
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
        "String(__schedulingReceiversResult !== null)",
        "true",
        "Window scheduling receiver regression should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__schedulingReceiversResult)")
            .expect("Window scheduling receiver observations"),
    )
    .unwrap();
    assert_eq!(result["checks"], 863, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(
        server.finish_targets().await,
        ["/local.html", "/remote.html"]
    );
}
