use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn redirect_filter_completion_keeps_status_text_and_explicit_filters() {
    use crate::types::AsyncSubresourceFetchResponseFilter::{Opaque, OpaqueRedirect};
    for status in [200, 302] {
        for redirect in ["follow", "manual"] {
            for filter in [None, Some(Opaque), Some(OpaqueRedirect)] {
                let mut vm = new_storage_test_vm("https://redirect-filter.test/");
                vm.set_fetch_subresource_interception(
                    true,
                    Some(crate::types::SubresourceResourceType::Fetch),
                );
                vm.eval(&format!(
                    "globalThis.filteredResult = 'pending'; fetch('/response', {{redirect: '{redirect}'}}).then(async response => {{ filteredResult = JSON.stringify([response.type, response.status, response.statusText, response.headers.get('x-visible'), await response.text()]); }});"
                )).unwrap();
                let requests = vm.take_pending_subresource_fetch_infos();
                assert_eq!(requests.len(), 1);
                let request = &requests[0];
                vm.complete_async_subresource_fetch(
                    crate::types::AsyncSubresourceFetchCompletion {
                        internal_id: request.internal_id,
                        request_url: request.url.clone(),
                        request_method: request.method.clone(),
                        request_headers: Vec::new(),
                        request_body: None,
                        response_status_text: Some("Override Text".to_owned()),
                        skip_fetch_security_validation: true,
                        response_filter: filter,
                        network_error_text: None,
                        result: Ok(
                            crate::protocol_types::NavigationResponse::from_head_and_body(
                                moli_fetch::ResponseHead {
                                    final_url: request.url.clone(),
                                    status,
                                    status_text: Some("Original Text".to_owned()),
                                    headers: vec![("X-Visible".to_owned(), "present".to_owned())],
                                    request_cookie_report: None,
                                    cookie_set_reports: Vec::new(),
                                    redirected: false,
                                    redirect_chain: Vec::new(),
                                    from_cache: false,
                                    negotiated_http_version: None,
                                },
                                "body".to_owned(),
                                b"body".to_vec(),
                            ),
                        ),
                    },
                )
                .unwrap();
                let expected_type = match filter {
                    Some(Opaque) => "opaque",
                    Some(OpaqueRedirect) => "opaqueredirect",
                    None if redirect == "manual" && status == 302 => "opaqueredirect",
                    None => "basic",
                };
                let filtered = expected_type != "basic";
                let expected = serde_json::json!([
                    expected_type,
                    if filtered { 0 } else { status },
                    if filtered { "" } else { "Override Text" },
                    if filtered { None } else { Some("present") },
                    if filtered { "" } else { "body" },
                ]);
                assert_eq!(
                    vm.eval("filteredResult").unwrap(),
                    expected.to_string(),
                    "{status}/{redirect}/{filter:?}"
                );
            }
        }
    }
}

