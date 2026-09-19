use super::*;

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

fn classic_script_markup(source: &str, external: bool) -> String {
    if external {
        let data = url::form_urlencoded::byte_serialize(source.as_bytes())
            .collect::<String>()
            .replace('+', "%20");
        format!("<script id=probe src=\"data:text/javascript,{data}\"></script>")
    } else {
        format!("<script id=probe>{source}</script>")
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_script_cleanup_retains_its_insertion_point_and_current_script() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut results = Vec::new();
    for external in [false, true] {
        for failure in ["none", "throw", "syntax"] {
            let source = match failure {
                "throw" => "throw new Error('parser cleanup');",
                "syntax" => "let = ;",
                _ => "parent.queueProbe(document);",
            };
            let script = classic_script_markup(source, external);
            let child = markup_url(
                &server,
                &format!(
                    r#"<!doctype html><body><p id=original>original</p>
                    <script>
                      onerror = () => {{
                        parent.queueProbe(document);
                        return true;
                      }};
                    </script>
                    {script}<p id=late>late</p>"#
                ),
            );
            let parent = format!(
                r#"<!doctype html><body><script>
                  window.results = [];
                  window.queueProbe = doc => Promise.resolve().then(() => probe(doc));
                  window.probe = doc => {{
                    const original = doc.getElementById('original');
                    const url = doc.URL;
                    const currentScript = doc.currentScript?.id;
                    const sameDocument = doc.open() === doc;
                    const retained = doc.getElementById('original') === original;
                    const sameURL = doc.URL === url;
                    const lateAbsent = !doc.getElementById('late');
                    doc.write('<b id=inserted>cleanup</b>');
                    doc.close();
                    results.push({{currentScript, sameDocument, retained, sameURL, lateAbsent,
                      inserted: doc.getElementById('inserted')?.textContent}});
                  }};
                </script><iframe id=target src="{}"></iframe>"#,
                child.replace('&', "&amp;")
            );
            let mut page = browser.fetch(&markup_url(&server, &parent)).await?;
            let observed = page
                .evaluate_runtime_expression_with_await_async(
                    r#"(() => {
                      const doc = document.getElementById('target').contentDocument;
                      const late = doc.getElementById('late')?.textContent;
                      const currentScriptRestored = doc.currentScript === null;
                      doc.open();
                      const laterOpenCleared = doc.childNodes.length === 0;
                      doc.close();
                      return JSON.stringify({results, late, currentScriptRestored, laterOpenCleared});
                    })()"#,
                    true,
                )
                .await?;
            let observed: serde_json::Value =
                serde_json::from_str(observed["value"].as_str().expect("parser cleanup result"))?;
            results.push((external, failure, observed));
        }
    }
    server.shutdown().await;
    for (external, failure, observed) in results {
        assert_eq!(
            observed,
            serde_json::json!({"results": [{"currentScript": "probe", "sameDocument": true,
              "retained": true, "sameURL": true, "lateAbsent": true, "inserted": "cleanup"}],
              "late": "late", "currentScriptRestored": true, "laterOpenCleared": true}),
            "external={external}, failure={failure}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn classic_script_cleanup_reports_errors_before_running_microtasks() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut results = Vec::new();
    for child in [false, true] {
        for external in [false, true] {
            let script = classic_script_markup(
                r#"log.push('body');
                Promise.resolve().then(() => log.push('microtask'));
                throw new Error('parser cleanup');"#,
                external,
            );
            let markup = format!(
                r#"<!doctype html><body><script>
                  window.log = [];
                  onerror = () => {{
                    log.push('error');
                    Promise.resolve().then(() => log.push('error microtask'));
                    return true;
                  }};
                </script>{script}<script>log.push('after');</script>"#
            );
            let url = markup_url(&server, &markup);
            let url = if child {
                markup_url(
                    &server,
                    &format!(
                        "<!doctype html><iframe id=target src=\"{}\"></iframe>",
                        url.replace('&', "&amp;")
                    ),
                )
            } else {
                url
            };
            let mut page = browser.fetch(&url).await?;
            let observed = page
                .evaluate_runtime_expression_with_await_async(
                    if child {
                        "JSON.stringify(document.getElementById('target').contentWindow.log)"
                    } else {
                        "JSON.stringify(log)"
                    },
                    true,
                )
                .await?;
            results.push((child, external, observed));
        }
    }
    server.shutdown().await;
    for (child, external, observed) in results {
        assert_eq!(
            observed["value"].as_str(),
            Some(r#"["body","error","microtask","error microtask","after"]"#),
            "child={child}, external={external}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_child_parser_scripts_leave_microtasks_for_the_outer_script_cleanup() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let markup = r#"<!doctype html><body><iframe id=target></iframe><script>
      window.log = [];
      const doc = document.getElementById('target').contentDocument;
      doc.open();
      doc.write(`<script>
        parent.log.push('child');
        Promise.resolve().then(() => parent.log.push('child microtask'));
      <\/script>`);
      log.push('parent');
      Promise.resolve().then(() => log.push('parent microtask'));
      doc.close();
    </script>"#;
    let mut page = browser.fetch(&markup_url(&server, markup)).await?;
    let observed = page
        .evaluate_runtime_expression_with_await_async("JSON.stringify(log)", true)
        .await?;
    server.shutdown().await;
    assert_eq!(
        observed["value"].as_str(),
        Some(r#"["child","parent","child microtask","parent microtask"]"#)
    );
    Ok(())
}
