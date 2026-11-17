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

#[tokio::test]
async fn window_onerror_preserves_original_data_during_ordered_dispatch() {
    let kind = "window";
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, _runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            "https://window-onerror-data.test/",
            &loader,
        );
    for mode in [
        "order",
        "values",
        "throw",
        "listener-shadow",
        "foreign",
        "prototype",
        "forged",
        "plain",
        "proxy",
        "borrowed",
        "body",
    ] {
        let source = SYNTHETIC_PROBE.replace(
            "__CONFIG__",
            &serde_json::json!({"kind":kind,"mode":mode}).to_string(),
        );
        let value = vm
            .eval(&format!("JSON.stringify({source})"))
            .unwrap_or_else(|error| panic!("{kind}/{mode}: {error:?}"));
        let actual: serde_json::Value = serde_json::from_str(&value).unwrap();
        let ordinary = matches!(mode, "plain" | "forged");
        let arguments = if ordinary {
            serde_json::json!([true])
        } else {
            serde_json::json!(["original", "source.js", 12, 34, true])
        };
        assert_eq!(
            actual,
            serde_json::json!({
                "trace":["before","handler","after"],
                "called":{"count":if ordinary {1} else {5},"receiver":true,"values":arguments},
                "reads":0,"proxyGets":0,"returned":false,"threw":null,
            }),
            "{kind}/{mode}"
        );
    }
}

#[test]
fn window_onerror_native_listener_exceptions_preserve_data_after_attribute_shadowing() {
    for kind in ["window", "child"] {
        for mode in ["values", "throw"] {
            let mut vm = new_storage_test_vm("https://window-onerror-data.test/");
            let source = NATIVE_PROBE.replace(
                "__CONFIG__",
                &serde_json::json!({"kind":kind,"mode":mode}).to_string(),
            );
            let value = vm
                .eval(&format!("JSON.stringify({source})"))
                .unwrap_or_else(|error| panic!("{kind}/{mode}: {error:?}"));
            let actual: serde_json::Value = serde_json::from_str(&value).unwrap();
            assert_eq!(
                actual,
                serde_json::json!({
                    "trace":["before","handler","after"],
                    "called":{"count":5,"receiver":true,"identity":true,"message":true,"locationTypes":["string","number","number"]},
                    "reads":0,"returned":true,"threw":null,
                }),
                "{kind}/{mode}"
            );
        }
    }
}

const SYNTHETIC_PROBE: &str = r#"(() => {
  const config = __CONFIG__;
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const child = frame.contentWindow;
  const realm = config.kind === 'child' ? child : window;
  const target = config.kind === 'popup' ? open('about:blank') : realm;
  if (!target) throw new Error('popup did not open');
  if (!realm.document.body) realm.document.documentElement.appendChild(realm.document.createElement('body'));
  const holder = config.mode === 'body' ? realm.document.body : target;
  const eventRealm = config.mode === 'foreign' ? (realm === window ? child : window) : realm;
  const marker = {};
  const ordinary = config.mode === 'plain' || config.mode === 'forged';
  const event = ordinary ? new eventRealm.Event('error', {cancelable:true}) :
    new eventRealm.ErrorEvent('error', {message:'original',filename:'source.js',lineno:12,colno:34,error:marker,cancelable:true});
  if (config.mode === 'forged') Object.setPrototypeOf(event, eventRealm.ErrorEvent.prototype);
  if (config.mode === 'prototype') Object.setPrototypeOf(event, eventRealm.Event.prototype);
  const trace = [];
  let reads = 0, proxyGets = 0, called = null;
  const shadow = () => {
    for (const [name, value] of Object.entries({message:'fake',filename:'fake.js',lineno:90,colno:91,error:42})) {
      Object.defineProperty(event, name, {configurable:true, get() {
        reads++;
        if (config.mode === 'throw') throw new Error('must not read ' + name);
        return value;
      }});
    }
  };
  if (['values','throw','foreign'].includes(config.mode)) shadow();
  const before = value => {
    if (value !== event) { trace.push('unexpected-error'); value.preventDefault(); return; }
    trace.push('before');
    if (config.mode === 'listener-shadow') shadow();
  };
  const after = value => { if (value === event) trace.push('after'); };
  let callback = function(...args) {
    trace.push('handler');
    called = {count:args.length,receiver:this===target,
      values:args.length===5 ? [args[0],args[1],args[2],args[3],args[4]===marker] : [args[0]===event]};
    return !ordinary;
  };
  if (config.mode === 'proxy') callback = new Proxy(callback, {get() {proxyGets++; throw new Error('callback getter');}});
  const old = holder.onerror;
  target.addEventListener('error', before);
  holder.onerror = callback;
  target.addEventListener('error', after);
  let returned = null, threw = null;
  try {
    returned = config.mode === 'borrowed' ? EventTarget.prototype.dispatchEvent.call(target, event) : target.dispatchEvent(event);
  } catch (error) { threw = error.name; }
  finally {
    holder.onerror = old;
    target.removeEventListener('error', before);
    target.removeEventListener('error', after);
    if (config.kind === 'popup') target.close();
    frame.remove();
  }
  return {trace,called,reads,proxyGets,returned,threw};
})()
"#;

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
fn window_handler_assignments_preserve_listener_positions_and_dispatch_snapshots() {
    let mut vm = new_storage_html_test_vm("https://window-handler-order.test/");
    assert_eq!(vm.eval(r#"(() => {
        const trace = [], batches = [];
        const before = () => trace.push('before'), after = () => trace.push('after');
        const fire = () => { dispatchEvent(new Event('selectstart')); batches.push(trace.splice(0)); };
        addEventListener('selectstart', before);
        onselectstart = () => trace.push('handler');
        addEventListener('selectstart', after);
        fire();
        onselectstart = () => trace.push('replacement');
        fire();
        onselectstart = null;
        onselectstart = () => trace.push('reactivated');
        fire();
        onselectstart = null;
        removeEventListener('selectstart', before); removeEventListener('selectstart', after);
        let first = true;
        const reenter = () => {
            trace.push('before');
            if (first) { first = false; onselectstart = null; onselectstart = () => trace.push('new'); }
        };
        addEventListener('selectstart', reenter);
        onselectstart = () => trace.push('old');
        addEventListener('selectstart', after);
        fire(); fire();
        removeEventListener('selectstart', reenter); removeEventListener('selectstart', after);
        onselectstart = () => trace.push('alone');
        fire(); onselectstart = null;
        return JSON.stringify(batches);
    })()"#).unwrap(), r#"[["before","handler","after"],["before","replacement","after"],["before","after","reactivated"],["before","after"],["before","after","new"],["alone"]]"#);
}

#[test]
fn special_window_handler_slots_preserve_listener_registration_order() {
    let mut vm = new_storage_html_test_vm("https://window-handler-order.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
        return JSON.stringify(['error', 'unhandledrejection', 'rejectionhandled'].map(type => {
            const trace = [];
            const before = () => trace.push('before');
            const after = () => trace.push('after');
            addEventListener(type, before);
            window['on' + type] = () => trace.push('handler');
            addEventListener(type, after);
            dispatchEvent(new Event(type));
            window['on' + type] = null;
            removeEventListener(type, before); removeEventListener(type, after);
            return trace;
        }));
    })()"#
        )
        .unwrap(),
        r#"[["before","handler","after"],["before","handler","after"],["before","handler","after"]]"#
    );
}