async fn check_redirect_filter_modes(worker: bool) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("http://{address}");
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut requests = 0;
        loop {
            let mut socket = tokio::select! {
                result = listener.accept() => result.unwrap().0,
                _ = &mut stop_rx => break,
            };
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                assert!(head.len() < 8192);
                let mut byte = [0];
                assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            let mut parts = head.lines().next().unwrap().split_whitespace();
            let method = parts.next().unwrap();
            let path = parts.next().unwrap();
            let status: u16 = path.trim_start_matches('/').parse().unwrap();
            let response = format!(
                "HTTP/1.1 {status} Matrix Response\r\nContent-Type: text/plain\r\nContent-Length: 13\r\nConnection: close\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: X-Visible\r\nX-Visible: present\r\n\r\n{}",
                if method == "HEAD" {
                    ""
                } else {
                    "redirect body"
                }
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            requests += 1;
        }
        requests
    });
    let mut config = moli_fetch::FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    let loader = ResourceRequestClient::new(&config).unwrap();
    let mut vm = new_page_task_executor_test_vm_with_loader(&format!("{base}/page"), &loader);
    let probe = format!(
        r#"(async () => {{
          const base = {base:?};
          let count = 0;
          for (const status of [200, 301, 302, 303, 307, 308]) {{
            for (const redirect of ['follow', 'manual', 'error']) {{
              for (const mode of ['cors', 'same-origin', 'no-cors']) {{
                for (const remote of [false, true]) {{
                  for (const method of ['GET', 'HEAD']) {{
                    for (const override of [false, true]) {{
                      const label = [status, redirect, mode, remote, method, override].join('/');
                      const assert = (value, field) => {{ if (!value) throw new Error(label + ': ' + field); }};
                      const url = new URL('/' + status, base);
                      if (remote) url.hostname = 'localhost';
                      const request = new Request(url.href, {{method, mode, redirect: override ? 'follow' : redirect}});
                      let response, error;
                      try {{ response = await fetch(request.clone(), override ? {{redirect}} : undefined); }}
                      catch (value) {{ error = value; }}
                      count++;
                      if ((remote && mode === 'same-origin') || (redirect === 'error' && status !== 200)) {{
                        assert(error instanceof TypeError && !response, 'TypeError rejection');
                        continue;
                      }}
                      assert(!error && response instanceof Response, 'response');
                      const manual = redirect === 'manual' && status !== 200;
                      const opaque = !manual && remote && mode === 'no-cors';
                      const filtered = manual || opaque;
                      assert(response.type === (manual ? 'opaqueredirect' : opaque ? 'opaque' : remote ? 'cors' : 'basic'), 'type=' + response.type);
                      assert(response.status === (filtered ? 0 : status), 'status');
                      assert(response.statusText === (filtered ? '' : 'Matrix Response'), 'statusText');
                      assert(response.ok === (!filtered && status === 200), 'ok');
                      assert(!response.redirected, 'redirected without Location');
                      assert(response.url === (opaque ? '' : url.href), 'url');
                      assert(response.headers.get('X-Visible') === (filtered ? null : 'present'), 'headers');
                      if (filtered) assert(response.headers.entries().next().done, 'filtered headers');
                      assert((response.body === null) === (filtered || method === 'HEAD'), 'body nullability');
                      const clone = response.clone();
                      assert(clone.url === response.url, 'clone url');
                      const expected = filtered || method === 'HEAD' ? '' : 'redirect body';
                      assert(await response.text() === expected, 'body');
                      assert(await clone.text() === expected, 'clone body');
                      assert(response.bodyUsed === (!filtered && method !== 'HEAD'), 'bodyUsed');
                    }}
                  }}
                }}
              }}
            }}
          }}
          return String(count);
        }})()"#
    );
    let script = if worker {
        let source = format!(
            "Promise.resolve().then(() => {probe}).then(value => {{ postMessage(value); close(); }}, error => {{ postMessage(String(error.stack || error)); close(); }});"
        );
        format!(
            "globalThis.redirectFilterResult = 'pending'; const worker = new Worker(URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}}))); worker.onmessage = event => {{ redirectFilterResult = event.data; }}; worker.onerror = event => {{ redirectFilterResult = event.message; event.preventDefault(); }};",
            serde_json::to_string(&source).unwrap()
        )
    } else {
        format!(
            "globalThis.redirectFilterResult = 'pending'; Promise.resolve().then(() => {probe}).then(value => {{ redirectFilterResult = value; }}, error => {{ redirectFilterResult = String(error.stack || error); }});"
        )
    };
    vm.eval(&script).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while vm.eval("redirectFilterResult === 'pending'").unwrap() == "true" {
            wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
                .await
                .unwrap();
        }
    })
    .await
    .expect("redirect response matrix should finish");
    stop_tx.send(()).unwrap();
    let requests = server.await.unwrap();
    assert_eq!(vm.eval("redirectFilterResult").unwrap(), "432");
    // Cross-origin same-origin-mode requests are rejected before transport.
    assert_eq!(requests, 360);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn redirect_filter_preserves_window_fetch_modes() {
    check_redirect_filter_modes(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn redirect_filter_preserves_worker_fetch_modes() {
    check_redirect_filter_modes(true).await;
}
