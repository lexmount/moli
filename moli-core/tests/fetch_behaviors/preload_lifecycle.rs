use super::*;

async fn evaluate_preload_probe(page: &mut Page, fixture: &str) -> Result<serde_json::Value> {
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(
            &format!("({fixture}).then(JSON.stringify)"),
            true,
        ),
    )
    .await??;
    Ok(serde_json::from_str(
        result["value"]
            .as_str()
            .expect("preload lifecycle observations"),
    )?)
}

#[tokio::test(flavor = "multi_thread")]
async fn ordinary_preloads_do_not_delay_window_load_or_lose_late_events() -> Result<()> {
    for dynamic in [false, true] {
        for (destination, content_type) in [
            ("fetch", "text/plain"),
            ("script", "text/javascript"),
            ("style", "text/css"),
            ("image", "image/svg+xml"),
            ("font", "font/ttf"),
            ("track", "text/vtt"),
        ] {
            for successful in [true, false] {
                let server =
                    gated_preload_server(destination, content_type, dynamic, successful).await?;
                let mut config = AppConfig::default();
                config.set_optional_resource_fetch_mask(
                    moli_page_types::OptionalResourceFetchMask::ALL,
                );
                let browser = Browser::new(config)?;
                let mut page = browser
                    .fetch_with_wait_until(
                        &server.url,
                        RenderedDomWaitUntil::Load,
                        Duration::from_secs(5),
                    )
                    .await?;
                tokio::time::timeout(Duration::from_secs(5), server.requested).await??;
                let before = evaluate_preload_probe(
                    &mut page,
                    "Promise.resolve({state: document.readyState, events: preloadEvents, \
                     count: performance.getEntriesByName(document.getElementById('preload').href).length})",
                )
                .await?;
                assert_eq!(
                    before,
                    serde_json::json!({"state":"complete","events":["window:complete"],"count":0}),
                    "{destination}, dynamic={dynamic}, successful={successful}"
                );

                server.release.send(()).expect("release preload response");
                let after = evaluate_preload_probe(
                    &mut page,
                    r#"(async () => {
                      const link = document.getElementById('preload');
                      if (!preloadDone) await new Promise(resolve => {
                        link.addEventListener('load', resolve, {once: true});
                        link.addEventListener('error', resolve, {once: true});
                      });
                      return {state: document.readyState, events: preloadEvents,
                        count: performance.getEntriesByName(link.href).length};
                    })()"#,
                )
                .await?;
                let event = if successful { "load" } else { "error" };
                assert_eq!(
                    after,
                    serde_json::json!({
                        "state":"complete", "events":["window:complete",format!("link:{event}:complete")],
                        "count":1
                    }),
                    "{destination}, dynamic={dynamic}, successful={successful}"
                );
                server.task.await??;
            }
        }
    }
    Ok(())
}

struct GatedPreloadServer {
    url: String,
    requested: tokio::sync::oneshot::Receiver<()>,
    release: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<()>>,
}

async fn gated_preload_server(
    destination: &str,
    content_type: &'static str,
    dynamic: bool,
    successful: bool,
) -> Result<GatedPreloadServer> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/page", listener.local_addr()?);
    let (request_tx, requested) = tokio::sync::oneshot::channel();
    let (release, release_rx) = tokio::sync::oneshot::channel();
    let link = if dynamic {
        format!(
            "<script>const link = document.createElement('link'); \
             Object.assign(link, {{id:'preload', rel:'preload', as:'{destination}', href:'/resource'}}); \
             link.onload = link.onerror = recordPreload; document.head.append(link);</script>"
        )
    } else {
        format!(
            "<link id=preload rel=preload as={destination} href=/resource \
             onload=recordPreload(event) onerror=recordPreload(event)>"
        )
    };
    let markup = format!(
        "<!doctype html><head><script> \
         globalThis.preloadEvents = []; globalThis.preloadDone = false; \
         addEventListener('load', () => preloadEvents.push('window:' + document.readyState)); \
         function recordPreload(event) {{ preloadDone = true; \
           preloadEvents.push('link:' + event.type + ':' + document.readyState); }} \
         </script>{link}</head><body>preload lifecycle"
    );
    let task = tokio::spawn(async move {
        let mut request_tx = Some(request_tx);
        let mut release_rx = Some(release_rx);
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await?;
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).await?;
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let (status, mime, body) = if request.starts_with(b"GET /page ") {
                ("200 OK", "text/html", markup.as_str())
            } else {
                assert!(request.starts_with(b"GET /resource "));
                let _ = request_tx.take().expect("one preload request").send(());
                let _ = release_rx.take().expect("one response gate").await;
                (
                    if successful {
                        "200 OK"
                    } else {
                        "404 Not Found"
                    },
                    content_type,
                    "body {}",
                )
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await?;
        }
        Ok(())
    });
    Ok(GatedPreloadServer {
        url,
        requested,
        release,
        task,
    })
}
