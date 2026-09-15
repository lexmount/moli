use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig};
use moli_test_support::FixtureServer;
use serde_json::{Value, json};
use tokio::time::Duration;
use url::Url;

async fn navigation_events(scenario: &str, target: &str, constructors: &str) -> Result<Value> {
    let markup = include_str!("fixtures/runtime/navigation_event_realms.html")
        .replace("__SCENARIO__", &serde_json::to_string(scenario)?)
        .replace("__TARGET__", &serde_json::to_string(target)?)
        .replace("__CONSTRUCTORS__", &serde_json::to_string(constructors)?);
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let mut url = Url::parse(&server.url("/compat/child-dynamic-markup-document"))?;
    url.query_pairs_mut().append_pair("markup", &markup);
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let mut page = browser.fetch(url.as_str()).await?;
        page.evaluate_runtime_expression_with_await_async(
            "finished.then(value => JSON.stringify(value))",
            true,
        )
        .await
    })
    .await??;
    let result = serde_json::from_str(result["value"].as_str().unwrap())?;
    server.shutdown().await;
    Ok(result)
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_window_navigation_events_use_the_target_realm_and_native_constructors() -> Result<()>
{
    for constructors in ["original", "overwritten"] {
        for target in ["top", "child"] {
            for scenario in ["success", "reject"] {
                let events = if scenario == "success" {
                    json!([
                        "navigate",
                        "currententrychange",
                        "navigatesuccess",
                        "navigate",
                        "currententrychange",
                        "dispose",
                        "navigatesuccess",
                        "currententrychange",
                        "navigate",
                        "currententrychange",
                        "navigatesuccess"
                    ])
                } else {
                    json!(["navigate", "currententrychange", "abort", "navigateerror"])
                };
                assert_eq!(
                    navigation_events(scenario, target, constructors).await?,
                    json!({"errors": [], "reads": 0, "events": events}),
                    "{scenario}/{target}/{constructors}"
                );
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_joint_history_events_use_the_parent_event_target_realm() -> Result<()> {
    for constructors in ["original", "overwritten"] {
        for scenario in ["joint-cancel", "joint-prune"] {
            let events = if scenario == "joint-cancel" {
                json!(["navigate", "abort", "navigateerror"])
            } else {
                json!(["dispose", "dispose"])
            };
            assert_eq!(
                navigation_events(scenario, "top", constructors).await?,
                json!({"errors": [], "reads": 0, "events": events}),
                "{scenario}/{constructors}"
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parent_initiated_child_navigation_constructs_events_in_the_retiring_child_realm()
-> Result<()> {
    for constructors in ["original", "overwritten"] {
        assert_eq!(
            navigation_events("cross-document", "child", constructors).await?,
            json!({"errors": [], "reads": 0, "events": ["navigate"]}),
            "{constructors}"
        );
    }
    Ok(())
}
