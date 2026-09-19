use moli_test_support as support;

use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig as AppConfig, RenderedDomWaitUntil};
use tokio::time::Duration;

async fn wait_for_body_attribute(
    browser: &Browser,
    page: &mut moli_core::page::Page,
    attr: &str,
    expected: &str,
) -> Result<()> {
    let attr = serde_json::to_string(attr)?;
    let expected = serde_json::to_string(expected)?;
    browser
        .wait_for_script_truthy(
            page,
            &format!("document.body?.getAttribute({attr}) === {expected}"),
            Duration::from_secs(2),
        )
        .await
}

async fn wait_for_body_attribute_contains(
    browser: &Browser,
    page: &mut moli_core::page::Page,
    attr: &str,
    needle: &str,
) -> Result<()> {
    let attr = serde_json::to_string(attr)?;
    let needle = serde_json::to_string(needle)?;
    browser
        .wait_for_script_truthy(
            page,
            &format!("document.body?.getAttribute({attr})?.includes({needle}) === true"),
            Duration::from_secs(2),
        )
        .await
}

fn assert_cross_document_navigation_destination_defaults(page_html: &str) {
    assert!(
        page_html.contains("data-currententrychange-count=\"0\""),
        "{page_html}"
    );
    assert!(
        page_html.contains("data-property-currententrychange-count=\"0\""),
        "{page_html}"
    );
    assert!(
        page_html.contains("data-history-state=\"null\""),
        "{page_html}"
    );
    assert!(
        page_html.contains("data-navigation-state=\"null\""),
        "{page_html}"
    );
}

fn assert_cross_document_traversal_activation_destination_surface(
    page_html: &str,
    source_url: &str,
    dest_url: &str,
) {
    assert_cross_document_navigation_destination_defaults(page_html);
    assert!(
        page_html.contains("data-transition-is-null=\"true\""),
        "{page_html}"
    );
    assert!(
        page_html.contains(&format!("data-entry-url=\"{source_url}\"")),
        "{page_html}"
    );
    assert!(
        page_html.contains(&format!("data-from-url=\"{dest_url}\"")),
        "{page_html}"
    );
    assert!(
        page_html.contains("data-navigation-type=\"traverse\""),
        "{page_html}"
    );
    assert!(
        page_html.contains(&format!("data-b-entry-url=\"{dest_url}\"")),
        "{page_html}"
    );
    assert!(
        page_html.contains(&format!("data-b-from-url=\"{source_url}\"")),
        "{page_html}"
    );
    assert!(
        page_html.contains("data-b-navigation-type=\"push\""),
        "{page_html}"
    );
    assert!(
        page_html.contains(&format!("data-b-current-url=\"{dest_url}\"")),
        "{page_html}"
    );
    assert!(
        page_html.contains("data-b-can-back=\"true\""),
        "{page_html}"
    );
    assert!(
        page_html.contains("data-b-can-forward=\"false\""),
        "{page_html}"
    );
    assert!(
        page_html.contains("data-source-sync=\"surface:true|sync:false,false\""),
        "{page_html}"
    );
    assert!(
        page_html.contains("data-source-log=\"before|after-call\""),
        "{page_html}"
    );
    assert!(
        page_html.contains("data-source-committed-url=\"null\""),
        "{page_html}"
    );
    assert!(
        page_html.contains("data-source-finished-url=\"null\""),
        "{page_html}"
    );
}

