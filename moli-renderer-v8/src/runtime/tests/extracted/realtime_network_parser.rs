use super::*;
use anyhow::{Context, ensure};
use futures_util::FutureExt;
use moli_page_types::{LayoutConfiguration, LayoutPolicy};
use std::panic::AssertUnwindSafe;

const INITIAL_HTML: &str = r#"<!doctype html><meta charset=utf-8>
<style>#target { display:block; width:100px; height:20px; padding:0; border:0 }</style>
<body><div id=target></div><script>
(() => {
  let result;
  try {
    window.__target = document.getElementById('target');
    window.__heldStyle = getComputedStyle(window.__target);
    const width = window.__target.offsetWidth;
    result = [width, window.__heldStyle.width, document.readyState];
  } catch (error) {
    result = {error: String(error), stack: String(error.stack)};
  }
  fetch('/initial?result=' + encodeURIComponent(JSON.stringify(result)));
})();
</script>"#;

const CONTINUED_HTML: &str = r#"<style>#target { width:200px }</style>
<script src=/blocking.js></script>
<div id=after-blocking></div><script>
fetch('/resumed?result=' + encodeURIComponent(JSON.stringify([
  document.getElementById('after-blocking') !== null, document.readyState
])));
</script>"#;
const FINAL_HTML: &str = "</body></html>";

const BLOCKING_SCRIPT: &str = r#"(() => {
  let result;
  try {
    // This held declaration is the first style/geometry observation after
    // the parser supplied the new rule. No test command repairs geometry.
    const heldWidth = window.__heldStyle.width;
    const width = window.__target.offsetWidth;
    const again = window.__target.offsetWidth;
    result = [heldWidth, width, again,
      document.getElementById('after-blocking') !== null, document.readyState];
  } catch (error) {
    result = {error: String(error), stack: String(error.stack)};
  }
  fetch('/observed?result=' + encodeURIComponent(JSON.stringify(result)));
})();"#;

fn request_url(request: &str) -> url::Url {
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("test request must include a path");
    url::Url::parse(&format!("http://fixture.test{path}")).expect("test request URL")
}

fn request_result(request: &url::Url) -> serde_json::Value {
    let result = request
        .query_pairs()
        .find(|(key, _)| key == "result")
        .expect("script must report its first observations")
        .1;
    serde_json::from_str(&result).expect("script observation must be valid JSON")
}

