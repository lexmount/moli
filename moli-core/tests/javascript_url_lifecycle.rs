use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig};
use moli_test_support::FixtureServer;
use serde_json::{Value, json};
use tokio::time::Duration;
use url::Url;

async fn child_navigation_lifecycle(via: &str, kind: &str, depth: usize) -> Result<Value> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let markup = format!(
        r#"<!doctype html><body><script>
          window.events = [];
          window.staleTimerRan = false;
          window.unloadNavigationRan = false;
          window.finished = (async () => {{
            async function makeFrame(owner, label) {{
              const frame = owner.document.createElement('iframe');
              const loaded = new Promise(resolve => frame.onload = resolve);
              frame.src = '/compat/child-dynamic-markup-document?markup=' +
                encodeURIComponent('<!doctype html><body>original');
              owner.document.body.append(frame);
              await loaded;
              const win = frame.contentWindow;
              for (const type of ['beforeunload', 'pagehide', 'unload'])
                win.addEventListener(type, () => events.push(label + ':' + type));
              win.document.addEventListener('visibilitychange', () =>
                events.push(label + ':visibilitychange'));
              win.addEventListener('unload', () => {{
                win.setTimeout(() => staleTimerRan = true, 0);
                win.location.href = 'javascript:top.unloadNavigationRan = true; void 0';
              }});
              return frame;
            }}
            const frame = await makeFrame(window, 'target');
            let owner = frame.contentWindow;
            for (let i = 0; i < {depth}; ++i)
              owner = (await makeFrame(owner, 'descendant-' + i)).contentWindow;
            const unrelated = await makeFrame(window, 'unrelated');
            const oldDocument = frame.contentDocument;
            const unrelatedDocument = unrelated.contentDocument;
            let resolveDone;
            const done = new Promise(resolve => resolveDone = resolve);
            let loads = 0;
            frame.onload = () => {{ loads++; resolveDone(); }};
            window.nonStringDone = () => setTimeout(resolveDone, 0);
            const kind = {kind:?};
            const url = kind === 'string' ? 'javascript:"<body>replacement"' :
              kind === 'undefined' ? 'javascript:top.nonStringDone(); void 0' :
              '/compat/child-dynamic-markup-document?markup=' +
                encodeURIComponent('<!doctype html><body>network');
            const via = {via:?};
            if (via === 'location') frame.contentWindow.location.href = url;
            else if (via === 'src') frame.src = url;
            else {{
              const anchor = frame.contentDocument.createElement('a');
              anchor.href = url;
              frame.contentDocument.body.append(anchor);
              anchor.click();
            }}
            await done;
            await new Promise(resolve => setTimeout(resolve, 0));
            return {{events, staleTimerRan, unloadNavigationRan, loads,
              sameDocument: frame.contentDocument === oldDocument,
              unrelatedUnchanged: unrelated.contentDocument === unrelatedDocument,
              text: frame.contentDocument.body.textContent,
              children: frame.contentWindow.length}};
          }})();
        </script>"#
    );
    let mut url = Url::parse(&server.url("/compat/child-dynamic-markup-document"))?;
    url.query_pairs_mut().append_pair("markup", &markup);
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let mut page = browser.fetch(url.as_str()).await?;
        page.evaluate_runtime_expression_with_await_async(
            "finished.then(value => JSON.stringify(value))",
            true,
        )
        .await
    })
    .await??;
    let result: Value = serde_json::from_str(result["value"].as_str().unwrap())?;
    server.shutdown().await;
    assert_eq!(result["unrelatedUnchanged"], true, "{via}/{kind}: {result}");
    assert_eq!(result["staleTimerRan"], false, "{via}/{kind}: {result}");
    assert_eq!(
        result["unloadNavigationRan"], false,
        "{via}/{kind}: {result}"
    );
    Ok(result)
}

#[tokio::test(flavor = "multi_thread")]
async fn child_javascript_url_string_unloads_without_beforeunload() -> Result<()> {
    for via in ["location", "src", "anchor"] {
        for depth in [0, 2] {
            let result = child_navigation_lifecycle(via, "string", depth).await?;
            assert_eq!(result["sameDocument"], false);
            assert_eq!(result["loads"], 1);
            assert_eq!(result["text"], "replacement");
            assert_eq!(result["children"], 0);
            let events = result["events"].as_array().unwrap();
            let labels = std::iter::once("target".to_owned())
                .chain((0..depth).map(|index| format!("descendant-{index}")));
            for label in labels {
                let actual: Vec<_> = events
                    .iter()
                    .filter(|event| event.as_str().unwrap().starts_with(&format!("{label}:")))
                    .cloned()
                    .collect();
                assert_eq!(
                    actual,
                    vec![
                        json!(format!("{label}:pagehide")),
                        json!(format!("{label}:visibilitychange")),
                        json!(format!("{label}:unload")),
                    ],
                    "{via}/{depth}: {result}"
                );
            }
            assert_eq!(events.len(), 3 * (depth + 1), "{via}/{depth}: {result}");
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_javascript_url_non_string_preserves_document_and_descendants() -> Result<()> {
    for via in ["location", "src", "anchor"] {
        let result = child_navigation_lifecycle(via, "undefined", 2).await?;
        assert_eq!(result["sameDocument"], true);
        assert_eq!(result["loads"], 0);
        assert_eq!(result["events"], json!([]));
        assert_eq!(result["children"], 1);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn ordinary_child_navigation_still_dispatches_beforeunload() -> Result<()> {
    for via in ["location", "src", "anchor"] {
        let result = child_navigation_lifecycle(via, "network", 0).await?;
        assert_eq!(result["sameDocument"], false);
        assert_eq!(result["loads"], 1);
        assert_eq!(result["text"], "network");
        let events: Vec<_> = result["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| *event != "target:visibilitychange")
            .cloned()
            .collect();
        assert_eq!(
            events,
            vec![
                json!("target:beforeunload"),
                json!("target:pagehide"),
                json!("target:unload")
            ],
            "{via}: {result}"
        );
    }
    Ok(())
}
