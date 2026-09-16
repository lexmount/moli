use super::*;

const INVENTORY_PROBE: &str = r#"
(() => {
  const rows = {};
  const errors = [];
  let marker;
  self.addEventListener('error', event => { errors.push({same: event.error === marker, name: event.error?.name}); event.preventDefault(); });
  for (const kind of ['global', 'target', 'reader', 'xhr', 'performance', 'broadcast']) {
    let target, type = 'load', cleanup = () => {};
    switch (kind) {
      case 'global': target = self; type = 'message'; break;
      case 'target': target = new EventTarget(); type = 'probe'; break;
      case 'reader': target = new FileReader(); break;
      case 'xhr': target = new XMLHttpRequest(); break;
      case 'performance': target = performance; type = 'resourcetimingbufferfull'; break;
      case 'broadcast': target = new BroadcastChannel('synthetic-errors'); type = 'message'; cleanup = () => target.close(); break;
    }
    for (const mode of ['function', 'getter', 'method', 'revoked', ...(kind === 'target' ? [] : ['handler'])]) {
      marker = new TypeError('marker');
      const callback = mode === 'getter' ? {get handleEvent() { throw marker; }}
        : mode === 'method' ? {handleEvent() { throw marker; }} : () => { throw marker; };
      let value = callback;
      if (mode === 'revoked') { const pair = Proxy.revocable(callback, {}); value = pair.proxy; pair.revoke(); }
      const after = [];
      const afterListener = () => after.push('after');
      if (mode === 'handler') target['on' + type] = value;
      else target.addEventListener(type, value);
      target.addEventListener(type, afterListener);
      let returned, escaped;
      try { returned = target.dispatchEvent(new Event(type, {cancelable:true})); }
      catch (error) { escaped = error.name; }
      rows[kind + '/' + mode] = {returned, escaped, after, errors: errors.splice(0)};
      if (mode === 'handler') target['on' + type] = null;
      else target.removeEventListener(type, value);
      target.removeEventListener(type, afterListener);
    }
    cleanup();
  }
  return rows;
})()
"#;

const REPORTING_PROBE: &str = r#"
const config = __CONFIG__;
const local = [], after = [], returned = [];
const outer = new TypeError('outer'), inner = new RangeError('inner'), again = new SyntaxError('again');
let current = outer, round = 0, nestedCalls = 0;
const tag = error => error === outer ? 'outer' : error === inner ? 'inner' : error === again ? 'again' : 'unexpected';
const nestedTarget = new EventTarget();
nestedTarget.addEventListener('probe', () => { throw inner; });
function nest(error) {
  if (error !== outer || nestedCalls) return;
  if (config.nested === 'dispatch') { nestedCalls++; nestedTarget.dispatchEvent(new Event('probe')); }
  if (config.nested === 'throw') { nestedCalls++; throw inner; }
}
if (config.handler === 'handler') {
  self.onerror = function(message, filename, line, column, error) {
    local.push({tag:tag(error), receiverOK:this === self, argumentsOK:arguments.length === 5});
    nest(error);
    return config.cancel;
  };
} else {
  self.addEventListener('error', function(event) {
    local.push({tag:tag(event.error), receiverOK:this === self, argumentsOK:event instanceof ErrorEvent && event.target === self});
    if (config.cancel) event.preventDefault();
    nest(event.error);
  });
}
const target = new EventTarget();
target.addEventListener('probe', () => { throw current; });
target.addEventListener('probe', () => after.push(tag(current)));
function finish() { postMessage({done:true, local, after, returned, nestedCalls}); if (config.closeAfter) close(); }
function run() {
  round++;
  current = round === 1 ? outer : again;
  if (config.trigger === 'synthetic') {
    returned.push(target.dispatchEvent(new Event('probe', {cancelable:true})));
    setTimeout(round === 1 ? run : finish, 0);
  } else {
    setTimeout(round === 1 ? run : finish, 0);
    throw current;
  }
}
self.addEventListener('message', event => {
  if (event.data === 'run' && config.trigger === 'message') run();
  else if (event.data?.ping) postMessage({pong:event.data.ping});
});
if (config.trigger === 'synthetic') run();
else if (config.trigger === 'timer') setTimeout(run, 0);
else if (config.trigger === 'microtask') queueMicrotask(run);

"#;

