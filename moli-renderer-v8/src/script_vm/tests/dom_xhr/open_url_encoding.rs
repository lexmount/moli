use super::*;

#[tokio::test(flavor = "current_thread")]
async fn xhr_query_encoding_reaches_the_wire_without_changing_paths_or_fragments() {
    for (encoding, query) in [
        ("windows-1252", "%26%2323478%3B%DF"),
        ("GBK", "%BC%D2%26%23223%3B"),
        ("Shift_JIS", "%89%C6%26%23223%3B"),
        ("UTF-8", "%E5%AE%B6%C3%9F"),
        ("UTF-16LE", "%E5%AE%B6%C3%9F"),
    ] {
        let server = StaticHttpServer::spawn(3).await;
        let base_url = server.base_url();
        let loader = static_http_loader(std::iter::empty::<String>());
        let mut vm = new_page_task_executor_test_vm_with_loader(base_url.as_str(), &loader);
        vm.document_runtime.set_document_character_set(encoding);
        vm.eval(
            r#"
          globalThis.queryEncodingResult = 'pending';
          (async () => {
            for (const input of ['/家?q=家ß#fragment', '?q=家\t\r\nß ', '/escaped?q=%DF']) {
              const xhr = new XMLHttpRequest();
              xhr.open('GET', input);
              await new Promise((resolve, reject) => {
                xhr.onload = resolve;
                xhr.onerror = () => reject(new Error('XHR failed'));
                xhr.send();
              });
            }
          })().then(() => queryEncodingResult = 'ok', error => queryEncodingResult = String(error));
        "#,
        )
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "queryEncodingResult",
            "ok",
            "XHR query encoding wire probe",
        )
        .await;
        let requests = server.finish().await;
        assert_eq!(requests.len(), 3, "{encoding}");
        assert_eq!(
            requests[0].target,
            format!("/%E5%AE%B6?q={query}"),
            "{encoding}"
        );
        assert_eq!(requests[1].target, format!("/?q={query}"), "{encoding}");
        assert_eq!(requests[2].target, "/escaped?q=%DF", "{encoding}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn borrowed_xhr_open_uses_the_owners_live_document_encoding_and_base() {
    let server = StaticHttpServer::spawn(4).await;
    let base_url = server.base_url();
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(base_url.as_str(), &loader);
    vm.document_runtime
        .set_document_character_set("windows-1252");
    vm.exec(
        r#"
      document.head.appendChild(document.createElement('base')).href = '/parent/';
      const frame = document.body.appendChild(document.createElement('iframe'));
      frame.id = 'query-frame';
      frame.srcdoc = '<!doctype html><head><base href="/child/"></head><body>';
    "#,
        None,
    )
    .unwrap();
    drain_pending_page_child_frame_work_for_test(&mut vm).await;
    let frame = vm
        .document_runtime
        .get_element_by_id("query-frame")
        .unwrap();
    let child_document = vm
        ._context_host
        .borrow()
        .child_browsing_context_document_handle(frame)
        .unwrap();
    vm.document_runtime
        .dom_host_mut()
        .set_document_character_set_for_handle(child_document, "GBK");
    vm.eval(r#"
      globalThis.queryEncodingResult = 'pending';
      (async () => {
        const child = document.getElementById('query-frame').contentWindow;
        for (const owner of [window, child]) {
          const xhr = new owner.XMLHttpRequest();
          for (const method of [XMLHttpRequest.prototype.open, child.XMLHttpRequest.prototype.open]) {
            method.call(xhr, 'GET', {toString() {
              owner.document.querySelector('base').href += 'converted/';
              return 'resource?q=家ß';
            }});
            // Base URL changes after open must not change the accepted URL.
            owner.document.querySelector('base').href = '/ignored/';
            await new Promise((resolve, reject) => {
              xhr.onload = resolve;
              xhr.onerror = () => reject(new Error('XHR failed'));
              xhr.send();
            });
          }
        }
      })().then(() => queryEncodingResult = 'ok', error => queryEncodingResult = String(error));
    "#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "queryEncodingResult",
        "ok",
        "XHR owner query encoding probe",
    )
    .await;
    let requests = server.finish().await;
    let expected = [
        "/parent/converted/resource?q=%26%2323478%3B%DF",
        "/ignored/converted/resource?q=%26%2323478%3B%DF",
        "/child/converted/resource?q=%BC%D2%26%23223%3B",
        "/ignored/converted/resource?q=%BC%D2%26%23223%3B",
    ];
    assert_eq!(requests.len(), expected.len());
    for (request, expected) in requests.iter().zip(expected) {
        assert_eq!(request.target, expected);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn worker_xhr_query_encoding_stays_utf8_in_a_legacy_encoded_creator() {
    let server = StaticHttpServer::spawn(1).await;
    let base_url = server.base_url();
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(base_url.as_str(), &loader);
    vm.document_runtime
        .set_document_character_set("windows-1252");
    let source = format!(
        r#"
      const xhr = new XMLHttpRequest();
      xhr.open('GET', {});
      xhr.onload = () => postMessage('ok');
      xhr.onerror = () => postMessage('error');
      xhr.send();
    "#,
        serde_json::to_string(&format!("{}worker?q=ß", base_url)).unwrap()
    );
    vm.eval(&format!(
        r#"
      globalThis.queryEncodingResult = 'pending';
      const url = URL.createObjectURL(new Blob([{}], {{type:'text/javascript'}}));
      const worker = new Worker(url);
      worker.onmessage = event => {{
        queryEncodingResult = event.data;
        worker.terminate();
        URL.revokeObjectURL(url);
      }};
      worker.onerror = event => {{ queryEncodingResult = event.message; event.preventDefault(); }};
    "#,
        serde_json::to_string(&source).unwrap()
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "queryEncodingResult",
        "ok",
        "worker XHR query encoding probe",
    )
    .await;
    let requests = server.finish().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].target, "/worker?q=%C3%9F");
}
