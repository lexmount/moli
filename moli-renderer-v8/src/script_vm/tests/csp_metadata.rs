use super::*;

#[test]
fn csp_metadata_uses_native_referrers_without_reading_document_overrides() {
    let mut vm = new_storage_page_task_executor_test_vm("https://native-csp-metadata.test/page");
    let mut policy = vm.document_runtime.document_policy_container().clone();
    policy.document_referrer = "https://native-referrer.test/previous".to_owned();
    vm.document_runtime
        .set_main_navigation_policy_container(policy);
    let document = vm.document_runtime.document_handle();
    vm.document_runtime
        .dom_host_mut()
        .set_document_source_response_status_for_handle(document, Some(201));
    vm.eval(r#"
globalThis.metadataFrame = document.body.appendChild(document.createElement('iframe'));
globalThis.metadataPopup = open();
globalThis.metadataRows = [];
const views = [window, metadataFrame.contentWindow, metadataPopup];
for (const [index, view] of views.entries()) {
  const doc = view.document;
  const nativeReferrer = Object.getOwnPropertyDescriptor(Document.prototype, 'referrer').get.call(doc);
  let authorReads = 0;
  Object.defineProperty(doc, 'referrer', {configurable:true, get() { authorReads++; throw Error('author referrer'); }});
  const meta = doc.createElement('meta');
  meta.httpEquiv = 'Content-Security-Policy'; meta.content = "connect-src 'none'";
  doc.head.appendChild(meta);
  doc.addEventListener('securitypolicyviolation', event => {
    metadataRows.push({index, matched:event.referrer === nativeReferrer, referrer:event.referrer,
      status:event.statusCode, trusted:event.isTrusted, target:event.target === doc, authorReads});
    delete doc.referrer;
  }, {once:true});
  view.fetch('data:text/plain,blocked').catch(error => {
    if (!(error instanceof (view === metadataPopup ? TypeError : view.TypeError))) throw error;
  });
}
"#).unwrap();
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval_after_selected_page_tasks(
            "JSON.stringify(metadataRows.sort((a,b) => a.index-b.index))",
        )
        .unwrap(),
    )
    .unwrap();
    let rows = result.as_array().unwrap();
    assert_eq!(rows.len(), 3, "{result}");
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["index"], index, "{result}");
        assert_eq!(row["matched"], true, "{result}");
        assert_eq!(row["trusted"], true, "{result}");
        assert_eq!(row["target"], true, "{result}");
        assert_eq!(row["authorReads"], 0, "{result}");
        assert_eq!(row["status"], if index == 0 { 201 } else { 0 }, "{result}");
    }
    assert_eq!(rows[0]["referrer"], "https://native-referrer.test/previous");
    vm.eval("metadataPopup.close(); metadataFrame.remove()")
        .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn queued_csp_event_and_http_report_share_creation_metadata_during_pending_navigation() {
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(parking_lot::Mutex::new(Vec::<(String, Vec<u8>)>::new()));
    let received = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let received = Arc::clone(&received);
            connections.spawn(async move {
                let mut bytes = Vec::new();
                let mut buffer = [0; 2048];
                let header_end = loop {
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 { return; }
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let head = String::from_utf8_lossy(&bytes[..header_end]);
                let path = head.split_whitespace().nth(1).unwrap().to_owned();
                let length = head.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                }).unwrap_or(0);
                while bytes.len() < header_end + length {
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 { return; }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                received.lock().push((path.clone(), bytes[header_end..header_end + length].to_vec()));
                if path == "/slow" {
                    std::future::pending::<()>().await;
                }
                let policy = if path == "/child" {
                    "Content-Security-Policy: connect-src 'none'; report-uri /report\r\n"
                } else { "" };
                let body = "<!doctype html><body>child";
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\n{policy}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = socket.write_all(response.as_bytes()).await;
            });
            while let Some(result) = connections.try_join_next() {
                result.unwrap();
            }
        }
    });
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader(&format!("{origin}/p1"), &loader);
    vm.eval("globalThis.frame = document.body.appendChild(document.createElement('iframe')); frame.src = '/child';").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(frame.contentDocument.URL.endsWith('/child'))",
        "true",
        "CSP child response",
    )
    .await;
    vm.eval(r#"
globalThis.originalDoc = frame.contentDocument;
globalThis.initialReferrer = originalDoc.referrer;
globalThis.metadataEvents = [];
history.pushState({}, '', '/p2');
originalDoc.addEventListener('securitypolicyviolation', event => {
  metadataEvents.push({referrer:event.referrer, status:event.statusCode,
    target:event.target === originalDoc, active:frame.contentDocument === originalDoc, trusted:event.isTrusted});
});
frame.contentWindow.fetch('/blocked').catch(error => globalThis.blockedRejection = error.name);
frame.src = '/slow';
globalThis.pendingReferrer = originalDoc.referrer;
"#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(metadataEvents.length)",
        "1",
        "queued CSP metadata event",
    )
    .await;
    let report = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(body) = requests
                .lock()
                .iter()
                .find_map(|(path, body)| (path == "/report").then(|| body.clone()))
            {
                break serde_json::from_slice::<serde_json::Value>(&body).unwrap();
            }
            vm.run_one_oldest_ready_page_task_executor_turn(&loader)
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("CSP violation must send an actual HTTP report");
    let observed: serde_json::Value = serde_json::from_str(&vm.eval("JSON.stringify({initialReferrer, pendingReferrer, event:metadataEvents[0], rejection:blockedRejection})").unwrap()).unwrap();
    assert_eq!(observed["initialReferrer"], format!("{origin}/p1"));
    assert_eq!(observed["event"]["target"], true);
    assert_eq!(observed["event"]["active"], true);
    assert_eq!(observed["event"]["trusted"], true);
    assert_eq!(observed["rejection"], "TypeError");
    assert_eq!(
        report["csp-report"]["referrer"],
        observed["initialReferrer"]
    );
    assert_eq!(
        observed["event"]["referrer"], report["csp-report"]["referrer"],
        "retained Document state: {observed}"
    );
    assert_eq!(
        observed["event"]["status"],
        report["csp-report"]["status-code"]
    );
    assert!(requests.lock().iter().all(|(path, _)| path != "/blocked"));
    server.abort();
}
