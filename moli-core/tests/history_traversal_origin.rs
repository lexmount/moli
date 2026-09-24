use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig};
use moli_test_support::FixtureServer;
use serde_json::{Value, json};
use tokio::time::Duration;

async fn traversal_origin(target: &str, scenario: &str) -> Result<Value> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut page = browser
        .fetch(&server.url("/compat/child-dynamic-markup-document?markup=%3Cbody%3E"))
        .await?;
    let script = format!(
        "{}\nnavigationTraversalOrigin({target:?}, {scenario:?}).then(JSON.stringify)",
        include_str!("fixtures/navigation-traversal-origin.js")
    );
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        page.evaluate_runtime_expression_with_await_async(&script, true),
    )
    .await??;
    let result = serde_json::from_str(result["value"].as_str().unwrap())?;
    server.shutdown().await;
    Ok(result)
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_origin_history_traversals_do_not_dispatch_navigation_events() -> Result<()> {
    for target in ["child", "popup"] {
        let result = traversal_origin(target, "cross").await?;
        assert_eq!(result["returned"], true, "{target}: {result}");
        assert_eq!(result["exposed"], false, "{target}: {result}");
        assert_eq!(result["events"], json!([]), "{target}: {result}");
        assert_eq!(result["details"], json!([]), "{target}: {result}");
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn same_origin_history_traversals_dispatch_outside_the_visible_entry_list() -> Result<()> {
    for target in ["child", "popup", "nested"] {
        for scenario in ["same", "gap", "srcdoc"] {
            if (target == "popup" && scenario == "srcdoc")
                || (target == "nested" && scenario != "srcdoc")
            {
                continue;
            }
            let result = traversal_origin(target, scenario).await?;
            assert_eq!(result["returned"], true, "{target}/{scenario}: {result}");
            assert_eq!(
                result["events"][0], "navigate",
                "{target}/{scenario}: {result}"
            );
            let details = result["details"].as_array().unwrap();
            assert_eq!(details.len(), 1, "{target}/{scenario}: {result}");
            let visible = scenario != "gap";
            assert_eq!(result["exposed"], visible, "{target}/{scenario}: {result}");
            assert_eq!(
                details[0],
                json!({
                    "type": "traverse", "url": true, "sameDocument": false,
                    "key": if visible { result["original"]["key"].clone() } else { json!("") },
                    "id": if visible { result["original"]["id"].clone() } else { json!("") },
                    "index": if visible { 0 } else { -1 },
                    "state": if visible { json!({"label": "A"}) } else { Value::Null },
                    "cancelable": false, "canIntercept": false,
                }),
                "{target}/{scenario}: {result}"
            );
        }
    }
    Ok(())
}