use support::FixtureServer;
#[tokio::test]
async fn history_pushstate_and_replacestate_resolve_relative_urls_and_update_location() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/history-relative/base/index.html"))
        .await?;

    assert_eq!(
        page.final_url().path(),
        "/compat/history-relative/base/replace"
    );
    assert_eq!(page.final_url().query(), Some("y=2"));
    assert_eq!(page.final_url().fragment(), Some("frag"));
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-push-path=\"/compat/history-relative/base/child/push?x=1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-replace-path=\"/compat/history-relative/base/replace?y=2#frag\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-length=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state=\"{&quot;step&quot;:2}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_pushstate_and_replacestate_clone_state_and_reject_uncloneable_values() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/history-state-clone-and-dataclone-error"))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-popstate-value", "1").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-push-state-value=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-push-state-distinct=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-popstate-value=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-replace-state-value=\"3\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-replace-state-distinct=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dataclone-threw=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dataclone-name=\"DataCloneError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-length=\"3\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_pushstate_and_replacestate_reject_cross_origin_urls_with_security_error()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/history-cross-origin-security-error"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-push=\"SecurityError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-replace=\"SecurityError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-href=\"{}\"",
                server.url("/compat/history-cross-origin-security-error")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-pathname=\"/compat/history-cross-origin-security-error\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-length=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_state_mutation_does_not_mutate_stored_snapshot() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/history-state-mutation-does-not-mutate-stored-snapshot"))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-timeout-state", r#"{"x":1}"#).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-state=\"{&quot;y&quot;:2}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-popstate-state=\"{&quot;x&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state=\"{&quot;x&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-state=\"{&quot;x&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_length_and_state_assignments_do_not_mutate_public_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server
                .url("/compat/history-length-and-state-assignments-do-not-mutate-public-surface"),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-before-length=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-length=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-before-state=\"{&quot;x&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-state=\"{&quot;x&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_navigation_brand_and_descriptor_surface_matches_chromium() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/history-navigation-brand-and-descriptor-surface"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-tag=\"[object History]\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-tag=\"[object Navigation]\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-instanceof=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-instanceof=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-length-own=\"missing\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state-own=\"missing\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-scroll-own=\"missing\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-back-own=\"missing\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-length-proto=\"function|undefined|none|none|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state-proto=\"function|undefined|none|none|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-scroll-proto=\"function|function|none|none|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-back-proto=\"undefined|undefined|function|true|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-current-own=\"missing\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-activation-own=\"missing\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-transition-own=\"missing\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-navigate-own=\"missing\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-cangoback-own=\"missing\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-cangoforward-own=\"missing\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-current-proto=\"function|undefined|none|none|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-navigation-activation-proto=\"function|undefined|none|none|true|true\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-navigation-transition-proto=\"function|undefined|none|none|true|true\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-navigation-navigate-proto=\"undefined|undefined|function|true|true|true\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-cangoback-proto=\"function|undefined|none|none|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-navigation-cangoforward-proto=\"function|undefined|none|none|true|true\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_go_zero_reloads_current_document() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/history-go-zero-reloads-current-document"))
        .await?;

    let url = server.url("/compat/history-go-zero-reloads-current-document");

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-count=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-href=\"{url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-snapshot=\"true|0|0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-nav-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-activation-type=\"reload\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-activation-entry=\"{url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-activation-from=\"{url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-currententrychange-count=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-currententrychange-count=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_go_nan_reloads_current_document() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/history-go-nan-reloads-current-document"))
        .await?;

    let url = server.url("/compat/history-go-nan-reloads-current-document");

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-count=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-href=\"{url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-snapshot=\"true|0|0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-nav-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-activation-type=\"reload\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-activation-entry=\"{url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-activation-from=\"{url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-currententrychange-count=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-currententrychange-count=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_go_no_argument_reloads_current_document() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/history-go-no-argument-reloads-current-document"))
        .await?;

    let url = server.url("/compat/history-go-no-argument-reloads-current-document");

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-count=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-href=\"{url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-snapshot=\"true|0|0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-nav-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-activation-type=\"reload\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-activation-entry=\"{url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-activation-from=\"{url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-currententrychange-count=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-currententrychange-count=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_go_rejects_symbol_and_bigint() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/history-go-rejects-symbol-and-bigint"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-symbol-count=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-symbol-result=\"TypeError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-bigint-count=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-bigint-result=\"TypeError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_go_string_minus_one_coerces_to_back_traversal() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/history-go-string-minus-one-traverses-back"))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-timeout-state", "1").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-state=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-popstate-step=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-popstate-history-state=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-state=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_back_traversal_is_not_synchronous() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/history-back-same-turn-traverses-asynchronously"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.dataset.timeoutState === '1'",
            Duration::from_secs(2),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-state=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-popstate-step=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-popstate-history-state=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-popstate-microtask-state=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-state=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_same_turn_back_then_forward_dispatches_both_popstates() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/history-back-forward-same-turn-queues"))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-popstate-log", "1,2").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-state=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-final-state=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-popstate-log=\"1,2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_back_ignores_page_tampered_queue_microtask() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/history-back-ignores-page-tampered-queue-microtask"))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-history-back-fired", "yes").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-queue-microtask-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-back-fired=\"yes\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-history-back-sync-url=\"{}#one\"",
                server.url("/compat/history-back-ignores-page-tampered-queue-microtask")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-history-back-async-url=\"{}\"",
                server.url("/compat/history-back-ignores-page-tampered-queue-microtask")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_scroll_restoration_ignores_invalid_values() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/history-scroll-restoration-invalid-value-ignored"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial=\"auto\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-manual=\"manual\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid=\"manual\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-auto=\"auto\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_location_hash_assignment_dispatches_popstate_and_hashchange() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(
            &server
                .url("/compat/history-location-hash-assignment-dispatches-popstate-and-hashchange"),
        )
        .await?;
    wait_for_body_attribute(
        &browser,
        &mut page,
        "data-order",
        "popstate,after-set,popstate-microtask,hashchange,hashchange-microtask",
    )
    .await?;

    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-order=\"popstate,after-set,popstate-microtask,hashchange,hashchange-microtask\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-popstate-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-href=\"{}#frag\"",
                server.url(
                    "/compat/history-location-hash-assignment-dispatches-popstate-and-hashchange"
                )
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-old=\"{}\"",
                server.url(
                    "/compat/history-location-hash-assignment-dispatches-popstate-and-hashchange"
                )
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-new=\"{}#frag\"",
                server.url(
                    "/compat/history-location-hash-assignment-dispatches-popstate-and-hashchange"
                )
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-len=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_initial_navigation_current_entry_index_starts_at_zero() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/history-initial-navigation-current-entry-index-starts-at-zero"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-length=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-index=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-current-url=\"{}\"",
                server.url("/compat/history-initial-navigation-current-entry-index-starts-at-zero")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_pushstate_does_not_set_navigation_current_entry_state() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/history-pushstate-does-not-set-navigation-current-entry-state"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial-nav-state-undefined=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-push-history-state=\"{&quot;step&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-push-nav-state-undefined=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-after-push-url=\"{}#one\"",
                server.url("/compat/history-pushstate-does-not-set-navigation-current-entry-state")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-push-index=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-replace-history-state=\"{&quot;step&quot;:2}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-replace-nav-state-undefined=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-after-replace-url=\"{}#two\"",
                server.url("/compat/history-pushstate-does-not-set-navigation-current-entry-state")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-replace-index=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_onpopstate_property_receives_restored_state_after_back() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(
            &server.url("/compat/history-onpopstate-property-receives-restored-state-after-back"),
        )
        .await?;
    wait_for_body_attribute(
        &browser,
        &mut page,
        "data-timeout-history-state",
        r#"{"new":"field","testComplete":true,"testInProgress":true}"#,
    )
    .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-onpopstate-fired=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-onpopstate-state=\"{&quot;new&quot;:&quot;field&quot;,&quot;testComplete&quot;:true,&quot;testInProgress&quot;:true}\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-history-state=\"{&quot;new&quot;:&quot;field&quot;,&quot;testComplete&quot;:true,&quot;testInProgress&quot;:true}\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-timeout-history-state=\"{&quot;new&quot;:&quot;field&quot;,&quot;testComplete&quot;:true,&quot;testInProgress&quot;:true}\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_back_fragment_traversal_dispatches_popstate_then_hashchange() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(
            &server
                .url("/compat/history-back-fragment-traversal-dispatches-popstate-then-hashchange"),
        )
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-completed-state", "1").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-hash=\"#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-state=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-order=\"currententrychange:traverse:#two,currententrychange-microtask:#one:1,popstate:1,popstate-microtask:1,hashchange:#one,hashchange-microtask:#one\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-state=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-old=\"{}#two\"",
                server.url(
                    "/compat/history-back-fragment-traversal-dispatches-popstate-then-hashchange"
                )
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-new=\"{}#one\"",
                server.url(
                    "/compat/history-back-fragment-traversal-dispatches-popstate-then-hashchange"
                )
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-from-url=\"{}#two\"",
                server.url(
                    "/compat/history-back-fragment-traversal-dispatches-popstate-then-hashchange"
                )
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-type=\"traverse\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_forward_fragment_traversal_dispatches_popstate_then_hashchange() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page =
        browser
            .fetch(&server.url(
                "/compat/history-forward-fragment-traversal-dispatches-popstate-then-hashchange",
            ))
            .await?;
    wait_for_body_attribute(&browser, &mut page, "data-completed-state", "2").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-state=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-order=\"currententrychange:traverse:#one,currententrychange-microtask:#two:2,popstate:2,popstate-microtask:2,hashchange:#two,hashchange-microtask:#two\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-hash=\"#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-state=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-old=\"{}#one\"",
                server.url(
                    "/compat/history-forward-fragment-traversal-dispatches-popstate-then-hashchange"
                )
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-new=\"{}#two\"",
                server.url(
                    "/compat/history-forward-fragment-traversal-dispatches-popstate-then-hashchange"
                )
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-from-url=\"{}#one\"",
                server.url(
                    "/compat/history-forward-fragment-traversal-dispatches-popstate-then-hashchange"
                )
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-type=\"traverse\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_location_replace_fragment_replaces_current_entry() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/history-location-replace-fragment-replaces-current-entry"))
        .await?;
    wait_for_body_attribute(
        &browser,
        &mut page,
        "data-final-href",
        &format!(
            "{}#frag",
            server.url("/compat/history-location-replace-fragment-replaces-current-entry")
        ),
    )
    .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-before-len=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-len=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-final-len=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-order=\"popstate,popstate-microtask,hashchange,hashchange-microtask\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-before-href=\"{}\"",
                server.url("/compat/history-location-replace-fragment-replaces-current-entry")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-sync-href=\"{}#frag\"",
                server.url("/compat/history-location-replace-fragment-replaces-current-entry")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-final-href=\"{}#frag\"",
                server.url("/compat/history-location-replace-fragment-replaces-current-entry")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-old=\"{}\"",
                server.url("/compat/history-location-replace-fragment-replaces-current-entry")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-new=\"{}#frag\"",
                server.url("/compat/history-location-replace-fragment-replaces-current-entry")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}
