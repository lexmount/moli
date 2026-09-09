use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn child_document_readiness_starts_loading_before_parser_scripts() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let result = page
        .evaluate_runtime_expression_with_await_async(
            r#"(async () => {
              const srcdoc = await new Promise(resolve => {
                const frame = document.createElement('iframe');
                frame.onload = () => {
                  resolve({states: frame.contentWindow.readyStates,
                    readyState: frame.contentDocument.readyState});
                  frame.remove();
                };
                frame.srcdoc = '<!doctype html><head><script>' +
                  'window.readyStates = [document.readyState];' +
                  'document.addEventListener("readystatechange", () => readyStates.push(document.readyState));' +
                  '<' + '/script></head><body>ready';
                document.body.append(frame);
              });
              const network = await new Promise(resolve => {
                const frame = document.createElement('iframe');
                frame.onload = () => {
                  resolve({duringScriptLoad: frame.contentWindow.parserConnectedLoadWriteReadyState,
                    readyState: frame.contentDocument.readyState});
                  frame.remove();
                };
                frame.src = '/compat/parser-connected-external-classic-load-document-write-insertion-point';
                document.body.append(frame);
              });
              return JSON.stringify({srcdoc, network, parent: document.readyState});
            })()"#,
            true,
        )
        .await?;
    server.shutdown().await;
    let value: serde_json::Value =
        serde_json::from_str(result["value"].as_str().expect("child readiness probe"))?;
    assert_eq!(
        value,
        serde_json::json!({
            "srcdoc": {"states": ["loading", "interactive", "complete"], "readyState": "complete"},
            "network": {"duringScriptLoad": "loading", "readyState": "complete"},
            "parent": "complete"
        })
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_readiness_keeps_initial_empty_and_detached_documents_complete() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let result = page
        .evaluate_runtime_expression_with_await_async(
            r#"(() => {
              const frame = document.createElement('iframe');
              document.body.append(frame);
              const initialEmpty = frame.contentDocument.readyState;
              frame.remove();
              return JSON.stringify({initialEmpty,
                detachedHtml: new DOMParser().parseFromString('<p>detached', 'text/html').readyState,
                detachedXml: new DOMParser().parseFromString('<detached/>', 'application/xml').readyState,
                createdHtml: document.implementation.createHTMLDocument('').readyState,
                constructed: new Document().readyState});
            })()"#,
            true,
        )
        .await?;
    server.shutdown().await;
    let value: serde_json::Value =
        serde_json::from_str(result["value"].as_str().expect("detached readiness probe"))?;
    assert_eq!(
        value,
        serde_json::json!({
            "initialEmpty": "complete", "detachedHtml": "complete", "detachedXml": "complete",
            "createdHtml": "complete", "constructed": "complete"
        })
    );
    Ok(())
}

async fn child_stream_readiness_probe(operation: &str) -> Result<serde_json::Value> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let expression = r#"(async () => {
      const frame = await new Promise(resolve => {
        const frame = document.createElement('iframe');
        frame.onload = () => resolve(frame);
        frame.src = '/static';
        document.body.append(frame);
      });
      frame.onload = null;
      // The load promise resumes during callback cleanup. Start a fresh
      // task so document.open() does not inherit the active load delivery.
      await new Promise(resolve => setTimeout(resolve, 0));
      const doc = frame.contentDocument;
      const win = frame.contentWindow;
      const oldBody = doc.body;
      let oldEvents = 0;
      doc.onreadystatechange = () => ++oldEvents;
      doc.addEventListener('readystatechange', () => ++oldEvents);
      win.addEventListener('readystatechange', () => ++oldEvents, true);
      oldBody.addEventListener('old-listener-probe', () => ++oldEvents);
      const operation = OPERATION;
      let opened = null;
      if (operation === 'open' || operation === 'reopen') {
        const same = doc.open() === doc;
        opened = {same, readyState: doc.readyState, children: doc.childNodes.length};
      }
      const markup = '<p id="written">new stream</p><script>' +
        'window.streamScriptReadyState = document.readyState;' + '<' + '/script>';
      if (operation === 'writeln') doc.writeln(markup);
      else doc.write(markup);
      let reopened = null;
      if (operation === 'reopen') {
        doc.addEventListener('readystatechange', () => ++oldEvents);
        const same = doc.open() === doc;
        reopened = {same, readyState: doc.readyState, children: doc.childNodes.length};
        doc.write(markup);
      }
      oldBody.dispatchEvent(new Event('old-listener-probe'));
      const during = {readyState: doc.readyState,
        scriptReadyState: win.streamScriptReadyState,
        sameDocument: doc === frame.contentDocument,
        text: doc.getElementById('written').textContent,
        handlerCleared: doc.onreadystatechange === null,
        trailingNewline: doc.body.lastChild.nodeType === Node.TEXT_NODE &&
          doc.body.lastChild.data === '\n'};
      const states = [];
      doc.addEventListener('readystatechange', () => states.push(doc.readyState));
      const finished = await new Promise(resolve => {
        win.addEventListener('load', () => resolve({readyState: doc.readyState,
          states, oldEvents, parentReadyState: document.readyState}), {once: true});
        doc.close();
      });
      frame.remove();
      return JSON.stringify({opened, reopened, during, finished});
    })()"#
        .replace("OPERATION", &serde_json::to_string(operation)?);
    let result = page
        .evaluate_runtime_expression_with_await_async(&expression, true)
        .await?;
    server.shutdown().await;
    serde_json::from_str(
        result["value"]
            .as_str()
            .expect("child stream readiness probe"),
    )
    .map_err(Into::into)
}

