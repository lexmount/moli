use super::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::JoinHandle,
};

const LAZY_PAGE: &str = r#"<!doctype html>
<script>
  const events = [];
  const windowLoaded = new Promise(resolve => {
    addEventListener('load', () => { events.push('window'); resolve(); }, {once: true});
  });
</script>
<img id="blocker" src="/blocker.svg">
<img id="plain" width="4" height="4" loading="lazy" src="/plain.svg">
<img id="changed" width="4" height="4" loading="lazy" src="/changed.svg">
<img id="removed" width="4" height="4" loading="lazy" src="/removed.svg">
<div style="height:8000px"></div>
<img id="resumed" width="4" height="4" loading="lazy" src="/resumed.svg">
<script>
  const imagesDone = Promise.all(Array.from(document.images, image => new Promise(resolve => {
    for (const type of ['load', 'error']) {
      image.addEventListener(type, () => { events.push(image.id + ':' + type); resolve(); });
    }
  })));
  document.getElementById('resumed').loading = 'eager';
</script>"#;

#[tokio::test]
async fn lazy_images_keep_their_load_behavior_when_loading_changes() -> Result<()> {
    let mut server = ImageServer::spawn(LAZY_PAGE, "").await?;
    let (_browser, mut page) = server.open_page(moli_core::LayoutPolicy::Realtime).await?;
    page.publish_layout_async().await?;
    let paths = [
        "/blocker.svg",
        "/plain.svg",
        "/changed.svg",
        "/removed.svg",
        "/resumed.svg",
    ];
    let mut requests = server.wait_for_images(&paths).await?;

    let before = evaluate(
        &mut page,
        r#"
        document.getElementById('changed').loading = 'eager';
        document.getElementById('removed').removeAttribute('loading');
        [document.readyState, events]
        "#,
    )
    .await?;
    assert_eq!(before, json!(["interactive", []]));

    finish_image(requests.remove("/blocker.svg").unwrap(), true).await?;
    let on_load = evaluate(
        &mut page,
        "windowLoaded.then(() => [document.readyState, events.slice(), \
            [...document.images].slice(1).map(image => image.complete)])",
    )
    .await
    .context("window load while all four lazy image responses are stalled")?;
    assert_eq!(
        on_load,
        json!([
            "complete",
            ["blocker:load", "window"],
            [false, false, false, false]
        ])
    );

    for &path in &paths[1..] {
        finish_image(requests.remove(path).unwrap(), path != "/removed.svg").await?;
    }
    let after = evaluate(&mut page, "imagesDone.then(() => events.slice(2).sort())").await?;
    assert_eq!(
        after,
        json!([
            "changed:load",
            "plain:load",
            "removed:error",
            "resumed:load"
        ])
    );
    Ok(())
}