#[tokio::test]
async fn navigation_currententrychange_fires_on_same_document_hash_navigation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/navigation-currententrychange-on-hash-navigation"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-fired=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-count=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-current-url=\"{}#1\"",
                server.url("/compat/navigation-currententrychange-on-hash-navigation")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_currententrychange_ignores_page_tampered_dispatch_event() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server
                .url("/compat/navigation-currententrychange-ignores-page-tampered-dispatch-event"),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-dispatch-event-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-currententrychange-fired=\"yes\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-currententrychange-fired=\"yes\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-currententrychange-navigation-type=\"push\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-location-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_update_current_entry_updates_state_and_fires_currententrychange() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/navigation-update-current-entry-updates-state-and-fires-currententrychange",
        ))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-order=\"currententrychange,currententrychange,after-call\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-count=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-missing-name=\"TypeError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-missing-required=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-empty-name=\"TypeError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-empty-state-required=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-undefined-name=\"TypeError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-undefined-state-required=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-null-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-function-name=\"DataCloneError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-function-state-unchanged=\"{&quot;x&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-same-entry=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-nav-state=\"{&quot;x&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-event-constructor=\"NavigationCurrentEntryChangeEvent\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-event-from-same-entry=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-event-navigation-type=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_pushstate_dispatches_navigation_currententrychange_event_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/history-pushstate-dispatches-navigation-currententrychange-event-surface",
        ))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-fired=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-constructor=\"NavigationCurrentEntryChangeEvent\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-event-from-same-entry=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
            "data-event-from-url=\"{}\"",
            server.url(
                "/compat/history-pushstate-dispatches-navigation-currententrychange-event-surface"
            )
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-event-navigation-type=\"push\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
            "data-current-url=\"{}#one\"",
            server.url(
                "/compat/history-pushstate-dispatches-navigation-currententrychange-event-surface"
            )
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state=\"{&quot;step&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_reload_reloads_current_document() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/navigation-reload-reloads-current-document"))
        .await?;

    let url = server.url("/compat/navigation-reload-reloads-current-document");

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-count=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-typeof-reload=\"function\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reload-length=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-return-shape=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-snapshot=\"true|0|0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-committed-shape=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-finished-shape=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-nav-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-activation-type=\"reload\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-activation-entry=\"{url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-activation-from=\"{url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-currententrychange-count=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-currententrychange-count=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]

