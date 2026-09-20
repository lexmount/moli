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
            parent.navigationReadiness = document.readyState;
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
              navigationType, navigationReadiness,
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
    assert_eq!(
        result["navigationReadiness"],
        match phase {
            "parser" | "reopen" => "loading",
            "DOMContentLoaded" => "interactive",
            _ => "complete",
        },
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

#[tokio::test(flavor = "multi_thread")]
async fn main_location_before_load_exposes_replace_navigation_events() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    for phase in ["parser", "DOMContentLoaded", "load", "pageshow"] {
        let browser = Browser::new(AppConfig::default())?;
        let source = format!(
            r#"<!doctype html><script>
              const startingLength = history.length;
              navigation.addEventListener('navigate', e => {{
                window.navigationType = e.navigationType;
                e.preventDefault();
              }});
              function go() {{
                location.assign('/destination');
                window.historyResult = JSON.stringify({{
                  navigationType, delta: history.length - startingLength,
                  readiness: document.readyState
                }});
              }}
              const phase = {};
              if (phase === 'parser') go();
              else if (phase === 'DOMContentLoaded') document.addEventListener(phase, go);
              else addEventListener(phase, go);
            </script>"#,
            serde_json::to_string(phase)?
        );
        // Observe the real main-document lifecycle without handing the Page
        // off to another navigation: Browser::fetch owns only this document.
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            let mut page = browser.fetch(&markup_url(&server, &source)).await?;
            page.evaluate_runtime_expression_with_await_async("historyResult", true)
                .await
        })
        .await??;
        let result: serde_json::Value = serde_json::from_str(
            result["value"]
                .as_str()
                .expect("main navigation history result"),
        )?;
        let readiness = match phase {
            "parser" => "loading",
            "DOMContentLoaded" => "interactive",
            _ => "complete",
        };
        assert_eq!(
            result,
            serde_json::json!({"delta": 0, "navigationType": "replace", "readiness": readiness}),
            "phase={phase}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_location_navigation_pushes_after_load_and_document_open() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    for phase in ["after-load", "reopen", "load-rewrite", "pageshow-rewrite"] {
        let browser = Browser::new(AppConfig::default())?;
        let source = format!(
            r#"<!doctype html><body><script>
              window.finished = new Promise(resolve => window.finish = resolve);
              function go() {{
                navigation.addEventListener('navigate', e => {{
                  finish({{type: e.navigationType, readiness: document.readyState}});
                  e.preventDefault();
                }}, {{once: true}});
                location.assign('/destination');
              }}
              const phase = {};
              if (phase === 'after-load' || phase === 'reopen') {{
                addEventListener('load', () => setTimeout(() => {{
                  if (phase === 'reopen') document.open();
                  go();
                }}, 0), {{once: true}});
              }} else {{
                const event = phase === 'load-rewrite' ? 'load' : 'pageshow';
                addEventListener(event, () => {{
                  document.open();
                  document.write('<!doctype html><body>rewritten');
                  document.close();
                  setTimeout(go, 0);
                }}, {{once: true}});
              }}
            </script>"#,
            serde_json::to_string(phase)?
        );
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            let mut page = browser.fetch(&markup_url(&server, &source)).await?;
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
                .expect("main post-load navigation result"),
        )?;
        assert_eq!(result["type"], "push", "phase={phase}, {result}");
        assert_eq!(
            result["readiness"],
            if phase == "reopen" {
                "loading"
            } else {
                "complete"
            },
            "phase={phase}"
        );
    }
    server.shutdown().await;
    Ok(())
}

