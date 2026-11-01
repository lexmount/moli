use super::*;

#[tokio::test(flavor = "current_thread")]
async fn worker_startup_enforces_response_coep_and_queues_creator_reports() {
    for kind in ["classic", "module"] {
        for (enforced, report_only, response, outcome, dispositions) in [
            ("", "", "require-corp", "started", vec![]),
            ("require-corp", "", "credentialless", "started", vec![]),
            ("credentialless", "", "require-corp", "started", vec![]),
            ("require-corp", "", "", "error", vec!["enforce"]),
            ("credentialless", "", "", "error", vec!["enforce"]),
            ("", "require-corp", "", "started", vec!["reporting"]),
            ("", "credentialless", "", "started", vec!["reporting"]),
            (
                "require-corp",
                "require-corp",
                "",
                "error",
                vec!["reporting", "enforce"],
            ),
        ] {
            let source = "self.onmessage=()=>postMessage('executed');";
            let response_headers = if response.is_empty() {
                String::new()
            } else {
                format!("Cross-Origin-Embedder-Policy: {response}\r\n")
            };
            let server=StaticHttpServer::spawn_with_raw_responder(1,move|_,_|format!("HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\n{response_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{source}",source.len())).await;
            let loader = static_http_loader([server.resolve_entry("localhost")]);
            let owner_url = server.url_for_host("localhost", "/owner");
            let script_url = server.url_for_host("localhost", "/worker.js");
            let mut vm =
                new_storage_page_task_executor_test_vm_with_loader(owner_url.as_str(), &loader);
            let headers = [
                (
                    "Cross-Origin-Embedder-Policy".to_owned(),
                    enforced.as_bytes().to_vec(),
                ),
                (
                    "Cross-Origin-Embedder-Policy-Report-Only".to_owned(),
                    report_only.as_bytes().to_vec(),
                ),
            ];
            vm.set_main_navigation_policy_container(
                crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
                    &headers, &owner_url,
                ),
            );
            assert_eq!(vm.eval("crossOriginIsolated").unwrap(), "false");
            vm.exec(&format!(r#"
              globalThis.reports=[];globalThis.result='pending';
              globalThis.worker=new Worker({url},{{type:{kind}}});
              const observer=new ReportingObserver(list=>reports.push(...list.map(r=>r.toJSON())));observer.observe();
              worker.onmessage=event=>{{result=event.data==='executed'?'started':'unexpected';}};
              worker.onerror=event=>{{event.preventDefault();result=event.constructor===Event&&!('message'in event)?'error':'wrong error event';}};
              worker.postMessage('go');
            "#,url=serde_json::to_string(script_url.as_str()).unwrap(),kind=serde_json::to_string(kind).unwrap()),None).unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "result",
                outcome,
                "DedicatedWorker startup COEP",
            )
            .await;
            if !dispositions.is_empty() {
                advance_page_task_executor_until_eval_equals(
                    &mut vm,
                    &loader,
                    "String(reports.length)",
                    &dispositions.len().to_string(),
                    "DedicatedWorker startup reports",
                )
                .await;
            }
            let reports: Vec<serde_json::Value> =
                serde_json::from_str(&vm.eval("JSON.stringify(reports)").unwrap()).unwrap();
            assert_eq!(
                reports.len(),
                dispositions.len(),
                "{kind}/{enforced}/{report_only}/{response}"
            );
            for (report, disposition) in reports.iter().zip(dispositions) {
                assert_eq!(report["type"], "coep");
                assert_eq!(report["url"], owner_url.as_str());
                assert_eq!(
                    report["body"],
                    serde_json::json!({"type":"worker initialization","blockedURL":script_url.as_str(),"disposition":disposition})
                );
            }
            vm.eval("worker.terminate()").unwrap();
            assert_eq!(server.finish_targets().await, ["/worker.js"]);
        }
    }
}
