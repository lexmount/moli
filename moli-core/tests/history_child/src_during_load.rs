use super::*;

const SCRIPT: &str = include_str!("../fixtures/runtime/iframe_same_src_during_load.js");

async fn same_src_during_load(phase: &str, cross_origin: bool) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let child_markup = if phase == "window-load" {
        "<!doctype html><iframe></iframe><iframe></iframe><script>\
         addEventListener('load', () => parent.reloadFrameInWindowLoad());</script>"
    } else {
        "<!doctype html><iframe></iframe><iframe></iframe>"
    };
    let mut child_url = Url::parse(&server.url("/compat/child-dynamic-markup-document"))?;
    child_url
        .query_pairs_mut()
        .append_pair("markup", child_markup);
    if cross_origin {
        child_url.set_host(Some("localhost"))?;
    }
    for write in ["property", "attribute"] {
        let mut page = browser.fetch(&server.url("/static")).await?;
        let options = serde_json::json!({
            "phase": phase,
            "write": write,
            "childURL": child_url.as_str(),
            "crossOrigin": cross_origin,
        });
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            page.evaluate_runtime_expression_with_await_async(
                &format!("({SCRIPT})({options}).then(JSON.stringify)"),
                true,
            ),
        )
        .await??;
        assert!(
            result["exceptionDetails"].is_null(),
            "phase={phase}, write={write}, cross_origin={cross_origin}: {result}"
        );
        let checks: Vec<serde_json::Value> = serde_json::from_str(
            result["value"]
                .as_str()
                .expect("same-src load result should be serialized checks"),
        )?;
        assert_eq!(checks.len(), if cross_origin { 10 } else { 16 });
        for check in checks {
            assert_eq!(check["pass"], true, "{check}");
        }
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_same_src_during_owner_load_reloads_same_and_cross_origin_documents() -> Result<()> {
    for cross_origin in [false, true] {
        same_src_during_load("owner-load", cross_origin).await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_same_src_during_window_load_reloads_documents() -> Result<()> {
    same_src_during_load("window-load", false).await
}