async fn popup_navigation_history(phase: &str, api: &str, replaces: bool) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let destination = markup_url(
        &server,
        "<!doctype html><script>opener.finish({before:opener.beforePopupNavigation,after:history.length});</script>",
    );
    let source = format!(
        r#"<!doctype html><body><script>
          function go() {{
            opener.beforePopupNavigation = history.length;
            const target = {};
            const api = {};
            if (api === 'href') location.href = target;
            else if (api === 'document') document.location = target;
            else if (api === 'window-open') window.open(target, '_self');
            else location.assign(target);
          }}
          const phase = {};
          if (phase === 'parser') go();
          else if (phase === 'load') addEventListener('load', go, {{once:true}});
          else addEventListener('load', () => setTimeout(() => {{
            if (phase === 'reopen') document.open();
            go();
          }}, 0), {{once:true}});
        </script>"#,
        serde_json::to_string(&destination)?.replace("</script>", "<\\/script>"),
        serde_json::to_string(api)?,
        serde_json::to_string(phase)?
    );
    let parent = format!(
        r#"<!doctype html><script>
          window.finished = new Promise(resolve => window.finish = resolve);
          window.popup = open({});
        </script>"#,
        serde_json::to_string(&markup_url(&server, &source))?.replace("</script>", "<\\/script>")
    );
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let mut page = browser.fetch(&markup_url(&server, &parent)).await?;
        let result = page
            .evaluate_runtime_expression_with_await_async(
                "finished.then(value => JSON.stringify(value))",
                true,
            )
            .await?;
        page.evaluate_runtime_expression_with_await_async("popup.close()", false)
            .await?;
        Ok::<_, anyhow::Error>(result)
    })
    .await??;
    let result: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .expect("popup navigation history result"),
    )?;
    assert_eq!(
        result["after"].as_u64().unwrap() - result["before"].as_u64().unwrap(),
        u64::from(!replaces),
        "phase={phase}, api={api}, {result}"
    );
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_location_navigation_replaces_until_load_callbacks_finish() -> Result<()> {
    for phase in ["parser", "load"] {
        for api in ["href", "assign", "document"] {
            popup_navigation_history(phase, api, true).await?;
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_location_navigation_pushes_after_complete_even_after_document_open() -> Result<()> {
    for phase in ["after-load", "reopen"] {
        for api in ["href", "assign", "document"] {
            popup_navigation_history(phase, api, false).await?;
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_window_open_self_before_load_keeps_push_history() -> Result<()> {
    for phase in ["parser", "load"] {
        popup_navigation_history(phase, "window-open", false).await?;
    }
    Ok(())
}

async fn popup_fragment_history(phase: &str, api: &str, mode: &str) -> Result<serde_json::Value> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let script = include_str!("popup_fragment_navigation.js")
        .replace("PHASE", &serde_json::to_string(phase)?)
        .replace("API", &serde_json::to_string(api)?)
        .replace("MODE", &serde_json::to_string(mode)?);
    let source = markup_url(
        &server,
        &format!("<!doctype html><body><script>{script}</script>"),
    );
    let parent = format!(
        r#"<!doctype html><script>
          window.finished = new Promise(resolve => window.finish = resolve);
          window.parentEvents = [];
          addEventListener('popstate', e => parentEvents.push(e.type));
          addEventListener('hashchange', e => parentEvents.push(e.type));
          window.popup = open({});
        </script>"#,
        serde_json::to_string(&source)?.replace("</script>", "<\\/script>")
    );
    let parent_url = markup_url(&server, &parent);
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let mut page = browser.fetch(&parent_url).await?;
        let result = page.evaluate_runtime_expression_with_await_async(
            "finished.then(result => JSON.stringify(result))", true,
        ).await?;
        let parent_result = page.evaluate_runtime_expression_with_await_async(
            "JSON.stringify({events:parentEvents, href:location.href, documentURL:document.URL})", false,
        ).await?;
        let parent_result: serde_json::Value = serde_json::from_str(parent_result["value"].as_str().unwrap())?;
        assert_eq!(parent_result, serde_json::json!({"events": [], "href": parent_url, "documentURL": parent_url}));
        page.evaluate_runtime_expression_with_await_async("popup.close()", false).await?;
        Ok::<_, anyhow::Error>(result)
    }).await??;
    server.shutdown().await;
    Ok(serde_json::from_str(
        result["value"].as_str().expect("popup fragment result"),
    )?)
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_fragment_navigation_updates_history_and_preserves_document() -> Result<()> {
    for phase in ["parser", "load", "after-load", "reopen"] {
        for api in ["hash", "assign", "replace", "open"] {
            let result = popup_fragment_history(phase, api, "normal").await?;
            let pushes =
                api != "replace" && (api == "open" || matches!(phase, "after-load" | "reopen"));
            let index = u64::from(pushes);
            let kind = if pushes { "push" } else { "replace" };
            let synchronous_events = serde_json::json!([
                format!("navigate:{kind}:true"),
                format!("currententrychange:{kind}"),
                "popstate:null:true"
            ]);
            assert_eq!(result["delta"], index, "phase={phase}, api={api}, {result}");
            assert_eq!(result["index"], index);
            for field in ["sameDocument", "documentURL", "currentURL"] {
                assert_eq!(result[field], true, "field={field}, {result}");
            }
            assert_eq!(result["hash"], "#fragment");
            assert_eq!(result["state"], serde_json::Value::Null);
            assert_eq!(
                result["navigationState"],
                serde_json::json!({"navigation": 1})
            );
            assert_eq!(result["sameKey"], !pushes);
            assert_eq!(result["sameId"], false);
            let entries: Vec<_> = (0..=index)
                .map(|index| serde_json::json!({"index": index, "sameDocument": true}))
                .collect();
            assert_eq!(result["entries"], serde_json::json!(entries));
            assert_eq!(result["synchronousEvents"], synchronous_events);
            assert_eq!(
                result["events"],
                serde_json::json!([
                    format!("navigate:{kind}:true"),
                    format!("currententrychange:{kind}"),
                    "popstate:null:true",
                    "hashchange::#fragment:true"
                ])
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_fragment_navigation_can_be_canceled_before_history_changes() -> Result<()> {
    let result = popup_fragment_history("after-load", "hash", "cancel").await?;
    assert_eq!(result["delta"], 0);
    assert_eq!(result["hash"], "");
    assert_eq!(result["sameId"], true);
    assert_eq!(result["state"], serde_json::json!({"classic": 1}));
    assert_eq!(result["events"], serde_json::json!(["navigate:push:true"]));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_fragment_navigation_stops_when_navigate_listener_closes_window() -> Result<()> {
    let result = popup_fragment_history("after-load", "hash", "close").await?;
    assert_eq!(
        result,
        serde_json::json!({"closed": true, "events": ["navigate:push:true"]})
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_location_same_url_reloads_without_pushing_history() -> Result<()> {
    let result = popup_fragment_history("after-load", "assign", "repeat").await?;
    assert_eq!(
        result,
        serde_json::json!({
            "loads": 2, "delta": 0, "documentChanged": true, "entryIndex": 0
        })
    );
    Ok(())
}