#[tokio::test]
async fn loading_became_eager_before_image_update_delays_window_load() -> Result<()> {
    let mut server = ImageServer::spawn(
        r#"<!doctype html>
        <img id="blocker" src="/blocker.svg">
        <script>
          const events = [];
          function trackLoad(image, name) {
            return new Promise(resolve => image.onload = () => {
              events.push(name);
              resolve(true);
            });
          }
          const blockerLoaded = trackLoad(document.getElementById('blocker'), 'blocker');
          const windowLoaded = new Promise(resolve => addEventListener('load', () => {
            events.push('window');
            resolve(events.slice());
          }, {once: true}));

          function makeImage(name, connected) {
            const image = new Image(4, 4);
            const loaded = trackLoad(image, name);
            if (connected) document.body.append(image);
            image.loading = 'lazy';
            image.src = '/' + name + '.svg';
            image.loading = 'eager';
            if (!connected) document.body.append(image);
            return loaded;
          }
          const detachedLoaded = makeImage('detached', false);
          const connectedLoaded = makeImage('connected', true);

          const replacement = new Image(4, 4);
          const replacementLoaded = trackLoad(replacement, 'replacement');
          replacement.loading = 'lazy';
          replacement.src = '/superseded.svg';
          queueMicrotask(() => { replacement.loading = 'eager'; });
          replacement.src = '/replacement.svg';
          document.body.append(replacement);

          const resumed = new Image(4, 4);
          const resumedLoaded = trackLoad(resumed, 'resumed');
          resumed.style.marginTop = '8000px';
          resumed.loading = 'lazy';
          resumed.src = '/resumed.svg';
          document.body.append(resumed);
          queueMicrotask(() => { resumed.loading = 'eager'; });
        </script>"#,
        "",
    )
    .await?;
    let (_browser, mut page) = server.open_page(moli_core::LayoutPolicy::Realtime).await?;
    let mut requests = server
        .wait_for_images(&[
            "/blocker.svg",
            "/detached.svg",
            "/connected.svg",
            "/replacement.svg",
            "/resumed.svg",
        ])
        .await?;

    finish_image(requests.remove("/blocker.svg").unwrap(), true).await?;
    assert_eq!(evaluate(&mut page, "blockerLoaded").await?, json!(true));
    assert_eq!(
        evaluate(&mut page, "[document.readyState, events]").await?,
        json!(["interactive", ["blocker"]])
    );

    finish_image(requests.remove("/detached.svg").unwrap(), true).await?;
    assert_eq!(evaluate(&mut page, "detachedLoaded").await?, json!(true));
    finish_image(requests.remove("/connected.svg").unwrap(), true).await?;
    assert_eq!(evaluate(&mut page, "connectedLoaded").await?, json!(true));
    finish_image(requests.remove("/replacement.svg").unwrap(), true).await?;
    assert_eq!(evaluate(&mut page, "replacementLoaded").await?, json!(true));
    assert_eq!(
        evaluate(&mut page, "windowLoaded").await?,
        json!(["blocker", "detached", "connected", "replacement", "window"])
    );

    finish_image(requests.remove("/resumed.svg").unwrap(), true).await?;
    assert_eq!(evaluate(&mut page, "resumedLoaded").await?, json!(true));
    Ok(())
}

#[tokio::test]
async fn script_disabled_document_load_waits_for_offscreen_lazy_image() -> Result<()> {
    let mut server = ImageServer::spawn(
        r#"<!doctype html>
        <script>
          let reportFrameLoad;
          const frameLoaded = new Promise(resolve => reportFrameLoad = resolve);
          function onFrameLoad(frame) {
            const doc = frame.contentDocument;
            const image = doc.getElementById('lazy');
            if (image) reportFrameLoad([doc.readyState, image.complete, image.naturalWidth]);
          }
        </script>
        <iframe id="child" sandbox="allow-same-origin" src="/child"
                onload="onFrameLoad(this)"></iframe>"#,
        r#"<!doctype html><div style="height:8000px"></div>
        <img id="lazy" width="4" height="4" loading="lazy" src="/child.svg">"#,
    )
    .await?;
    let (_browser, mut page) = server.open_page(moli_core::LayoutPolicy::Realtime).await?;
    let mut requests = server.wait_for_images(&["/child.svg"]).await?;

    let before = evaluate(
        &mut page,
        r#"(async () => {
          const doc = document.getElementById('child').contentDocument;
          if (doc.readyState === 'loading') {
            await new Promise(resolve => doc.addEventListener('DOMContentLoaded', resolve, {once: true}));
          }
          return [doc.readyState, doc.getElementById('lazy').complete];
        })()"#,
    )
    .await?;
    assert_eq!(before, json!(["interactive", false]));
    finish_image(requests.remove("/child.svg").unwrap(), true).await?;
    assert_eq!(
        evaluate(&mut page, "frameLoaded").await?,
        json!(["complete", true, 4])
    );
    Ok(())
}

