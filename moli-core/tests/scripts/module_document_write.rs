use super::*;

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

fn module_url(source: &str) -> String {
    format!(
        "data:text/javascript,{}",
        url::form_urlencoded::byte_serialize(source.as_bytes())
            .collect::<String>()
            .replace('+', "%20")
    )
}

async fn child_module_load_precedes_delayed_write(variant: &str) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    for timer_owner in ["parent", "window"] {
        let write = r#"
          parent.events.push('timer:' + document.readyState);
          document.write('replacement');
          document.close();
          parent.events.push('write');
          parent.finishWrite();
        "#;
        let source = if variant == "tla" {
            format!("await new Promise(resolve => {timer_owner}.setTimeout(resolve, 0)); {write}")
        } else {
            format!("{timer_owner}.setTimeout(() => {{ {write} }}, 0);")
        };
        let script = match variant {
            "import" => format!(
                "<script type=module>import {};</script>",
                serde_json::to_string(&module_url(&source))?
            ),
            "external" => format!(
                "<script type=module src=\"{}\"></script>",
                module_url(&source)
            ),
            _ => format!("<script type=module>{source}</script>"),
        };
        let child = markup_url(
            &server,
            &format!("<!doctype html><head>{script}</head><body>original"),
        );
        let parent = format!(
            r#"<!doctype html><body><script>
              window.events = [];
              window.writeDone = new Promise(resolve => window.finishWrite = resolve);
              const frame = document.createElement('iframe');
              frame.id = 'target';
              frame.onload = () => events.push('load:' + frame.contentDocument.body.textContent);
              frame.src = {};
              document.body.append(frame);
            </script>"#,
            serde_json::to_string(&child)?
        );
        let mut page = browser.fetch(&markup_url(&server, &parent)).await?;
        let result = page.evaluate_runtime_expression_with_await_async(
            "writeDone.then(() => JSON.stringify({events, body: document.getElementById('target').contentDocument.body.textContent}))",
            true,
        ).await?;
        let result: serde_json::Value =
            serde_json::from_str(result["value"].as_str().expect("delayed write observation"))?;
        let events = result["events"].as_array().expect("child event sequence");
        assert_eq!(
            events.iter().take(3).collect::<Vec<_>>(),
            vec![
                &serde_json::json!("load:original"),
                &serde_json::json!("timer:complete"),
                &serde_json::json!("write")
            ],
            "variant={variant}, timer_owner={timer_owner}, events={events:?}"
        );
        assert_eq!(
            result["body"], "replacement",
            "variant={variant}, timer_owner={timer_owner}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_inline_module_load_precedes_delayed_document_write() -> Result<()> {
    child_module_load_precedes_delayed_write("inline").await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_imported_module_load_precedes_delayed_document_write() -> Result<()> {
    child_module_load_precedes_delayed_write("import").await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_external_module_load_precedes_delayed_document_write() -> Result<()> {
    child_module_load_precedes_delayed_write("external").await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_module_load_precedes_timer_that_resumes_top_level_await() -> Result<()> {
    child_module_load_precedes_delayed_write("tla").await
}
