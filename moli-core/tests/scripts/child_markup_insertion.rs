use super::*;

async fn evaluate_child_xml_probe(body: &str) -> Result<serde_json::Value> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let expression = r#"(async () => {
      const frames = [];
      async function loadFrame(type, markup) {
        markup ??= type === 'image/svg+xml'
          ? '<svg xmlns="http://www.w3.org/2000/svg"><g id="kept"/></svg>'
          : '<html xmlns="http://www.w3.org/1999/xhtml"><head/><body><p id="kept">original</p></body></html>';
        const url = '/compat/child-dynamic-markup-document?type=' + encodeURIComponent(type) +
          '&markup=' + encodeURIComponent(markup);
        return new Promise(resolve => {
          const frame = document.createElement('iframe');
          frames.push(frame);
          frame.onload = () => resolve(frame);
          frame.src = url;
          document.body.append(frame);
        });
      }
      try {
        return JSON.stringify(await (async () => { __PROBE_BODY__ })());
      } finally {
        for (const frame of frames) frame.remove();
      }
    })()"#
        .replace("__PROBE_BODY__", body);
    let result = page
        .evaluate_runtime_expression_with_await_async(&expression, true)
        .await?;
    server.shutdown().await;
    serde_json::from_str(result["value"].as_str().expect("child XML markup probe"))
        .map_err(Into::into)
}

