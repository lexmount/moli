use super::*;
use crate::worker::WorkerGlobalKind;

#[tokio::test]
async fn worker_xhr_csp_tasks_preserve_send_and_failure_order_without_reentrancy() {
    ensure_v8();
    for shared in [false, true] {
        for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
            for asynchronous in [false, true] {
                for report_only in [false, true] {
                    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let base = format!("http://{}", listener.local_addr().unwrap());
                    let script_url = url::Url::parse(&format!("{base}/worker.js")).unwrap();
                    let expected = if report_only { 4 } else { 2 };
                    let source = r#"
const base = BASE;
const events = [], outcomes = {}, microtasks = {};
let inSend = false, inListener = false, nestedStarted = false, published = false;
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
  const xhr = new XMLHttpRequest();
  xhr.open('GET', base + '/' + label, ASYNCHRONOUS);
  xhr.onerror = event => {
    outcomes[label] = {event:event.type, status:xhr.status, readyState:xhr.readyState,
      native:event instanceof ProgressEvent, target:event.target === xhr};
    finish();
  };
  inSend = true;
  try { xhr.send(); }
  catch (error) {
    outcomes[label] = {name:error.name, code:error.code, native:error instanceof DOMException,
      status:xhr.status, readyState:xhr.readyState};
  }
  inSend = false;
  queueMicrotask(() => { microtasks[label] = true; });
  finish();
};
addEventListener('securitypolicyviolation', event => {
  const label = new URL(event.blockedURI).pathname.slice(1);
  events.push({label, inSend, inListener, afterMicrotask:microtasks[label] === true,
    afterFailure:!!outcomes[label], disposition:event.disposition,
    directive:event.effectiveDirective, native:event instanceof SecurityPolicyViolationEvent,
    trusted:event.isTrusted, target:event.target === self, sourceFile:event.sourceFile,
    line:event.lineNumber, column:event.columnNumber});
  inListener = true;
  if (!nestedStarted) { nestedStarted = true; load('nested'); }
  inListener = false;
  finish();
});
load('outer');
"#
                    .replace("BASE", &serde_json::to_string(&base).unwrap())
                    .replace("EXPECTED", &expected.to_string())
                    .replace("ASYNCHRONOUS", if asynchronous { "true" } else { "false" });
                    let source = match kind {
                        WorkerScriptKind::Classic => source,
                        WorkerScriptKind::Module => format!("{source}\nawait finished;"),
                    };
                    let global = if shared {
                        WorkerGlobalKind::Shared {
                            name: "xhr-csp-tasks".into(),
                            storage_key: moli_storage_key::MoliStorageKey::first_party_from_url(
                                &script_url,
                                None,
                            ),
                        }
                    } else {
                        WorkerGlobalKind::Dedicated {
                            name: "xhr-csp-tasks".into(),
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
                                WorkerToParentMessage::Console(console) => break console.message,
                                WorkerToParentMessage::SubresourceNetwork(_) => {}
                                other => panic!("unexpected worker message: {other:?}"),
                            }
                        }
                    })
                    .await
                    .expect("XHR failures and CSP tasks must settle during module evaluation");
                    worker.terminate_and_join();
                    let result: serde_json::Value =
                        serde_json::from_str(result.strip_prefix("log: ").unwrap()).unwrap();
                    let outcome = if asynchronous {
                        serde_json::json!({
                            "event":"error", "status":0, "readyState":4,
                            "native":true, "target":true,
                        })
                    } else {
                        serde_json::json!({
                            "name":"NetworkError", "code":19, "native":true,
                            "status":0, "readyState":4,
                        })
                    };
                    assert_eq!(result["outcomes"]["outer"], outcome, "{result}");
                    assert_eq!(result["outcomes"]["nested"], outcome, "{result}");
                    let events = result["events"].as_array().unwrap();
                    assert_eq!(events.len(), expected);
                    for (index, event) in events.iter().enumerate() {
                        assert_eq!(
                            event["label"],
                            if index < expected / 2 {
                                "outer"
                            } else {
                                "nested"
                            }
                        );
                        assert_eq!(event["inSend"], false, "{result}");
                        assert_eq!(event["inListener"], false, "{result}");
                        assert_eq!(event["afterFailure"], !asynchronous, "{result}");
                        assert_eq!(
                            event["disposition"],
                            if report_only && index % 2 == 0 {
                                "report"
                            } else {
                                "enforce"
                            }
                        );
                        for flag in ["afterMicrotask", "native", "trusted", "target"] {
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
                        "CSP must block before opening a network connection"
                    );
                }
            }
        }
    }
}
