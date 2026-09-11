use super::*;

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

async fn observe_parser_document(
    browser: &Browser,
    server: &FixtureServer,
    markup: &str,
    child: bool,
    expression: &str,
) -> Result<serde_json::Value> {
    let mut target = markup_url(server, markup);
    if child {
        target = markup_url(
            server,
            &format!(
                "<!doctype html><iframe id=target src=\"{}\"></iframe>",
                target.replace('&', "&amp;")
            ),
        );
    }
    let mut page = browser.fetch(&target).await?;
    let result = page
        .evaluate_runtime_expression_with_await_async(
            &format!(
                "JSON.stringify((() => {{
                  const win = document.getElementById('target')?.contentWindow || window;
                  const doc = win.document;
                  return ({expression});
                }})())"
            ),
            true,
        )
        .await?;
    Ok(serde_json::from_str(
        result["value"].as_str().expect("parser observation"),
    )?)
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_preparation_observes_microtask_changes_to_type_and_source() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let markup = r#"<!doctype html><body><script>
      window.executed = [];
      window.preparationErrors = 0;
      addEventListener('error', () => preparationErrors++, true);
      new MutationObserver(records => {
        for (const record of records) {
          for (const node of record.addedNodes) {
            if (node.localName !== 'script' || !node.id) continue;
            if (node.id === 'promoted') node.type = 'text/javascript';
            else if (node.id === 'rewritten') node.text = "executed.push('rewritten')";
            else node.type = 'text/plain';
          }
        }
      }).observe(document.body, {childList: true, subtree: true});
    </script>
    <script id=classic>executed.push('classic')</script>
    <script id=module type=module>executed.push('module')</script>
    <script id=async async src="data:text/javascript,executed.push('async')"></script>
    <script id=defer defer src="data:text/javascript,executed.push('defer')"></script>
    <script id=asyncmodule type=module async
      src="data:text/javascript,executed.push('asyncmodule')"></script>
    <script id=invalid src=""></script>
    <script id=promoted type=text/plain>executed.push('promoted')</script>
    <script id=rewritten>executed.push('stale source')</script><p id=late>late</p>"#;
    for child in [false, true] {
        let observed = observe_parser_document(
            &browser,
            &server,
            markup,
            child,
            "{executed: win.executed, errors: win.preparationErrors,
              late: !!doc.getElementById('late'), ready: doc.readyState}",
        )
        .await?;
        assert_eq!(
            observed,
            serde_json::json!({"executed": ["promoted", "rewritten"], "errors": 0,
              "late": true, "ready": "complete"}),
            "child={child}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_preparation_checkpoint_can_replace_the_document() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    for child in [false, true] {
        for attributes in [
            "",
            "type=module",
            "async src=\"data:text/javascript,window.staleScriptRan=true\"",
            "defer src=\"data:text/javascript,window.staleScriptRan=true\"",
        ] {
            let markup = format!(
                r#"<!doctype html><body><script>
                  window.staleScriptRan = false;
                  new MutationObserver(records => {{
                    for (const record of records) {{
                      for (const node of record.addedNodes) {{
                        if (node.localName !== 'script' || node.id !== 'probe') continue;
                        document.write('<body><p id=replacement>replacement</p>');
                        document.close();
                      }}
                    }}
                  }}).observe(document.body, {{childList: true}});
                </script><p>old body</p>
                <script id=probe {attributes}>window.staleScriptRan = true;</script>
                <p id=late>old parser tail</p>"#
            );
            let observed = observe_parser_document(
                &browser,
                &server,
                &markup,
                child,
                "{ran: win.staleScriptRan, text: doc.body.textContent,
                  late: !!doc.getElementById('late'), ready: doc.readyState}",
            )
            .await?;
            assert_eq!(
                observed,
                serde_json::json!({"ran": false, "text": "replacement", "late": false,
                  "ready": "complete"}),
                "child={child}, attributes={attributes}"
            );
        }
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_preparation_does_not_checkpoint_inside_document_write_script() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let markup = r#"<!doctype html><body><script>
      window.order = [];
      new MutationObserver(records => {
        for (const record of records) {
          for (const node of record.addedNodes) {
            if (node.localName !== 'script' || node.id !== 'nested') continue;
            order.push('observer');
            node.type = 'text/plain';
          }
        }
      }).observe(document.body, {childList: true});
      document.write('<script id=nested>order.push("nested")<' + '/script>');
      order.push('outer-end');
    </script><p id=late>late</p>"#;
    for child in [false, true] {
        let observed =
            observe_parser_document(&browser, &server, markup, child, "win.order").await?;
        assert_eq!(
            observed,
            serde_json::json!(["nested", "outer-end", "observer"]),
            "child={child}"
        );
    }
    server.shutdown().await;
    Ok(())
}
