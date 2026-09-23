use super::preload_as::evaluate_preload_probe;
use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn preload_events_and_resource_timing_belong_to_the_originating_child_document() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let observed = evaluate_preload_probe(
        &mut page,
        include_str!("../fixtures/preload-document-owner.js"),
    )
    .await?;
    assert_eq!(observed["observations"].as_array().unwrap().len(), 13);
    assert_eq!(observed["failures"], serde_json::json!([]));
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_preload_completions_cannot_target_a_replacement_document() -> Result<()> {
    for replace in ["navigate", "open"] {
        for destination in ["fetch", "script", "style", "image", "font", "track"] {
            let server = gated_preload_response_server(
                "<!doctype html><head></head><body>parent".to_owned(),
                "text/css",
                true,
                true,
            )
            .await?;
            let mut config = AppConfig::default();
            config
                .set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
            let browser = Browser::new(config)?;
            let mut page = browser.fetch(&server.url).await?;
            evaluate_preload_probe(&mut page, &format!(r#"(async () => {{
              globalThis.preloadOldEvents = [];
              globalThis.preloadFrame = document.createElement('iframe');
              preloadFrame.srcdoc = '<!doctype html><body>old';
              await new Promise(resolve => {{ preloadFrame.onload = resolve; document.body.append(preloadFrame); }});
              const child = preloadFrame.contentWindow;
              const link = child.document.createElement('link');
              Object.assign(link, {{rel:'preload', as:'{destination}', href:new URL('/resource',location.href).href}});
              link.onload = link.onerror = event => preloadOldEvents.push(event.type);
              child.document.head.append(link);
              return true;
            }})()"#)).await?;
            tokio::time::timeout(Duration::from_secs(5), server.requested).await??;
            let replacement = if replace == "navigate" {
                "new Promise(resolve => { preloadFrame.onload = resolve; preloadFrame.srcdoc = '<!doctype html><body>replacement'; })"
            } else {
                "(async () => { const doc=preloadFrame.contentDocument; doc.open(); doc.write('<!doctype html><body>replacement'); doc.close(); })()"
            };
            evaluate_preload_probe(&mut page, &format!("({replacement}).then(() => true)")).await?;
            server
                .release
                .send(())
                .expect("release retired Document's preload");
            let request_url = server.url.replace("/page", "/resource");
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    evaluate_preload_probe(&mut page, "Promise.resolve(true)").await?;
                    if page
                        .subresource_network_records()
                        .iter()
                        .any(|record| record.url().as_str() == request_url)
                    {
                        return Ok::<_, anyhow::Error>(());
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await??;
            let original_frame = page
                .subresource_network_records()
                .iter()
                .find(|record| record.url().as_str() == request_url)
                .and_then(|record| record.frame_id())
                .map(str::to_owned)
                .expect("the child preload's historical record retains its frame identity");
            // A fresh link event is a FIFO barrier after the old completion's
            // DOM-manipulation task, and proves the replacement remains usable.
            let observed = evaluate_preload_probe(&mut page, r#"(async () => {
              const child=preloadFrame.contentWindow;
              const link=child.document.createElement('link');
              Object.assign(link,{rel:'preload',as:'fetch',href:new URL('/marker',location.href).href});
              const terminal=await new Promise(resolve => {
                link.onload=link.onerror=e=>resolve({type:e.type,ownRealm:e instanceof child.Event});
                child.document.head.append(link);
              });
              return {oldEvents:preloadOldEvents,terminal,
                parentOld:performance.getEntriesByName(new URL('/resource',location.href).href).length,
                childOld:child.performance.getEntriesByName(new URL('/resource',location.href).href).length,
                parentMarker:performance.getEntriesByName(link.href).length,
                childMarker:child.performance.getEntriesByName(link.href).length};
            })()"#).await?;
            assert_eq!(
                observed,
                serde_json::json!({
                    "oldEvents":[],"terminal":{"type":"load","ownRealm":true},
                    "parentOld":0,"childOld":usize::from(replace == "open"),"parentMarker":0,"childMarker":1
                }),
                "replace={replace}, destination={destination}"
            );
            let marker_url = server.url.replace("/page", "/marker");
            let marker_frame = page
                .subresource_network_records()
                .iter()
                .find(|record| record.url().as_str() == marker_url)
                .and_then(|record| record.frame_id());
            assert_eq!(marker_frame, Some(original_frame.as_str()));
            server.task.await??;
        }
    }
    Ok(())
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
    gated_preload_response_server(markup, content_type, successful, false).await
}

async fn gated_preload_response_server(
    markup: String,
    content_type: &'static str,
    successful: bool,
    followup_request: bool,
) -> Result<GatedPreloadServer> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/page", listener.local_addr()?);
    let (request_tx, requested) = tokio::sync::oneshot::channel();
    let (release, release_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut request_tx = Some(request_tx);
        let mut release_rx = Some(release_rx);
        for _ in 0..(2 + usize::from(followup_request)) {
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
            } else if request.starts_with(b"GET /marker ") {
                ("200 OK", "text/plain", "marker")
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
