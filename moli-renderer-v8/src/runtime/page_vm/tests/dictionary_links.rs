use super::*;

async fn request_head(stream: &mut tokio::net::TcpStream) -> anyhow::Result<String> {
    let mut bytes = Vec::new();
    while !bytes.windows(4).any(|chunk| chunk == b"\r\n\r\n") {
        let mut buffer = [0; 4096];
        let count =
            tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer)).await??;
        anyhow::ensure!(count != 0 && bytes.len() < 65536, "incomplete HTTP request");
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(String::from_utf8(bytes)?)
}

fn request_header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(field, _)| field.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
}

#[tokio::test]
async fn intercepted_fetch_and_dictionary_recheck_csp_before_contacting_replacement_url() {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mut config = FetchConfig::default();
        config.set_request_timeout_ms(1000);
        let loader = crate::network::ResourceRequestClient::new(&config).unwrap();
        for dictionary in [false, true] {
            let mut page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), Url::parse(&format!("{base}/page")).unwrap());
            let executor = page_vm.local_executor.clone();
            let base = base.clone();
            executor.run(async move {
                let resource_type = if dictionary { SubresourceResourceType::Dictionary } else { SubresourceResourceType::Fetch };
                page_vm.vm_mut().set_fetch_subresource_interception(true, Some(resource_type));
                page_vm.vm_mut().eval(&format!(r#"
const policy = document.createElement('meta');
policy.httpEquiv = 'Content-Security-Policy';
policy.content = "connect-src {base}/allowed";
document.head.appendChild(policy);
"#))?;
                if dictionary {
                    page_vm.vm_mut().start_main_response_dictionary_loads(&[Url::parse(&format!("{base}/allowed"))?])?;
                } else {
                    page_vm.vm_mut().eval("fetch('/allowed').catch(() => {})")?;
                }
                let pending = page_vm.vm_mut().take_pending_subresource_fetch_infos();
                assert_eq!(pending.len(), 1, "dictionary request: {dictionary}");
                page_vm.continue_pending_subresource_fetch(pending[0].internal_id, Some(Url::parse(&format!("{base}/blocked"))?), None, None, None, false, false)?;
                drain_page_work_until_no_pending_subresources(&mut page_vm, "intercepted CSP replacement URL").await?;
                anyhow::Ok(())
            }).await.expect("CSP request admission regression");
            assert!(matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock), "{resource} contacted a CSP-blocked URL", resource=if dictionary { "dictionary" } else { "fetch" });
        }
    }).await;
}

#[tokio::test]
async fn removing_iframe_cancels_header_dictionary_without_registering_partial_bytes() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut document, _) = listener.accept().await?;
            let request = request_head(&mut document).await?;
            assert!(request.starts_with("GET /child "));
            document.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nLink: </dictionary>; rel=\"compression-dictionary\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
            document.shutdown().await?;

            let (mut dictionary, _) = listener.accept().await?;
            let request = request_head(&mut dictionary).await?;
            assert!(request.starts_with("GET /dictionary "));
            dictionary.write_all(b"HTTP/1.1 200 OK\r\nCache-Control: max-age=600\r\nUse-As-Dictionary: match=\"/*\"\r\nContent-Length: 27\r\nConnection: close\r\n\r\nThis is ").await?;
            let _ = started_tx.send(request);
            let _ = release_rx.await;
            // The retired Document may already have closed this connection.
            let _ = dictionary.write_all(b"a test dictionary.\n").await;
            let _ = dictionary.shutdown().await;

            let (mut probe, _) = listener.accept().await?;
            let request = request_head(&mut probe).await?;
            assert!(request.starts_with("GET /probe "));
            let body = serde_json::to_vec(&json!({"dictionary": request_header(&request, "available-dictionary")}))?;
            let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            probe.write_all(head.as_bytes()).await?;
            probe.write_all(&body).await?;
            probe.shutdown().await?;
            anyhow::Ok(())
        });
        let mut page_vm = test_page_vm_with_document_url(Url::parse(&format!("{base}/page")).unwrap());
        let executor = page_vm.local_executor.clone();
        executor.run(async move {
            page_vm.vm_mut().eval(r#"
document.cookie = 'retained=1; path=/';
globalThis.dictionaryFrame = document.createElement('iframe');
dictionaryFrame.id = 'dictionary-frame';
dictionaryFrame.onload = () => globalThis.dictionaryFrameReady = true;
dictionaryFrame.src = '/child';
document.body.appendChild(dictionaryFrame);
"#)?;
            drive_websocket_until_done(&mut page_vm, "String(globalThis.dictionaryFrameReady === true)", "header dictionary child navigation").await?;
            let request = tokio::time::timeout(Duration::from_secs(5), started_rx).await??;
            assert_eq!(request_header(&request, "sec-fetch-mode"), Some("cors"));
            assert_eq!(request_header(&request, "sec-fetch-dest"), Some("empty"));
            assert_eq!(request_header(&request, "cookie"), None);
            assert_eq!(request_header(&request, "referer"), None);
            let handle = page_vm.vm().element_handle_by_id_for_test("dictionary-frame").unwrap();
            let loader = page_vm.vm_mut().with_default_context_scope_for_test(|_, host_ptr| {
                let host = unsafe { &*host_ptr };
                Ok(host.document_resource_loader_for_dispatch_scope(crate::native_bridge::OwnerDispatchScope::Child(handle)).unwrap())
            })?;
            assert_eq!(loader.diagnostics().active_ordinary_load_count, 1);
            page_vm.vm_mut().eval("dictionaryFrame.remove()")?;
            assert_ne!(loader.state(), crate::network::context::DocumentResourceLoaderState::Active);
            assert_eq!(loader.diagnostics().active_ordinary_load_count, 0);
            let _ = release_tx.send(());
            page_vm.vm_mut().eval(r#"
fetch('/probe').then(response => response.json()).then(value => {
  globalThis.dictionaryProbe = value;
  globalThis.dictionaryProbeDone = true;
});
"#)?;
            drive_websocket_until_done(&mut page_vm, "String(globalThis.dictionaryProbeDone === true)", "dictionary registration after iframe removal").await?;
            let observed = page_vm.vm_mut().eval("JSON.stringify(globalThis.dictionaryProbe)")?;
            assert_eq!(serde_json::from_str::<serde_json::Value>(&observed)?, json!({"dictionary": null}));
            server.await??;
            anyhow::Ok(())
        }).await.expect("header dictionary removal regression");
    }).await;
}