#[tokio::test(flavor = "current_thread")]
async fn realtime_streamed_css_is_current_on_blocking_script_first_read() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let (initial_tx, initial_rx) = oneshot::channel();
    let (continue_tx, continue_rx) = oneshot::channel();
    let (requested_tx, requested_rx) = oneshot::channel();
    let (script_tx, script_rx) = oneshot::channel();
    let (observed_tx, observed_rx) = oneshot::channel();
    let (resumed_tx, resumed_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
    let mut server = tokio::spawn(async move {
        let (mut document_stream, _) = listener.accept().await.unwrap();
        let request = read_owner_wake_http_request_head(&mut document_stream).await;
        assert_eq!(request_url(&request).path(), "/stream.html");
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            INITIAL_HTML.len() + CONTINUED_HTML.len() + FINAL_HTML.len(),
        );
        document_stream.write_all(headers.as_bytes()).await.unwrap();
        document_stream
            .write_all(INITIAL_HTML.as_bytes())
            .await
            .unwrap();

        // The first parser-inserted script must execute before the next body
        // bytes exist. Its report proves that the old 100px sample was read.
        let (mut initial_stream, _) = listener.accept().await.unwrap();
        let request = read_owner_wake_http_request_head(&mut initial_stream).await;
        let request = request_url(&request);
        assert_eq!(request.path(), "/initial");
        initial_stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        initial_tx.send(request_result(&request)).unwrap();
        drop(initial_stream);
        continue_rx.await.unwrap();
        document_stream
            .write_all(CONTINUED_HTML.as_bytes())
            .await
            .unwrap();

        let (mut script_stream, _) = listener.accept().await.unwrap();
        let request = read_owner_wake_http_request_head(&mut script_stream).await;
        assert_eq!(request_url(&request).path(), "/blocking.js");
        requested_tx.send(()).unwrap();
        script_rx.await.unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            BLOCKING_SCRIPT.len(),
            BLOCKING_SCRIPT,
        );
        script_stream.write_all(response.as_bytes()).await.unwrap();
        drop(script_stream);

        // HTTP requests may be delivered in either order. The values were
        // frozen in each script, so transport order cannot repair a bad read.
        let mut observed_tx = Some(observed_tx);
        let mut resumed_tx = Some(resumed_tx);
        for _ in 0..2 {
            let (mut witness_stream, _) = listener.accept().await.unwrap();
            let request = read_owner_wake_http_request_head(&mut witness_stream).await;
            let request = request_url(&request);
            let value = request_result(&request);
            match request.path() {
                "/observed" => observed_tx
                    .take()
                    .expect("one first-read report")
                    .send(value)
                    .unwrap(),
                "/resumed" => resumed_tx
                    .take()
                    .expect("one parser-resume report")
                    .send(value)
                    .unwrap(),
                path => panic!("unexpected parser witness {path}"),
            }
            witness_stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        }
        assert!(observed_tx.is_none() && resumed_tx.is_none());
        finish_rx.await.unwrap();
        document_stream
            .write_all(FINAL_HTML.as_bytes())
            .await
            .unwrap();
        document_stream.shutdown().await.unwrap();
    });

    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_configuration(LayoutConfiguration {
            policy: LayoutPolicy::Realtime,
            ..LayoutConfiguration::default()
        })
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse(&format!("{base_url}/initial-document")).unwrap(),
        "<!doctype html><body></body>",
    )
    .await;
    let testing = RendererPageTestingHandle::new_for_testing(&page);
    let case = AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(10), async {
        let navigation = page
            .enqueue_protocol_command(
                RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                    expression: format!("location.href = '{base_url}/stream.html'; 'navigating'"),
                    await_promise: false,
                },
            )
            .context("normal owner navigation should enqueue")?;
        let initial = initial_rx.await.context("initial HTTP witness channel")?;
        ensure!(
            initial == serde_json::json!([100, "100px", "loading"]),
            "old geometry witness differs: {initial}",
        );
        continue_tx
            .send(())
            .map_err(|()| anyhow::anyhow!("continued body receiver closed"))?;
        requested_rx
            .await
            .context("blocking script HTTP request channel")?;
        // A preload request alone is insufficient. Observe the actual resident
        // parser, its exact resume permit and still-unready source before
        // releasing the HTTP response. This existing owner command is passive:
        // it neither runs JS nor asks for layout or a Page task.
        loop {
            let observation = testing
                .layout_working_set_observation_async()
                .await
                .context("observe parked parser source state")?;
            if observation.main_parser_source_waiting {
                break;
            }
            tokio::task::yield_now().await;
        }
        script_tx
            .send(())
            .map_err(|()| anyhow::anyhow!("blocking script receiver closed"))?;
        let observed = observed_rx
            .await
            .context("first-read HTTP witness channel")?;
        ensure!(
            observed == serde_json::json!(["200px", 200, 200, false, "loading"]),
            "held CSSOM first-read witness differs: {observed}",
        );
        let resumed = resumed_rx
            .await
            .context("parser-resume HTTP witness channel")?;
        ensure!(
            resumed == serde_json::json!([true, "loading"]),
            "before-EOF parser-resume witness differs: {resumed}",
        );
        finish_tx
            .send(())
            .map_err(|()| anyhow::anyhow!("main response EOF receiver closed"))?;
        navigation
            .wait()
            .await
            .context("streamed navigation should finish")?;
        Ok::<(), anyhow::Error>(())
    }))
    .catch_unwind()
    .await;

    // Keep cleanup outside the timed case. A failed witness, timeout or panic
    // must not release EOF to rescue the case. Abort a failed server and join
    // it, then request Page removal regardless of that result. Bound both cleanup
    // deadlock guards. Original case failures are preserved below.
    let failed = !matches!(&case, Ok(Ok(Ok(()))));
    if failed {
        server.abort();
    }
    let server_result = match tokio::time::timeout(Duration::from_secs(5), &mut server).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) if failed && error.is_cancelled() => Ok(()),
        Ok(Err(error)) => Err(anyhow::anyhow!("HTTP server task failed: {error}")),
        Err(error) => {
            server.abort();
            let after_abort = tokio::time::timeout(Duration::from_secs(5), &mut server).await;
            Err(anyhow::anyhow!(
                "HTTP server cleanup deadline: {error}; join after abort: {after_abort:?}"
            ))
        }
    };
    let close_result = tokio::time::timeout(Duration::from_secs(5), page.close_async())
        .await
        .context("Page close cleanup deadline")
        .and_then(|result| result.context("Page close cleanup"));
    let case_result = match case {
        Err(panic) => {
            if close_result.is_err() || server_result.is_err() {
                eprintln!(
                    "parser case panicked; Page close={close_result:#?}; server cleanup={server_result:#?}"
                );
            }
            std::panic::resume_unwind(panic);
        }
        Ok(Err(error)) => Err(anyhow::anyhow!(
            "normal streaming parser and blocking source continuation deadline: {error}"
        )),
        Ok(Ok(result)) => result,
    };
    match (case_result, close_result, server_result) {
        (Ok(()), Ok(()), Ok(())) => Ok(()),
        (case, close, server) => Err(anyhow::anyhow!(
            "parser case={case:#?}; Page close={close:#?}; HTTP server cleanup={server:#?}"
        )),
    }
}
