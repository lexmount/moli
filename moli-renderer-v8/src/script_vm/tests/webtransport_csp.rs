use super::*;

const CSP_PROBE: &str = include_str!("../../../tests/fixtures/webtransport-csp.js");
const TRANSPORT_URL: &str = "https://webtransport-endpoint.test:8443/session?private=1";

async fn settle_webtransport_csp_probe(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    browser: &crate::runtime::RendererBrowserContextRuntimeOwner,
    loader: &ResourceRequestClient,
) -> serde_json::Value {
    let result = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            browser.drain_shared_worker_service_lane();
            if vm
                .eval("globalThis.webTransportCspResult !== undefined")
                .unwrap()
                == "true"
            {
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
    assert!(result.is_ok(), "WebTransport CSP probe did not finish");
    serde_json::from_str(&vm.eval("JSON.stringify(webTransportCspResult)").unwrap()).unwrap()
}

async fn check_webtransport_csp(scope: &str) {
    for policy in ["allow", "header", "meta", "report-only", "multiple"] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, browser) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                "https://webtransport-policy.test/page",
                &loader,
            );
        let owner = vm.current_main_document_task_owner().unwrap();
        let interactive = vm.finish_current_main_document_parsing(owner).unwrap();
        vm.apply_main_document_interactive_lifecycle_action(interactive)
            .unwrap();
        vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
        match policy {
            "header" => vm.set_response_content_security_policies(&["connect-src 'none'".into()]),
            "meta" => {
                vm.eval("document.head.appendChild(Object.assign(document.createElement('meta'), {httpEquiv:'Content-Security-Policy',content:\"connect-src 'none'\"}));true").unwrap();
            }
            "report-only" => vm
                .set_response_content_security_report_only_policies(&["connect-src 'none'".into()]),
            "multiple" => {
                vm.set_response_content_security_policies(&[
                    "connect-src 'none'".into(),
                    "connect-src 'self'".into(),
                ]);
                vm.set_response_content_security_report_only_policies(&[
                    "connect-src 'none'".into()
                ]);
            }
            "allow" => {}
            _ => unreachable!(),
        }
        let dispositions = match policy {
            "header" | "meta" => vec!["enforce"],
            "report-only" => vec!["report"],
            "multiple" => vec!["enforce", "enforce", "report"],
            _ => vec![],
        };
        let config = serde_json::json!({"url":TRANSPORT_URL,"reportCount":dispositions.len()});
        let probe = format!(
            "{CSP_PROBE}\nrunWebTransportCspProbe({config}).then(value=>{{globalThis.webTransportCspResult=value;}});"
        );
        let script = match scope {
            "window" => probe,
            "dedicated" | "shared" => {
                let shared = scope == "shared";
                let source = if shared {
                    format!(
                        "{CSP_PROBE}\nonconnect=event=>{{const port=event.ports[0];port.start();runWebTransportCspProbe({config}).then(value=>{{port.postMessage(value);close();}});}};"
                    )
                } else {
                    format!(
                        "{CSP_PROBE}\nrunWebTransportCspProbe({config}).then(value=>{{postMessage(value);close();}});"
                    )
                };
                let constructor = if shared { "SharedWorker" } else { "Worker" };
                let port = if shared { "worker.port" } else { "worker" };
                let start = if shared { "port.start();" } else { "" };
                format!(
                    "const worker=new {constructor}(URL.createObjectURL(new Blob([{}],{{type:'application/javascript'}})));const port={port};port.onmessage=event=>{{globalThis.webTransportCspResult=event.data;}};worker.onerror=event=>{{globalThis.webTransportCspResult={{error:event.message}};}};{start}",
                    serde_json::to_string(&source).unwrap()
                )
            }
            _ => unreachable!(),
        };
        vm.eval(&script).unwrap();
        let result = settle_webtransport_csp_probe(&mut vm, &browser, &loader).await;
        assert_eq!(
            result["name"], "WebTransportError",
            "{scope}/{policy}: {result}"
        );
        assert_eq!(result["source"], "session");
        assert!(result["streamErrorCode"].is_null());
        assert_eq!(result["errorRealm"], true);
        assert_eq!(result["sameError"], true);
        assert_eq!(
            result["pendingAfterMicrotask"],
            !dispositions.contains(&"enforce")
        );
        let reports = result["reports"].as_array().unwrap();
        assert_eq!(
            reports.len(),
            dispositions.len(),
            "{scope}/{policy}: {result}"
        );
        for (report, disposition) in reports.iter().zip(dispositions) {
            assert_eq!(report["directive"], "connect-src");
            assert_eq!(report["disposition"], disposition);
            assert_eq!(report["blockedURI"], TRANSPORT_URL);
            assert_eq!(report["hasSourceLocation"], true);
            assert_eq!(report["trusted"], true);
            assert_eq!(report["duringConstructor"], false);
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn webtransport_csp_window_policy_reports_and_rejection_order() {
    check_webtransport_csp("window").await;
}

#[tokio::test(flavor = "current_thread")]
async fn webtransport_csp_dedicated_worker_policy_reports_and_rejection_order() {
    check_webtransport_csp("dedicated").await;
}

#[tokio::test(flavor = "current_thread")]
async fn webtransport_csp_shared_worker_policy_reports_and_rejection_order() {
    check_webtransport_csp("shared").await;
}

#[tokio::test(flavor = "current_thread")]
async fn webtransport_csp_uses_constructor_realm_and_validates_before_reporting() {
    for (child_blocked, validation, close_immediately) in [
        (true, "none", false),
        (false, "none", false),
        (true, "url", false),
        (true, "dictionary", false),
        (true, "none", true),
        (true, "new-target", false),
        (false, "new-target", false),
    ] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, browser) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                "https://webtransport-policy.test/page",
                &loader,
            );
        let owner = vm.current_main_document_task_owner().unwrap();
        let interactive = vm.finish_current_main_document_parsing(owner).unwrap();
        vm.apply_main_document_interactive_lifecycle_action(interactive)
            .unwrap();
        vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
        vm.eval(
            r#"
globalThis.frame = document.createElement('iframe');
document.body.appendChild(frame);
globalThis.other = frame.contentWindow;
globalThis.outerCspReports = [];
addEventListener('securitypolicyviolation', event => outerCspReports.push(event.effectiveDirective));
"#,
        )
        .unwrap();
        let document = if child_blocked {
            "other.document"
        } else {
            "document"
        };
        vm.eval(&format!(
            r#"
const meta = {document}.createElement('meta');
meta.httpEquiv = 'Content-Security-Policy';
meta.content = "connect-src 'none'";
{document}.head.appendChild(meta);
"#,
        ))
        .unwrap();
        let valid_constructor = matches!(validation, "none" | "new-target");
        let count = usize::from(child_blocked && valid_constructor);
        let config = serde_json::json!({
            "url": TRANSPORT_URL,
            "reportCount": count,
            "validation": validation,
            "closeImmediately": close_immediately,
        });
        vm.eval(&format!(
            r#"{CSP_PROBE}
runWebTransportCspProbe({config}, other).then(value => {{
  globalThis.webTransportCspResult = value;
}});
"#,
        ))
        .unwrap();
        let result = settle_webtransport_csp_probe(&mut vm, &browser, &loader).await;
        assert_eq!(vm.eval("JSON.stringify(outerCspReports)").unwrap(), "[]");
        let reports = result["reports"].as_array().unwrap();
        assert_eq!(reports.len(), count, "{result}");
        if valid_constructor {
            assert_eq!(result["name"], "WebTransportError");
            assert_eq!(result["errorRealm"], true);
            assert_eq!(result["sameError"], true);
            assert_eq!(result["pendingAfterMicrotask"], !child_blocked);
            for report in reports {
                assert_eq!(report["directive"], "connect-src");
                assert_eq!(report["disposition"], "enforce");
                assert_eq!(report["blockedURI"], TRANSPORT_URL);
                assert_eq!(report["hasSourceLocation"], true);
                assert_eq!(report["duringConstructor"], false);
            }
        } else {
            assert_eq!(
                result["thrown"],
                if validation == "url" {
                    "SyntaxError"
                } else {
                    "RangeError"
                }
            );
        }
    }
}
