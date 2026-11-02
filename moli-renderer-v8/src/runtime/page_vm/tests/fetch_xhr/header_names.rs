use super::*;

#[tokio::test]
async fn window_fetch_header_name_inputs_preserve_native_wire_case() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for _ in 0..12 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_http_request_head(&mut socket).await.unwrap();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    request.len(), request
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let mut page_vm = test_page_vm_with_document_url(
            Url::parse(&format!("{base}/page.html")).unwrap(),
        );
        let source = format!(
            "{}\nfetchHeaderNameInputs('{base}').then(value => {{ globalThis.__headerNames = value; }}, error => {{ globalThis.__headerNames = String(error); }}).finally(() => {{ globalThis.__headerNamesDone = true; }});",
            include_str!("../../../../../tests/fixtures/fetch-header-name-inputs.js"),
        );
        let local_executor = page_vm.local_executor.clone();
        let observed = local_executor.run(async move {
            page_vm.vm_mut().eval(&source)?;
            drive_websocket_until_done(
                &mut page_vm,
                "String(globalThis.__headerNamesDone === true)",
                "header-name requests should finish",
            ).await?;
            page_vm.vm_mut().eval("globalThis.__headerNames")
        }).await.unwrap();
        assert_eq!(observed, "pass");
        tokio::time::timeout(Duration::from_secs(10), server).await.unwrap().unwrap();
    }).await;
}
