use super::*;

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

async fn child_navigation_history(phase: &str, api: &str, replaces: bool) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let destination = server.url("/static?location-destination");
    let source = format!(
        r#"<!doctype html><body><script>
          function go() {{
            navigation.addEventListener('navigate', e => parent.navigationType = e.navigationType);
            const target = {};
            if ({} === 'window-open') window.open(target, '_self');
            else location.assign(target);
          }}
          const phase = {};
          if (phase === 'parser') go();
          else if (phase === 'DOMContentLoaded') document.addEventListener(phase, go, {{once: true}});
          else if (phase === 'load' || phase === 'pageshow') addEventListener(phase, go, {{once: true}});
          else addEventListener('load', () => setTimeout(() => {{
            if (phase === 'reopen') document.open();
            go();
          }}, 0), {{once: true}});
        </script>"#,
        serde_json::to_string(&destination)?,
        serde_json::to_string(api)?,
        serde_json::to_string(phase)?
    );
    let parent = format!(
        r#"<!doctype html><body><script>
          window.finished = new Promise(resolve => window.finish = resolve);
          const initialLength = history.length;
          const frame = document.createElement('iframe');
          frame.onload = () => {{
            if (frame.contentWindow.location.href === {}) setTimeout(() => finish({{
              delta: history.length - initialLength,
              navigationType,
              childEntryIndex: frame.contentWindow.navigation.currentEntry.index
            }}), 0);
          }};
          frame.src = {};
          document.body.append(frame);
        </script>"#,
        serde_json::to_string(&destination)?,
        serde_json::to_string(&markup_url(&server, &source))?.replace("</script>", "<\\/script>")
    );
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let mut page = browser.fetch(&markup_url(&server, &parent)).await?;
        page.evaluate_runtime_expression_with_await_async(
            "finished.then(value => JSON.stringify(value))",
            true,
        )
        .await
    })
    .await??;
    let result: serde_json::Value =
        serde_json::from_str(result["value"].as_str().expect("navigation history result"))?;
    let expected_delta = if replaces { 0 } else { 1 };
    assert_eq!(result["delta"], expected_delta, "phase={phase}, {result}");
    assert_eq!(result["childEntryIndex"], expected_delta, "phase={phase}");
    assert_eq!(
        result["navigationType"],
        if replaces { "replace" } else { "push" },
        "phase={phase}"
    );
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_location_navigation_replaces_until_load_callbacks_finish() -> Result<()> {
    for phase in ["parser", "DOMContentLoaded", "load", "pageshow"] {
        child_navigation_history(phase, "location", true).await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_location_navigation_pushes_after_complete_even_after_document_open() -> Result<()> {
    for phase in ["after-load", "reopen"] {
        child_navigation_history(phase, "location", false).await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_window_open_self_before_load_keeps_push_history() -> Result<()> {
    for phase in ["parser", "DOMContentLoaded", "load", "pageshow"] {
        child_navigation_history(phase, "window-open", false).await?;
    }
    Ok(())
}
