use moli_test_support as support;

use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig as AppConfig};
use support::FixtureServer;

fn evaluated_string(value: serde_json::Value) -> Option<String> {
    value
        .get("value")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

#[tokio::test]
async fn child_browsing_context_initial_history_seed_keeps_current_entry_index_zero() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/history-initial-child-entry-seed-parent"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-history-length=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-length=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-index=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-current-url=\"{}\"",
                server.url("/compat/history-initial-child-entry-seed-child.html")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn window_length_tracks_live_child_browsing_context_lifecycle() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-length"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-length-initial=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-length-after-append=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-length-after-remove=\"1\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_performance_survives_content_window_getter_refresh() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-length"))
        .await?;

    let result = evaluated_string(
        page.evaluate_runtime_expression_async(
            "(() => {\
              const frame = document.getElementById('initial-frame');\
              const firstWindow = frame.contentWindow;\
              const firstDocument = frame.contentDocument;\
              const firstPerformance = firstWindow.performance;\
              const firstOrigin = firstPerformance.timeOrigin;\
              for (let i = 0; i < 20; i++) {\
                void frame.contentWindow;\
                void frame.contentDocument;\
                void firstWindow.document;\
              }\
              return JSON.stringify({\
                windowSame: firstWindow === frame.contentWindow,\
                documentSame: firstDocument === frame.contentDocument,\
                performanceSame: firstPerformance === frame.contentWindow.performance,\
                originSame: Object.is(firstOrigin, frame.contentWindow.performance.timeOrigin),\
                topPerformanceAliased: firstPerformance === window.performance,\
                nowFinite: Number.isFinite(frame.contentWindow.performance.now())\
              });\
            })()",
        )
        .await?,
    );
    assert_eq!(
        result,
        Some(
            r#"{"windowSame":true,"documentSame":true,"performanceSame":true,"originSame":true,"topPerformanceAliased":false,"nowFinite":true}"#
                .to_owned()
        )
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn disabled_subframe_loading_keeps_iframe_dom_without_child_context() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default().with_subframe_loading_enabled(false))?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-length"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-length-initial=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-length-after-append=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-length-after-remove=\"0\"")
    );
    assert_eq!(
        evaluated_string(
            page.evaluate_runtime_expression_async(
                "(() => {\
                  const frame = document.getElementById('initial-frame');\
                  return JSON.stringify({\
                    iframeCount: document.querySelectorAll('iframe').length,\
                    contentWindow: frame.contentWindow === null,\
                    contentDocument: frame.contentDocument === null\
                  });\
                })()"
            )
            .await?
        ),
        Some(r#"{"iframeCount":1,"contentWindow":true,"contentDocument":true}"#.to_owned())
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn iframe_srcdoc_navigation_preserves_initial_empty_document_until_commit() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-snapshot"))
        .await?;

    assert!(
        page.serialize_html_async().await.unwrap()
            .contains("data-initial=\"&lt;html&gt;&lt;head&gt;&lt;/head&gt;&lt;body&gt;&lt;/body&gt;&lt;/html&gt;\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap()
            .contains("data-after-srcdoc=\"&lt;html&gt;&lt;head&gt;&lt;/head&gt;&lt;body&gt;&lt;/body&gt;&lt;/html&gt;\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-remove=\"null\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_default_execution_context_allows_local_parent_bindings() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-window-graph"))
        .await?;

    let child_realms = page
        .live_child_default_runtime_realm_inventory_async()
        .await?;
    let child_context_id = child_realms
        .iter()
        .map(|realm| realm.context_id)
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing child default execution context id"))?;

    let value = page
        .evaluate_runtime_expression_in_execution_context_with_await_async(
            child_context_id,
            "(() => { const parent = 'shadow-ok'; return parent; })()",
            false,
        )
        .await?;

    assert_eq!(value["value"].as_str(), Some("shadow-ok"), "{value}");

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_default_execution_context_inherits_secure_context_from_creator_origin() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-window-graph"))
        .await?;

    let child_realms = page
        .live_child_default_runtime_realm_inventory_async()
        .await?;
    let child_context_ids = child_realms
        .iter()
        .map(|realm| realm.context_id)
        .collect::<Vec<_>>();
    assert!(
        child_context_ids.len() >= 2,
        "expected about:blank and srcdoc child contexts: {child_realms:?}"
    );

    for child_context_id in child_context_ids {
        let value = page
            .evaluate_runtime_expression_in_execution_context_with_await_async(
                child_context_id,
                r#"[
                    typeof crypto.randomUUID,
                    String("subtle" in crypto),
                    typeof crypto.subtle,
                    String("SubtleCrypto" in globalThis),
                    typeof SubtleCrypto,
                    String("CryptoKey" in globalThis),
                    typeof CryptoKey
                ].join("|")"#,
                false,
            )
            .await?;

        assert_eq!(
            value["value"].as_str(),
            Some("function|true|object|true|function|true|function"),
            "{value}"
        );
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn iframe_named_target_navigation_is_not_synchronous() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-target-name"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-name=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-id=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-old-name=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-new-name=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-src-attr-after-name=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-src-attr-after-id=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-src-attr-after-new-name=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn parse_time_named_iframe_target_navigation_sees_parser_created_child_contexts() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-parse-time-target"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-parse-time-click=\"\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn iframe_form_targets_track_live_child_browsing_context_store() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-form-targets"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-form-submit=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-formtarget=\"\"")
    );

    server.shutdown().await;
    Ok(())
}