#[tokio::test]
async fn worker_synthetic_listener_exceptions_report_original_values_and_continue_dispatch() {
    ensure_v8();
    let mut handle = spawn_worker(
        format!("postMessage({INVENTORY_PROBE}); close();"),
        "https://worker-errors.test/inventory.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("worker inventory timed out")
        .expect("worker channel closed");
    let actual: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    let mut expected = serde_json::Map::new();
    for kind in [
        "global",
        "target",
        "reader",
        "xhr",
        "performance",
        "broadcast",
    ] {
        for mode in ["function", "getter", "method", "revoked", "handler"] {
            if kind == "target" && mode == "handler" {
                continue;
            }
            expected.insert(
                format!("{kind}/{mode}"),
                serde_json::json!({
                    "returned": true,
                    "after": ["after"],
                    "errors": [{"same": mode != "revoked", "name": "TypeError"}],
                }),
            );
        }
    }
    assert_eq!(actual, serde_json::Value::Object(expected));
}

#[tokio::test]
async fn worker_synthetic_error_reporting_cancels_propagation_and_prevents_reentrancy() {
    ensure_v8();
    for (trigger, handler, cancel, nested, module, expected_errors) in [
        ("synthetic", "listener", true, "none", false, &[][..]),
        (
            "synthetic",
            "listener",
            false,
            "none",
            false,
            &["again", "outer"][..],
        ),
        ("synthetic", "handler", true, "none", false, &[][..]),
        (
            "synthetic",
            "handler",
            false,
            "none",
            false,
            &["again", "outer"][..],
        ),
        (
            "synthetic",
            "listener",
            true,
            "dispatch",
            false,
            &["inner"][..],
        ),
        (
            "synthetic",
            "listener",
            false,
            "dispatch",
            false,
            &["again", "inner", "outer"][..],
        ),
        (
            "synthetic",
            "handler",
            true,
            "dispatch",
            false,
            &["inner"][..],
        ),
        ("timer", "listener", true, "dispatch", false, &["inner"][..]),
        ("timer", "handler", true, "dispatch", false, &["inner"][..]),
        (
            "timer",
            "listener",
            false,
            "throw",
            false,
            &["again", "inner", "outer"][..],
        ),
        (
            "timer",
            "handler",
            true,
            "throw",
            false,
            &["inner", "outer"][..],
        ),
        ("message", "listener", true, "none", false, &[][..]),
        ("microtask", "handler", true, "none", false, &[][..]),
        ("synthetic", "listener", true, "none", true, &[][..]),
    ] {
        let config = serde_json::json!({
            "trigger": trigger, "handler": handler, "cancel": cancel,
            "nested": nested, "module": module, "closeAfter": true,
        });
        let source = REPORTING_PROBE.replace("__CONFIG__", &config.to_string());
        let kind = if module {
            WorkerScriptKind::Module
        } else {
            WorkerScriptKind::Classic
        };
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(source, "https://worker-errors.test/reporting.js".into())
                .with_script_kind(kind),
        );
        if trigger == "message" {
            handle.post_message(serialize_test_string("run"));
        }
        let mut errors = Vec::new();
        let actual = timeout(TIMEOUT, async {
            loop {
                match handle.recv().await.expect("worker channel closed") {
                    WorkerToParentMessage::Post(payload) => {
                        break serde_json::from_str::<serde_json::Value>(&stringify_payload(
                            &payload,
                        ))
                        .unwrap();
                    }
                    WorkerToParentMessage::Error { message, phase, .. } => {
                        assert_eq!(phase, WorkerErrorPhase::Runtime, "{config}");
                        let tag = message.split_whitespace().last().expect("error message");
                        assert!(
                            ["outer", "inner", "again"].contains(&tag),
                            "{config}: {message}"
                        );
                        errors.push(tag.to_owned());
                    }
                    other => panic!("{config}: unexpected worker message {other:?}"),
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{config}: worker reporting timed out"));
        let synthetic = trigger == "synthetic";
        assert_eq!(
            actual,
            serde_json::json!({
                "done": true,
                "local": [
                    {"tag": "outer", "receiverOK": true, "argumentsOK": true},
                    {"tag": "again", "receiverOK": true, "argumentsOK": true},
                ],
                "after": if synthetic { vec!["outer", "again"] } else { vec![] },
                "returned": if synthetic { vec![true, true] } else { vec![] },
                "nestedCalls": u8::from(nested != "none"),
            }),
            "{config}"
        );
        errors.sort();
        assert_eq!(errors, expected_errors, "{config}");
    }
}
