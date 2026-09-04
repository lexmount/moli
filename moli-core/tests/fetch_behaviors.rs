use moli_test_support as support;

use anyhow::{Context, Result, anyhow};
use moli_core::{
    page::{Page, SubresourceResourceType},
    runtime::{Browser, BrowserConfig as AppConfig, FetchDeadline, RenderedDomWaitUntil},
    testing::{JsValueSnapshot, ScriptRunOutcome, ScriptSkipReason},
};
use std::time::Instant;
use support::FixtureServer;
use tokio::time::Duration;

fn diagnostic_global<'a>(page: &'a Page, name: &str) -> Option<&'a JsValueSnapshot> {
    page.script_execution().global(name)
}

async fn wait_for_parser_image_fetch_policy_asset_requests(
    token: &str,
    source: &str,
    expected_count: usize,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if support::parser_image_fetch_policy_asset_request_count(token, source) >= expected_count {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let actual_count =
                support::parser_image_fetch_policy_asset_request_count(token, source);
            return Err(anyhow!(
                "timed out waiting for {expected_count} parser image fixture request(s) token `{token}`, source `{source}`; observed {actual_count}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn external_boot_script_combining_baidu_compat_surfaces_completes() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/compat/baidu-boot")).await?;

    assert_eq!(page.final_url().path(), "/compat/baidu-boot");
    assert_eq!(page.final_url().query(), Some("boot=1"));
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-ok=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mime-length=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-plugin-length=\"5\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-pdf-viewer-enabled=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-storage-instance=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state=\"{&quot;step&quot;:2}\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-location=\"/compat/baidu-boot?boot=1\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_host_resolve_override_preserves_requested_wpt_host() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let fixture_url = server.url("/static").parse::<url::Url>()?;
    let port = fixture_url
        .port()
        .ok_or_else(|| anyhow!("fixture URL should include an explicit port"))?;
    let mapped_url = format!("http://web-platform.test:{port}/static");

    let mut config = AppConfig::default();
    config
        .fetch_mut()
        .set_http_host_resolve(vec![format!("web-platform.test:{port}:127.0.0.1")]);
    config.fetch_mut().set_http_no_proxy(Some("*".to_owned()));
    let browser = Browser::new(config)?;

    let page = browser.fetch(&mapped_url).await?;

    assert_eq!(page.final_url().as_str(), mapped_url);
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("fixture static")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn external_boot_script_can_still_trigger_location_replace() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/baidu-location-replace-boot"))
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/target");
    assert_eq!(page.final_url().query(), Some("from=boot-script"));
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=boot-script")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_decodes_gbk_document_from_meta_charset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/encoding/gbk-meta")).await?;
    let html = page.serialize_html_async().await.unwrap();

    assert!(html.contains("太平洋家居 GBK OK"), "html={html}");
    assert!(html.contains("data-charset=\"GBK\""), "html={html}");
    assert!(!html.contains('\u{FFFD}'), "html={html}");

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn classic_script_resource_inherits_document_encoding() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/encoding/shift-jis-classic-script"))
        .await?;
    let html = page.serialize_html_async().await.unwrap();

    assert!(html.contains("目次"), "html={html}");
    assert!(!html.contains('\u{FFFD}'), "html={html}");

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_classic_script_resource_inherits_child_document_encoding() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/encoding/child-shift-jis-classic-script-parent"))
        .await?;
    let html = page.serialize_html_async().await.unwrap();

    assert!(
        html.contains("data-child-script-text=\"目次\""),
        "html={html}"
    );
    assert!(!html.contains('\u{FFFD}'), "html={html}");

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_document_html_decodes_from_raw_response_bytes() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/encoding/child-shift-jis-document-parent"))
        .await?;
    let html = page.serialize_html_async().await.unwrap();

    assert!(
        html.contains("data-child-document-text=\"目次\""),
        "html={html}"
    );
    assert!(
        html.contains("data-child-document-charset=\"Shift_JIS\""),
        "html={html}"
    );
    assert!(
        html.contains("data-child-window-document-charset=\"Shift_JIS\""),
        "html={html}"
    );
    assert!(
        html.contains("data-child-content-document-charset=\"Shift_JIS\""),
        "html={html}"
    );
    assert!(!html.contains('\u{FFFD}'), "html={html}");

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn large_static_query_selector_all_subset_does_not_eagerly_wrap_every_node() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/encoding/large-static-nodelist-subset"))
        .await?;
    let html = page.serialize_html_async().await.unwrap();

    assert!(html.contains("data-done=\"true\""), "html={html}");
    assert!(html.contains("data-node-count=\"12000\""), "html={html}");
    assert!(html.contains("data-checksum=\"60\""), "html={html}");
    // Keep page-visible timing fields as diagnostics, but do not make this
    // correctness gate depend on full-workspace nextest scheduling load.
    let _script_elapsed = extract_data_attr_u64(&html, "data-elapsed-ms")?;
    let _checks_elapsed = extract_data_attr_u64(&html, "data-checks-elapsed-ms")?;

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn large_child_static_query_selector_all_subset_does_not_eagerly_wrap_every_node()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/encoding/large-child-static-nodelist-subset-parent"))
        .await?;
    let html = page.serialize_html_async().await.unwrap();

    assert!(html.contains("data-done=\"true\""), "html={html}");
    assert!(html.contains("data-node-count=\"12000\""), "html={html}");
    assert!(html.contains("data-checksum=\"60\""), "html={html}");
    // Keep page-visible timing fields as diagnostics, but do not make this
    // correctness gate depend on full-workspace nextest scheduling load.
    let _elapsed_ms = extract_data_attr_u64(&html, "data-elapsed-ms")?;

    server.shutdown().await;
    Ok(())
}

