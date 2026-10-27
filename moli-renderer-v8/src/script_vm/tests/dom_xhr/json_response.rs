use super::*;

const JSON_PROBE: &str = r#"
(async () => {
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  const json = Array.from(new TextEncoder().encode('{"project":["game"],"text":"中文"}'));
  const bom = [239,187,191];
  const cases = [
    [json, {project:['game'],text:'中文'}],
    [[...bom,...json], {project:['game'],text:'中文'}],
    [[...bom,...bom,...json], null],
    [[...bom,123], null],
    [[255,254,123,0,125,0], null],
    [[123,34,120,34,58,34,255,34,125], {x:'\ufffd'}],
    [[], null]
  ];
  const request = (url, type) => new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open('GET', url);
    xhr.responseType = type;
    assert(xhr.response === null, 'non-text response before completion');
    xhr.overrideMimeType('application/json; charset=windows-1252');
    let loaded = false;
    xhr.onload = () => { loaded = true; };
    xhr.onloadend = () => loaded ? resolve(xhr.response) : reject(new Error('load not dispatched'));
    xhr.onerror = () => reject(new Error('XHR failed'));
    xhr.send();
  });
  for (const [bytes,expected] of cases) {
    const url = 'data:application/json;charset=windows-1252;base64,' + btoa(String.fromCharCode(...bytes));
    const response = await request(url, 'json');
    assert(JSON.stringify(response) === JSON.stringify(expected), 'JSON response for '+bytes);
    const binary = Array.from(new Uint8Array(await request(url, 'arraybuffer')));
    assert(JSON.stringify(binary) === JSON.stringify(bytes), 'binary response changed');
  }
  const originalParse = JSON.parse;
  let response;
  try {
    JSON.parse = () => { throw new Error('author parser must not be called'); };
    response = await request('data:application/json,%EF%BB%BF%7B%22value%22%3A7%7D', 'json');
  } finally {
    JSON.parse = originalParse;
  }
  assert(response && response.value === 7, 'JSON parsing uses the intrinsic parser');
  return 'ok';
})()
"#;

async fn check_json_response(worker: bool) {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader("https://xhr-json-bom.test/", &loader);
    let source = if worker {
        let worker_source = format!(
            "Promise.resolve().then(() => {JSON_PROBE}).then(value => {{ postMessage(value); close(); }}, error => {{ postMessage(String(error.stack || error)); close(); }});"
        );
        format!(
            "globalThis.__jsonBomResult = 'pending'; const worker = new Worker('data:text/javascript,' + encodeURIComponent({})); worker.onmessage = event => {{ __jsonBomResult = event.data; }}; worker.onerror = event => {{ __jsonBomResult = event.message; event.preventDefault(); }};",
            serde_json::to_string(&worker_source).unwrap()
        )
    } else {
        format!(
            "globalThis.__jsonBomResult = 'pending'; Promise.resolve().then(() => {JSON_PROBE}).then(value => {{ __jsonBomResult = value; }}, error => {{ __jsonBomResult = String(error.stack || error); }});"
        )
    };
    vm.eval(&source).expect("start JSON XHR probe");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "globalThis.__jsonBomResult",
        "ok",
        "JSON XHR byte decoding",
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn window_xhr_json_decodes_utf8_bom_and_preserves_binary_responses() {
    check_json_response(false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn worker_xhr_json_decodes_utf8_bom_and_preserves_binary_responses() {
    check_json_response(true).await;
}

#[test]
fn streaming_xhr_json_decodes_a_bom_split_across_chunks() {
    let mut vm = new_storage_test_vm("https://xhr-json-stream.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Xhr));
    vm.eval(
        r#"
globalThis.__jsonStream = new XMLHttpRequest();
globalThis.__jsonStreamEvents = [];
__jsonStream.open('GET', '/project.json');
__jsonStream.responseType = 'json';
__jsonStream.onload = () => __jsonStreamEvents.push('load');
__jsonStream.onloadend = () => __jsonStreamEvents.push('loadend');
__jsonStream.send();
"#,
    )
    .expect("start streaming JSON XHR");
    let pending = vm.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);
    let pending = &pending[0];
    let body_source_id = crate::network_host::new_network_body_source_id();
    vm.start_streaming_async_subresource_fetch(crate::types::AsyncSubresourceStreamingStarted {
        skip_fetch_security_validation: false,
        response_filter: None,
        internal_id: pending.internal_id,
        request_url: pending.url.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new().into(),
        request_body: None,
        body_source_id,
        head: moli_fetch::ResponseHead {
            final_url: pending.url.clone(),
            status: 200,
            status_text: None,
            headers: vec![("Content-Type".to_owned(), b"application/json".to_vec())],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
        network_request_headers: None,
    })
    .expect("deliver streaming JSON headers");
    for chunk in [b"\xef".as_slice(), b"\xbb", b"\xbf", br#"{"project":[1]}"#] {
        vm.append_streaming_async_subresource_fetch_chunk(body_source_id, chunk.to_vec());
        assert_eq!(vm.eval("__jsonStream.response === null").unwrap(), "true");
    }
    vm.finish_streaming_async_subresource_fetch(pending.internal_id, body_source_id, Ok(()))
        .expect("finish streaming JSON XHR");
    assert_eq!(
        vm.eval("JSON.stringify([__jsonStream.response,__jsonStreamEvents])")
            .unwrap(),
        r#"[{"project":[1]},["load","loadend"]]"#
    );
}