#[tokio::test]
async fn scroll_into_view_during_image_update_starts_lazy_request() -> Result<()> {
    let mut server = ImageServer::spawn(
        r#"<!doctype html>
        <div style="height:8000px"></div>
        <img id="image" loading="lazy" width="4" height="4">
        <div style="height:1000px"></div>"#,
        "",
    )
    .await?;
    let (_browser, mut page) = server.open_page(moli_core::LayoutPolicy::Lazy).await?;
    page.publish_layout_async().await?;

    assert_eq!(
        evaluate(
            &mut page,
            r#"
            const lazyImage = document.getElementById('image');
            const imageLoaded = new Promise(resolve => {
              lazyImage.onload = () => resolve(lazyImage.naturalWidth);
            });
            lazyImage.src = '/revealed.svg';
            lazyImage.scrollIntoView();
            [scrollY > 0, lazyImage.loading]
            "#,
        )
        .await?,
        json!([true, "lazy"])
    );
    let mut requests = server.wait_for_images(&["/revealed.svg"]).await?;
    finish_image(requests.remove("/revealed.svg").unwrap(), true).await?;
    assert_eq!(evaluate(&mut page, "imageLoaded").await?, json!(4));
    Ok(())
}

async fn evaluate(page: &mut Page, expression: &str) -> Result<Value> {
    let reply = tokio::time::timeout(
        Duration::from_secs(5),
        page.evaluate_runtime_expression_by_value_async(expression),
    )
    .await??;
    reply
        .get("value")
        .cloned()
        .with_context(|| format!("expected a value from the page, got {reply}"))
}

struct ImageServer {
    url: String,
    requests: mpsc::UnboundedReceiver<(String, TcpStream)>,
    task: JoinHandle<Result<()>>,
}

impl ImageServer {
    async fn spawn(page: &'static str, child: &'static str) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/page", listener.local_addr()?);
        let (sender, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await?;
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                if stream.read_line(&mut line).await? == 0 {
                    continue;
                }
                let path = line
                    .split_ascii_whitespace()
                    .nth(1)
                    .context("request path")?
                    .to_owned();
                loop {
                    line.clear();
                    if stream.read_line(&mut line).await? == 0 || line == "\r\n" {
                        break;
                    }
                }
                let stream = stream.into_inner();
                match path.as_str() {
                    "/page" => respond(stream, "200 OK", "text/html", page).await?,
                    "/child" => respond(stream, "200 OK", "text/html", child).await?,
                    _ if path.ends_with(".svg") => sender.send((path, stream))?,
                    _ => respond(stream, "404 Not Found", "text/plain", "").await?,
                }
            }
        });
        Ok(Self {
            url,
            requests,
            task,
        })
    }

    async fn open_page(&self, layout_policy: moli_core::LayoutPolicy) -> Result<(Browser, Page)> {
        let browser = Browser::new(
            AppConfig::default()
                .with_image_fetch_enabled(true)
                .with_layout_policy(layout_policy),
        )?;
        let page = browser
            .fetch_with_wait_until(
                &self.url,
                RenderedDomWaitUntil::DomContentLoaded,
                Duration::from_secs(5),
            )
            .await?;
        Ok((browser, page))
    }

    async fn wait_for_images(&mut self, paths: &[&str]) -> Result<HashMap<String, TcpStream>> {
        let mut requests = HashMap::new();
        tokio::time::timeout(Duration::from_secs(5), async {
            for _ in paths {
                let (path, stream) = self.requests.recv().await.context("image request")?;
                assert!(paths.contains(&path.as_str()), "unexpected request {path}");
                assert!(
                    requests.insert(path, stream).is_none(),
                    "duplicate image request"
                );
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .with_context(|| format!("waiting for {paths:?}, received {:?}", requests.keys()))??;
        Ok(requests)
    }
}

impl Drop for ImageServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn finish_image(stream: TcpStream, success: bool) -> Result<()> {
    if success {
        respond(
            stream,
            "200 OK",
            "image/svg+xml",
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4"/></svg>"#,
        )
        .await
    } else {
        respond(stream, "200 OK", "image/svg+xml", "invalid image").await
    }
}

async fn respond(
    mut stream: TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}
