use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn child_text_idl_getters_include_cdata_but_not_nested_elements() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let result = page.evaluate_runtime_expression_with_await_async(
        r#"(() => {
          const xml = document.implementation.createDocument(null, 'root');
          return JSON.stringify([['script', 'text'], ['title', 'text'], ['textarea', 'defaultValue']].map(([tag, property]) => {
            const el = document.createElement(tag);
            const cdata = xml.createCDATASection('CDATA 🦀');
            const nested = document.createElement('span');
            nested.textContent = 'ignored descendant';
            el.append(document.createTextNode('leading '), cdata,
              document.createComment('ignored comment'),
              document.createProcessingInstruction('probe', 'ignored instruction'), nested,
              document.createTextNode(' trailing'));
            const initial = el[property], recursive = el.textContent;
            el.normalize();
            const normalized = el[property], cdataPreserved = cdata.parentNode === el && cdata.nodeType === 4;
            cdata.replaceData(0, cdata.length, 'changed');
            const changed = el[property];
            cdata.remove();
            return {tag, initial, recursive, normalized, cdataPreserved, changed, removed: el[property]};
          }));
        })()"#, true,
    ).await?;
    server.shutdown().await;
    let result: serde_json::Value =
        serde_json::from_str(result["value"].as_str().expect("child text getters"))?;
    assert_eq!(
        result,
        serde_json::json!(
            ["script", "title", "textarea"].map(|tag| serde_json::json!({
                "tag": tag, "initial": "leading CDATA 🦀 trailing",
                "recursive": "leading CDATA 🦀ignored descendant trailing",
                "normalized": "leading CDATA 🦀 trailing", "cdataPreserved": true,
                "changed": "leading changed trailing", "removed": "leading  trailing"
            }))
        )
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn xml_child_parser_executes_mixed_text_and_cdata_script_source() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let result = page
        .evaluate_runtime_expression_with_await_async(
            r#"(async () => {
          const results = [];
          for (const type of ['application/xhtml+xml', 'application/xml', 'image/svg+xml']) {
            const svg = type === 'image/svg+xml';
            const start = svg ? '<svg xmlns="http://www.w3.org/2000/svg">' :
              '<html xmlns="http://www.w3.org/1999/xhtml"><head/><body>';
            const end = svg ? '</svg>' : '</body></html>';
            const markup = start + '<script>' +
              '<![CDATA[window.cdataSteps = ["first"];]]>' +
              '<![CDATA[cdataSteps.push("second");]]>' +
              '<ignored>throw new Error("must not execute descendants")</ignored>' +
              'cdataSteps.push("third");' + '<' + '/script>' +
              '<script>window.afterCdata = typeof cdataSteps;' + '<' + '/script>' + end;
            const frame = await new Promise(resolve => {
              const frame = document.createElement('iframe');
              frame.onload = () => resolve(frame);
              frame.src = '/compat/child-dynamic-markup-document?type=' + encodeURIComponent(type) +
                '&markup=' + encodeURIComponent(markup);
              document.body.append(frame);
            });
            results.push({type, contentType: frame.contentDocument.contentType,
              steps: frame.contentWindow.cdataSteps, after: frame.contentWindow.afterCdata,
              readyState: frame.contentDocument.readyState});
            frame.remove();
          }
          return JSON.stringify(results);
        })()"#,
            true,
        )
        .await?;
    server.shutdown().await;
    let result: serde_json::Value =
        serde_json::from_str(result["value"].as_str().expect("XML script text"))?;
    assert_eq!(
        result,
        serde_json::json!(
            ["application/xhtml+xml", "application/xml", "image/svg+xml"].map(
                |mime| serde_json::json!({
                    "type": mime, "contentType": mime, "steps": ["first", "second", "third"],
                    "after": "object", "readyState": "complete"
                })
            )
        )
    );
    Ok(())
}
