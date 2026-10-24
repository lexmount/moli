use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Copy, Debug)]
enum RequestScope {
    Window,
    DedicatedWorker,
    SharedWorker,
}

async fn settle_redirect_probe(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    browser: &crate::runtime::RendererBrowserContextRuntimeOwner,
    loader: &ResourceRequestClient,
    predicate: &str,
    context: &str,
) {
    let result = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            browser.drain_shared_worker_service_lane();
            if vm.eval(predicate).unwrap() == "true" {
                break;
            }
            if !vm
                .run_one_oldest_ready_page_task_executor_turn(loader)
                .await
                .unwrap()
            {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(100),
                    vm.wait_for_task_executor_work_arrival(),
                )
                .await;
            }
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "{context}: {}",
        vm.eval("JSON.stringify({outcome:redirectOutcome,violations:redirectViolations})")
            .unwrap()
    );
}

async fn check_csp_redirects(api: &str, scope: RequestScope) {
    let worker = !matches!(scope, RequestScope::Window);
    for status in [301, 302, 303, 307, 308] {
        for policy_source in ["header", "meta", "report-only"] {
            let report_only = policy_source == "report-only";
            let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let target_url = format!(
                "http://{}/target?private=target",
                target.local_addr().unwrap()
            );
            let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
            let target_server = tokio::spawn(async move {
                let mut socket = tokio::select! {
                    result = target.accept() => result.unwrap().0,
                    _ = stop_rx => return None,
                };
                let mut head = Vec::new();
                let mut byte = [0; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    assert!(head.len() < 8192);
                    assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
                    head.push(byte[0]);
                }
                let body = "data: payload\n\n";
                socket.write_all(format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                ).as_bytes()).await.unwrap();
                Some(String::from_utf8(head).unwrap())
            });
            let source = StaticHttpServer::spawn_with_raw_responder(2, move |index, request| {
                assert_eq!(request.method, "GET");
                let destination = if index == 0 {
                    assert_eq!(request.target, "/redirect?private=source");
                    format!("http://{}/hop", request.host.as_ref().unwrap())
                } else {
                    assert_eq!(request.target, "/hop");
                    target_url.clone()
                };
                format!("HTTP/1.1 {status} Redirect\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            }).await;
            let loader = static_http_loader(std::iter::empty::<String>());
            let (mut vm, browser) =
                new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                    source.base_url().join("/page").unwrap().as_str(),
                    &loader,
                );
            let owner = vm.current_main_document_task_owner().unwrap();
            let interactive = vm.finish_current_main_document_parsing(owner).unwrap();
            vm.apply_main_document_interactive_lifecycle_action(interactive)
                .unwrap();
            vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
            match policy_source {
                "header" => {
                    vm.set_response_content_security_policies(&["connect-src 'self'".to_owned()])
                }
                "meta" => {
                    vm.eval("document.head.appendChild(Object.assign(document.createElement('meta'), {httpEquiv:'Content-Security-Policy', content:\"connect-src 'self'\"}));true").unwrap();
                }
                "report-only" => vm.set_response_content_security_report_only_policies(&[
                    "connect-src 'self'".to_owned(),
                ]),
                _ => unreachable!(),
            }
            let initial_url = source
                .base_url()
                .join("/redirect?private=source")
                .unwrap()
                .to_string();
            let probe = format!(
                r#"
                globalThis.redirectOutcome = null;
                globalThis.redirectViolations = [];
                let publishing = false;
                const publish = () => {{
                    if ({worker} && !publishing && redirectOutcome !== null && redirectViolations.length > 0) {{
                        publishing = true;
                        setTimeout(() => {{
                            postMessage({{outcome:redirectOutcome, violations:redirectViolations}});
                            close();
                        }}, 0);
                    }}
                }};
                addEventListener('securitypolicyviolation', event => {{
                    redirectViolations.push({{
                        directive: event.effectiveDirective,
                        disposition: event.disposition,
                        blockedURI: event.blockedURI,
                        hasSourceLocation: event.lineNumber > 0 && event.columnNumber > 0,
                        trusted: event.isTrusted,
                    }});
                    publish();
                }});
                if ({api:?} === 'event-source') {{
                    const source = new EventSource({initial_url:?});
                    source.onmessage = event => {{
                        redirectOutcome = {{event:'message', state:source.readyState, data:event.data}};
                        source.close();
                    }};
                    source.onerror = () => {{
                        redirectOutcome = {{event:'error', state:source.readyState}};
                        source.close();
                    }};
                }} else {{
                    const xhr = new XMLHttpRequest();
                    const finish = event => {{
                        redirectOutcome = {{event, state:xhr.readyState, status:xhr.status, text:xhr.responseText}};
                        publish();
                    }};
                    xhr.onload = () => finish('load');
                    xhr.onerror = () => finish('error');
                    xhr.open('GET', {initial_url:?} + '#fragment', {api:?} !== 'sync-xhr');
                    try {{
                        xhr.send();
                        if ({api:?} === 'sync-xhr') finish('load');
                    }} catch (error) {{ finish(error.name); }}
                }}
                true
            "#
            );
            let script = match scope {
                RequestScope::Window => probe,
                RequestScope::DedicatedWorker | RequestScope::SharedWorker => {
                    let (worker_source, constructor, port, start) = match scope {
                        RequestScope::DedicatedWorker => (probe, "Worker", "worker", ""),
                        RequestScope::SharedWorker => (
                            format!(
                                "onconnect = event => {{ const port = event.ports[0]; port.start(); const postMessage = value => port.postMessage(value); {probe} }}"
                            ),
                            "SharedWorker",
                            "worker.port",
                            "port.start();",
                        ),
                        RequestScope::Window => unreachable!(),
                    };
                    format!(
                        "globalThis.redirectOutcome=null;globalThis.redirectViolations=[];const worker=new {constructor}(URL.createObjectURL(new Blob([{}],{{type:'application/javascript'}})));const port={port};port.onmessage=event=>{{redirectOutcome=event.data.outcome;redirectViolations=event.data.violations;}};worker.onerror=event=>{{redirectOutcome={{event:'worker-error',message:event.message}};}};{start}true",
                        serde_json::to_string(&worker_source).unwrap(),
                    )
                }
            };
            vm.eval(&script).unwrap();
            settle_redirect_probe(
                &mut vm,
                &browser,
                &loader,
                "String(redirectOutcome !== null && (redirectViolations.length > 0 || redirectOutcome.event === 'worker-error'))",
                &format!("scope={scope:?}, api={api}, status={status}, policy={policy_source}"),
            )
            .await;
            vm.eval("globalThis.redirectStable=false;setTimeout(()=>redirectStable=true,0);true")
                .unwrap();
            settle_redirect_probe(
                &mut vm,
                &browser,
                &loader,
                "String(redirectStable)",
                "redirect reports must not be duplicated by response delivery",
            )
            .await;
            let observed: serde_json::Value = serde_json::from_str(
                &vm.eval(
                    "JSON.stringify({outcome:redirectOutcome, violations:redirectViolations})",
                )
                .unwrap(),
            )
            .unwrap();
            let expected_outcome = match (api, report_only) {
                ("event-source", false) => serde_json::json!({"event":"error", "state":2}),
                ("event-source", true) => {
                    serde_json::json!({"event":"message", "state":1, "data":"payload"})
                }
                (_, true) => {
                    serde_json::json!({"event":"load", "state":4, "status":200, "text":"data: payload\n\n"})
                }
                ("sync-xhr", false) => {
                    serde_json::json!({"event":"NetworkError", "state":4, "status":0, "text":""})
                }
                _ => serde_json::json!({"event":"error", "state":4, "status":0, "text":""}),
            };
            assert_eq!(
                observed,
                serde_json::json!({
                    "outcome":expected_outcome,
                    "violations":[{
                        "directive":"connect-src",
                        "disposition":if report_only { "report" } else { "enforce" },
                        "blockedURI":initial_url,
                        "hasSourceLocation":api == "sync-xhr",
                        "trusted":true,
                    }],
                }),
                "scope={scope:?}, api={api}, status={status}, policy={policy_source}"
            );
            let _ = stop_tx.send(());
            let target_request = target_server.await.unwrap();
            assert_eq!(
                target_request.is_some(),
                report_only,
                "enforced CSP must reject before the target receives any request: scope={scope:?}, api={api}, status={status}, policy={policy_source}"
            );
            if let Some(request) = target_request {
                assert!(request.starts_with("GET /target?private=target HTTP/1.1\r\n"));
            }
            assert_eq!(source.finish().await.len(), 2);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn window_xhr_csp_redirects_block_before_wire_and_report_initial_url() {
    check_csp_redirects("xhr", RequestScope::Window).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn window_sync_xhr_csp_redirects_block_before_wire_and_report_initial_url() {
    check_csp_redirects("sync-xhr", RequestScope::Window).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn window_event_source_csp_redirects_close_before_wire_and_report_initial_url() {
    check_csp_redirects("event-source", RequestScope::Window).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_worker_xhr_csp_redirects_block_before_wire_and_report_initial_url() {
    check_csp_redirects("xhr", RequestScope::DedicatedWorker).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_worker_sync_xhr_csp_redirects_block_before_wire_and_report_initial_url() {
    check_csp_redirects("sync-xhr", RequestScope::DedicatedWorker).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_xhr_csp_redirects_block_before_wire_and_report_initial_url() {
    check_csp_redirects("xhr", RequestScope::SharedWorker).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_sync_xhr_csp_redirects_block_before_wire_and_report_initial_url() {
    check_csp_redirects("sync-xhr", RequestScope::SharedWorker).await;
}
