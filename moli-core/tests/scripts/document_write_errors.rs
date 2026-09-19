use super::*;

const ERROR_SCRIPT: &str = include_str!("../fixtures/runtime/document_write_reported_errors.js");

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

async fn written_script_errors_are_reported(child: bool, script_created: bool) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = format!("<!doctype html><body><script id=outer>{ERROR_SCRIPT}</script>");
    let markup = if script_created {
        "<!doctype html><body>".to_owned()
    } else {
        fixture.clone()
    };
    let markup = if child {
        format!(
            "<!doctype html><body><iframe id=target src=\"{}\"></iframe>",
            markup_url(&server, &markup)
        )
    } else {
        markup
    };
    let mut page = tokio::time::timeout(
        Duration::from_secs(10),
        browser.fetch(&markup_url(&server, &markup)),
    )
    .await??;
    let receiver = if child {
        "document.getElementById('target').contentWindow"
    } else {
        "window"
    };
    if script_created {
        let script = format!(
            r#"new Promise(resolve => {{
              const doc = {receiver}.document;
              doc.open();
              doc.addEventListener('DOMContentLoaded', () => resolve(true), {{once: true}});
              doc.write({});
              doc.close();
            }})"#,
            serde_json::to_string(&fixture)?
        );
        tokio::time::timeout(
            Duration::from_secs(10),
            page.evaluate_runtime_expression_with_await_async(&script, true),
        )
        .await??;
    }
    let observation = page
        .evaluate_runtime_expression_with_await_async(
            &format!("JSON.stringify({receiver}.writeErrorResult())"),
            true,
        )
        .await?;
    let observation: serde_json::Value = serde_json::from_str(
        observation["value"]
            .as_str()
            .expect("error report observation"),
    )?;
    assert_eq!(
        observation,
        serde_json::json!({"checks": 19, "failures": []})
    );
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_parser_reports_written_script_errors_without_throwing_into_writer() -> Result<()> {
    written_script_errors_are_reported(false, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn main_open_parser_reports_written_script_errors_without_throwing_into_writer() -> Result<()>
{
    written_script_errors_are_reported(false, true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_reports_written_script_errors_without_throwing_into_writer() -> Result<()> {
    written_script_errors_are_reported(true, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_open_parser_reports_written_script_errors_without_throwing_into_writer() -> Result<()>
{
    written_script_errors_are_reported(true, true).await
}