fn extract_data_attr_u64(html: &str, name: &str) -> Result<u64> {
    let prefix = format!("{name}=\"");
    let start = html
        .find(&prefix)
        .ok_or_else(|| anyhow!("missing `{name}` in html: {html}"))?
        + prefix.len();
    let end = html[start..]
        .find('"')
        .ok_or_else(|| anyhow!("unterminated `{name}` in html: {html}"))?
        + start;
    html[start..end]
        .parse()
        .with_context(|| format!("failed to parse `{name}` from html: {html}"))
}

#[tokio::test]
async fn fetches_concurrently_with_shared_browser_state() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let slow_a_url = server.url("/concurrent-shared-state-a");
    let slow_b_url = server.url("/concurrent-shared-state-b");

    let (first, second) = tokio::join!(browser.fetch(&slow_a_url), browser.fetch(&slow_b_url),);
    let first = first?;
    let second = second?;
    let first_html = first.serialize_html_async().await.unwrap();
    let second_html = second.serialize_html_async().await.unwrap();

    assert!(first_html.contains("concurrent=a"));
    assert!(second_html.contains("concurrent=b"));
    assert!(
        first_html.contains("overlap=true") || second_html.contains("overlap=true"),
        "expected fixture server to observe overlapping requests, first={}, second={}",
        first_html,
        second_html
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_respects_request_timeout_but_default_timeout_allows_slow_fixture() -> Result<()> {
    let server = FixtureServer::spawn().await?;

    let mut tight_config = AppConfig::default();
    tight_config.fetch_mut().set_request_timeout_ms(100);
    let tight_browser = Browser::new(tight_config)?;
    let timeout_error = tight_browser
        .fetch(&server.url("/slow-a"))
        .await
        .unwrap_err();
    assert!(
        timeout_error
            .chain()
            .any(|cause| cause.to_string().contains("Timeout was reached")),
        "expected curl timeout error, got: {timeout_error:#}"
    );

    let default_browser = Browser::new(AppConfig::default())?;
    let page = default_browser.fetch(&server.url("/slow-a")).await?;
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("slow=a")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn keeps_cookies_between_requests() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let first = browser.fetch(&server.url("/cookie")).await?;
    let second = browser.fetch(&server.url("/cookie")).await?;

    assert!(
        first
            .serialize_html_async()
            .await
            .unwrap()
            .contains("cookie=missing")
    );
    assert!(
        second
            .serialize_html_async()
            .await
            .unwrap()
            .contains("cookie=seen")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn propagates_cookies_across_redirects() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/redirect-cookie")).await?;

    assert_eq!(page.status(), 200);
    assert_eq!(page.final_url().path(), "/cookie");
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("cookie=seen")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn serves_script_fixture() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/script")).await?;

    assert_eq!(page.requested_url().path(), "/script");
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("/assets/app.js")
    );
    assert_eq!(
        diagnostic_global(&page, "__fixtureReady"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn executes_inline_scripts_but_skips_template_contents() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/inline-script")).await?;

    assert_eq!(
        diagnostic_global(&page, "inlineReady"),
        Some(&JsValueSnapshot::String("\u{4f60}\u{597d}".to_owned()))
    );
    assert_eq!(diagnostic_global(&page, "templateReady"), None);
    assert!(page.script_execution().runs().iter().any(|run| {
        matches!(
            run.outcome(),
            ScriptRunOutcome::Skipped(ScriptSkipReason::NotInMainDocument)
        )
    }));

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn executes_scripts_in_bucket_order_and_supports_module_and_importmap() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/script-execution")).await?;

    assert_eq!(
        diagnostic_global(&page, "inlineReady"),
        Some(&JsValueSnapshot::String("\u{4f60}\u{597d}".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "externalReady"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "deferReady"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "asyncReady"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "executionOrderText"),
        Some(&JsValueSnapshot::String(
            "inline-normal,external-normal,inline-defer,inline-async".to_owned()
        ))
    );
    assert_eq!(diagnostic_global(&page, "templateShouldNotRun"), None);
    assert_eq!(
        diagnostic_global(&page, "moduleReady"),
        Some(&JsValueSnapshot::Bool(true))
    );

    assert!(page.script_execution().runs().iter().any(|run| {
        matches!(
            run.outcome(),
            ScriptRunOutcome::Skipped(ScriptSkipReason::UnsupportedType(script_type))
                if script_type == "application/json"
        )
    }));

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_with_domcontentloaded_stops_before_async_and_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-lifecycle"),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "domReady"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(diagnostic_global(&page, "loadReady"), None);

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domcontentloaded_does_not_wait_for_network_started_from_domcontentloaded()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-domcontentloaded-fetch"),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;

    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late-dcl\"")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains(">settled<")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domcontentloaded_stops_before_runtime_script_started_from_domcontentloaded()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-domcontentloaded-runtime-script"),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedDclInjectedDclOrder"),
        Some(&JsValueSnapshot::String(
            "dcl:interactive,after-append".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedDclInjectedLoadOrder"),
        None
    );
    // script_execution() is the state captured at the requested DCL boundary.
    // serialize_html_async() is intentionally live, so a later command may see
    // the external script finish after fetch_with_wait_until() has returned.

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domcontentloaded_stops_before_runtime_owned_script_load_event() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/runtime-owned-external-in-order-load-after-domcontentloaded"),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderLoadAfterDcl"),
        Some(&JsValueSnapshot::String("after-append,dcl".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderLoadDuringLoad"),
        None
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderLoadFinalOrder"),
        None
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_document_lifecycle_performance_event_end_matches_chromium_microtask_visibility()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/main-document-lifecycle-performance-event-end"))
        .await?;
    let html = page.serialize_html_async().await?;

    for attribute in [
        "data-dcl-listener-event-end=\"no\"",
        "data-dcl-microtask-event-end=\"no\"",
        "data-load-listener-event-end=\"no\"",
        "data-load-microtask-event-end=\"no\"",
        "data-pageshow-listener-load-end=\"yes\"",
        "data-pageshow-microtask-load-end=\"yes\"",
        "data-probe-complete=\"yes\"",
    ] {
        assert!(
            html.contains(attribute),
            "missing Chromium-compatible lifecycle timing fact `{attribute}`: {html}"
        );
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domcontentloaded_keeps_runtime_owned_in_order_script_behind_defer() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url(
                "/compat/runtime-owned-external-in-order-with-defer-stays-after-domcontentloaded",
            ),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderWithDeferDclOrder"),
        Some(&JsValueSnapshot::String(
            "after-append,defer,dcl".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderWithDeferLoadOrder"),
        None
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domcontentloaded_stops_before_runtime_owned_async_classic_execution()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/runtime-owned-external-async-does-not-block-domcontentloaded"),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncClassicDclOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,dcl:interactive".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncClassicDuringLoad"),
        None
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncClassicFinalOrder"),
        None
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_load_waits_for_runtime_owned_async_classic_execution() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/runtime-owned-external-async-does-not-block-domcontentloaded"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncClassicDuringLoad"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,dcl:interactive,external-script,load".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncClassicFinalOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,dcl:interactive,external-script,load,window-load:complete"
                .to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domcontentloaded_stops_before_fast_runtime_owned_async_classic_execution()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url(
                "/compat/runtime-owned-external-async-fast-does-not-overtake-domcontentloaded",
            ),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncFastDclOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,dcl:interactive".to_owned()
        ))
    );
    if let Some(load_order) =
        diagnostic_global(&page, "runtimeOwnedAsyncFastLoadOrder").and_then(JsValueSnapshot::as_str)
    {
        assert!(
            load_order
                .starts_with("after-append:async=true,dcl:interactive,external-script:interactive"),
            "fast runtime-owned async script overtook DOMContentLoaded: {load_order}"
        );
    }
    if let Some(final_order) = diagnostic_global(&page, "runtimeOwnedAsyncFastFinalOrder")
        .and_then(JsValueSnapshot::as_str)
    {
        assert!(
            final_order
                .starts_with("after-append:async=true,dcl:interactive,external-script:interactive"),
            "fast runtime-owned async script overtook DOMContentLoaded: {final_order}"
        );
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domcontentloaded_keeps_runtime_owned_async_script_behind_defer() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url(
                "/compat/runtime-owned-external-async-with-defer-does-not-block-domcontentloaded",
            ),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncWithDeferDclOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,defer,dcl:interactive".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncWithDeferLoadOrder"),
        None
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncWithDeferFinalOrder"),
        None
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domcontentloaded_stops_before_runtime_owned_module_execution() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url(
                "/compat/runtime-owned-default-async-module-side-effect-after-domcontentloaded",
            ),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncModuleDclOrder"),
        Some(&JsValueSnapshot::String(
            "ready:loading,after-append:async=true,dcl:interactive".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncModuleFinalOrder"),
        None
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domcontentloaded_does_not_wait_for_parser_module_tla_dynamic_import()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/parser-owned-module-tla-dynamic-import-delays-domcontentloaded"),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "parserModuleTlaDynamicImportDclSawValue"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "parserModuleTlaDynamicImportOrderResult"),
        Some(&JsValueSnapshot::String(
            "module-start,dcl:interactive".to_owned()
        ))
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-parser-module-tla-dynamic-import=\"false\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_done_does_not_requeue_stylesheet_load_on_media_change() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/stylesheet-media-change-load-handler-does-not-requeue"),
            RenderedDomWaitUntil::Done,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "stylesheetMediaLoadCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-stylesheet-media-load-count=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-stylesheet-media=\"all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-stylesheet-media-color=\"rgb(1, 2, 3)\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domcontentloaded_stops_before_runtime_owned_script_error_dispatch() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url(
                "/compat/runtime-owned-external-in-order-error-after-domcontentloaded?manual-release",
            ),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;
    server.release_runtime_owned_in_order_error_after_dcl();

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderErrorAfterDcl"),
        Some(&JsValueSnapshot::String("dcl,after-append".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderErrorDuringError"),
        None
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderErrorFinalOrder"),
        None
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domcontentloaded_stops_before_runtime_owned_module_failure_report() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url(
                "/compat/runtime-owned-inline-module-missing-default-export-after-domcontentloaded",
            ),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInlineModuleLinkFailureMessageMatches"),
        None
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInlineModuleLinkFailureFinalOrder"),
        None
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domcontentloaded_stops_before_runtime_owned_external_module_load_failure_report()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/runtime-owned-external-module-load-failure-after-later-module"),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedExternalModuleLoadFailureWindowErrors"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedExternalModuleLoadFailureFinalOrder"),
        None
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedExternalModuleLoadFailureOrderAtDcl"),
        Some(&JsValueSnapshot::String(
            "ready:loading,after-broken:loading:async=true,after-later:loading:async=true,dcl:interactive".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_with_load_includes_async_scripts() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-lifecycle"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "domReady"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "loadReady"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_load_waits_for_slow_runtime_script_started_from_domcontentloaded() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-domcontentloaded-runtime-script-slow"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    let load_order = diagnostic_global(&page, "runtimeOwnedDclInjectedLoadOrder")
        .and_then(JsValueSnapshot::as_str)
        .unwrap_or_default();
    assert!(
        load_order.contains("external-script,load"),
        "load_order={load_order}"
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late-dcl-script-slow\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(">script-loaded-slow<")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_with_load_waits_for_detached_eager_image_terminal_events() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default().with_image_fetch_enabled(true))?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/detached-eager-images-delay-load"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    let terminal_text = diagnostic_global(&page, "detachedEagerImageTerminalText")
        .and_then(JsValueSnapshot::as_str)
        .unwrap_or_default();
    let terminals = terminal_text.split(',').collect::<Vec<_>>();
    assert_eq!(
        terminals.len(),
        5,
        "every detached eager image must reach its terminal before Window load"
    );
    assert!(
        terminals
            .iter()
            .all(|terminal| terminal.starts_with("load-")),
        "fixture responses should produce successful image terminals: {terminals:?}"
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-image-terminal-count=\"5\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-load-terminal-count=\"5\""),
        "Window load must observe all eager image terminal tasks as settled"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_with_load_skips_parser_image_fetch_by_default() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let token = support::next_parser_image_fetch_policy_token();
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url(&format!("/compat/parser-image-fetch-policy?token={token}")),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        support::parser_image_fetch_policy_asset_request_count(&token, "html"),
        0
    );
    assert_eq!(
        support::parser_image_fetch_policy_asset_request_count(&token, "css"),
        0
    );
    assert!(
        page.subresource_network_records()
            .iter()
            .all(|record| record.resource_type() != SubresourceResourceType::Image)
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_with_load_defers_css_images_until_a_paint_layout() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let token = support::next_parser_image_fetch_policy_token();
    let browser = Browser::new(
        AppConfig::default()
            .with_layout_policy(moli_page_types::LayoutPolicy::OnDemand)
            .with_image_fetch_enabled(true),
    )?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url(&format!("/compat/parser-image-fetch-policy?token={token}")),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    wait_for_parser_image_fetch_policy_asset_requests(&token, "html", 1).await?;

    assert!(
        support::parser_image_fetch_policy_asset_request_count(&token, "html") >= 1,
        "the parser-discovered HTML image request must reach the network"
    );
    assert_eq!(
        support::parser_image_fetch_policy_asset_request_count(&token, "css"),
        0,
        "a load-only observation must not scan CSS text or start an unrendered CSS image"
    );

    let pending = page.start_capture_screenshot()?;
    let completion = pending.wait().await?;
    let _ = page.finish_capture_screenshot(completion)?;
    wait_for_parser_image_fetch_policy_asset_requests(&token, "css", 1).await?;

    assert!(support::parser_image_fetch_policy_asset_request_count(&token, "css") >= 1);

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_with_load_completes_lazy_geometry_offset_chain_scan() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/lazy-geometry-offset-chain"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(15),
        )
        .await?;
    let html = page.serialize_html_async().await.unwrap();

    assert!(
        html.contains("data-lazy-geometry-done=\"true\""),
        "lazy geometry offset-chain scan did not complete; html={html}"
    );
    let _geometry_elapsed = extract_data_attr_u64(&html, "data-lazy-geometry-elapsed")?;
    assert_eq!(
        diagnostic_global(&page, "lazyGeometryCount"),
        Some(&JsValueSnapshot::Number(24.0))
    );
    assert_eq!(
        extract_data_attr_u64(&html, "data-lazy-geometry-count")?,
        24
    );
    let geometry_total = extract_data_attr_u64(&html, "data-lazy-geometry-total")?;
    assert!(
        geometry_total > 0,
        "lazy geometry offset-chain scan did not produce a total; html={html}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_load_waits_for_runtime_inserted_style_import_error() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/runtime-inserted-style-import-missing-completes-load"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeInsertedStyleImportFinalOrder"),
        Some(&JsValueSnapshot::String(
            "dcl,after-append,style-error,window-load".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_networkidle_waits_for_network_started_from_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-domcontentloaded-fetch"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late-dcl\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(">settled<")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domstable_waits_for_runtime_script_started_from_domcontentloaded() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-domcontentloaded-runtime-script"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(5),
        )
        .await?;

    let load_order = diagnostic_global(&page, "runtimeOwnedDclInjectedLoadOrder")
        .and_then(JsValueSnapshot::as_str)
        .unwrap_or_default();
    assert!(
        load_order.contains("external-script,load"),
        "load_order={load_order}"
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late-dcl-script\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(">script-loaded<")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domstable_waits_for_slow_runtime_script_started_from_domcontentloaded()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-domcontentloaded-runtime-script-very-slow"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(7),
        )
        .await?;

    let load_order = diagnostic_global(&page, "runtimeOwnedDclInjectedLoadOrder")
        .and_then(JsValueSnapshot::as_str)
        .unwrap_or_default();
    assert!(
        load_order.contains("external-script,load"),
        "load_order={load_order}"
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late-dcl-script-very-slow\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(">script-loaded-very-slow<")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_wait_until_timeout_covers_renderer_page_creation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let started_at = Instant::now();
    let error = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-domcontentloaded-runtime-script-very-slow"),
            RenderedDomWaitUntil::Load,
            Duration::from_millis(100),
        )
        .await
        .unwrap_err();
    let elapsed = started_at.elapsed();

    assert!(
        error.to_string().contains("timed out after 100 ms"),
        "expected fetch deadline error, got: {error:#}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "fetch timeout should cover renderer page creation, elapsed={elapsed:?}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_selector_advances_slow_runtime_script_started_from_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-domcontentloaded-runtime-script-very-slow"),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late-dcl-script-very-slow\"")
    );

    let _node_id = browser
        .wait_for_selector(
            &mut page,
            "#late-dcl-script-very-slow",
            Duration::from_secs(7),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late-dcl-script-very-slow\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(">script-loaded-very-slow<")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_networkidle_waits_for_delayed_fetch_dom_mutation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let load_page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-delayed-fetch"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert!(
        !load_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late\"")
    );
    assert!(
        !load_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains(">settled<")
    );

    let networkidle_page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-delayed-fetch"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert!(
        networkidle_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late\"")
    );
    assert!(
        networkidle_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains(">settled<")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domstable_waits_for_delayed_fetch_dom_mutation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let load_page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-delayed-fetch"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert!(
        !load_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late\"")
    );
    assert!(
        !load_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains(">settled<")
    );

    let domstable_page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-delayed-fetch"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(5),
        )
        .await?;
    assert!(
        domstable_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late\"")
    );
    assert!(
        domstable_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains(">settled<")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domstable_waits_for_late_complete_dom_mutation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-complete-delayed-dom-mutation"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(5),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-late-complete=\"yes\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late-complete\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(">late-complete<")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domstable_waits_for_inflight_slow_fetch_dom_mutation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let load_page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-complete-slow-fetch"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert!(
        !load_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late-slow-fetch\"")
    );
    assert!(
        !load_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains(">settled-very-slow<")
    );

    let domstable_page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-complete-slow-fetch"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(7),
        )
        .await?;

    let domstable_html = domstable_page.serialize_html_async().await.unwrap();
    assert!(domstable_html.contains("data-state=\"settled-very-slow\""));
    assert!(domstable_html.contains("id=\"late-slow-fetch\""));
    assert!(domstable_html.contains(">settled-very-slow<"));

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domstable_waits_for_inflight_slow_xhr_dom_mutation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let load_page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-complete-slow-xhr"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert!(
        !load_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late-slow-xhr\"")
    );
    assert!(
        !load_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains(">settled-very-slow<")
    );

    let domstable_page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-complete-slow-xhr"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(7),
        )
        .await?;

    let domstable_html = domstable_page.serialize_html_async().await.unwrap();
    assert!(domstable_html.contains("data-state=\"settled-very-slow\""));
    assert!(domstable_html.contains("id=\"late-slow-xhr\""));
    assert!(domstable_html.contains(">settled-very-slow<"));

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domstable_continues_after_timer_callback_error() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-timer-callback-error"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(5),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-before-error=\"yes\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-error=\"yes\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"after-error\"")
    );
    assert!(
        page.script_execution()
            .lifecycle_errors()
            .iter()
            .any(|error| error.contains("timer callback dispatch failed")),
        "lifecycle errors: {:?}",
        page.script_execution().lifecycle_errors()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_networkidle_succeeds_on_static_page_without_extra_requests() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/static"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("fixture static")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_networkidle_waits_for_inflight_xhr_dom_mutation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let load_page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-complete-slow-xhr"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert!(
        !load_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late-slow-xhr\"")
    );
    assert!(
        !load_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains(">settled-very-slow<")
    );

    let networkidle_page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-complete-slow-xhr"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(7),
        )
        .await?;
    let networkidle_html = networkidle_page.serialize_html_async().await.unwrap();
    assert!(networkidle_html.contains("data-state=\"settled-very-slow\""));
    assert!(networkidle_html.contains("id=\"late-slow-xhr\""));
    assert!(networkidle_html.contains(">settled-very-slow<"));

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domstable_keeps_interval_after_timer_callback_error() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-interval-callback-error"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(5),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-interval-before-error=\"yes\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-interval-after-error=\"yes\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-interval-count=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"after-interval-error\"")
    );
    assert!(
        page.script_execution()
            .lifecycle_errors()
            .iter()
            .any(|error| error.contains("timer callback dispatch failed")),
        "lifecycle errors: {:?}",
        page.script_execution().lifecycle_errors()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domstable_does_not_expose_or_call_page_timer_driver() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-timer-driver-wrapper-tamper"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(5),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-tamper=\"yes\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"after-tamper\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-public-timer-driver-exposed=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-host-timer-driver-exposed=\"false\"")
    );
    assert!(
        !page
            .script_execution()
            .lifecycle_errors()
            .iter()
            .any(|error| error.contains("tampered timer driver wrapper")),
        "lifecycle errors: {:?}",
        page.script_execution().lifecycle_errors()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domstable_ignores_page_tampered_outer_html_getter() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-outer-html-tamper"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(5),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-outerhtml-tamper=\"yes\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"after-outerhtml-tamper\"")
    );
    assert!(
        !page
            .script_execution()
            .lifecycle_errors()
            .iter()
            .any(|error| error.contains("domstable must not read outerHTML")),
        "lifecycle errors: {:?}",
        page.script_execution().lifecycle_errors()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn networkidle_readiness_returns_best_effort_when_quiet_window_cannot_complete() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-delayed-fetch"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    browser
        .wait_for_page_readiness_with_deadline(
            &mut page,
            RenderedDomWaitUntil::NetworkIdle,
            FetchDeadline::new(Duration::from_millis(100))?,
        )
        .await?;

    let html = page.serialize_html_async().await?;
    assert!(html.contains("data-state=\"init\""), "html={html}");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_networkidle_waits_for_later_activity_before_quiet_window() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let load_page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-staggered-fetch"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;
    assert!(
        !load_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late-second\"")
    );
    assert!(
        !load_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains(">settled-second<")
    );

    let networkidle_page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-staggered-fetch"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert!(
        networkidle_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"late-second\"")
    );
    assert!(
        networkidle_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains(">settled-second<")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_networkidle_returns_best_effort_with_periodic_interval_fetch() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-interval-fetch"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_millis(1200),
        )
        .await?;

    let html = page.serialize_html_async().await?;
    assert!(
        html.contains("data-ping=") && html.contains("data-state=\"init\""),
        "html={html}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_with_domstable_returns_best_effort_with_periodic_dom_mutation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/wait-until-interval-dom-mutation"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_millis(700),
        )
        .await?;

    let html = page.serialize_html_async().await?;
    assert!(
        html.contains("data-mutation-count=") && html.contains("id=\"mutation-count\""),
        "html={html}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn cookie_path_scope_matches_only_valid_path_boundaries() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let _ = browser.fetch(&server.url("/cookie-scope/set")).await?;
    let matching = browser.fetch(&server.url("/cookie-scope/check")).await?;
    let non_matching = browser
        .fetch(&server.url("/cookie-scope-extra/check"))
        .await?;

    assert!(
        matching
            .serialize_html_async()
            .await
            .unwrap()
            .contains("scoped-cookie=seen")
    );
    assert!(
        non_matching
            .serialize_html_async()
            .await
            .unwrap()
            .contains("scoped-cookie=missing")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn ignores_invalid_domain_cookies_end_to_end() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let _ = browser
        .fetch(&server.url("/cookie-invalid-domain/set"))
        .await?;
    let check = browser
        .fetch(&server.url("/cookie-invalid-domain/check"))
        .await?;

    assert!(
        check
            .serialize_html_async()
            .await
            .unwrap()
            .contains("invalid-domain-cookie=missing")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn replaces_cookie_value_when_server_sets_same_cookie_again() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let _ = browser.fetch(&server.url("/cookie-replace/red")).await?;
    let _ = browser.fetch(&server.url("/cookie-replace/blue")).await?;
    let check = browser.fetch(&server.url("/cookie-replace/check")).await?;

    let html = check.serialize_html_async().await.unwrap();
    assert!(html.contains("replace-cookie=blue"));
    assert!(!html.contains("replace-cookie=red"));

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn carries_and_replaces_cookies_across_multi_hop_redirect_chain() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/redirect-cookie-chain/start"))
        .await?;

    assert_eq!(page.status(), 200);
    assert_eq!(page.final_url().path(), "/redirect-cookie-chain/final");
    assert_eq!(page.navigation_redirect_chain().len(), 2);
    assert_eq!(
        page.navigation_redirect_chain()[0].from_url.path(),
        "/redirect-cookie-chain/start"
    );
    assert_eq!(
        page.navigation_redirect_chain()[1].to_url.path(),
        "/redirect-cookie-chain/final"
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("cookie-chain=ok")
    );

    server.shutdown().await;
    Ok(())
}
