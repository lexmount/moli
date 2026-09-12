use super::*;

const REJECTIONS: [(&str, &str, &str); 6] = [
    (
        "scheme",
        "invalid-protocol://example.test/xhr",
        "not supported",
    ),
    ("file", "file:///moli-policy-must-not-open", "not supported"),
    ("port", "http://example.test:25/xhr", "blocked bad port"),
    (
        "blocked",
        "http://example.test/xhr",
        "net::ERR_BLOCKED_BY_CLIENT",
    ),
    (
        "offline",
        "http://example.test/xhr",
        "Network emulation offline",
    ),
    ("csp", "http://example.test/xhr", "Content Security Policy"),
];

fn rejected_xhr_worker(case: &str, script: String) -> WorkerTestHandle {
    let policy = WorkerNetworkPolicy {
        blocked_url_patterns: if case == "blocked" {
            vec!["http://example.test/*".to_owned()]
        } else {
            vec![]
        },
        network_offline: case == "offline",
        ..WorkerNetworkPolicy::default()
    };
    spawn_test_worker_with_options(
        WorkerSpawnOptions::new(script, "http://example.test/worker.js".to_owned())
            .with_network_policy(policy)
            .with_content_security_policies(if case == "csp" {
                vec!["connect-src 'none'".to_owned()]
            } else {
                vec![]
            }),
    )
}

async fn collect_rejected_xhr_output(
    mut handle: WorkerTestHandle,
) -> (serde_json::Value, Vec<SubresourceNetworkRecord>) {
    let mut posts = vec![];
    let mut records = vec![];
    while let Some(message) = timeout(TIMEOUT, handle.recv())
        .await
        .expect("worker XHR should finish and close")
    {
        match message {
            WorkerToParentMessage::Post(payload) => {
                posts.push(serde_json::from_str(&stringify_payload(&payload)).unwrap());
            }
            WorkerToParentMessage::SubresourceNetwork(record) => records.push(record),
            other => panic!("unexpected worker XHR message: {other:?}"),
        }
    }
    assert_eq!(posts.len(), 1, "worker should post exactly one result");
    (posts.remove(0), records)
}

#[tokio::test]
async fn worker_xhr_early_failure_runs_after_send_and_microtasks() {
    ensure_v8();
    for (case, url, error_text) in REJECTIONS {
        let script = r#"
            const events = [];
            const xhr = new XMLHttpRequest();
            xhr.onreadystatechange = () => events.push("state:" + xhr.readyState);
            for (const [target, prefix] of [[xhr, "xhr"], [xhr.upload, "upload"]]) {
                for (const type of ["loadstart", "progress", "error", "abort", "load", "loadend"]) {
                    target.addEventListener(type, event => events.push(
                        `${prefix}:${type}:${event.loaded}:${event.total}:${event.lengthComputable}`));
                }
            }
            xhr.onloadend = () => {
                postMessage({ events, readyState: xhr.readyState, status: xhr.status });
                close();
            };
            xhr.open("POST", URL);
            xhr.send("payload");
            events.push("returned:" + xhr.readyState);
            queueMicrotask(() => {
                events.push("microtask");
                xhr.onerror = () => events.push("late-error-listener");
            });
        "#
        .replace("URL", &serde_json::to_string(url).unwrap());
        let (result, records) =
            collect_rejected_xhr_output(rejected_xhr_worker(case, script)).await;
        assert_eq!(
            result,
            serde_json::json!({
                "events": [
                    "state:1", "xhr:loadstart:0:0:false", "upload:loadstart:0:7:true",
                    "returned:1", "microtask", "state:4", "upload:error:0:0:false",
                    "upload:loadend:0:0:false", "xhr:error:0:0:false", "late-error-listener",
                    "xhr:loadend:0:0:false"
                ],
                "readyState": 4, "status": 0
            }),
            "{case}"
        );
        assert_eq!(records.len(), 1, "{case}: exactly one failure record");
        assert_eq!(records[0].url().as_str(), url);
        assert_eq!(records[0].resource_type(), SubresourceResourceType::Xhr);
        assert!(
            matches!(
                records[0].outcome(),
                SubresourceNetworkOutcome::Failure { error_text: actual } if actual.contains(error_text)
            ),
            "{case}: {:?}",
            records[0].outcome()
        );
    }
}

