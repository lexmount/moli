use super::*;

#[tokio::test]
async fn window_stop_cancels_requests_in_the_receiver_and_descendants_asynchronously() {
    for (target, cancelled) in [(0, 4), (1, 2), (4, 1)] {
        for borrowed in [false, true] {
            let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
                .expect("resource loader");
            let mut vm = new_parsed_page_task_executor_test_vm(
                "https://window-stop-requests.test/",
                "<!doctype html><body>parent",
                &loader,
            );
            vm.eval(
                r#"
globalThis.__windowStopLog = [];
const child = document.body.appendChild(document.createElement('iframe')).contentWindow;
const grandchild = child.document.body.appendChild(child.document.createElement('iframe')).contentWindow;
const sibling = document.body.appendChild(document.createElement('iframe')).contentWindow;
const popup = window.open('');
const windows = [window, child, grandchild, sibling, popup];
const requests = windows.map((owner, index) => {
  for (const keepalive of [false, true]) {
    owner.fetch('/pending-fetch-' + index + '-' + keepalive, {keepalive}).then(
      () => __windowStopLog.push(['unexpected fetch', index]),
      error => __windowStopLog.push(['fetch', index, keepalive, error.name, error instanceof owner.TypeError]));
  }
  const xhr = new owner.XMLHttpRequest();
  xhr.open('GET', '/pending-xhr-' + index);
  const events = [];
  for (const type of ['readystatechange', 'abort', 'load', 'error', 'loadend']) {
    xhr.addEventListener(type, () => {
      events.push([type, xhr.readyState, xhr.status]);
      if (type === 'loadend') __windowStopLog.push(['xhr', index, events]);
    });
  }
  xhr.send();
  return xhr;
});
"#,
            )
            .expect("register top, child, descendant, sibling and popup requests");
            {
                let host = vm._context_host.borrow();
                assert_eq!(host.active_window_fetch_contexts_for_test().len(), 10);
                assert_eq!(
                    host.pending_window_xhr_execution_contexts_for_test().len(),
                    5
                );
            }
            let stop = if borrowed {
                format!("window.stop.call(windows[{target}])")
            } else {
                format!("windows[{target}].stop()")
            };
            assert_eq!(
                vm.eval(&format!("{stop}; JSON.stringify(__windowStopLog)"))
                    .expect("stop the receiver's document requests"),
                "[]",
                "terminal events must be queued: target={target}, borrowed={borrowed}"
            );
            {
                let host = vm._context_host.borrow();
                assert_eq!(
                    host.active_window_fetch_contexts_for_test().len(),
                    2 * (5 - cancelled),
                    "Fetch scope: target={target}, borrowed={borrowed}"
                );
                assert_eq!(
                    host.pending_window_xhr_execution_contexts_for_test().len(),
                    5 - cancelled,
                    "XHR scope: target={target}, borrowed={borrowed}"
                );
            }
            vm.eval("window.stop(); popup.stop();")
                .expect("cancel remaining requests before dispatching terminal tasks");
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(__windowStopLog.length)",
                "15",
                "asynchronous window.stop request termination",
            )
            .await;
            assert_eq!(
                vm.eval(
                    r#"
(() => {
  const expected = windows.flatMap((owner, index) => [
    ['fetch', index, false, 'TypeError', true],
    ['fetch', index, true, 'TypeError', true],
    ['xhr', index, [['readystatechange', 4, 0], ['abort', 4, 0], ['loadend', 4, 0]]]
  ]);
  const sorted = rows => rows.map(row => JSON.stringify(row)).sort();
  return JSON.stringify(sorted(__windowStopLog)) === JSON.stringify(sorted(expected));
})()
"#,
                )
                .expect("compare exact terminal events and relevant error realms"),
                "true",
                "target={target}, borrowed={borrowed}"
            );
            let host = vm._context_host.borrow();
            assert!(host.active_window_fetch_contexts_for_test().is_empty());
            assert!(
                host.pending_window_xhr_execution_contexts_for_test()
                    .is_empty()
            );
        }
    }
}

#[tokio::test]
async fn window_stop_queued_abort_does_not_change_a_reopened_xhr() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://window-stop-reopened-xhr.test/",
        "<!doctype html><body>parent",
        &loader,
    );
    assert_eq!(
        vm.eval(
            r#"
globalThis.__windowStopReuse = [];
fetch('/pending-fetch').catch(() => __windowStopReuse.push('fetch'));
const xhr = new XMLHttpRequest();
xhr.open('GET', '/pending-xhr');
xhr.onabort = () => __windowStopReuse.push('stale abort');
xhr.send();
stop();
xhr.open('GET', '/replacement-xhr');
JSON.stringify([__windowStopReuse, xhr.readyState, xhr.status]);
"#,
        )
        .unwrap(),
        "[[],1,0]"
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "JSON.stringify(__windowStopReuse)",
        r#"["fetch"]"#,
        "reopened XHR suppresses the queued stop abort",
    )
    .await;
    assert_eq!(vm.eval("String(xhr.readyState)").unwrap(), "1");
}

#[test]
fn window_stop_on_a_retired_window_preserves_replacement_requests() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://window-stop-retired-receiver.test/",
        "<!doctype html><body>parent",
        &loader,
    );
    vm.eval(
        r#"
const oldFrame = document.body.appendChild(document.createElement('iframe'));
const retired = oldFrame.contentWindow;
oldFrame.remove();
const current = document.body.appendChild(document.createElement('iframe')).contentWindow;
current.fetch('/pending-fetch');
const xhr = new current.XMLHttpRequest();
xhr.open('GET', '/pending-xhr'); xhr.send();
"#,
    )
    .unwrap();
    let (fetches, xhrs) = {
        let host = vm._context_host.borrow();
        (
            host.active_window_fetch_contexts_for_test(),
            host.pending_window_xhr_execution_contexts_for_test(),
        )
    };
    assert_eq!(fetches.len(), 1);
    assert_eq!(xhrs.len(), 1);
    vm.eval("window.stop.call(retired)").unwrap();
    let host = vm._context_host.borrow();
    assert_eq!(host.active_window_fetch_contexts_for_test(), fetches);
    assert_eq!(host.pending_window_xhr_execution_contexts_for_test(), xhrs);
}

