use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig};
use moli_test_support::FixtureServer;
use serde_json::Value;
use tokio::time::Duration;
use url::Url;

async fn stream_operation_during_unload(
    event: &str,
    operation: &str,
    target: &str,
) -> Result<Value> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let markup = format!(
        r#"<!doctype html><body><script>
          window.finished = (async () => {{
            async function frameIn(owner) {{
              const frame = owner.document.createElement('iframe');
              const loaded = new Promise(resolve => frame.onload = resolve);
              frame.src = '/compat/child-dynamic-markup-document?markup=' +
                encodeURIComponent('<!doctype html><body><p id="retained">retained');
              owner.document.body.append(frame);
              await loaded;
              return frame;
            }}
            const frame = await frameIn(window);
            const target = {target:?};
            let listenerFrame = frame;
            let doc = frame.contentDocument;
            if (target === 'other') doc = (await frameIn(window)).contentDocument;
            if (target === 'ancestor') {{
              const middle = await frameIn(frame.contentWindow);
              doc = middle.contentDocument;
              listenerFrame = await frameIn(middle.contentWindow);
            }}
            const root = doc.documentElement;
            const url = doc.URL;
            const originalLength = doc.childNodes.length;
            let listenerCount = 0;
            doc.addEventListener('retained-listener', () => listenerCount++);
            let finish;
            const result = new Promise(resolve => finish = resolve);
            const listener = () => {{
              let converted = 0;
              const input = {{toString() {{ converted++; return '<body>overwritten'; }}}};
              const operation = {operation:?};
              let returnedDocument = false;
              let error = null;
              try {{
                if (operation === 'open') returnedDocument = doc.open() === doc;
                else if (operation === 'prototype-open')
                  returnedDocument = Document.prototype.open.call(doc) === doc;
                else if (operation === 'prototype-write')
                  Document.prototype.write.call(doc, input);
                else doc[operation](input);
              }} catch (exception) {{ error = exception.name; }}
              doc.dispatchEvent(new Event('retained-listener'));
              finish({{sameRoot: doc.documentElement === root,
                sameLength: doc.childNodes.length === originalLength,
                sameURL: doc.URL === url, listenerCount,
                converted, returnedDocument, error}});
              if (target === 'other' || target === 'synthetic') {{
                doc.write('<body>finished');
                doc.close();
              }}
            }};
            const event = {event:?};
            const listenerTarget = event === 'visibilitychange' ?
              listenerFrame.contentDocument : listenerFrame.contentWindow;
            listenerTarget.addEventListener(event, listener, {{once: true}});
            if (target === 'synthetic') listenerTarget.dispatchEvent(new Event(event));
            else frame.src = target === 'ancestor' ? 'javascript:"<body>destination"' :
              '/compat/child-dynamic-markup-document?markup=' +
                encodeURIComponent('<!doctype html><body>destination');
            return result;
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
    assert!(
        result["error"].is_null(),
        "{event}/{operation}/{target}: {result}"
    );
    Ok(result)
}

#[tokio::test(flavor = "multi_thread")]
async fn document_stream_operations_have_no_side_effects_during_unload() -> Result<()> {
    for event in ["beforeunload", "pagehide", "unload"] {
        for operation in [
            "open",
            "prototype-open",
            "write",
            "writeln",
            "prototype-write",
        ] {
            let result = stream_operation_during_unload(event, operation, "self").await?;
            assert_eq!(result["sameRoot"], true, "{event}/{operation}: {result}");
            assert_eq!(result["sameLength"], true);
            assert_eq!(result["sameURL"], true);
            assert_eq!(result["listenerCount"], 1);
            let is_open = operation.ends_with("open");
            assert_eq!(result["returnedDocument"], is_open);
            assert_eq!(result["converted"], usize::from(!is_open));
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn ancestor_unload_counter_covers_descendant_callbacks() -> Result<()> {
    for event in ["pagehide", "visibilitychange", "unload"] {
        let result = stream_operation_during_unload(event, "open", "ancestor").await?;
        assert_eq!(result["sameRoot"], true, "{event}: {result}");
        assert_eq!(result["sameLength"], true);
        assert_eq!(result["listenerCount"], 1);
        assert_eq!(result["returnedDocument"], true);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn unload_does_not_block_opening_another_document() -> Result<()> {
    for event in ["beforeunload", "pagehide", "unload"] {
        let result = stream_operation_during_unload(event, "open", "other").await?;
        assert_eq!(result["sameRoot"], false, "{event}: {result}");
        assert_eq!(result["listenerCount"], 0);
        assert_eq!(result["returnedDocument"], true);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn synthetic_unload_events_do_not_block_document_open() -> Result<()> {
    for event in ["beforeunload", "pagehide", "visibilitychange", "unload"] {
        let result = stream_operation_during_unload(event, "open", "synthetic").await?;
        assert_eq!(result["sameRoot"], false, "{event}: {result}");
        assert_eq!(result["listenerCount"], 0);
        assert_eq!(result["returnedDocument"], true);
    }
    Ok(())
}