fn assert_child_stream_readiness(result: &serde_json::Value, newline: bool) {
    assert_eq!(
        result["during"],
        serde_json::json!({
            "readyState": "loading", "scriptReadyState": "loading", "sameDocument": true,
            "text": "new stream", "handlerCleared": true, "trailingNewline": newline
        }),
        "{result}"
    );
    assert_eq!(
        result["finished"],
        serde_json::json!({
            "readyState": "complete", "states": ["interactive", "complete"],
            "oldEvents": 0, "parentReadyState": "complete"
        }),
        "{result}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_open_resets_readiness_after_erasing_listeners() -> Result<()> {
    let result = child_stream_readiness_probe("open").await?;
    assert_eq!(
        result["opened"],
        serde_json::json!({"same": true, "readyState": "loading", "children": 0}),
        "{result}"
    );
    assert_child_stream_readiness(&result, false);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_write_implicit_open_resets_readiness() -> Result<()> {
    let result = child_stream_readiness_probe("write").await?;
    assert_child_stream_readiness(&result, false);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_writeln_implicit_open_resets_readiness() -> Result<()> {
    let result = child_stream_readiness_probe("writeln").await?;
    assert_child_stream_readiness(&result, true);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_open_replaces_an_already_loading_stream() -> Result<()> {
    let result = child_stream_readiness_probe("reopen").await?;
    assert_eq!(
        result["reopened"],
        serde_json::json!({"same": true, "readyState": "loading", "children": 0}),
        "{result}"
    );
    assert_child_stream_readiness(&result, false);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_open_during_parser_script_keeps_state_and_listeners() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let result = page
        .evaluate_runtime_expression_with_await_async(
            r#"new Promise(resolve => {
              const frame = document.createElement('iframe');
              frame.onload = () => {
                resolve(JSON.stringify({during: frame.contentWindow.duringOpen,
                  states: frame.contentWindow.readyStates,
                  hasTail: frame.contentDocument.getElementById('tail') !== null}));
                frame.remove();
              };
              frame.srcdoc = '<!doctype html><body><p id="before">before</p><script>' +
                'window.readyStates = [];' +
                'document.addEventListener("readystatechange", () => readyStates.push(document.readyState));' +
                'const same = document.open() === document;' +
                'window.duringOpen = {same, readyState: document.readyState, kept: !!document.getElementById("before")};' +
                '<' + '/script><p id="tail">tail</p>';
              document.body.append(frame);
            })"#,
            true,
        )
        .await?;
    server.shutdown().await;
    let value: serde_json::Value =
        serde_json::from_str(result["value"].as_str().expect("parser-script open probe"))?;
    assert_eq!(
        value,
        serde_json::json!({
            "during": {"same": true, "readyState": "loading", "kept": true},
            "states": ["interactive", "complete"], "hasTail": true
        })
    );
    Ok(())
}
