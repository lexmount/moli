use super::*;
use crate::util::v8str;

fn start_probe(vm: &mut crate::runtime::PageVmTaskExecutorTestHarness, probe: &str, worker: bool) {
    let source = if worker {
        let script = format!(
            "Promise.resolve().then(() => {probe}).then(value => {{ postMessage(value); close(); }}, error => {{ postMessage(String(error.stack || error)); close(); }});"
        );
        format!(
            r#"globalThis.__nullBodyResult = 'pending';
            const worker = new Worker(URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}})));
            worker.onmessage = event => {{ globalThis.__nullBodyResult = event.data; }};
            worker.onerror = event => {{ globalThis.__nullBodyResult = event.message; event.preventDefault(); }};"#,
            serde_json::to_string(&script).unwrap()
        )
    } else {
        format!(
            r#"globalThis.__nullBodyResult = 'pending';
            Promise.resolve().then(() => {probe}).then(
                value => {{ globalThis.__nullBodyResult = value; }},
                error => {{ globalThis.__nullBodyResult = String(error.stack || error); }});"#
        )
    };
    vm.eval(&source).expect("start null response body probe");
}

#[tokio::test(flavor = "current_thread")]
async fn xhr_null_body_data_urls_preserve_headers_and_response_types_in_window_and_worker() {
    for worker in [false, true] {
        let loader = static_http_loader(std::iter::empty::<String>());
        let mut vm =
            new_page_task_executor_test_vm_with_loader("https://xhr-null-body.test/", &loader);
        start_probe(
            &mut vm,
            r#"
(async () => {
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const worker = typeof document === 'undefined';
  for (const async of [true, false]) {
    const cases = [
      ['', 'text/plain;charset=utf-8', 'hello'],
      ['text', 'text/plain', 'hello'],
      ['arraybuffer', 'application/octet-stream', '%00%FF'],
      ['blob', 'application/octet-stream', '%00%FF'],
      ['json', 'application/json', '%7B%22value%22%3A7%7D'],
      ['document', 'text/html', '%3Cp%3Ehello']
    ];
    for (const [type, mime, body] of cases) {
      if ((!async && !worker && type !== '') || (worker && type === 'document')) continue;
      const xhr = new XMLHttpRequest(), events = [];
      xhr.onreadystatechange = () => events.push(xhr.readyState);
      for (const name of ['loadstart', 'progress', 'load', 'loadend']) {
        xhr.addEventListener(name, e => events.push(`${name}:${e.loaded}:${e.total}:${e.lengthComputable}`));
      }
      const done = new Promise(resolve => xhr.addEventListener('loadend', resolve));
      const url = `data:${mime},${body}`;
      xhr.open('hEaD', url, async);
      if (async || worker) xhr.responseType = type;
      xhr.send();
      if (async) await done;
      assert(xhr.status === 200 && xhr.responseURL === url, 'HEAD status and URL');
      assert(xhr.getResponseHeader('content-type') === mime, 'HEAD preserves MIME');
      assert(xhr.getResponseHeader('content-length') === null, 'data URLs do not acquire Content-Length');
      if (type === '' || type === 'text') assert(xhr.responseText === '' && xhr.response === '', 'HEAD text is empty');
      else if (type === 'arraybuffer') assert(xhr.response instanceof ArrayBuffer && xhr.response.byteLength === 0, 'HEAD ArrayBuffer is empty');
      else if (type === 'blob') assert(xhr.response instanceof Blob && xhr.response.size === 0 && xhr.response.type === mime, 'HEAD Blob is empty and retains MIME');
      else assert(xhr.response === null, 'HEAD JSON and Document are null');
      if (!worker && (type === '' || type === 'document')) assert(xhr.responseXML === null, 'HEAD has no response Document');
      const expected = async
        ? [1, 'loadstart:0:0:false', 2, 'progress:0:0:false', 4, 'load:0:0:false', 'loadend:0:0:false']
        : [1, 4, 'load:0:0:false', 'loadend:0:0:false'];
      assert(JSON.stringify(events) === JSON.stringify(expected), `HEAD ${type}/${async}: ${JSON.stringify(events)}`);
    }
  }
  for (const method of ['GET', 'POST', 'PUT', 'DELETE', 'UNICORN']) {
    const xhr = new XMLHttpRequest();
    const done = new Promise(resolve => xhr.onloadend = resolve);
    xhr.open(method, 'data:text/plain;base64,aGVsbG8=');
    xhr.send();
    await done;
    assert(xhr.status === 200 && xhr.responseText === 'hello', method + ' keeps the data body');
  }
  if (!worker) {
    const xhr = new XMLHttpRequest();
    const done = new Promise(resolve => xhr.onloadend = resolve);
    xhr.open('GET', 'data:text/html,');
    xhr.responseType = 'document';
    xhr.send();
    await done;
    assert(xhr.response instanceof Document && xhr.response.documentElement.localName === 'html', 'an empty non-null HTML body still creates a Document');
  }
  {
    const xhr = new XMLHttpRequest(), events = [];
    xhr.onprogress = () => { events.push('progress:' + xhr.readyState); xhr.abort(); };
    xhr.onabort = () => events.push('abort');
    xhr.onload = () => events.push('load');
    const done = new Promise(resolve => xhr.onloadend = () => { events.push('loadend'); resolve(); });
    xhr.open('HEAD', 'data:text/plain,hello');
    xhr.send();
    await done;
    assert(xhr.readyState === 0 && xhr.status === 0, 'abort from zero-byte progress clears the response');
    assert(events.join(',') === 'progress:2,abort,loadend', 'abort from progress prevents load: ' + events);
  }
  for (const url of ['data:text/plain,hello', 'data:application/octet-stream;base64,AP8=']) {
    const response = await fetch(url, {method: 'HEAD'});
    const clone = response.clone();
    assert(response.status === 200 && response.body === null && clone.body === null, 'Fetch keeps a null body');
    assert(await response.text() === '' && (await clone.arrayBuffer()).byteLength === 0, 'Fetch HEAD consumers are empty');
    assert(!response.bodyUsed && !clone.bodyUsed, 'null bodies remain undisturbed');
  }
  const blobURL = URL.createObjectURL(new Blob(['hello']));
  for (const url of [blobURL, 'data:missing-comma']) {
    const xhr = new XMLHttpRequest();
    let failed = false;
    const done = new Promise(resolve => xhr.onloadend = resolve);
    xhr.onerror = () => { failed = true; };
    xhr.open('HEAD', url);
    xhr.send();
    await done;
    assert(failed && xhr.status === 0, 'HEAD preserves local URL failures: ' + url);
  }
  URL.revokeObjectURL(blobURL);
  return 'ok';
})()
"#,
            worker,
        );
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "globalThis.__nullBodyResult",
            "ok",
            "Window/Worker HEAD data response",
        )
        .await;
    }
}

