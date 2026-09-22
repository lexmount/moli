use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig};
use moli_test_support::FixtureServer;
use serde_json::{Value, json};
use tokio::time::Duration;
use url::Url;

async fn iframe_attribute_history(phase: &str, api: &str, attribute: &str) -> Result<Value> {
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
      const attribute = __ATTRIBUTE__;
      const initialLength = history.length;
      const frame = document.createElement('iframe');
      const destination = new URL('/compat/child-dynamic-markup-document?label=destination&markup=' +
        encodeURIComponent('<!doctype html><body>destination'), location.href).href;
      window.navigateFrame = () => {
        window.navigationReadiness = frame.contentDocument.readyState;
        const value = attribute === 'srcdoc' ? '<!doctype html><body>destination' : destination;
        if (api === 'property') frame[attribute] = value;
        else frame.setAttribute(attribute, value);
      };
      frame.onload = () => {
        if (frame.contentWindow.location.href === (attribute === 'srcdoc' ? 'about:srcdoc' : destination)) setTimeout(() => {
          const win = frame.contentWindow;
          finish({delta: history.length - initialLength, readiness: navigationReadiness,
            entries: win.navigation.entries().map(entry => entry.url === 'about:srcdoc' ? 'srcdoc' : new URL(entry.url).searchParams.get('label')),
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
        .replace("__ATTRIBUTE__", &serde_json::to_string(attribute)?)
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
            let result = iframe_attribute_history(phase, api, "src").await?;
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
            let result = iframe_attribute_history(phase, api, "src").await?;
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

#[tokio::test(flavor = "multi_thread")]
async fn iframe_srcdoc_history_uses_load_state_when_the_attribute_changes() -> Result<()> {
    for api in ["property", "setAttribute"] {
        for phase in [
            "parser",
            "DOMContentLoaded",
            "load",
            "pageshow",
            "owner-load",
            "after-load",
            "reopen",
        ] {
            let result = iframe_attribute_history(phase, api, "srcdoc").await?;
            let pushes = matches!(phase, "after-load" | "reopen");
            let readiness = match phase {
                "parser" | "reopen" => "loading",
                "DOMContentLoaded" => "interactive",
                _ => "complete",
            };
            assert_eq!(
                result,
                json!({
                    "delta": u32::from(pushes), "readiness": readiness,
                    "entries": if pushes { vec!["source", "srcdoc"] } else { vec!["srcdoc"] },
                    "index": u32::from(pushes), "activation": if pushes { "push" } else { "replace" }
                }),
                "{phase}/{api}"
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_srcdoc_commits_share_history_with_their_top_or_popup_owner() -> Result<()> {
    let fixture = include_str!("fixtures/iframe-srcdoc-history.js");
    let server = FixtureServer::spawn().await?;
    for mode in ["top", "popup"] {
        for api in ["property", "setAttribute"] {
            let browser = Browser::new(BrowserConfig::default())?;
            let markup = format!(
                r#"<!doctype html><body><script>{fixture}
                window.finished = (async () => {{
                  const root = {mode:?} === 'popup' ? open('/static?srcdoc-popup') : window;
                  if (root !== window) await new Promise(resolve => root.addEventListener('load', resolve, {{once:true}}));
                  const openerLength = history.length;
                  try {{
                    const result = await iframeSrcdocHistory(root, {api:?}, new URL('/static?srcdoc-original', location.href).href);
                    if (root !== window && history.length !== openerLength) throw new Error('popup changed opener history');
                    return result;
                  }} finally {{ if (root !== window) root.close(); }}
                }})();</script>"#
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
            let result: Value =
                serde_json::from_str(result["value"].as_str().expect("srcdoc history result"))?;
            assert_eq!(
                result["fragment"],
                json!({"delta": 2, "childDelta": 2, "index": 2, "url": "about:srcdoc#part"}),
                "{mode}/{api}: {result}"
            );
            let expected = [
                (
                    "same-url-src",
                    0,
                    0,
                    "replace",
                    true,
                    Value::Null,
                    vec!["ordinary"],
                ),
                (
                    "first-srcdoc",
                    1,
                    1,
                    "push",
                    false,
                    json!("one"),
                    vec!["ordinary", "about:srcdoc"],
                ),
                (
                    "same-url-srcdoc",
                    1,
                    1,
                    "replace",
                    true,
                    json!("two"),
                    vec!["ordinary", "about:srcdoc"],
                ),
                (
                    "after-fragment",
                    3,
                    3,
                    "push",
                    false,
                    json!("three"),
                    vec![
                        "ordinary",
                        "about:srcdoc",
                        "about:srcdoc#part",
                        "about:srcdoc",
                    ],
                ),
            ];
            let steps = result["steps"].as_array().expect("srcdoc steps");
            assert_eq!(steps.len(), expected.len(), "{mode}/{api}: {result}");
            for (step, (label, delta, index, kind, same_key, text, entries)) in
                steps.iter().zip(expected)
            {
                assert_eq!(
                    step,
                    &json!({
                        "label": label, "delta": delta, "childDelta": delta, "index": index,
                        "type": kind, "sameKey": same_key, "newId": true, "text": text, "entries": entries,
                        "during": {"sameDocument": true, "sameEntry": true, "delta": 0}
                    }),
                    "{mode}/{api}: {result}"
                );
            }
        }
    }
    server.shutdown().await;
    Ok(())
}
