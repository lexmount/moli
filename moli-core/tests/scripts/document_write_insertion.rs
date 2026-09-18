use super::*;

const NESTED_WRITE: &str =
    include_str!("../fixtures/runtime/document_write_nested_external_input.html");

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

async fn nested_external_writes_keep_script_insertion_points(
    child: bool,
    script_created: bool,
) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let markup = if script_created {
        if child {
            "<!doctype html><body><iframe id=target></iframe>".to_owned()
        } else {
            "<!doctype html><body>".to_owned()
        }
    } else if child {
        format!(
            "<!doctype html><body><iframe id=target src=\"{}\"></iframe>",
            markup_url(&server, NESTED_WRITE)
        )
    } else {
        NESTED_WRITE.to_owned()
    };
    let mut page = tokio::time::timeout(
        Duration::from_secs(10),
        browser.fetch(&markup_url(&server, &markup)),
    )
    .await??;
    let receiver = if child {
        "document.getElementById('target').contentDocument"
    } else {
        "document"
    };
    if script_created {
        let script = format!(
            r#"new Promise(resolve => {{
              const doc = {receiver};
              doc.open();
              doc.addEventListener('DOMContentLoaded', () => resolve(true), {{once: true}});
              doc.write({});
              doc.close();
            }})"#,
            serde_json::to_string(NESTED_WRITE)?
        );
        tokio::time::timeout(
            Duration::from_secs(10),
            page.evaluate_runtime_expression_with_await_async(&script, true),
        )
        .await??;
    }
    let result = page
        .evaluate_runtime_expression_with_await_async(
            &format!(
                r#"(() => {{
                  const doc = {receiver};
                  return JSON.stringify({{
                    text: Array.from(doc.getElementById('written').childNodes)
                      .filter(node => node.nodeType === 3).map(node => node.data).join('').trim(),
                    tail: doc.getElementById('tail')?.textContent
                  }});
                }})()"#
            ),
            true,
        )
        .await?;
    let result: serde_json::Value =
        serde_json::from_str(result["value"].as_str().expect("nested write observation"))?;
    assert_eq!(
        result,
        serde_json::json!({"text": "worked", "tail": "tail"})
    );
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_parser_nested_external_writes_keep_each_script_insertion_point() -> Result<()> {
    nested_external_writes_keep_script_insertion_points(false, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn main_open_parser_nested_external_writes_keep_each_script_insertion_point() -> Result<()> {
    nested_external_writes_keep_script_insertion_points(false, true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_nested_external_writes_keep_each_script_insertion_point() -> Result<()> {
    nested_external_writes_keep_script_insertion_points(true, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_open_parser_nested_external_writes_keep_each_script_insertion_point() -> Result<()> {
    nested_external_writes_keep_script_insertion_points(true, true).await
}
