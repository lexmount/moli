use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig};
use moli_test_support::FixtureServer;
use serde_json::{Value, json};
use tokio::time::Duration;
use url::Url;

async fn history_visibility(flow: &str, api: &str, depth: usize) -> Result<Value> {
    let child = r#"<!doctype html><body><p>child</p><script>
      const label = new URL(location.href).searchParams.get('label');
      function record(event) {
        top.events.push({label, type: event.type, hidden: document.hidden,
          target: event.target === document ? 'document' : event.target === window ? 'window' : 'other',
          currentWindow: event.currentTarget === window, trusted: event.isTrusted,
          bubbles: event.bubbles, cancelable: event.cancelable});
      }
      for (const type of ['beforeunload', 'pagehide', 'unload', 'load', 'pageshow'])
        addEventListener(type, record);
      for (const type of ['load', 'pageshow', 'pagehide', 'unload'])
        document.addEventListener(type, () => top.documentWindowEventCalls++, true);
      document.addEventListener('visibilitychange', record);
      document.addEventListener('DOMContentLoaded', event => {
        window.savedDOMContentLoaded = event;
      });
      addEventListener('pageshow', event => {
        if (event.isTrusted) {
          top.pageReady(label);
        }
      });
    </script>"#;
    let child = serde_json::to_string(child)?.replace("</script>", "<\\/script>");
    let markup = r#"<!doctype html><body><script>
      window.events = [];
      window.documentWindowEventCalls = 0;
      let onReady = null;
      window.pageReady = label => { if (onReady) onReady(label); };
      window.finished = (async () => {
        const childMarkup = __CHILD__;
        const flow = __FLOW__;
        const api = __API__;
        function childURL(label) {
          return '/compat/child-dynamic-markup-document?label=' + label +
            '&markup=' + encodeURIComponent(childMarkup);
        }
        async function waitForPage(label, action) {
          const ready = new Promise(resolve => {
            onReady = observed => {
              if (observed === label) {
                onReady = null;
                setTimeout(resolve, 0);
              }
            };
          });
          action();
          await ready;
        }
        async function makeFrame(owner, label) {
          const frame = owner.document.createElement('iframe');
          await waitForPage(label, () => {
            frame.src = childURL(label);
            owner.document.body.append(frame);
          });
          return frame;
        }
        const frame = await makeFrame(window, 'A');
        if (flow === 'synthetic') {
          const native = events.slice();
          const win = frame.contentWindow;
          const doc = win.document;
          let documentCalls = 0;
          for (const type of ['load', 'pageshow', 'pagehide', 'unload'])
            doc.addEventListener(type, () => documentCalls++, true);
          win.addEventListener('DOMContentLoaded', win.record);
          const saved = win.savedDOMContentLoaded;
          const completedBeforeRedispatch = saved.eventPhase === 0 && saved.currentTarget === null;
          const wasTrusted = saved.isTrusted;
          events = [];
          for (const type of ['load', 'pagehide', 'unload']) win.dispatchEvent(new Event(type));
          win.dispatchEvent(new PageTransitionEvent('pageshow', {persisted: true}));
          win.dispatchEvent(saved);
          return {native, synthetic: events, documentCalls, documentWindowEventCalls, wasTrusted,
            completedBeforeRedispatch,
            currentTargetCleared: saved.currentTarget === null,
            phase: saved.eventPhase, hidden: doc.hidden};
        }
        if (flow === 'same-document') {
          const win = frame.contentWindow;
          const doc = win.document;
          win.history.pushState({}, '', '#one');
          events = [];
          await new Promise(resolve => {
            win.addEventListener('popstate', () => setTimeout(resolve, 0), {once: true});
            win[api].back();
          });
          return {events, sameDocument: win.document === doc, hidden: doc.hidden};
        }
        await waitForPage('B', () => frame.contentWindow.location.href = childURL('B'));
        let owner = frame.contentWindow;
        for (let i = 0; i < __DEPTH__; ++i)
          owner = (await makeFrame(owner, 'descendant-' + i)).contentWindow;
        const unrelated = await makeFrame(window, 'unrelated');
        const unrelatedDocument = unrelated.contentDocument;
        const beforeBack = frame.contentDocument;
        events = [];
        await waitForPage('A', () => frame.contentWindow[api].back());
        const back = {events: events.slice(), oldHidden: beforeBack.hidden,
          newHidden: frame.contentDocument.hidden, sameDocument: beforeBack === frame.contentDocument};
        const beforeForward = frame.contentDocument;
        events = [];
        await waitForPage('B', () => frame.contentWindow[api].forward());
        const forward = {events: events.slice(), oldHidden: beforeForward.hidden,
          newHidden: frame.contentDocument.hidden, sameDocument: beforeForward === frame.contentDocument};
        return {back, forward, unrelatedUnchanged: unrelated.contentDocument === unrelatedDocument};
      })();
    </script>"#
        .replace("__CHILD__", &child)
        .replace("__FLOW__", &serde_json::to_string(flow)?)
        .replace("__API__", &serde_json::to_string(api)?)
        .replace("__DEPTH__", &depth.to_string());
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
async fn native_window_event_targets_are_distinct_from_script_dispatch() -> Result<()> {
    let result = history_visibility("synthetic", "history", 0).await?;
    let native = result["native"].as_array().unwrap();
    assert_eq!(native.len(), 2, "{result}");
    for (event, kind) in native.iter().zip(["load", "pageshow"]) {
        assert_eq!(event["type"], kind);
        assert_eq!(event["target"], "document", "{result}");
        assert_eq!(event["currentWindow"], true);
        assert_eq!(event["trusted"], true);
    }
    let synthetic = result["synthetic"].as_array().unwrap();
    assert_eq!(synthetic.len(), 5);
    for event in synthetic {
        assert_eq!(event["target"], "window", "{result}");
        assert_eq!(event["currentWindow"], true);
        assert_eq!(event["trusted"], false, "{result}");
    }
    assert_eq!(result["documentCalls"], 0);
    assert_eq!(result["documentWindowEventCalls"], 0);
    assert_eq!(result["wasTrusted"], true);
    assert_eq!(result["completedBeforeRedispatch"], true);
    assert_eq!(result["currentTargetCleared"], true);
    assert_eq!(result["phase"], 0);
    assert_eq!(result["hidden"], false);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn history_traversal_updates_visibility_and_unloads_descendants_once() -> Result<()> {
    for api in ["history", "navigation"] {
        for depth in [0, 2] {
            let result = history_visibility("roundtrip", api, depth).await?;
            assert_eq!(result["unrelatedUnchanged"], true);
            for (phase, old, new, descendant_count) in
                [("back", "B", "A", depth), ("forward", "A", "B", 0)]
            {
                let phase = &result[phase];
                assert_eq!(phase["oldHidden"], true, "{api}/{depth}: {result}");
                assert_eq!(phase["newHidden"], false);
                assert_eq!(phase["sameDocument"], false);
                let events = phase["events"].as_array().unwrap();
                let labels: Vec<_> = std::iter::once(old.to_owned())
                    .chain((0..descendant_count).map(|n| format!("descendant-{n}")))
                    .collect();
                assert!(
                    events
                        .iter()
                        .take(labels.len())
                        .all(|e| e["type"] == "beforeunload"),
                    "{api}/{depth}: {result}"
                );
                for label in &labels {
                    let actual: Vec<_> = events.iter().filter(|e| e["label"] == *label).collect();
                    assert_eq!(actual.len(), 4, "{api}/{depth}: {result}");
                    for (event, kind) in actual.iter().zip([
                        "beforeunload",
                        "pagehide",
                        "visibilitychange",
                        "unload",
                    ]) {
                        assert_eq!(event["type"], kind, "{api}/{depth}: {result}");
                        assert_eq!(
                            event["hidden"],
                            matches!(kind, "visibilitychange" | "unload")
                        );
                        if kind != "beforeunload" {
                            assert_eq!(event["target"], "document", "{result}");
                        }
                        assert_eq!(event["trusted"], true);
                    }
                }
                let loaded: Vec<_> = events.iter().filter(|e| e["label"] == new).collect();
                assert_eq!(loaded.len(), 2, "{api}/{depth}: {result}");
                for (event, kind) in loaded.iter().zip(["load", "pageshow"]) {
                    assert_eq!(event["type"], kind);
                    assert_eq!(event["target"], "document");
                    assert_eq!(event["hidden"], false);
                }
                assert_eq!(events.len(), labels.len() * 4 + 2);
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_traversal_keeps_visibility_and_skips_unload() -> Result<()> {
    for api in ["history", "navigation"] {
        let result = history_visibility("same-document", api, 0).await?;
        assert_eq!(result["events"], json!([]), "{api}: {result}");
        assert_eq!(result["sameDocument"], true);
        assert_eq!(result["hidden"], false);
    }
    Ok(())
}
