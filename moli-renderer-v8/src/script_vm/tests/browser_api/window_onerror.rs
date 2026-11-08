use super::*;

const PROBE: &str = r#"(() => {
  const config = __CONFIG__;
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  if (!document.body) html.appendChild(document.createElement('body'));
  const frame = document.body.appendChild(document.createElement('iframe'));
  const child = frame.contentWindow;
  const realm = config.kind === 'child' ? child : window;
  const target = config.kind === 'popup' ? open('about:blank') : realm;
  const eventRealm = config.mode === 'foreign' ? (realm === window ? child : window) : realm;
  const marker = {};
  const ordinary = config.mode === 'plain' || config.mode === 'forged';
  const event = ordinary ? new eventRealm.Event('error', {cancelable:true}) :
    new eventRealm.ErrorEvent('error', {message:'original',filename:'source.js',lineno:12,colno:34,error:marker,cancelable:true});
  if (config.mode === 'forged') Object.setPrototypeOf(event, eventRealm.ErrorEvent.prototype);
  if (config.mode === 'prototype') Object.setPrototypeOf(event, eventRealm.Event.prototype);
  let reads = 0, proxyGets = 0, calls = 0, called = null;
  if (config.mode === 'shadow') {
    for (const name of ['message','filename','lineno','colno','error']) {
      Object.defineProperty(event, name, {configurable:true, get() {reads++;throw new Error('author getter '+name);}});
    }
  }
  let callback = function(...args) {
    calls++;
    called = {count:args.length,receiver:this===target,
      values:ordinary ? [args[0]===event] : [args[0],args[1],args[2],args[3],args[4]===marker]};
    return ordinary ? !config.cancel : config.cancel;
  };
  if (config.mode === 'proxy') callback = new Proxy(callback, {get() {proxyGets++;throw new Error('callback getter');}});
  const old = target.onerror;
  target.onerror = callback;
  let returned = null, threw = null;
  try { returned = config.mode === 'borrowed' ? EventTarget.prototype.dispatchEvent.call(target,event) : target.dispatchEvent(event); }
  catch (error) {threw=error.name;}
  finally {target.onerror=old;if(config.kind==='popup')target.close();frame.remove();}
  return {calls,called,reads,proxyGets,returned,threw};
})()
"#;

#[tokio::test]
async fn window_onerror_uses_native_event_kind_for_arguments_and_cancellation() {
    for kind in ["window", "child", "popup"] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, _runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                "https://window-onerror-data.test/",
                &loader,
            );
        for mode in [
            "native",
            "plain",
            "forged",
            "prototype",
            "shadow",
            "foreign",
            "proxy",
            "borrowed",
        ] {
            if mode == "borrowed" && kind != "popup" {
                continue;
            }
            for cancel in [false, true] {
                let source = PROBE.replace(
                    "__CONFIG__",
                    &serde_json::json!({"kind":kind,"mode":mode,"cancel":cancel}).to_string(),
                );
                let value = vm
                    .eval(&format!("JSON.stringify({source})"))
                    .unwrap_or_else(|error| panic!("{kind}/{mode}/cancel={cancel}: {error:?}"));
                let actual: serde_json::Value = serde_json::from_str(&value).unwrap();
                let ordinary = matches!(mode, "plain" | "forged");
                let values = if ordinary {
                    serde_json::json!([true])
                } else {
                    serde_json::json!(["original", "source.js", 12, 34, true])
                };
                assert_eq!(
                    actual,
                    serde_json::json!({
                        "calls":1,
                        "called":{"count":if ordinary {1} else {5},"receiver":true,"values":values},
                        "reads":0,"proxyGets":0,"returned":!cancel,"threw":null,
                    }),
                    "{kind}/{mode}/cancel={cancel}"
                );
            }
        }
    }
}
