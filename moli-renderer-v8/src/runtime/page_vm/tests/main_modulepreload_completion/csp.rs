use super::*;

async fn held_module_response(
    json_module: bool,
) -> (
    String,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
    JoinHandle<anyhow::Result<()>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let head = read_http_request_head(&mut stream).await?;
        assert!(head.starts_with("GET /module.js "), "{head}");
        seen_tx.send(()).unwrap();
        release_rx.await?;
        let (body, mime) = if json_module {
            (r#"{"answer":42}"#, "application/json")
        } else {
            ("export default 42;", "text/javascript")
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        );
        stream.write_all(response.as_bytes()).await?;
        anyhow::Ok(())
    });
    (base, seen_rx, release_tx, server)
}

#[tokio::test(flavor = "current_thread")]
async fn modulepreload_csp_reports_dispatch_while_response_headers_are_held() {
    run_page_vm_async_test(async move {
        for json_module in [false, true] {
        for child in [false, true] {
            for count in [1, 2] {
                let (base, seen, release, server) = held_module_response(json_module).await;
                let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
                let (mut page_vm, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                    &loader,
                    Url::parse(&format!("{base}/page.html"))?,
                );
                let policies = if json_module {
                    page_vm.vm_mut().set_response_content_security_policies(&["script-src 'none'".to_owned()]);
                    ["connect-src 'none'", "default-src 'none'"]
                } else {
                    ["script-src 'none'", "default-src 'none'"]
                };
                page_vm.vm_mut().set_response_content_security_report_only_policies(
                    &policies[..count].iter().map(|p| p.to_string()).collect::<Vec<_>>(),
                );
                if child {
                    page_vm.vm_mut().eval(
                        "const frame=document.createElement('iframe');frame.id='preload';document.body.appendChild(frame);",
                    )?;
                    materialize_child_realm_through_page_turn_for_test(&mut page_vm, "preload")?;
                }
                page_vm.vm_mut().eval(&format!(r#"
globalThis.cspPreloadMessages=[];
globalThis.cspPreloadReturned=false;
globalThis.cspPreloadLoaded=false;
const doc={};
const view=doc.defaultView;
doc.addEventListener('securitypolicyviolation', event=>{{
  if(!event.blockedURI.endsWith('/module.js'))return;
  cspPreloadMessages.push([event.originalPolicy,event instanceof view.SecurityPolicyViolationEvent,event.isTrusted,cspPreloadReturned]);
  queueMicrotask(()=>cspPreloadMessages.push('microtask'));
}});
const link=doc.createElement('link');link.rel='modulepreload';link.href={:?};
if({})link.as='json';
link.onload=()=>cspPreloadLoaded=true;
doc.head.appendChild(link);cspPreloadReturned=true;
"#, if child { "document.getElementById('preload').contentDocument" } else { "document" },format!("{base}/module.js"),json_module))?;
                if child {
                    assert!(page_vm.run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ModulepreloadStart, &loader,
                    ).await?);
                }
                tokio::time::timeout(Duration::from_secs(5), seen).await??;
                let mut expected = Vec::new();
                for policy in &policies[..count] {
                    assert!(page_vm.run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ResourceCompletion, &loader,
                    ).await?, "each violation must own a networking task before the response exists");
                    expected.push(json!([policy,true,true,true]));
                    expected.push(json!("microtask"));
                    let messages = page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                        "JSON.stringify(cspPreloadMessages)",
                    )?;
                    assert_eq!(serde_json::from_str::<serde_json::Value>(&messages)?, json!(expected));
                    assert_eq!(page_vm.vm_mut().eval_without_microtask_checkpoint_for_test("String(cspPreloadLoaded)")?, "false");
                }
                let local_executor = page_vm.local_executor.clone();
                local_executor.run(async move {
                    release.send(()).unwrap();
                    drive_websocket_until_done(&mut page_vm, "String(cspPreloadLoaded)", "modulepreload response release").await?;
                    assert_eq!(serde_json::from_str::<serde_json::Value>(&page_vm.vm_mut().eval("JSON.stringify(cspPreloadMessages)")?)?,json!(expected));
                    server.await??;
                    anyhow::Ok(())
                }).await?;
            }
        }
        }
        anyhow::Ok(())
    }).await.expect("modulepreload CSP events must not wait for response headers");
}

#[tokio::test(flavor = "current_thread")]
async fn modulepreload_csp_tasks_precede_enforced_module_map_failure() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let document_url = Url::parse(&format!("{base}/page.html"))?;
        let request_url = Url::parse(&format!("{base}/blocked.js"))?;
        let (mut page_vm, _, _) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url.clone());
        let policies = vec![
            "script-src 'none'".to_owned(),
            "default-src 'none'".to_owned(),
        ];
        page_vm
            .vm_mut()
            .set_response_content_security_policies(&policies);
        page_vm
            .vm_mut()
            .set_response_content_security_report_only_policies(&policies);
        page_vm.vm_mut().eval(
            r#"
globalThis.cspPreloadOrder=[];
addEventListener('securitypolicyviolation', event=>{
  cspPreloadOrder.push(event.disposition+':'+event.originalPolicy);
  queueMicrotask(()=>cspPreloadOrder.push('microtask'));
});
"#,
        )?;
        let (key, request) = modulepreload_request(&document_url, &request_url);
        assert_eq!(
            page_vm
                .vm_mut()
                .register_native_modulepreload_for_owner(request)
                .map_err(anyhow::Error::msg)?,
            Some(ModulePreloadJobRun::Scheduled)
        );
        let entry = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&key)
            .expect("reserved module map entry");
        let mut expected = Vec::new();
        for (disposition, policy) in [
            ("report", &policies[0]),
            ("report", &policies[1]),
            ("enforce", &policies[0]),
            ("enforce", &policies[1]),
        ] {
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ResourceCompletion,
                        &loader
                    )
                    .await?
            );
            expected.push(json!(format!("{disposition}:{policy}")));
            expected.push(json!("microtask"));
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(
                    &page_vm
                        .vm_mut()
                        .eval_without_microtask_checkpoint_for_test(
                            "JSON.stringify(cspPreloadOrder)"
                        )?
                )?,
                json!(expected)
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .native_module_entry_state(entry),
                ModuleMapEntryState::Fetching
            );
        }
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ResourceCompletion,
                    &loader
                )
                .await?
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(entry),
            ModuleMapEntryState::Failed
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(30), listener.accept())
                .await
                .is_err(),
            "enforced CSP must prevent network contact"
        );
        anyhow::Ok(())
    })
    .await
    .expect("modulepreload CSP and terminal must share FIFO with per-task checkpoints");
}