async fn navigation_activation_initial_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/navigation-activation-initial-surface"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-activation-type=\"object\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-activation-is-null=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-transition-type=\"object\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-transition-is-null=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-activation-assign-ignored=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-transition-assign-ignored=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-entry-assign-ignored=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-entry-descriptor=\"function|undefined|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-activation-descriptor=\"function|undefined|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-transition-descriptor=\"function|undefined|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-entry-url=\"{}\"",
                server.url("/compat/navigation-activation-initial-surface")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-from-url=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-type=\"push\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_activation_same_document_navigation_stays_initial() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/navigation-activation-same-document-navigation-stays-initial"))
        .await?;
    browser
        .wait_for_page_delay(&mut page, Duration::from_millis(250))
        .await?;

    let base = server.url("/compat/navigation-activation-same-document-navigation-stays-initial");
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-initial=\"href={base}|activationEntry={base}|activationFrom=|activationType=push|current={base}|transitionIsNull=true\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-after-push-call=\"href={base}#one|activationEntry={base}|activationFrom=|activationType=push|current={base}#one|transitionIsNull=true\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-after-push-timeout=\"href={base}#one|activationEntry={base}|activationFrom=|activationType=push|current={base}#one|transitionIsNull=true\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-after-back-timeout=\"href={base}|activationEntry={base}|activationFrom=|activationType=push|current={base}|transitionIsNull=true\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-after-forward-timeout=\"href={base}#one|activationEntry={base}|activationFrom=|activationType=push|current={base}#one|transitionIsNull=true\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-after-traverse-timeout=\"href={base}|activationEntry={base}|activationFrom=|activationType=push|current={base}|transitionIsNull=true\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_activation_cross_document_destination_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server.url("/compat/navigation-activation-cross-document-destination-surface-source"),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-activation-type=\"object\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-activation-is-null=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-transition-is-null=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-length=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-entry-url=\"{}\"",
                server.url("/compat/navigation-activation-cross-document-destination-surface-dest")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-from-url=\"{}\"",
                server
                    .url("/compat/navigation-activation-cross-document-destination-surface-source")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-type=\"push\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert_cross_document_navigation_destination_defaults(
        &page.serialize_html_async().await.unwrap(),
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_activation_cross_document_back_destination_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let source_url =
        server.url("/compat/navigation-activation-cross-document-back-destination-surface-source");
    let dest_url =
        server.url("/compat/navigation-activation-cross-document-back-destination-surface-dest");

    let page = browser
        .fetch_with_wait_until(
            &source_url,
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(2),
        )
        .await?;

    assert_eq!(page.final_url().as_str(), source_url);
    assert_cross_document_traversal_activation_destination_surface(
        &page.serialize_html_async().await.unwrap(),
        &source_url,
        &dest_url,
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_activation_cross_document_traverse_to_destination_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let source_url = server
        .url("/compat/navigation-activation-cross-document-traverse-to-destination-surface-source");
    let dest_url = server
        .url("/compat/navigation-activation-cross-document-traverse-to-destination-surface-dest");

    let page = browser
        .fetch_with_wait_until(
            &source_url,
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(2),
        )
        .await?;

    assert_eq!(page.final_url().as_str(), source_url);
    assert_cross_document_traversal_activation_destination_surface(
        &page.serialize_html_async().await.unwrap(),
        &source_url,
        &dest_url,
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_navigate_cross_document_push_destination_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server
                .url("/compat/navigation-navigate-cross-document-push-destination-surface-source"),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-length=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-activation-type=\"push\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-entry-url=\"{}\"",
                server.url(
                    "/compat/navigation-navigate-cross-document-push-destination-surface-dest"
                )
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-from-url=\"{}\"",
                server.url(
                    "/compat/navigation-navigate-cross-document-push-destination-surface-source"
                )
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-same-document=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-entry-same-document-values=\"false|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-source-log=\"before|after-call\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-result-order=\"surface:true|sync:false,false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert_cross_document_navigation_destination_defaults(
        &page.serialize_html_async().await.unwrap(),
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_navigate_same_document_push_updates_history_and_events() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(
            &server
                .url("/compat/navigation-navigate-same-document-push-updates-history-and-events"),
        )
        .await?;
    wait_for_body_attribute_contains(
        &browser,
        &mut page,
        "data-final",
        "finished:#dest-push:true",
    )
    .await?;

    assert!(
        page.serialize_html_async().await.unwrap().contains(&"data-sync=\"hash=#dest-push|len=2|current=#dest-push|beforeLen=1|beforeUrl=#base|history=null|entry={&quot;step&quot;:7}|hasPromises=true|committed=false|finished=false|order=cec:replace:#base:(none),cec:push:#dest-push:#base,popstate:#dest-push\"".to_string()),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&"data-final=\"hash=#dest-push|len=2|current=#dest-push|history=null|entry={&quot;step&quot;:7}|committed=true|finished=true|order=cec:replace:#base:(none),cec:push:#dest-push:#base,popstate:#dest-push,cec-micro:#dest-push:null,cec-micro:#dest-push:null,committed:#dest-push:true,finished:#dest-push:true,hashchange:#dest-push|canBack=true|canForward=false\"".to_string()),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_navigate_same_document_replace_updates_history_and_events() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page =
        browser
            .fetch(&server.url(
                "/compat/navigation-navigate-same-document-replace-updates-history-and-events",
            ))
            .await?;
    wait_for_body_attribute_contains(
        &browser,
        &mut page,
        "data-final",
        "finished:#dest-replace:true",
    )
    .await?;

    assert!(
        page.serialize_html_async().await.unwrap().contains(&"data-sync=\"hash=#dest-replace|len=1|current=#dest-replace|beforeLen=1|beforeUrl=#base|history=null|entry={&quot;step&quot;:9}|hasPromises=true|committed=false|finished=false|order=cec:replace:#base:(none),cec:replace:#dest-replace:#base,popstate:#dest-replace\"".to_string()),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&"data-final=\"hash=#dest-replace|len=1|current=#dest-replace|history=null|entry={&quot;step&quot;:9}|committed=true|finished=true|order=cec:replace:#base:(none),cec:replace:#dest-replace:#base,popstate:#dest-replace,cec-micro:#dest-replace:null,cec-micro:#dest-replace:null,committed:#dest-replace:true,finished:#dest-replace:true,hashchange:#dest-replace|canBack=false|canForward=false\"".to_string()),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_navigate_argument_validation_matches_chromium() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/navigation-navigate-argument-validation"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-missing-name=\"TypeError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-missing-required=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-name=\"TypeError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-value=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-enum=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-location=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-symbol-name=\"TypeError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-symbol-location=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-url-shape=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-url-location=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-url-committed=\"SyntaxError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-url-finished=\"SyntaxError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-auto-shape=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-auto-location=\"#auto\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_navigate_same_document_result_promises_settle_before_hashchange() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/navigation-navigate-same-document-result-promises-settle-before-hashchange",
        ))
        .await?;
    wait_for_body_attribute_contains(&browser, &mut page, "data-timeout", "finished=true").await?;

    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-sync=\"hash=#dest|committed=false|finished=false|order=cec:replace:#base,cec:push:#dest,popstate:#dest\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-result-keys=\"committed,finished\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-result-props=\"committed,finished\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-timeout=\"hash=#dest|committed=true|finished=true|order=cec:replace:#base,cec:push:#dest,popstate:#dest,committed:#dest,finished:#dest,hashchange:#dest\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-committed-type=\"[object NavigationHistoryEntry]\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-finished-type=\"[object NavigationHistoryEntry]\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-same-committed=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-same-finished=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_navigate_same_document_state_uses_structured_clone() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/navigation-navigate-same-document-state-uses-structured-clone"))
        .await?;
    wait_for_body_attribute_contains(&browser, &mut page, "data-final", "[1,2]").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync=\"true|2020-01-02T03:04:05.000Z|true|null|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-final=\"true|2020-01-02T03:04:05.000Z|true|[1,2]|null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_back_surface_and_fragment_traversal() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/navigation-back-surface-and-fragment-traversal"))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-cangoforward-after", "true").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-back-type=\"function\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-forward-type=\"function\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-noop-back-sync=\"true|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-noop-forward-sync=\"true|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-noop-back-committed=\"InvalidStateError|Cannot go back\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-noop-back-finished=\"InvalidStateError|Cannot go back\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-noop-forward-committed=\"InvalidStateError|Cannot go forward\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-noop-forward-finished=\"InvalidStateError|Cannot go forward\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-cangoback-before=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-cangoforward-before=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-entries-length=\"3\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-hash=\"#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-state=\"{&quot;n&quot;:2}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-return-shape=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-state=\"{&quot;n&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-cangoback-after=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-cangoforward-after=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_traverse_to_key_fragment_traversal() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/navigation-traverse-to-key-fragment-traversal"))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-completed-state", r#"{"n":1}"#).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-traverseto-type=\"function\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-target-key-present=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-cangoback-before=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-cangoforward-before=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-key-sync=\"true|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-key-committed=\"[object NavigationHistoryEntry]|#two|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-key-finished=\"[object NavigationHistoryEntry]|#two|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-missing-key-sync=\"true|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-missing-key-committed=\"InvalidStateError|Invalid key\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-missing-key-finished=\"InvalidStateError|Invalid key\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-missing-argument-name=\"TypeError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-missing-argument-required=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-symbol-name=\"TypeError\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-undefined-key-sync=\"true|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-undefined-key-committed=\"InvalidStateError|Invalid key\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-undefined-key-finished=\"InvalidStateError|Invalid key\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-return-shape=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-hash=\"#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-state=\"{&quot;n&quot;:2}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-state=\"{&quot;n&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-cangoback-after=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-cangoforward-after=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-order=\"currententrychange:traverse:#two,currententrychange-microtask:#one:1,popstate:1,popstate-microtask:1,hashchange:#one,hashchange-microtask:#one\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-from-url=\"{}#two\"",
                server.url("/compat/navigation-traverse-to-key-fragment-traversal")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-type=\"traverse\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_oncurrententrychange_property_receives_traverse_event_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/navigation-oncurrententrychange-property-receives-traverse-event-surface",
        ))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-timeout-state", r#"{"n":1}"#).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-oncurrententrychange-fired=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-hash=\"#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-state=\"{&quot;n&quot;:2}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-state=\"{&quot;n&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-order=\"currententrychange,currententrychange-microtask:#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-type=\"traverse\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
            "data-from-url=\"{}#two\"",
            server.url(
                "/compat/navigation-oncurrententrychange-property-receives-traverse-event-surface",
            )
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_forward_dispatches_currententrychange_traverse_event_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page =
        browser
            .fetch(&server.url(
                "/compat/navigation-forward-dispatches-currententrychange-traverse-event-surface",
            ))
            .await?;
    wait_for_body_attribute(&browser, &mut page, "data-timeout-state", r#"{"n":2}"#).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-oncurrententrychange-fired=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-state=\"{&quot;n&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-hash=\"#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-state=\"{&quot;n&quot;:2}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-order=\"currententrychange,currententrychange-microtask:#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-type=\"traverse\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
            "data-from-url=\"{}#one\"",
            server.url(
                "/compat/navigation-forward-dispatches-currententrychange-traverse-event-surface",
            )
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_forward_result_promises_settle_after_async_traversal() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(
            &server.url("/compat/navigation-forward-result-promises-settle-after-async-traversal"),
        )
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-timeout-finished", "true").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-committed=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-finished=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-result-keys=\"committed,finished\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-result-props=\"committed,finished\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-current-same-before=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-hash=\"#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-committed=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-finished=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-order=\"committed:#two,finished:#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-committed-type=\"[object NavigationHistoryEntry]\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-finished-type=\"[object NavigationHistoryEntry]\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-current-same-committed=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-current-same-finished=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-before-same-committed=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-before-same-finished=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_back_result_promises_settle_after_async_traversal() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/navigation-back-result-promises-settle-after-async-traversal"))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-completed-finished", "true").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-committed=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-finished=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-result-keys=\"committed,finished\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-result-props=\"committed,finished\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-current-same-before=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-committed=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-finished=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap()
            .contains("data-order=\"cec:traverse:#two,committed:#one:true:false,cec-micro:#one:1,finished:#one:true:false,popstate:1,popstate-micro:1,hashchange:#one,hashchange-micro:#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-committed-type=\"[object NavigationHistoryEntry]\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-finished-type=\"[object NavigationHistoryEntry]\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-current-same-committed=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-current-same-finished=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-before-same-committed=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-before-same-finished=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_traverse_to_result_promises_settle_after_async_traversal() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(
            &server
                .url("/compat/navigation-traverse-to-result-promises-settle-after-async-traversal"),
        )
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-completed-finished", "true").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-committed=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-finished=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-result-keys=\"committed,finished\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-result-props=\"committed,finished\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-current-same-before=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-committed=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-finished=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap()
            .contains("data-order=\"cec:traverse:#two,committed:#one:true:false,cec-micro:#one:1,finished:#one:true:false,popstate:1,popstate-micro:1,hashchange:#one,hashchange-micro:#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-committed-type=\"[object NavigationHistoryEntry]\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-finished-type=\"[object NavigationHistoryEntry]\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-current-same-committed=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-current-same-finished=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-before-same-committed=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-completed-before-same-finished=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_back_restores_navigation_entry_state_separately_from_history_state()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/navigation-back-restores-navigation-entry-state-separately-from-history-state",
        ))
        .await?;
    wait_for_body_attribute(
        &browser,
        &mut page,
        "data-timeout-nav-state",
        r#"{"nav":0}"#,
    )
    .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-history-state=\"{&quot;step&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-nav-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-hash=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-nav-state=\"{&quot;nav&quot;:0}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-event-navigation-type=\"traverse\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-event-from-url=\"{}#one\"",
            server.url(
                "/compat/navigation-back-restores-navigation-entry-state-separately-from-history-state",
            )
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-event-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-event-nav-state=\"{&quot;nav&quot;:0}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_traverse_to_restores_navigation_entry_state_separately_from_history_state()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/navigation-traverse-to-restores-navigation-entry-state-separately-from-history-state",
        ))
        .await?;
    wait_for_body_attribute(
        &browser,
        &mut page,
        "data-timeout-nav-state",
        r#"{"nav":0}"#,
    )
    .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-target-key=\"")
            && !page
                .serialize_html_async()
                .await
                .unwrap()
                .contains("data-target-key=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-history-state=\"{&quot;step&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-nav-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-hash=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-timeout-nav-state=\"{&quot;nav&quot;:0}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-event-navigation-type=\"traverse\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-event-from-url=\"{}#one\"",
            server.url(
                "/compat/navigation-traverse-to-restores-navigation-entry-state-separately-from-history-state",
            )
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-event-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-event-nav-state=\"{&quot;nav&quot;:0}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_navigate_persists_state_to_destination_entry() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/navigation-navigate-state-persists-to-destination"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-state-nav-test-in-progress=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-index-advanced=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-entry-index=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-current-entry-url=\"{}\"",
                server.url("/compat/navigation-navigate-state-destination")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_navigate_cross_document_result_promises_do_not_settle_before_destination_load()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server.url(
                "/compat/navigation-navigate-cross-document-result-promises-do-not-settle-before-destination-load",
            ),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-result-order=\"surface:true|sync:false,false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-source-log=\"before|after-call\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-committed-type=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-finished-type=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-committed-url=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-finished-url=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_navigate_cross_document_does_not_dispatch_currententrychange_in_source()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/navigation-navigate-cross-document-does-not-dispatch-currententrychange-in-source",
        ))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navlog=\"before|after-call\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-entry-url=\"{}\"",
            server.url(
                "/compat/navigation-navigate-cross-document-does-not-dispatch-currententrychange-destination"
            )
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert_cross_document_navigation_destination_defaults(
        &page.serialize_html_async().await.unwrap(),
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-activation-entry-url=\"{}\"",
            server.url(
                "/compat/navigation-navigate-cross-document-does-not-dispatch-currententrychange-destination"
            )
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-activation-from-url=\"{}\"",
            server.url(
                "/compat/navigation-navigate-cross-document-does-not-dispatch-currententrychange-in-source"
            )
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-activation-navigation-type=\"push\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigation_entries_expose_current_entry_metadata_and_identity() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/navigation-entries-expose-current-entry-metadata-and-identity"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-type=\"object\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-entry-type=\"object\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-id-type=\"string\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-key-type=\"string\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-url-type=\"string\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-same-document-type=\"boolean\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-same-document=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-index=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-entries-type=\"object\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-entries-length=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-last-entry-same=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-last-entry-same-document=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-last-entry-url=\"{}\"",
                server.url("/compat/navigation-entries-expose-current-entry-metadata-and-identity")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-last-entry-index=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}