#[tokio::test]
async fn worker_xhr_early_failure_can_abort_or_reopen_before_delivery() {
    ensure_v8();
    for (case, url, _) in REJECTIONS {
        for action in ["abort", "microtask-abort", "reopen"] {
            let script = r#"
                const events = [];
                const xhr = new XMLHttpRequest();
                for (const target of [xhr, xhr.upload]) {
                    for (const type of ["error", "abort", "load", "loadend"])
                        target.addEventListener(type, () => events.push(
                            `${target === xhr ? 'xhr' : 'upload'}:${type}`));
                }
                xhr.open("POST", URL);
                xhr.send("payload");
                const returnedState = xhr.readyState;
                function cancel() {
                    if (ACTION === "reopen") xhr.open("GET", "data:text/plain,reopened");
                    else xhr.abort();
                }
                if (ACTION === "microtask-abort") queueMicrotask(cancel);
                else cancel();
                // A second XHR completes through the same queue, after the
                // canceled failure, so the observation cannot race a timer.
                const barrier = new XMLHttpRequest();
                barrier.onloadend = () => {
                    postMessage({ returnedState, readyState: xhr.readyState, status: xhr.status, events });
                    close();
                };
                barrier.open("GET", URL);
                barrier.send();
            "#
            .replace("URL", &serde_json::to_string(url).unwrap())
            .replace("ACTION", &serde_json::to_string(action).unwrap());
            let (result, records) =
                collect_rejected_xhr_output(rejected_xhr_worker(case, script)).await;
            let events = if action == "reopen" {
                vec![]
            } else {
                vec!["upload:abort", "upload:loadend", "xhr:abort", "xhr:loadend"]
            };
            assert_eq!(
                result,
                serde_json::json!({
                    "returnedState": 1, "readyState": if action == "reopen" { 1 } else { 0 },
                    "status": 0, "events": events
                }),
                "{case}/{action}"
            );
            assert_eq!(
                records.len(),
                1,
                "{case}/{action}: only the barrier may report failure"
            );
        }
    }
}

#[tokio::test]
async fn worker_xhr_early_failure_does_not_overwrite_a_new_send() {
    ensure_v8();
    let script = r#"
        const xhr = new XMLHttpRequest();
        const events = [];
        for (const type of ["error", "abort", "load", "loadend"])
            xhr.addEventListener(type, () => events.push(type));
        xhr.onloadend = () => {
            postMessage({ events, status: xhr.status, text: xhr.responseText });
            close();
        };
        xhr.open("GET", "invalid-protocol://example.test/old");
        xhr.send();
        xhr.open("GET", "data:text/plain,new-request");
        xhr.send();
    "#;
    let (result, records) =
        collect_rejected_xhr_output(rejected_xhr_worker("scheme", script.to_owned())).await;
    assert_eq!(
        result,
        serde_json::json!({
            "events": ["load", "loadend"], "status": 200, "text": "new-request"
        })
    );
    assert!(
        records.is_empty(),
        "superseded request must not report a late failure"
    );
}

#[tokio::test]
async fn worker_xhr_early_failure_still_throws_for_synchronous_requests() {
    ensure_v8();
    for (case, url, _) in REJECTIONS {
        let script = r#"
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.onreadystatechange = () => events.push("state:" + xhr.readyState);
            for (const target of [xhr, xhr.upload])
                for (const type of ["loadstart", "progress", "error", "abort", "load", "loadend"])
                    target.addEventListener(type, () => events.push(type));
            xhr.open("POST", URL, false);
            let exception = null;
            try { xhr.send("payload"); }
            catch (error) { exception = { name: error.name, domException: error instanceof DOMException }; }
            postMessage({ exception, events, readyState: xhr.readyState, status: xhr.status });
            close();
        "#
        .replace("URL", &serde_json::to_string(url).unwrap());
        let (result, records) =
            collect_rejected_xhr_output(rejected_xhr_worker(case, script)).await;
        assert_eq!(
            result,
            serde_json::json!({
                "exception": { "name": "NetworkError", "domException": true },
                "events": ["state:1"], "readyState": 4, "status": 0
            }),
            "{case}"
        );
        assert_eq!(records.len(), 1, "{case}: exactly one synchronous failure");
    }
}
