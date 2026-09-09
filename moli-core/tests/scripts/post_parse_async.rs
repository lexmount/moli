use super::*;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
};

async fn async_completion_after_parser_eof(
    blocked_mode: &'static str,
    source_failure: bool,
) -> Result<Vec<String>> {
    let browser = Browser::new(AppConfig::default())?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let parser_finished = Arc::new(Notify::new());
    let async_executed = Arc::new(Notify::new());
    let server = tokio::spawn(async move {
        let mut requests = tokio::task::JoinSet::new();
        loop {
            let (mut stream, _) = listener.accept().await.expect("script fixture connection");
            let parser_finished = parser_finished.clone();
            let async_executed = async_executed.clone();
            requests.spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = stream.read(&mut buffer).await.expect("script fixture request");
                    if read == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..read]);
                }
                let request = String::from_utf8(request).expect("HTTP request text");
                let path = request.split_whitespace().nth(1).expect("request path");
                let (status, mime, body) = match path {
                    "/parsed" => {
                        parser_finished.notify_one();
                        (200, "text/plain", String::new())
                    }
                    "/executed" => {
                        async_executed.notify_one();
                        (200, "text/plain", String::new())
                    }
                    "/blocked.js" => {
                        async_executed.notified().await;
                        (200, "text/javascript", "order.push('blocked');".to_owned())
                    }
                    "/ready.js" => {
                        parser_finished.notified().await;
                        if source_failure {
                            (404, "text/javascript", String::new())
                        } else {
                            (
                                200,
                                "text/javascript",
                                "order.push('async'); queueMicrotask(() => order.push('async-microtask'));"
                                    .to_owned(),
                            )
                        }
                    }
                    _ => (
                        200,
                        "text/html",
                        format!(
                            r#"<!doctype html><body><script>
                              window.order = [];
                              document.addEventListener('readystatechange', () => {{
                                if (document.readyState === 'interactive') {{
                                  order.push('interactive');
                                  fetch('/parsed');
                                }}
                              }});
                              document.addEventListener('DOMContentLoaded', () => order.push('dcl'));
                              addEventListener('load', () => order.push('load'));
                            </script>
                            <script {blocked_mode} src='/blocked.js'></script>
                            <script async src='/ready.js'
                              onload="order.push('async-load'); fetch('/executed')"
                              onerror="order.push('async-error'); fetch('/executed')"></script>"#
                        ),
                    ),
                };
                let response = format!(
                    "HTTP/1.1 {status} Response\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    let result = async {
        let mut page = tokio::time::timeout(
            Duration::from_secs(5),
            browser.fetch(&format!("http://{address}/")),
        )
        .await??;
        let result = page
            .evaluate_runtime_expression_with_await_async("JSON.stringify(order)", true)
            .await?;
        Ok(serde_json::from_str(
            result["value"]
                .as_str()
                .expect("async completion event order"),
        )?)
    }
    .await;
    server.abort();
    let _ = server.await;
    result
}

#[tokio::test(flavor = "multi_thread")]
async fn post_parse_async_script_runs_while_earlier_defer_source_is_pending() -> Result<()> {
    assert_eq!(
        async_completion_after_parser_eof("defer", false).await?,
        [
            "interactive",
            "async",
            "async-microtask",
            "async-load",
            "blocked",
            "dcl",
            "load"
        ]
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn post_parse_async_error_runs_while_earlier_defer_source_is_pending() -> Result<()> {
    assert_eq!(
        async_completion_after_parser_eof("defer", true).await?,
        ["interactive", "async-error", "blocked", "dcl", "load"]
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn post_parse_async_scripts_run_in_source_completion_order() -> Result<()> {
    let order = async_completion_after_parser_eof("async", false).await?;
    assert_eq!(order.first().map(String::as_str), Some("interactive"));
    assert_eq!(order.last().map(String::as_str), Some("load"));
    assert_eq!(order.iter().filter(|event| *event == "dcl").count(), 1);
    assert_eq!(
        order
            .iter()
            .filter(|event| *event != "dcl")
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "interactive",
            "async",
            "async-microtask",
            "async-load",
            "blocked",
            "load"
        ]
    );
    Ok(())
}
