use super::*;

async fn child_script_error_write_probe(
    nested: bool,
    unknown_scheme: bool,
) -> Result<serde_json::Value> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let source = if unknown_scheme {
        "no-such-script-scheme:missing".to_owned()
    } else {
        server.url("/missing-child-parser-script.js")
    };
    let expression = r#"new Promise(resolve => {
      let during = null;
      globalThis.childErrorObserved = value => { during = value; };
      const frame = document.createElement('iframe');
      frame.onload = () => resolve(JSON.stringify({
        during, after: frame.contentDocument.body.textContent,
        nestedRan: frame.contentWindow.nestedRan === true
      }));
      const written = NESTED
        ? '<script>globalThis.nestedRan = true; document.write("text");<' + '/script>'
        : 'text';
      const handler = `function failed(event) {
        document.write(${JSON.stringify(written).replaceAll('<', '\\u003c')});
        parent.childErrorObserved({text: document.body.textContent,
          currentScriptIsNull: document.currentScript === null,
          tailMissing: document.getElementById('tail') === null,
          readyState: document.readyState, eventType: event.type,
          nestedRan: globalThis.nestedRan === true});
      }`;
      frame.srcdoc = '<!doctype html><head><script>' + handler + '<' + '/script></head>' +
        '<body>Some <script src="' + SOURCE + '" onerror="failed(event)"><' + '/script>' +
        '<span id="tail"> tail</span>';
      document.body.append(frame);
    })"#
    .replace("NESTED", if nested { "true" } else { "false" })
    .replace("SOURCE", &serde_json::to_string(&source)?);
    let result = page
        .evaluate_runtime_expression_with_await_async(&expression, true)
        .await?;
    server.shutdown().await;
    serde_json::from_str(result["value"].as_str().expect("child error probe")).map_err(Into::into)
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_fetch_error_write_is_synchronous() -> Result<()> {
    let result = child_script_error_write_probe(false, false).await?;
    assert_eq!(result["during"]["text"], "Some text", "{result}");
    assert_eq!(result["during"]["currentScriptIsNull"], true, "{result}");
    assert_eq!(result["during"]["tailMissing"], true, "{result}");
    assert_eq!(result["during"]["eventType"], "error", "{result}");
    assert_eq!(result["after"], "Some text tail", "{result}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_fetch_error_write_handles_unknown_schemes() -> Result<()> {
    let result = child_script_error_write_probe(false, true).await?;
    assert_eq!(result["during"]["text"], "Some text", "{result}");
    assert_eq!(result["during"]["tailMissing"], true, "{result}");
    assert_eq!(result["after"], "Some text tail", "{result}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_fetch_error_write_executes_nested_inline_scripts() -> Result<()> {
    let result = child_script_error_write_probe(true, false).await?;
    assert_eq!(result["during"]["nestedRan"], true, "{result}");
    assert_eq!(result["during"]["tailMissing"], true, "{result}");
    assert_eq!(result["nestedRan"], true, "{result}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_fetch_error_handler_observes_loading_document() -> Result<()> {
    let result = child_script_error_write_probe(false, false).await?;
    assert_eq!(result["during"]["readyState"], "loading", "{result}");
    Ok(())
}
