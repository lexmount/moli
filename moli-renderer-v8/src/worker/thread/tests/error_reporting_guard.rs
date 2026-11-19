use super::*;

const REPORTING_PROBE: &str = r#"
const config = __CONFIG__;
const local = [];
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
function finish() { postMessage({done:true, local, nestedCalls}); close(); }
function run() {
  round++;
  current = round === 1 ? outer : again;
  setTimeout(round === 1 ? run : finish, 0);
  throw current;
}
setTimeout(run, 0);

"#;

#[tokio::test]
async fn worker_error_reporting_suppresses_recursive_events_and_resets_between_errors() {
    ensure_v8();
    for (handler, cancel, nested, expected_errors) in [
        ("listener", true, "dispatch", &["inner"][..]),
        ("handler", true, "dispatch", &["inner"][..]),
        ("listener", false, "throw", &["again", "inner", "outer"][..]),
        ("handler", true, "throw", &["inner", "outer"][..]),
    ] {
        let config = serde_json::json!({
            "handler": handler, "cancel": cancel, "nested": nested,
        });
        let source = REPORTING_PROBE.replace("__CONFIG__", &config.to_string());
        let mut handle = spawn_test_worker_with_options(WorkerSpawnOptions::new(
            source,
            "https://worker-errors.test/reporting.js".into(),
        ));
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
                    WorkerToParentMessage::Error(WorkerErrorReport {
                        ref message, phase, ..
                    }) => {
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
        assert_eq!(
            actual,
            serde_json::json!({
                "done": true,
                "local": [
                    {"tag": "outer", "receiverOK": true, "argumentsOK": true},
                    {"tag": "again", "receiverOK": true, "argumentsOK": true},
                ],
                "nestedCalls": u8::from(nested != "none"),
            }),
            "{config}"
        );
        errors.sort();
        assert_eq!(errors, expected_errors, "{config}");
    }
}
