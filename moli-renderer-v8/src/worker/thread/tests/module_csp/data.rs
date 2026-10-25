use super::*;

const WORKER_URL: &str = "https://example.test/worker/main.js";

fn data_url(kind: &str) -> &'static str {
    match kind {
        "json" => "data:application/json,%7B%22answer%22%3A42%7D",
        "text" => "data:text/plain,module-text",
        "javascript" => {
            "data:text/javascript,globalThis.runs%3D(globalThis.runs%7C%7C0)%2B1%3Bexport%20default%2042%3B"
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn worker_data_module_csp_checks_all_policies_before_decoding() {
    ensure_v8();
    for script_kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        for global in global_kinds() {
            for malformed in [false, true] {
                let requests = ["json", "text", "javascript"].map(|kind| {
                    json!([
                        if malformed {
                            "data:text/plain;base64,!!!"
                        } else {
                            data_url(kind)
                        },
                        kind
                    ])
                });
                let worker = spawn_test_worker_with_options(
                    WorkerSpawnOptions::new(probe(json!(requests)), WORKER_URL.into())
                        .with_script_kind(script_kind)
                        .with_global_kind(global.clone())
                        .with_content_security_policies(vec![
                            "default-src 'none'".into(),
                            "script-src 'none'; connect-src 'none'".into(),
                        ])
                        .with_content_security_report_only_policies(vec![
                            "default-src 'none'".into(),
                            "script-src 'none'; connect-src 'none'".into(),
                        ]),
                );
                let actual = result(worker).await;
                for (index, row) in actual.as_array().unwrap().iter().enumerate() {
                    assert_eq!(row["error"], "TypeError", "{actual}");
                    assert_eq!(row["native"], true);
                    let events = row["events"].as_array().unwrap();
                    assert_eq!(events.len(), 4, "{actual}");
                    for (event, disposition) in events
                        .iter()
                        .zip(["report", "report", "enforce", "enforce"])
                    {
                        assert_eq!(event["disposition"], disposition);
                        assert_eq!(
                            event["directive"],
                            if index == 2 {
                                "script-src-elem"
                            } else {
                                "connect-src"
                            }
                        );
                        assert_eq!(event["blockedURI"], "data");
                        assert_eq!(event["documentURI"], WORKER_URL);
                        assert_eq!(event["sourceFile"], WORKER_URL);
                        assert!(event["line"].as_i64().unwrap() > 0);
                        assert!(event["column"].as_i64().unwrap() > 0);
                        assert_eq!(event["native"], true);
                        assert_eq!(event["trusted"], true);
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn worker_data_module_csp_keeps_offline_local_imports_working() {
    ensure_v8();
    for script_kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        for global in global_kinds() {
            let worker = spawn_test_worker_with_options(
                WorkerSpawnOptions::new(
                    probe(json!([
                        [data_url("json"), "json"],
                        [data_url("text"), "text"],
                        [data_url("javascript"), "javascript"]
                    ])),
                    WORKER_URL.into(),
                )
                .with_script_kind(script_kind)
                .with_global_kind(global)
                .with_network_policy(WorkerNetworkPolicy {
                    network_offline: true,
                    ..Default::default()
                })
                .with_content_security_policies(vec!["default-src data:".into()]),
            );
            let actual = result(worker).await;
            for (row, value) in
                actual
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip([json!(42), json!("module-text"), json!(42)])
            {
                assert_eq!(row["value"], value, "{actual}");
                assert_eq!(row["events"], json!([]));
            }
        }
    }
}

#[tokio::test]
async fn worker_data_module_csp_uses_connect_for_json_and_text() {
    ensure_v8();
    for script_kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        for global in global_kinds() {
            let worker = spawn_test_worker_with_options(
                WorkerSpawnOptions::new(
                    probe(json!([
                        [data_url("json"), "json"],
                        [data_url("text"), "text"],
                        [data_url("javascript"), "javascript"]
                    ])),
                    WORKER_URL.into(),
                )
                .with_script_kind(script_kind)
                .with_global_kind(global)
                .with_content_security_policies(vec!["script-src 'none'; connect-src data:".into()])
                .with_content_security_report_only_policies(vec!["connect-src 'none'".into()]),
            );
            let actual = result(worker).await;
            assert_eq!(actual[0]["value"], 42, "{actual}");
            assert_eq!(actual[1]["value"], "module-text", "{actual}");
            assert_eq!(actual[2]["error"], "TypeError", "{actual}");
            for (index, row) in actual.as_array().unwrap().iter().enumerate() {
                assert_eq!(row["events"].as_array().unwrap().len(), 1);
                assert_eq!(
                    row["events"][0]["directive"],
                    if index == 2 {
                        "script-src-elem"
                    } else {
                        "connect-src"
                    }
                );
                assert_eq!(
                    row["events"][0]["disposition"],
                    if index == 2 { "enforce" } else { "report" }
                );
            }
        }
    }
}

#[tokio::test]
async fn worker_data_module_csp_reuses_concurrent_and_cached_import_results() {
    ensure_v8();
    for script_kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        for global in global_kinds() {
            for kind in ["json", "text", "javascript"] {
                for blocked in [false, true] {
                    let source = r#"
const events = [];
const modules = [];
addEventListener('securitypolicyviolation', event => events.push(event.disposition));
const load = () => (KIND === 'javascript' ? import(URL) : import(URL, {with: {type: KIND}})).then(
  module => { modules.push(module); return {value: KIND === 'json' ? module.default.answer : module.default}; },
  error => ({error: error.name, native: error instanceof TypeError, events: events.length}));
(async () => {
  const results = await Promise.all([load(), load(), load()]);
  results.push(await load());
  console.log(JSON.stringify({results, events, same: modules.every(module => module === modules[0]), runs: globalThis.runs || 0}));
})();
"#
                        .replace("KIND", &serde_json::to_string(kind).unwrap())
                        .replace("URL", &serde_json::to_string(data_url(kind)).unwrap());
                    let worker = spawn_test_worker_with_options(
                        WorkerSpawnOptions::new(source, WORKER_URL.into())
                            .with_script_kind(script_kind)
                            .with_global_kind(global.clone())
                            .with_content_security_policies(vec![
                                if blocked {
                                    "default-src 'none'"
                                } else {
                                    "default-src data:"
                                }
                                .into(),
                            ])
                            .with_content_security_report_only_policies(vec![
                                "default-src 'none'".into(),
                            ]),
                    );
                    let actual = result(worker).await;
                    assert_eq!(actual["same"], true, "{actual}");
                    assert_eq!(
                        actual["runs"],
                        usize::from(!blocked && kind == "javascript")
                    );
                    assert_eq!(
                        actual["events"],
                        if blocked {
                            json!(["report", "enforce"])
                        } else {
                            json!(["report"])
                        },
                        "{actual}"
                    );
                    let results = actual["results"].as_array().unwrap();
                    assert_eq!(results.len(), 4);
                    for row in results {
                        assert_eq!(
                            *row,
                            if blocked {
                                json!({"error": "TypeError", "native": true, "events": 2})
                            } else {
                                json!({"value": if kind == "text" { json!("module-text") } else { json!(42) }})
                            },
                            "{actual}"
                        );
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn worker_data_module_csp_reuses_dependency_results_across_graphs() {
    ensure_v8();
    for script_kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        for global in global_kinds() {
            for kind in ["json", "text", "javascript"] {
                for blocked in [false, true] {
                    let attributes = if kind == "javascript" {
                        String::new()
                    } else {
                        format!(" with {{type: '{kind}'}}")
                    };
                    let body = format!(
                        "import value from {}{attributes}; export default {};",
                        serde_json::to_string(data_url(kind)).unwrap(),
                        if kind == "json" {
                            "value.answer"
                        } else {
                            "value"
                        },
                    );
                    let server = ModuleServer::spawn(move |_| format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len(),
                    )).await;
                    let worker = spawn_test_worker_with_options(
                        WorkerSpawnOptions::new(
                            probe(json!([
                                [format!("{}/a.js", server.url), "javascript"],
                                [format!("{}/b.js", server.url), "javascript"],
                                [format!("{}/a.js", server.url), "javascript"]
                            ])),
                            format!("{}/worker.js", server.url),
                        )
                        .with_script_kind(script_kind)
                        .with_global_kind(global.clone())
                        .with_content_security_policies(vec![
                            if blocked {
                                "script-src 'self'; connect-src 'none'"
                            } else {
                                "script-src 'self' data:; connect-src data:"
                            }
                            .into(),
                        ])
                        .with_content_security_report_only_policies(if blocked {
                            vec![]
                        } else {
                            vec!["script-src 'self'; connect-src 'none'".into()]
                        }),
                    );
                    let actual = result(worker).await;
                    for (index, row) in actual.as_array().unwrap().iter().enumerate() {
                        if blocked {
                            assert_eq!(row["error"], "TypeError", "{actual}");
                            assert_eq!(row["native"], true);
                        } else {
                            assert_eq!(
                                row["value"],
                                if kind == "text" {
                                    json!("module-text")
                                } else {
                                    json!(42)
                                },
                                "{actual}"
                            );
                        }
                        assert_eq!(
                            row["events"].as_array().unwrap().len(),
                            usize::from(index == 0),
                            "{actual}"
                        );
                    }
                    let event = &actual[0]["events"][0];
                    assert_eq!(
                        event["directive"],
                        if kind == "javascript" {
                            "script-src-elem"
                        } else {
                            "connect-src"
                        }
                    );
                    assert_eq!(event["blockedURI"], "data");
                    assert_eq!(event["sourceFile"], "");
                    assert_eq!(event["line"], 0);
                    assert_eq!(event["column"], 0);
                    assert_eq!(server.requests.lock().len(), 2);
                }
            }
        }
    }
}

#[tokio::test]
async fn worker_data_module_static_import_obeys_creator_policy() {
    ensure_v8();
    for global in global_kinds() {
        for kind in ["json", "text", "javascript"] {
            for blocked in [false, true] {
                let attributes = if kind == "javascript" {
                    String::new()
                } else {
                    format!(" with {{type: '{kind}'}}")
                };
                let source = format!(
                    "import value from {}{attributes}; console.log(JSON.stringify({}));",
                    serde_json::to_string(data_url(kind)).unwrap(),
                    if kind == "json" {
                        "value.answer"
                    } else {
                        "value"
                    },
                );
                let mut worker = spawn_test_worker_with_options(
                    WorkerSpawnOptions::new(source, WORKER_URL.into())
                        .with_script_kind(WorkerScriptKind::Module)
                        .with_global_kind(global.clone())
                        .with_module_static_import_content_security_policies(vec![
                            if blocked {
                                "default-src 'none'"
                            } else {
                                "default-src data:"
                            }
                            .into(),
                        ])
                        .with_content_security_policies(vec![
                            if blocked {
                                "default-src data:"
                            } else {
                                "default-src 'none'"
                            }
                            .into(),
                        ]),
                );
                if blocked {
                    match timeout(TIMEOUT, worker.recv()).await.unwrap().unwrap() {
                        WorkerToParentMessage::Error { message, phase, .. } => {
                            assert!(message.contains("Content Security Policy"), "{message}");
                            assert_eq!(phase, WorkerErrorPhase::Bootstrap);
                        }
                        other => panic!("expected CSP bootstrap error, got {other:?}"),
                    }
                    worker.terminate_and_join();
                } else {
                    assert_eq!(
                        result(worker).await,
                        if kind == "text" {
                            json!("module-text")
                        } else {
                            json!(42)
                        }
                    );
                }
            }
        }
    }
}
