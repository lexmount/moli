use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig};
use moli_test_support::FixtureServer;
use tokio::time::Duration;
use url::Url;

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

#[tokio::test(flavor = "multi_thread")]
async fn top_level_back_skips_replaced_initial_child_entries() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    for (replace, blank_predecessor) in [(false, false), (true, false), (true, true)] {
        let browser = Browser::new(BrowserConfig::default())?;
        let destination = server.url("/static?replacement");
        let source = if replace {
            markup_url(
                &server,
                &format!(
                    "<!doctype html><script>location.replace({});</script>",
                    serde_json::to_string(&destination)?
                ),
            )
        } else {
            destination.clone()
        };
        let previous = if blank_predecessor {
            "about:blank".to_owned()
        } else {
            server.url("/static?sentinel-start")
        };
        let markup = format!(
            r#"<!doctype html><body><script>
              function loaded(frame, target) {{
                return new Promise(resolve => {{
                  const listener = () => {{
                    if (frame.contentWindow.location.href === new URL(target, location.href).href) {{
                      frame.removeEventListener('load', listener);
                      setTimeout(resolve, 0);
                    }}
                  }};
                  frame.addEventListener('load', listener);
                }});
              }}
              window.finished = (async () => {{
                const sentinel = document.createElement('iframe');
                let pending = loaded(sentinel, '/static?sentinel-start');
                sentinel.src = '/static?sentinel-start';
                document.body.append(sentinel);
                await pending;
                const previous = {};
                if (previous === 'about:blank') {{
                  pending = loaded(sentinel, previous);
                  sentinel.src = previous;
                  await pending;
                }}
                pending = loaded(sentinel, '/static?sentinel-end');
                sentinel.src = '/static?sentinel-end';
                await pending;
                const initialLength = history.length;
                const other = document.createElement('iframe');
                pending = loaded(other, {});
                other.src = {};
                document.body.append(other);
                await pending;
                pending = loaded(sentinel, previous);
                history.back();
                await pending;
                return {{
                  unchangedLength: history.length === initialLength,
                  sentinelURL: sentinel.contentWindow.location.href,
                  otherURL: other.contentWindow.location.href,
                  sentinelIndex: sentinel.contentWindow.navigation.currentEntry.index,
                  otherIndex: other.contentWindow.navigation.currentEntry.index
                }};
              }})();
            </script>"#,
            serde_json::to_string(&previous)?,
            serde_json::to_string(&destination)?,
            serde_json::to_string(&source)?.replace("</script>", "<\\/script>")
        );
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            let mut page = browser.fetch(&markup_url(&server, &markup)).await?;
            page.evaluate_runtime_expression_with_await_async(
                "finished.then(value => JSON.stringify(value))",
                true,
            )
            .await
        })
        .await??;
        let result: serde_json::Value = serde_json::from_str(
            result["value"]
                .as_str()
                .expect("joint history traversal result"),
        )?;
        assert_eq!(
            result,
            serde_json::json!({
                "unchangedLength": true,
                "sentinelURL": previous,
                "otherURL": destination,
                "sentinelIndex": u32::from(blank_predecessor),
                "otherIndex": 0
            }),
            "replace={replace}, blank_predecessor={blank_predecessor}"
        );
    }
    server.shutdown().await;
    Ok(())
}
