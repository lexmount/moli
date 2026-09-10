use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig};
use moli_test_support::FixtureServer;
use serde_json::{Value, json};
use tokio::time::Duration;
use url::Url;

async fn iframe_attribute_history(phase: &str, api: &str) -> Result<Value> {
    let child = r#"<!doctype html><body><script>
      const phase = new URL(location.href).searchParams.get('phase');
      const go = () => parent.navigateFrame();
      if (phase === 'parser') go();
      else if (phase === 'DOMContentLoaded') document.addEventListener(phase, go, {once: true});
      else if (phase === 'load' || phase === 'pageshow') addEventListener(phase, go, {once: true});
      else if (phase !== 'owner-load') addEventListener('load', () => setTimeout(() => {
        if (phase === 'reopen') document.open();
        go();
      }, 0), {once: true});
    </script>"#;
    let child = serde_json::to_string(child)?.replace("</script>", "<\\/script>");
    let markup = r#"<!doctype html><body><script>
      window.finished = new Promise(resolve => window.finish = resolve);
      const phase = __PHASE__;
      const api = __API__;
      const initialLength = history.length;
      const frame = document.createElement('iframe');
      const destination = new URL('/compat/child-dynamic-markup-document?label=destination&markup=' +
        encodeURIComponent('<!doctype html><body>destination'), location.href).href;
      window.navigateFrame = () => {
        window.navigationReadiness = frame.contentDocument.readyState;
        if (api === 'property') frame.src = destination;
        else frame.setAttribute('src', destination);
      };
      frame.onload = () => {
        if (frame.contentWindow.location.href === destination) setTimeout(() => {
          const win = frame.contentWindow;
          finish({delta: history.length - initialLength, readiness: navigationReadiness,
            entries: win.navigation.entries().map(entry => new URL(entry.url).searchParams.get('label')),
            index: win.navigation.currentEntry.index, activation: win.navigation.activation.navigationType});
        }, 0);
        else if (phase === 'owner-load') navigateFrame();
      };
      frame.src = '/compat/child-dynamic-markup-document?label=source&phase=' + phase +
        '&markup=' + encodeURIComponent(__CHILD__);
      document.body.append(frame);
    </script>"#
        .replace("__PHASE__", &serde_json::to_string(phase)?)
        .replace("__API__", &serde_json::to_string(api)?)
        .replace("__CHILD__", &child);
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
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
    let result = serde_json::from_str(result["value"].as_str().unwrap())?;
    server.shutdown().await;
    Ok(result)
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_src_changes_replace_history_until_child_load_finishes() -> Result<()> {
    for api in ["property", "setAttribute"] {
        for phase in [
            "parser",
            "DOMContentLoaded",
            "load",
            "pageshow",
            "owner-load",
        ] {
            let result = iframe_attribute_history(phase, api).await?;
            let readiness = match phase {
                "parser" => "loading",
                "DOMContentLoaded" => "interactive",
                _ => "complete",
            };
            assert_eq!(
                result,
                json!({"delta": 0, "readiness": readiness, "entries": ["destination"],
                    "index": 0, "activation": "replace"}),
                "{phase}/{api}"
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_src_changes_push_history_after_load_even_when_document_is_reopened() -> Result<()> {
    for api in ["property", "setAttribute"] {
        for phase in ["after-load", "reopen"] {
            let result = iframe_attribute_history(phase, api).await?;
            assert_eq!(
                result,
                json!({"delta": 1, "readiness": if phase == "reopen" { "loading" } else { "complete" },
                    "entries": ["source", "destination"], "index": 1, "activation": "push"}),
                "{phase}/{api}"
            );
        }
    }
    Ok(())
}
