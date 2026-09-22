use super::*;

async fn assert_pipe_disturbed(scenario: &str) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = format!(
        "{}\nrunPipeDisturbedProbe({}).then(finish, error => finish({{error: String(error)}}));",
        include_str!("../fixtures/runtime/pipe_disturbed.js"),
        serde_json::to_string(scenario)?,
    );
    for target in ["window", "child", "worker"] {
        let observed = tokio::time::timeout(
            Duration::from_secs(20),
            run_probe(&browser, &server, target, &source),
        )
        .await??;
        assert_eq!(
            observed,
            serde_json::json!({"errors": []}),
            "{scenario}/{target}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn piping_disturbs_fetch_bodies_before_reads_and_abort_handling() -> Result<()> {
    assert_pipe_disturbed("start").await
}

#[tokio::test(flavor = "multi_thread")]
async fn rejected_piping_preserves_undisturbed_fetch_bodies() -> Result<()> {
    assert_pipe_disturbed("reject").await
}

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

pub(super) async fn run_probe(
    browser: &Browser,
    server: &FixtureServer,
    target: &str,
    source: &str,
) -> Result<serde_json::Value> {
    let markup = if target == "worker" {
        let source = serde_json::to_string(&format!(
            "self.finish = value => postMessage(value);\n{source}"
        ))?;
        format!(
            r#"<!doctype html><script>
              const worker = new Worker(URL.createObjectURL(new Blob([{source}], {{type: 'text/javascript'}})));
              window.done = new Promise(resolve => {{
                worker.onmessage = event => {{ resolve(event.data); worker.terminate(); }};
              }});
              worker.postMessage('go');
            </script>"#
        )
    } else {
        format!(
            "<!doctype html><script>self.done = new Promise(resolve => self.finish = resolve);\n{source}</script>"
        )
    };
    let url = markup_url(server, &markup);
    let url = if target == "child" {
        markup_url(
            server,
            &format!(
                "<!doctype html><iframe id=target src=\"{}\"></iframe>",
                url.replace('&', "&amp;")
            ),
        )
    } else {
        url
    };
    let mut page = browser.fetch(&url).await?;
    let expression = if target == "child" {
        "document.getElementById('target').contentWindow.done.then(JSON.stringify)"
    } else {
        "done.then(JSON.stringify)"
    };
    let value = page
        .evaluate_runtime_expression_with_await_async(expression, true)
        .await?;
    Ok(serde_json::from_str(
        value["value"].as_str().expect("callback probe result"),
    )?)
}
