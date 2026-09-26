use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_worker_script_load_failure_does_not_dispatch_window_error() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker script 404 server");
    let addr = listener
        .local_addr()
        .expect("worker script 404 server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker script request");
        let mut buf = [0_u8; 1024];
        let n = stream
            .read(&mut buf)
            .await
            .expect("read worker script request");
        let request = String::from_utf8_lossy(&buf[..n]);
        assert!(
            request.starts_with("GET /does-not-exist.js "),
            "unexpected worker script request: {request}"
        );
        stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write worker script 404 response");
    });
    let url = url::Url::parse(&format!("http://{addr}/page.html")).unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><body>worker load failure</body>",
    )
    .await;

    let (installed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
(() => {
  globalThis.__lm_worker_script_error_events = [];
  window.onerror = message => {
    globalThis.__lm_worker_script_error_events.push("window:" + String(message));
    return true;
  };
  const worker = new Worker("does-not-exist.js");
  globalThis.__lm_missing_script_worker = worker;
  worker.onerror = event => {
    globalThis.__lm_worker_script_error_events.push([
      "worker",
      event.type,
      Object.getPrototypeOf(event) === Event.prototype,
      event.bubbles, event.cancelable, event.composed, event.isTrusted,
      ['message', 'filename', 'lineno', 'colno', 'error'].some(name => name in event)
    ].join(":"));
  };
  return "installed";
})()
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("worker script load failure probe should install");
    assert_eq!(
        renderer_json_value(installed),
        Some(serde_json::json!("installed"))
    );

    page.run_async_command(RendererPageCommand::WaitForScriptTruthy {
        expression: r#"globalThis.__lm_worker_script_error_events?.some(event => event.startsWith("worker:"))"#.to_owned(),
        timeout_ms: 2_000,
        loader: loader.clone(),
    })
    .await
    .expect("worker script load failure should dispatch worker error");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify(globalThis.__lm_worker_script_error_events)"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("worker script load failure events should evaluate");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!(
            "[\"worker:error:true:false:false:false:true:false\"]"
        )),
        "worker script load failure must not bubble to window.onerror"
    );

    server
        .await
        .expect("worker script 404 server should finish");
    page.close_async()
        .await
        .expect("worker script load failure page should close");
}
