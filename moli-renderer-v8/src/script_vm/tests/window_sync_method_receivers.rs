use super::*;

#[test]
fn window_sync_methods_accept_native_bridge_wrappers_without_inheriting_their_brand() {
    let mut vm = new_storage_test_vm("https://window-sync-receivers.test/");
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const native = __moliNativeBridge.window;
  const element = document.documentElement || document.appendChild(document.createElement('html'));
  let traps = 0, conversions = 0;
  const proxy = new Proxy(native, {get() { traps++; throw new Error('unexpected trap'); }});
  const input = {toString() { conversions++; return 'a'; }};
  const calls = [
    ['btoa', ['a'], 'string'], ['atob', ['YQ=='], 'string'],
    ['getComputedStyle', [element], 'object'], ['getSelection', [], 'object'],
    ['matchMedia', ['all'], 'object'], ['find', [''], 'boolean'],
    ['captureEvents', [], 'undefined'], ['releaseEvents', [], 'undefined'],
    ['print', [], 'undefined'], ['clearImmediate', [], 'undefined']
  ];
  for (const [name, args, type] of calls) {
    for (const receiver of [proxy, Object.create(native), {__moliNativeBridge}]) {
      try {
        window[name].call(receiver, input);
        return name + ' accepted forged receiver';
      } catch (error) {
        if (!(error instanceof TypeError)) return name + ' wrong receiver error';
      }
    }
    if (typeof window[name].apply(native, args) !== type) return name + ' native result';
  }
  return traps === 0 && conversions === 0 ? 'ok' : 'receiver check ran author code';
})()
"#,
        )
        .unwrap(),
        "ok",
    );
}

#[tokio::test(flavor = "current_thread")]
async fn window_sync_methods_check_native_receivers_before_conversion_and_use_callee_errors() {
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/window-sync-method-receivers.js"
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
globalThis.__syncMethodReceiversResult = null;
({script})({{sameURL: {same_url:?}, crossURL: {cross_url:?}}}).then(
  result => {{ __syncMethodReceiversResult = result; }},
  error => {{ __syncMethodReceiversResult = {{error: String(error)}}; }}
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
        "String(__syncMethodReceiversResult !== null)",
        "true",
        "Window synchronous method receiver regression should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__syncMethodReceiversResult)")
            .expect("Window synchronous method receiver observations"),
    )
    .unwrap();
    assert_eq!(result["checks"], 1169, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(
        server.finish_targets().await,
        ["/local.html", "/remote.html"]
    );
}