#[tokio::test(flavor = "multi_thread")]
async fn child_xml_document_stream_methods_throw_without_side_effects() -> Result<()> {
    let result = evaluate_child_xml_probe(
        r#"const results = [];
        const operations = [['open'], ['open', 'text/html'], ['open', 'text/html', 'replace'],
          ['write'], ['write', '<p>replacement</p>'], ['writeln'],
          ['writeln', '<p>replacement</p>'], ['close']];
        for (const type of ['text/xml', 'application/xml', 'application/xhtml+xml', 'image/svg+xml']) {
          for (const [method, ...args] of operations) {
            const frame = await loadFrame(type);
            const doc = frame.contentDocument, win = frame.contentWindow;
            const root = doc.documentElement, html = root.outerHTML, url = doc.URL;
            const contentType = doc.contentType, readyState = doc.readyState;
            const events = [];
            doc.addEventListener('probe', () => events.push('document'));
            win.addEventListener('probe', () => events.push('window'));
            root.addEventListener('probe', () => events.push('root'));
            const observer = new MutationObserver(() => {});
            observer.observe(doc, {childList: true, subtree: true});
            Object.defineProperty(doc, 'contentType', {get() { throw new Error('must use native document type'); }});
            let error = null;
            try { doc[method](...args); }
            catch (e) { error = {name: e.name, code: e.code, relevantRealm: e instanceof win.DOMException}; }
            doc.dispatchEvent(new Event('probe'));
            win.dispatchEvent(new Event('probe'));
            root.dispatchEvent(new Event('probe'));
            results.push({type, method, argc: args.length, contentType, error, events,
              sameDocument: frame.contentDocument === doc,
              sameRoot: doc.documentElement === root, sameMarkup: doc.documentElement?.outerHTML === html,
              sameURL: doc.URL === url, sameReadiness: doc.readyState === readyState,
              mutations: observer.takeRecords().length});
            observer.disconnect();
          }
        }
        return results;"#,
    )
    .await?;
    let rows = result.as_array().expect("XML stream method matrix");
    assert_eq!(rows.len(), 32);
    for row in rows {
        assert_eq!(row["contentType"], row["type"], "{row}");
        assert_eq!(
            row["error"],
            serde_json::json!({"name": "InvalidStateError", "code": 11, "relevantRealm": true}),
            "{row}"
        );
        assert_eq!(
            row["events"],
            serde_json::json!(["document", "window", "root"]),
            "{row}"
        );
        for field in [
            "sameDocument",
            "sameRoot",
            "sameMarkup",
            "sameURL",
            "sameReadiness",
        ] {
            assert_eq!(row[field], true, "{field}: {row}");
        }
        assert_eq!(row["mutations"], 0, "{row}");
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_xml_document_write_converts_arguments_before_rejecting_xml() -> Result<()> {
    let result = evaluate_child_xml_probe(
        r#"const results = [];
        for (const method of ['write', 'writeln']) {
          const frame = await loadFrame('application/xhtml+xml');
          const doc = frame.contentDocument, root = doc.documentElement;
          const conversions = [];
          const sentinel = {};
          let conversionError = false;
          try {
            doc[method]({toString() { conversions.push('throw'); throw sentinel; }});
          } catch (e) { conversionError = e === sentinel; }
          let error = null;
          try {
            doc[method]({toString() { conversions.push('first'); return '<p>'; }},
              {toString() { conversions.push('second'); return '</p>'; }});
          } catch (e) { error = {name: e.name, relevantRealm: e instanceof frame.contentWindow.DOMException}; }
          results.push({method, conversionError, conversions, error,
            sameRoot: doc.documentElement === root});
        }
        return results;"#,
    )
    .await?;
    assert_eq!(
        result,
        serde_json::json!(["write", "writeln"].map(|method| serde_json::json!({
            "method": method, "conversionError": true,
            "conversions": ["throw", "first", "second"], "sameRoot": true,
            "error": {"name": "InvalidStateError", "relevantRealm": true}
        })))
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_xml_document_stream_methods_reject_during_parser_script() -> Result<()> {
    let result = evaluate_child_xml_probe(
        r#"const frame = await loadFrame('application/xhtml+xml',
          '<html xmlns="http://www.w3.org/1999/xhtml"><head/><body><p id="before">before</p>' +
          '<script>' +
          'window.streamErrors = []; window.readyStates = [];' +
          'document.addEventListener("readystatechange", () => readyStates.push(document.readyState));' +
          'for (const method of ["open", "write", "writeln", "close"]) {' +
          ' let error = null; try { document[method](); }' +
          ' catch (e) { error = {name: e.name, relevantRealm: e instanceof DOMException}; }' +
          ' streamErrors.push({method, error, readyState: document.readyState});' +
          '}' +
          '<' + '/script><p id="tail">tail</p></body></html>');
        return {errors: frame.contentWindow.streamErrors, states: frame.contentWindow.readyStates,
          before: !!frame.contentDocument.getElementById('before'),
          tail: !!frame.contentDocument.getElementById('tail')};"#,
    )
    .await?;
    assert_eq!(
        result["errors"],
        serde_json::json!(
            ["open", "write", "writeln", "close"].map(|method| serde_json::json!({
                "method": method, "error": {"name": "InvalidStateError", "relevantRealm": true},
                "readyState": "loading"
            }))
        ),
        "{result}"
    );
    assert_eq!(
        result["states"],
        serde_json::json!(["interactive", "complete"])
    );
    assert_eq!(result["before"], true);
    assert_eq!(result["tail"], true);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_xml_document_stream_methods_reject_during_unload() -> Result<()> {
    let result = evaluate_child_xml_probe(
        r#"const frame = await loadFrame('application/xhtml+xml');
        const doc = frame.contentDocument, win = frame.contentWindow;
        const root = doc.documentElement, errors = [];
        for (const event of ['beforeunload', 'pagehide', 'unload']) {
          win.addEventListener(event, () => {
            for (const method of ['open', 'write', 'writeln', 'close']) {
              let error = null;
              try { doc[method](); }
              catch (e) { error = {name: e.name, relevantRealm: e instanceof win.DOMException}; }
              errors.push({event, method, error, sameRoot: doc.documentElement === root});
            }
          });
        }
        await new Promise(resolve => { frame.onload = resolve; frame.src = '/static'; });
        return errors;"#,
    )
    .await?;
    let rows = result.as_array().expect("XML unload markup calls");
    let expected = ["beforeunload", "pagehide", "unload"]
        .into_iter()
        .flat_map(|event| {
            ["open", "write", "writeln", "close"].map(|method| {
                serde_json::json!({
                    "event": event, "method": method, "sameRoot": true,
                    "error": {"name": "InvalidStateError", "relevantRealm": true}
                })
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(rows, &expected);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_write_converts_arguments_before_parser_constructor_guard() -> Result<()> {
    let result = evaluate_child_xml_probe(
        r#"const frame = await loadFrame('text/html');
        const doc = frame.contentDocument, win = frame.contentWindow, results = [];
        class MarkupProbe extends win.HTMLElement {
          constructor() {
            super();
            for (const method of ['write', 'writeln']) {
              const conversions = [], sentinel = {};
              let conversionError = false, error = null;
              try { doc[method]({toString() { conversions.push('throw'); throw sentinel; }}); }
              catch (e) { conversionError = e === sentinel; }
              try { doc[method]({toString() { conversions.push('string'); return '<b>blocked</b>'; }}); }
              catch (e) { error = {name: e.name, relevantRealm: e instanceof win.DOMException}; }
              results.push({method, conversions, conversionError, error});
            }
          }
        }
        win.customElements.define('markup-probe', MarkupProbe);
        doc.open();
        doc.write('<!doctype html><body><markup-probe></markup-probe>');
        doc.close();
        return results;"#,
    )
    .await?;
    assert_eq!(
        result,
        serde_json::json!(["write", "writeln"].map(|method| serde_json::json!({
            "method": method, "conversions": ["throw", "string"], "conversionError": true,
            "error": {"name": "InvalidStateError", "relevantRealm": true}
        })))
    );
    Ok(())
}