#[test]
fn xhr_null_body_statuses_discard_buffered_and_streamed_bytes() {
    for streaming in [false, true] {
        for (method, status) in [
            ("HEAD", 200),
            ("GET", 101),
            ("GET", 103),
            ("GET", 204),
            ("POST", 205),
            ("GET", 304),
        ] {
            for response_type in ["", "text", "json", "arraybuffer", "blob", "document"] {
                let mut vm = new_storage_test_vm("https://xhr-null-status.test/");
                vm.set_fetch_subresource_interception(
                    true,
                    Some(crate::types::SubresourceResourceType::Xhr),
                );
                vm.eval(&format!(r#"
                    globalThis.xhr = new XMLHttpRequest();
                    globalThis.events = [];
                    xhr.onreadystatechange = () => events.push(xhr.readyState);
                    for (const name of ['progress', 'load', 'loadend'])
                        xhr.addEventListener(name, e => events.push(`${{name}}:${{e.loaded}}:${{e.total}}:${{e.lengthComputable}}`));
                    xhr.open('{method}', '/response');
                    xhr.responseType = '{response_type}';
                    xhr.send();
                "#)).unwrap();
                let pending = vm.take_pending_subresource_fetch_infos();
                assert_eq!(pending.len(), 1);
                let request = &pending[0];
                let head = moli_fetch::ResponseHead {
                    final_url: request.url.clone(),
                    status,
                    status_text: Some("Preserved".to_owned()),
                    headers: vec![
                        ("Content-Type".to_owned(), "text/html".to_owned()),
                        ("Content-Length".to_owned(), "7".to_owned()),
                    ],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                };
                if streaming {
                    let body_source_id = crate::network_host::new_network_body_source_id();
                    vm.start_streaming_async_subresource_fetch(
                        crate::types::AsyncSubresourceStreamingStarted {
                            internal_id: request.internal_id,
                            request_url: request.url.clone(),
                            request_method: method.to_owned(),
                            request_headers: Vec::new(),
                            request_body: None,
                            body_source_id,
                            head,
                            network_request_headers: None,
                        },
                    )
                    .unwrap();
                    let complete_events =
                        r#"[1,2,"progress:0:7:true",4,"load:0:7:true","loadend:0:7:true"]"#;
                    assert_eq!(
                        vm.eval("JSON.stringify(events)").unwrap(),
                        complete_events,
                        "a null body completes at headers, before transport EOF: {method}/{status}/{response_type}"
                    );
                    vm.append_streaming_async_subresource_fetch_chunk(
                        body_source_id,
                        b"<p>body".to_vec(),
                    );
                    assert_eq!(
                        vm.eval("JSON.stringify(events)").unwrap(),
                        complete_events,
                        "null-body chunks must not expose additional events: {method}/{status}/{response_type}"
                    );
                    vm.finish_streaming_async_subresource_fetch(
                        request.internal_id,
                        body_source_id,
                        if status == 205 {
                            Err("invalid response body was truncated".to_owned())
                        } else {
                            Ok(())
                        },
                    )
                    .unwrap();
                } else {
                    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context;
                    vm.renderer_document_isolate
                        .with_entered_renderer_document_isolate(move |isolate| {
                            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                            let scope = &mut scope.init();
                            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                            let scope = &mut v8::ContextScope::new(scope, context);
                            let xhr = context
                                .global(scope)
                                .get(scope, v8str(scope, "xhr").into())
                                .unwrap()
                                .to_object(scope)
                                .unwrap();
                            crate::network_host::apply_xhr_response_body_source(
                                scope,
                                xhr,
                                head,
                                moli_fetch::ResponseBody::materialized_bytes(b"<p>body".to_vec()),
                            );
                            Ok(())
                        })
                        .unwrap();
                }
                assert_eq!(vm.eval(r#"JSON.stringify([
                    xhr.status, xhr.statusText, xhr.getResponseHeader('content-type'), xhr.getResponseHeader('content-length'),
                    xhr.responseType === 'arraybuffer' ? xhr.response.byteLength === 0 :
                    xhr.responseType === 'blob' ? xhr.response.size === 0 && xhr.response.type === 'text/html' :
                    xhr.responseType === 'document' || xhr.responseType === 'json' ? xhr.response === null : xhr.responseText === '',
                    events])"#).unwrap(), serde_json::json!([status, "Preserved", "text/html", "7", true,
                    [1, 2, "progress:0:7:true", 4, "load:0:7:true", "loadend:0:7:true"]]).to_string(),
                    "streaming={streaming}, {method}/{status}/{response_type}");
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn xhr_null_body_empty_http_and_data_responses_finish_without_loading() {
    for worker in [false, true] {
        let server = StaticHttpServer::spawn_with_bodies(vec![String::new()]).await;
        let url = server.base_url();
        let loader = static_http_loader(std::iter::empty::<String>());
        let mut vm = new_page_task_executor_test_vm_with_loader(url.as_str(), &loader);
        let probe = r#"
(async () => {
  const assert = (value, message) => { if (!value) throw new Error(message); };
  for (const url of [HTTP_URL, 'data:text/plain,', URL.createObjectURL(new Blob([]))]) {
    const xhr = new XMLHttpRequest(), events = [];
    xhr.onreadystatechange = () => events.push(xhr.readyState);
    for (const name of ['loadstart', 'progress', 'load', 'loadend'])
      xhr.addEventListener(name, e => events.push(`${name}:${e.loaded}:${e.total}:${e.lengthComputable}`));
    const done = new Promise(resolve => xhr.onloadend = resolve);
    xhr.open('GET', url);
    xhr.send();
    await done;
    assert(xhr.status === 200 && xhr.responseText === '', 'empty response succeeds');
    assert(JSON.stringify(events) === JSON.stringify([1, 'loadstart:0:0:false', 2, 'progress:0:0:false', 4, 'load:0:0:false', 'loadend:0:0:false']), JSON.stringify(events));
    if (url.startsWith('blob:')) URL.revokeObjectURL(url);
  }
  return 'ok';
})()
"#.replace("HTTP_URL", &serde_json::to_string(url.as_str()).unwrap());
        start_probe(&mut vm, &probe, worker);
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "globalThis.__nullBodyResult",
            "ok",
            "empty HTTP/local XHR response",
        )
        .await;
        assert_eq!(server.finish_targets().await, ["/"]);
    }
}

#[test]
fn xhr_null_body_late_completion_preserves_a_replacement_requests_progress() {
    let mut vm = new_storage_test_vm("https://xhr-null-body-reuse.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Xhr));
    vm.eval(
        r#"
        globalThis.xhr = new XMLHttpRequest();
        xhr.onprogress = event => progressEvents.push(event.loaded);
    "#,
    )
    .unwrap();
    let mut streams = Vec::new();
    for (method, path) in [("HEAD", "/head"), ("GET", "/replacement")] {
        vm.eval(&format!(
            "globalThis.progressEvents = []; xhr.open('{method}', '{path}'); xhr.send();"
        ))
        .unwrap();
        let pending = vm.take_pending_subresource_fetch_infos();
        assert_eq!(pending.len(), 1);
        let request = &pending[0];
        let body_source_id = crate::network_host::new_network_body_source_id();
        vm.start_streaming_async_subresource_fetch(
            crate::types::AsyncSubresourceStreamingStarted {
                internal_id: request.internal_id,
                request_url: request.url.clone(),
                request_method: method.to_owned(),
                request_headers: Vec::new(),
                request_body: None,
                body_source_id,
                head: moli_fetch::ResponseHead {
                    final_url: request.url.clone(),
                    status: 200,
                    status_text: None,
                    headers: vec![
                        ("Content-Type".to_owned(), "text/plain".to_owned()),
                        ("Content-Length".to_owned(), "3".to_owned()),
                    ],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
                network_request_headers: None,
            },
        )
        .unwrap();
        streams.push((request.internal_id, body_source_id));
        assert_eq!(
            vm.eval("xhr.readyState").unwrap(),
            if method == "HEAD" { "4" } else { "2" }
        );
    }
    let (old_id, old_body) = streams[0];
    let (new_id, new_body) = streams[1];
    vm.append_streaming_async_subresource_fetch_chunk(new_body, b"a".to_vec());
    vm.append_streaming_async_subresource_fetch_chunk(new_body, b"b".to_vec());
    vm.finish_streaming_async_subresource_fetch(old_id, old_body, Ok(()))
        .unwrap();
    vm.append_streaming_async_subresource_fetch_chunk(new_body, b"c".to_vec());
    assert_eq!(
        vm.eval("JSON.stringify([xhr.readyState, xhr.responseText, progressEvents])")
            .unwrap(),
        r#"[3,"abc",[1]]"#,
        "the old HEAD completion must not cancel the replacement's progress gate"
    );
    vm.finish_streaming_async_subresource_fetch(new_id, new_body, Ok(()))
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([xhr.readyState, xhr.responseText, progressEvents])")
            .unwrap(),
        r#"[4,"abc",[1,3]]"#,
        "replacement completion still flushes its last deferred progress event"
    );
}
