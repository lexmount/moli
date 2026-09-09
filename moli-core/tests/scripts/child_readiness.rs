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
