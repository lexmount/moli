use super::*;

#[test]
fn window_event_receivers_accept_native_bridge_wrappers_without_inheriting_their_brand() {
    let mut vm = new_storage_test_vm("https://window-event-receivers.test/");
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const native = __moliNativeBridge.window;
  if (native.window !== window || native.self !== window || native.console !== window.console) {
    return 'native Window aliases';
  }
  let traps = 0;
  const proxy = new Proxy(native, {
    get() { traps++; throw new Error('unexpected trap'); }
  });
  for (const name of ['onmouseenter', 'onmouseleave', 'onclick', 'onerror']) {
    const descriptor = Object.getOwnPropertyDescriptor(window, name);
    const callback = () => {};
    descriptor.set.call(native, callback);
    if (descriptor.get.call(native) !== callback || window[name] !== callback) {
      return name + ' native handler';
    }
    const lenient = name === 'onmouseenter' || name === 'onmouseleave';
    for (const receiver of [proxy, Object.create(native), {__moliNativeBridge}]) {
      for (const operation of [
        () => descriptor.get.call(receiver),
        () => descriptor.set.call(receiver, null)
      ]) {
        try {
          if (operation() !== undefined || !lenient) return name + ' accepted forged receiver';
        } catch (error) {
          if (lenient || !(error instanceof TypeError)) return name + ' wrong receiver error';
        }
      }
      if (window[name] !== callback) return name + ' forged write mutated handler';
    }
    descriptor.set.call(native, null);
    if (descriptor.get.call(native) !== null) return name + ' native clear';
  }
  return traps === 0 ? 'ok' : 'receiver invoked traps';
})()
"#,
        )
        .unwrap(),
        "ok",
    );
}

#[tokio::test(flavor = "current_thread")]
async fn window_event_receivers_preserve_leniency_native_brands_and_origin_checks() {
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/window-event-receivers.js"
    ));
    let child = format!("<!doctype html><body><script>({script})({{role:'child'}})</script>");
    let server = StaticHttpServer::spawn_with_bodies(vec![child; 4]).await;
    let loader = static_http_loader([
        server.resolve_entry("www.example.test"),
        server.resolve_entry("remote.example.test"),
    ]);
    let parent_url = server.url_for_host("www.example.test", "/page.html");
    let cross_url = server.url_for_host("remote.example.test", "/child.html");
    let same_url = server.url_for_host("www.example.test", "/child.html");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.exec(
        &format!(
            r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__windowEventReceiverResult = null;
({script})({{crossURL: {cross_url:?}, sameURL: {same_url:?}}}).then(
  result => {{ __windowEventReceiverResult = result; }},
  error => {{ __windowEventReceiverResult = {{error: String(error)}}; }}
);
"#,
            cross_url = cross_url.as_str(),
            same_url = same_url.as_str(),
        ),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__windowEventReceiverResult !== null)",
        "true",
        "Window event receiver probe should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__windowEventReceiverResult)")
            .expect("Window event receiver observations"),
    )
    .unwrap();
    assert_eq!(result["checks"], 868, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(server.finish_targets().await, vec!["/child.html"; 4]);
}
