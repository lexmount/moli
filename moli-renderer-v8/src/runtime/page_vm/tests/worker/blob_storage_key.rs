use super::*;

#[tokio::test]
async fn blob_worker_scripts_use_creator_storage_keys_in_window_and_nested_workers() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_http_request_head(&mut stream).await.unwrap();
                let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                let body = match path {
                    "/outer" => format!(r#"<iframe src="http://blob-worker.test:{port}/inner"></iframe>"#),
                    "/inner" => "<script>parent.parent.postMessage('blob-worker-child-ready', '*')</script>".to_owned(),
                    _ => panic!("unexpected blob worker fixture request: {path}"),
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let mut config = FetchConfig::default();
        config.set_http_host_resolve(vec![
            format!("blob-worker.test:{port}:127.0.0.1"),
            format!("cross-site.test:{port}:127.0.0.1"),
        ]);
        config.set_http_no_proxy(Some("*".to_owned()));
        let loader = crate::network::ResourceRequestClient::new(&config).unwrap();
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse(&format!("http://blob-worker.test:{port}/")).unwrap(),
        );
        let local_executor = page_vm.local_executor.clone();
        let result = local_executor.run(async move {
            let fixture = include_str!("../../../../../tests/fixtures/blob-worker-storage-key.js");
            page_vm.vm_mut().eval(&format!(
                "globalThis.blobWorkerStorageResult = null; {fixture}.then(value => {{blobWorkerStorageResult = value;}}, error => {{blobWorkerStorageResult = String(error.stack || error);}});"
            ))?;
            drive_service_worker_and_shared_worker_page_vm_until_done(
                &mut page_vm,
                "String(blobWorkerStorageResult !== null)",
                "Blob worker script storage-key probes should finish",
            ).await?;
            page_vm.vm_mut().eval("JSON.stringify(blobWorkerStorageResult)")
        }).await.unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        let rows = result.as_array().unwrap_or_else(|| panic!("worker probe should return rows: {result}"));
        assert_eq!(rows.len(), 12);
        let failures: Vec<_> = rows.iter().filter(|row| row["observed"] != row["expected"]).collect();
        assert!(failures.is_empty(), "{failures:?}");
        server.await.unwrap();
    }).await;
}