#[tokio::test]
async fn window_stop_preserves_event_source_and_intercepted_websocket_requests() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://window-stop-independent-connections.test/",
        "<!doctype html><body>parent",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, None);
    vm.eval(
        r#"
globalThis.__windowStopFetch = 'pending';
fetch('/fetch').catch(error => { __windowStopFetch = error.name; });
const source = new EventSource('/event-stream');
const socket = new WebSocket('wss://window-stop-independent-connections.test/socket');
"#,
    )
    .unwrap();
    // EventSource starts its first connection in a later networking turn.
    // Observe all three paused requests before exercising cancellation.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while vm
            ._context_host
            .borrow()
            .pending_subresource_fetch_info_count()
            < 3
        {
            wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
                .await
                .expect("initial EventSource connection turn");
        }
    })
    .await
    .expect("Fetch, EventSource and WebSocket must all register before stop");
    let independent = vm
        ._context_host
        .borrow_mut()
        .take_pending_subresource_fetch_infos()
        .into_iter()
        .filter(|info| matches!(info.url.path(), "/event-stream" | "/socket"))
        .map(|info| info.internal_id)
        .collect::<Vec<_>>();
    assert_eq!(independent.len(), 2);
    vm.eval("stop()").unwrap();
    for internal_id in independent {
        assert!(
            vm._context_host
                .borrow()
                .async_subresource_fetch_event_target_is_current(
                    crate::types::AsyncSubresourceFetchEventTarget::Completion { internal_id }
                ),
            "independent connection {internal_id} must remain registered"
        );
    }
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__windowStopFetch",
        "TypeError",
        "stop cancels Fetch while independent connections remain paused",
    )
    .await;
    assert_eq!(
        vm.eval("[source.readyState, socket.readyState].join('|')")
            .unwrap(),
        "0|0"
    );
    vm.eval("source.close(); socket.close();").unwrap();
}

#[tokio::test]
async fn window_stop_clears_streaming_xhr_response_before_queued_abort() {
    for ready_state in [2, 3] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, mut completions) =
            new_storage_test_vm_with_loader_and_resource_completion_queue(
                "https://window-stop-streaming-xhr.test/",
                &loader,
            );
        vm.set_fetch_subresource_interception(
            true,
            Some(crate::types::SubresourceResourceType::Xhr),
        );
        vm.eval(&format!(
            r#"
globalThis.events = [];
globalThis.immediate = null;
globalThis.xhr = new XMLHttpRequest();
const snapshot = () => [xhr.readyState, xhr.status, xhr.statusText,
  xhr.responseText, xhr.responseURL, xhr.getAllResponseHeaders(),
  xhr.getResponseHeader('content-type')];
xhr.onreadystatechange = () => {{
  events.push(['readystatechange', xhr.readyState, xhr.status]);
  if (xhr.readyState === {ready_state}) {{
    if (xhr.status !== 200) throw new Error('missing successful streaming response');
    events.length = 0;
    stop();
    immediate = [snapshot(), events.slice()];
  }}
}};
for (const type of ['abort', 'load', 'error', 'loadend'])
  xhr.addEventListener(type, () => events.push([type, xhr.readyState, xhr.status]));
xhr.open('GET', '/stream'); xhr.send();
"#,
        ))
        .unwrap();
        let pending = vm.take_pending_subresource_fetch_infos();
        let [request] = pending.as_slice() else {
            panic!("expected one intercepted XHR request");
        };
        let body_source_id = crate::network_host::new_network_body_source_id();
        vm.start_streaming_async_subresource_fetch(
            crate::types::AsyncSubresourceStreamingStarted {
                internal_id: request.internal_id,
                request_url: request.url.clone(),
                request_method: "GET".to_owned(),
                request_headers: Default::default(),
                request_body: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                body_source_id,
                head: moli_fetch::ResponseHead {
                    status_text: None,
                    final_url: request.url.clone(),
                    status: 200,
                    headers: moli_fetch::headers_from_byte_strings(&[(
                        "Content-Type".to_owned(),
                        "text/plain".to_owned(),
                    )])
                    .unwrap(),
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    cache_state: Default::default(),
                    preload_state: Default::default(),
                    negotiated_http_version: None,
                },
                network_request_headers: None,
            },
        )
        .unwrap();
        if ready_state == 3 {
            vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"partial".to_vec());
        }
        assert_eq!(
            vm.eval("JSON.stringify(immediate)").unwrap(),
            format!(r#"[[{ready_state},0,"","","","",null],[]]"#),
            "response reset must precede asynchronous termination"
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while vm.eval("events.length === 3").unwrap() != "true" {
                assert!(completions.wait_for_arrival_without_timeout().await);
                while let Some(event) = completions.pop_next_async_subresource_event() {
                    let activity = vm
                        .complete_async_subresource_fetch_event_body(event)
                        .unwrap();
                    vm.finish_async_subresource_body_checkpoint_for_test(activity)
                        .unwrap();
                }
            }
        })
        .await
        .expect("queued streaming XHR stop abort should finish");
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            r#"[["readystatechange",4,0],["abort",4,0],["loadend",4,0]]"#
        );
    }
}
