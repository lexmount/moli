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
  let shadowedByListener = false;
  const shadow = () => {
    for (const [name,value] of Object.entries({message:'fake',filename:'fake.js',lineno:90,colno:91,error:42})) {
      Object.defineProperty(event, name, {configurable:true, get() {
        reads++;
        if (config.mode !== 'shadow-values') throw new Error('author getter '+name);
        return value;
      }});
    }
  };
  const before = received => {
    if (received === event) {
      shadow();
      shadowedByListener = true;
    }
  };
  if (config.mode === 'shadow' || config.mode === 'shadow-values') shadow();
  if (config.mode === 'listener-shadow') target.addEventListener('error', before);
  let callback = function(...args) {
    calls++;
    called = {count:args.length,receiver:this===target,
      values:ordinary ? [args[0]===event] : [args[0],args[1],args[2],args[3],args[4]===marker],
      shadowedByListener};
    return ordinary ? !config.cancel : config.cancel;
  };
  if (config.mode === 'proxy') callback = new Proxy(callback, {get() {proxyGets++;throw new Error('callback getter');}});
  const old = target.onerror;
  target.onerror = callback;
  let returned = null, threw = null;
  try { returned = config.mode === 'borrowed' ? EventTarget.prototype.dispatchEvent.call(target,event) : target.dispatchEvent(event); }
  catch (error) {threw=error.name;}
  finally {
    target.onerror=old;
    if (config.mode === 'listener-shadow') target.removeEventListener('error',before);
    if(config.kind==='popup')target.close();
    frame.remove();
  }
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
            "shadow-values",
            "listener-shadow",
            "foreign",
            "proxy",
            "borrowed",
        ] {
            if (mode == "borrowed" && kind != "popup")
                || (mode == "listener-shadow" && kind == "window")
            {
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
                        "called":{"count":if ordinary {1} else {5},"receiver":true,"values":values,
                            "shadowedByListener":mode == "listener-shadow"},
                        "reads":0,"proxyGets":0,"returned":!cancel,"threw":null,
                    }),
                    "{kind}/{mode}/cancel={cancel}"
                );
            }
        }
    }
}

const NATIVE_PROBE: &str = r#"(() => {
  const config = __CONFIG__;
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const frame = config.kind === 'child' ? document.createElement('iframe') : null;
  if (frame) document.body.appendChild(frame);
  const target = frame ? frame.contentWindow : window;
  const marker = new target.TypeError('native marker');
  target.__nativeMarker = marker;
  const trace = [];
  let reads = 0, called = null;
  const before = event => {
    trace.push('before');
    for (const [name, value] of Object.entries({message:'fake',filename:'fake.js',lineno:90,colno:91,error:42})) {
      Object.defineProperty(event, name, {configurable:true,get() {
        reads++;
        if (config.mode === 'throw') throw new Error('must not read ' + name);
        return value;
      }});
    }
  };
  const after = () => trace.push('after');
  const old = target.onerror;
  target.addEventListener('error', before);
  target.onerror = function(message,source,line,column,error) {
    trace.push('handler');
    called = {count:arguments.length,receiver:this===target,identity:error===marker,
      message:typeof message==='string' && message.includes('native marker'),
      locationTypes:[typeof source,typeof line,typeof column]};
    return true;
  };
  target.addEventListener('error', after);
  let returned = null, threw = null;
  try {
    const button = target.document.createElement('button');
    button.addEventListener('probe', new target.Function('throw globalThis.__nativeMarker'));
    returned = button.dispatchEvent(new target.Event('probe'));
  } catch (error) { threw = error.name; }
  finally {
    target.onerror = old;
    target.removeEventListener('error', before);
    target.removeEventListener('error', after);
    delete target.__nativeMarker;
    if (frame) frame.remove();
  }
  return {trace,called,reads,returned,threw};
})()
"#;

#[test]
fn child_window_onerror_preserves_native_exception_after_listener_shadows_attributes() {
    for mode in ["values", "throw"] {
        let mut vm = new_storage_test_vm("https://window-onerror-data.test/");
        let source = NATIVE_PROBE.replace(
            "__CONFIG__",
            &serde_json::json!({"kind":"child","mode":mode}).to_string(),
        );
        let value = vm
            .eval(&format!("JSON.stringify({source})"))
            .unwrap_or_else(|error| panic!("{mode}: {error:?}"));
        let actual: serde_json::Value = serde_json::from_str(&value).unwrap();
        assert_eq!(
            actual,
            serde_json::json!({
                "trace":["before","handler","after"],
                "called":{"count":5,"receiver":true,"identity":true,"message":true,"locationTypes":["string","number","number"]},
                "reads":0,"returned":true,"threw":null,
            }),
            "{mode}"
        );
    }
}
