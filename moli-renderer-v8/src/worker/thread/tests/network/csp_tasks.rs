use super::*;
use crate::worker::WorkerGlobalKind;

#[tokio::test]
async fn worker_fetch_csp_tasks_follow_microtasks_without_reentering_listeners() {
    ensure_v8();
    for shared in [false, true] {
        for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
            for report_only in [false, true] {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let base = format!("http://{}", listener.local_addr().unwrap());
                let script_url = url::Url::parse(&format!("{base}/worker.js")).unwrap();
                let expected = if report_only { 4 } else { 2 };
                let source = r#"
const base = BASE;
const events = [];
const outcomes = {};
const microtasks = {};
let inFetch = false;
let inListener = false;
let nestedStarted = false;
let published = false;
let finishEvaluation;
const finished = new Promise(resolve => finishEvaluation = resolve);
const finish = () => {
  if (!published && events.length === EXPECTED && outcomes.outer && outcomes.nested) {
    published = true;
    console.log(JSON.stringify({events, outcomes}));
    finishEvaluation();
  }
};
const load = label => {
  inFetch = true;
  const promise = fetch(base + '/' + label);
  inFetch = false;
  queueMicrotask(() => { microtasks[label] = true; });
  promise.then(() => {
    outcomes[label] = {unexpectedFulfillment:true};
    finish();
  }, error => {
    outcomes[label] = {name:error.name, native:error instanceof TypeError};
    finish();
  });
};
addEventListener('securitypolicyviolation', event => {
  const label = new URL(event.blockedURI).pathname.slice(1);
  events.push({label, inFetch, inListener,
    afterMicrotask:microtasks[label] === true, afterRejection:!!outcomes[label],
    disposition:event.disposition, directive:event.effectiveDirective,
    native:event instanceof SecurityPolicyViolationEvent, trusted:event.isTrusted,
    target:event.target === self, sourceFile:event.sourceFile,
    line:event.lineNumber, column:event.columnNumber});
  const previous = inListener;
  inListener = true;
  if (label === 'outer' && !nestedStarted) {
    nestedStarted = true;
    load('nested');
  }
  inListener = previous;
  finish();
});
load('outer');
"#
                .replace("BASE", &serde_json::to_string(&base).unwrap())
                .replace("EXPECTED", &expected.to_string());
                let source = match kind {
                    WorkerScriptKind::Classic => source,
                    WorkerScriptKind::Module => format!("{source}\nawait finished;"),
                };
                let global = if shared {
                    WorkerGlobalKind::Shared {
                        name: "csp-tasks".into(),
                        storage_key: moli_storage_key::MoliStorageKey::first_party_from_url(
                            &script_url,
                            None,
                        ),
                    }
                } else {
                    WorkerGlobalKind::Dedicated {
                        name: "csp-tasks".into(),
                    }
                };
                let mut options = WorkerSpawnOptions::new(source, script_url.to_string())
                    .with_global_kind(global)
                    .with_script_kind(kind)
                    .with_content_security_policies(vec!["connect-src 'none'".into()]);
                if report_only {
                    options = options.with_content_security_report_only_policies(vec![
                        "connect-src 'none'".into(),
                    ]);
                }
                let mut worker = spawn_test_worker_with_options(options);
                let result = timeout(TIMEOUT, async {
                    loop {
                        match worker.recv().await.expect("worker must report its result") {
                            WorkerToParentMessage::Console(console) => {
                                break console.message;
                            }
                            WorkerToParentMessage::SubresourceNetwork(_) => {}
                            other => panic!("unexpected worker message: {other:?}"),
                        }
                    }
                })
                .await
                .expect("CSP tasks and their promise microtasks must settle");
                worker.terminate_and_join();
                let result: serde_json::Value =
                    serde_json::from_str(result.strip_prefix("log: ").unwrap()).unwrap();
                assert_eq!(
                    result["outcomes"],
                    serde_json::json!({
                        "outer":{"name":"TypeError", "native":true},
                        "nested":{"name":"TypeError", "native":true},
                    }),
                    "shared={shared}, kind={kind:?}, report_only={report_only}"
                );
                let events = result["events"].as_array().unwrap();
                assert_eq!(events.len(), expected);
                assert_eq!(events[0]["disposition"], "enforce");
                for label in ["outer", "nested"] {
                    assert_eq!(
                        events
                            .iter()
                            .filter(|event| event["label"] == label)
                            .count(),
                        if report_only { 2 } else { 1 }
                    );
                }
                for event in events {
                    assert_eq!(event["inFetch"], false, "{result}");
                    assert_eq!(event["inListener"], false, "{result}");
                    for flag in [
                        "afterMicrotask",
                        "afterRejection",
                        "native",
                        "trusted",
                        "target",
                    ] {
                        assert_eq!(event[flag], true, "{flag}: {result}");
                    }
                    assert_eq!(event["directive"], "connect-src");
                    assert_eq!(event["sourceFile"], script_url.as_str());
                    assert!(event["line"].as_u64().unwrap() > 0);
                    assert!(event["column"].as_u64().unwrap() > 0);
                }
                assert!(
                    timeout(Duration::from_millis(50), listener.accept())
                        .await
                        .is_err(),
                    "CSP must still block before connecting"
                );
            }
        }
    }
}
