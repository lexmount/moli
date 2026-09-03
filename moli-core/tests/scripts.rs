use moli_test_support as support;

use anyhow::Result;
use moli_core::{
    runtime::{Browser, BrowserConfig as AppConfig, RenderedDomWaitUntil},
    testing::JsValueSnapshot,
};
use support::FixtureServer;
use tokio::time::Duration;

fn diagnostic_global<'a>(
    page: &'a moli_core::page::Page,
    name: &str,
) -> Option<&'a JsValueSnapshot> {
    page.script_execution().global(name)
}

fn diagnostic_global_string<'a>(page: &'a moli_core::page::Page, name: &str) -> Option<&'a str> {
    match diagnostic_global(page, name) {
        Some(JsValueSnapshot::String(value)) => Some(value.as_str()),
        _ => None,
    }
}

fn assert_order_contains(order: &str, marker: &str) {
    assert!(
        order.split(',').any(|part| part == marker),
        "expected order to contain `{marker}`, got `{order}`"
    );
}

fn assert_order_before(order: &str, before: &str, after: &str) {
    let parts: Vec<_> = order.split(',').collect();
    let before_index = parts
        .iter()
        .position(|part| *part == before)
        .unwrap_or_else(|| panic!("expected order to contain `{before}`, got `{order}`"));
    let after_index = parts
        .iter()
        .position(|part| *part == after)
        .unwrap_or_else(|| panic!("expected order to contain `{after}`, got `{order}`"));
    assert!(
        before_index < after_index,
        "expected `{before}` before `{after}`, got `{order}`"
    );
}

#[tokio::test]
async fn parse_time_inline_classic_scripts_observe_partial_dom_during_initial_parse() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-inline-classic"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-inline-before-late=\"missing\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-inline-after-late=\"seen\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-inline-trace=\"inline-1\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_external_classic_scripts_observe_partial_dom_during_initial_parse() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-external-classic"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-external-before-late=\"missing\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-external-trace=\"external\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_external_script_src_uses_document_base_url() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/script-src-base-alpha/page"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "scriptSrcBaseResult"),
        Some(&JsValueSnapshot::String("beta".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_defer_classic_scripts_still_run_post_parse_before_domcontentloaded()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-defer-classic"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "parseTimeDeferSawTail"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parseTimeDeferSawDcl"),
        Some(&JsValueSnapshot::Bool(false))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_async_classic_scripts_can_run_during_parse_before_late_dom_and_domcontentloaded()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-async-classic"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "parseTimeAsyncSawTail"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parseTimeAsyncSawDcl"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("<div id=\"late\">late</div>"),
        "late DOM after the async handoff must remain visible in the final page HTML"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_async_classic_scripts_can_run_between_parser_chunks_before_late_dom()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-async-classic-chunked"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "parseTimeAsyncSawTail"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "parseTimeAsyncSawDcl"),
        Some(&JsValueSnapshot::Bool(false))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_async_slow_classic_script_does_not_block_later_streaming_chunks() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-async-classic-slow-chunked-tail"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "slowChunkDclOrder"),
        Some(&JsValueSnapshot::String("defer,dcl".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "slowChunkFinalOrder"),
        Some(&JsValueSnapshot::String("defer,dcl,slow-async".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_async_classic_scripts_can_run_inside_single_outer_chunk_via_parse_pump()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-async-classic-pumped"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "parseTimeAsyncSawTail"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parseTimeAsyncSawDcl"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_slow_async_classic_scripts_can_remain_after_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-async-classic-slow"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "parseTimeAsyncSlowSawTail"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parseTimeAsyncSlowSawDcl"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_async_classic_scripts_keep_microtasks_between_page_task_turns() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-async-classic-task-turns"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "asyncTaskOrderResult"),
        Some(&JsValueSnapshot::String(
            "first-script,first-microtask,second-script".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_async_classic_first_task_turn_stays_before_late_dom_and_domcontentloaded()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-async-classic-task-turn-visibility"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "asyncTaskVisibilityOrderResult"),
        Some(&JsValueSnapshot::String(
            "first-script,first-microtask,second-script".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "asyncTaskVisibilityFirstSawTail"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "asyncTaskVisibilityFirstSawDcl"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_async_classic_post_parse_turns_keep_microtasks_and_stay_before_dcl()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-async-classic-post-parse-turns"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "postParseAsyncTaskOrderResult"),
        Some(&JsValueSnapshot::String("dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_async_classic_fast_vs_slow_post_parse_claims_split_at_dcl_boundary()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let fast_page = browser
        .fetch(&server.url("/compat/parse-time-async-classic-post-parse-turns"))
        .await?;
    let slow_page = browser
        .fetch(&server.url("/compat/parse-time-async-classic-post-parse-slow-second"))
        .await?;

    assert_eq!(
        diagnostic_global(&fast_page, "postParseAsyncTaskOrderResult"),
        Some(&JsValueSnapshot::String("dcl".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&slow_page, "postParseSlowDclOrder"),
        Some(&JsValueSnapshot::String("dcl".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&slow_page, "postParseSlowFinalOrder"),
        Some(&JsValueSnapshot::String(
            "dcl,first-script,first-microtask,second-script,second-after-dcl".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_async_classic_post_parse_slow_second_falls_back_after_dcl() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-async-classic-post-parse-slow-second"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "postParseSlowDclOrder"),
        Some(&JsValueSnapshot::String("dcl".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "postParseSlowFinalOrder"),
        Some(&JsValueSnapshot::String(
            "dcl,first-script,first-microtask,second-script,second-after-dcl".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_defer_and_module_scripts_share_one_document_ordered_phase() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-defer-module-order"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "deferLikeDclOrder"),
        Some(&JsValueSnapshot::String(
            "defer-left,defer-left-microtask,module,module-microtask,defer-right,defer-right-microtask,dcl".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn final_parser_classic_terminal_follows_script_reaction_and_precedes_dcl() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-final-classic-terminal-before-dcl"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "finalClassicTerminalDclOrder"),
        Some(&JsValueSnapshot::String(
            "script-body,script-body-microtask,script-load,script-load-microtask,dcl".to_owned()
        )),
        "classic evaluation reactions must settle before its terminal body, and the terminal task must settle before exact DCL"
    );
    assert_eq!(
        diagnostic_global(&page, "finalClassicTerminalTimerAtDcl"),
        Some(&JsValueSnapshot::Bool(false)),
        "ordinary timer arbitration must not open between parser completion and exact DCL"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn final_parser_module_terminal_microtask_precedes_dcl_without_timer_arbitration()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-final-module-terminal-before-dcl"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "finalModuleTerminalDclOrder"),
        Some(&JsValueSnapshot::String(
            "module-body,module-body-microtask,module-load,module-load-microtask,dcl".to_owned()
        )),
        "the final parser module task must finish its terminal reactions before exact DCL"
    );
    assert_eq!(
        diagnostic_global(&page, "finalModuleTerminalTimerAtDcl"),
        Some(&JsValueSnapshot::Bool(false)),
        "ordinary timer arbitration must not open between parser completion and exact DCL"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_blocking_external_script_waits_for_blocking_stylesheet() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/blocking-stylesheet-parser-blocking-external"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetParserBlockedEnough"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetParserSawLate"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetParserSawDcl"),
        Some(&JsValueSnapshot::Bool(false))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_blocking_external_document_write_keeps_insertion_point_after_blocking_stylesheet()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/blocking-stylesheet-parser-blocking-document-write"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetParserWriteBlockedEnough"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetParserWriteSawLate"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetParserWriteResult"),
        Some(&JsValueSnapshot::String(
            "ran|no-late|written-during-script|before-late".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn defer_script_waits_for_blocking_stylesheet_before_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/blocking-stylesheet-defer"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetDeferBlockedEnough"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetDeferSawLate"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetDeferSawDcl"),
        Some(&JsValueSnapshot::Bool(false))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_script_waits_for_blocking_stylesheet_before_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/blocking-stylesheet-module"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetModuleBlockedEnough"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetModuleSawLate"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetModuleSawDcl"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetModuleDclOrder"),
        Some(&JsValueSnapshot::String(
            "module,module-microtask,dcl".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn phase_two_custom_element_upgrade_stylesheet_does_not_block_parser_owned_defer()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/phase-two-upgrade-runtime-style-defer"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "phaseTwoUpgradeDeferSawLoaded"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "phaseTwoUpgradeDeferSawLate"),
        Some(&JsValueSnapshot::Bool(true))
    );
    match diagnostic_global(&page, "phaseTwoUpgradeDeferDclOrder") {
        Some(JsValueSnapshot::String(order)) => {
            assert!(order.starts_with("connected,defer"));
        }
        other => panic!("unexpected defer order snapshot: {other:?}"),
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn phase_two_custom_element_upgrade_stylesheet_does_not_block_parser_owned_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/phase-two-upgrade-runtime-style-module"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "phaseTwoUpgradeModuleSawLoaded"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "phaseTwoUpgradeModuleSawLate"),
        Some(&JsValueSnapshot::Bool(true))
    );
    match diagnostic_global(&page, "phaseTwoUpgradeModuleDclOrder") {
        Some(JsValueSnapshot::String(order)) => {
            assert!(order.starts_with("connected,module,module-microtask"));
        }
        other => panic!("unexpected module order snapshot: {other:?}"),
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_blocking_external_script_waits_for_parser_created_style_import() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/blocking-stylesheet-parser-created-style-import-parser-blocking-external",
        ))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "blockingStyleImportParserBlockedEnough"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStyleImportParserSawLate"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStyleImportParserSawDcl"),
        Some(&JsValueSnapshot::Bool(false))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_script_waits_for_parser_created_style_import_before_domcontentloaded() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/blocking-stylesheet-parser-created-style-import-module"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "blockingStyleImportModuleBlockedEnough"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStyleImportModuleSawLate"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStyleImportModuleSawDcl"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStyleImportModuleDclOrder"),
        Some(&JsValueSnapshot::String(
            "module,module-microtask,dcl".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn domcontentloaded_is_indirectly_delayed_by_blocking_stylesheet_gated_defer_work()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/blocking-stylesheet-defer"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetDeferDclBlockedEnough"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetDeferDclOrder"),
        Some(&JsValueSnapshot::String("defer,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn alternate_stylesheet_does_not_block_parser_blocking_script() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/blocking-stylesheet-alternate-non-blocking"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetAlternateBlocked"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetAlternateSawLate"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "blockingStylesheetAlternateSawDcl"),
        Some(&JsValueSnapshot::Bool(false))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_inserted_stylesheet_load_waits_for_fetch_completion() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/runtime-inserted-stylesheet-load"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeInsertedStylesheetLoadBlockedEnough"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeInsertedStylesheetLoadFinalOrder"),
        Some(&JsValueSnapshot::String("dcl,style-load".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_inserted_stylesheet_load_mutation_syncs_back_into_parser_before_later_script()
-> Result<()> {
    // Runtime-inserted stylesheets are intentionally not parser/script-blocking
    // stylesheets under HTML's parser-created stylesheet rule. The fixture keeps
    // the runtime stylesheet fast and the parser-owned stylesheet slow, so this
    // test covers only the renderer/parser snapshot boundary: a mutation made by
    // an already-fired runtime stylesheet load handler must be visible before
    // the later parser-inserted script executes.
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/runtime-inserted-stylesheet-load-syncs-parser-snapshot"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeInsertedStyleSnapshotSawMarker"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeInsertedStyleSnapshotSawLate"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeInsertedStyleSnapshotParserSawDcl"),
        Some(&JsValueSnapshot::Bool(false))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_inserted_stylesheet_load_can_trigger_location_replace() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/runtime-inserted-stylesheet-load-triggers-location-replace"))
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/target");
    assert_eq!(page.final_url().query(), Some("from=style-load"));
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=style-load")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_inserted_stylesheet_href_mutation_uses_fresh_fetch_for_parser_blocking_progress()
-> Result<()> {
    // The stale stylesheet response is gated until the following parser script
    // requests its probe. The probe reports whether it arrived before the
    // stale-response fallback fired, so this checks parser progress without a
    // wall-clock threshold in page script.
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/runtime-inserted-stylesheet-href-mutation-uses-fresh-fetch"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeInsertedHrefMutationParserNotBlocked"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeInsertedHrefMutationSawLate"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert!(
        page.subresource_network_records().iter().any(|record| {
            record.url().path() == "/assets/runtime_inserted_stylesheet_href_mutation_fresh.css"
        }),
        "the retargeted stylesheet must start the fresh fetch"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_inserted_preload_and_modulepreload_do_not_overtake_later_parser_script()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/runtime-inserted-preload-and-modulepreload-parser-progress"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimePreloadProgressParserSawLate"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimePreloadProgressParserSawPreloadLoad"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimePreloadProgressParserSawModulepreloadLoad"),
        Some(&JsValueSnapshot::Bool(false))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_module_graph_waits_for_modulepreloaded_static_dependency() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/modulepreload-shared-static-dependency"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "modulepreloadSharedError"),
        Some(&JsValueSnapshot::String(String::new()))
    );
    assert_eq!(
        diagnostic_global(&page, "modulepreloadSharedValue"),
        Some(&JsValueSnapshot::String("leaf-ok".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "modulepreloadSharedSawLeafBeforeMid"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_order_contains(
        diagnostic_global_string(&page, "modulepreloadSharedFinalOrder").unwrap_or(""),
        "root",
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_module_graph_waits_for_duplicate_modulepreloaded_static_dependency() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/modulepreload-duplicate-shared-static-dependency"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "modulepreloadDuplicateError"),
        Some(&JsValueSnapshot::String(String::new()))
    );
    assert_eq!(
        diagnostic_global(&page, "modulepreloadDuplicateValue"),
        Some(&JsValueSnapshot::String("shared-ok|shared-ok".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "modulepreloadDuplicateLeafEvalCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    let final_order =
        diagnostic_global_string(&page, "modulepreloadDuplicateFinalOrder").unwrap_or("");
    assert_order_before(final_order, "leaf", "parent-a");
    assert_order_before(final_order, "leaf", "parent-b");
    assert_order_before(final_order, "parent-a", "root");
    assert_order_before(final_order, "parent-b", "root");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_parser_module_root_evaluates_once_but_each_script_finishes() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/duplicate-module-root-eval"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "duplicateModuleRootError"),
        Some(&JsValueSnapshot::String(String::new()))
    );
    assert_eq!(
        diagnostic_global(&page, "duplicateModuleRootEvalCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    let final_order =
        diagnostic_global_string(&page, "duplicateModuleRootFinalOrder").unwrap_or("");
    assert_order_contains(final_order, "root");
    assert_order_contains(final_order, "root-microtask");
    assert_order_contains(final_order, "load-a");
    assert_order_contains(final_order, "load-b");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_parser_module_root_with_nested_dependencies_evaluates_once() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/duplicate-module-root-with-nested-dependencies"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "duplicateNestedModuleFinalLog"),
        Some(&JsValueSnapshot::String(
            "this-undefined,this-nested".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_top_level_fetch_and_mime_errors_dispatch_script_error() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-top-level-fetch-and-mime-errors"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "moduleTopLevelFailureFinalEvents"),
        Some(&JsValueSnapshot::String(
            "missing-error,mime-error,later-module".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_module_graph_waits_for_modulepreloaded_reused_parent_pending_child() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/modulepreload-reused-parent-pending-child"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "modulepreloadReusedParentError"),
        Some(&JsValueSnapshot::String(String::new()))
    );
    assert_eq!(
        diagnostic_global(&page, "modulepreloadReusedParentValue"),
        Some(&JsValueSnapshot::String("child-ok".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "modulepreloadReusedParentSawChildBeforeParent"),
        Some(&JsValueSnapshot::Bool(true))
    );
    let final_order =
        diagnostic_global_string(&page, "modulepreloadReusedParentFinalOrder").unwrap_or("");
    assert_order_before(final_order, "child", "parent");
    assert_order_before(final_order, "parent", "root");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn modulepreloaded_css_module_feeds_declarative_shadow_adopted_stylesheets() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/declarative-shadow-adopted-stylesheets-modulepreload"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(3),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "shadowModulepreloadResult"),
        Some(&JsValueSnapshot::String(
            "1|1|1|span { color: blue; }|true|rgb(0, 0, 255)|rgb(0, 0, 255)".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn css_dynamic_import_fills_declarative_shadow_adopted_stylesheet_placeholder() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/declarative-shadow-adopted-stylesheets-dynamic-import"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(3),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "shadowCssImportResult"),
        Some(&JsValueSnapshot::String(
            "true|true|1|span { color: blue; }|rgb(0, 0, 255)".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn phase_two_shared_blocking_stylesheet_keeps_parser_owned_defer_phase_order() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/phase-two-shared-blocking-stylesheet-defer"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "sharedBlockingDeferBlockedEnough"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "sharedBlockingDeferOrderResult"),
        Some(&JsValueSnapshot::String(
            "defer-left,defer-left-microtask,defer-right,defer-right-microtask,dcl".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn phase_two_shared_blocking_stylesheet_keeps_parser_owned_module_phase_order() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/phase-two-shared-blocking-stylesheet-module"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "sharedBlockingModuleBlockedEnough"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "sharedBlockingModuleOrderResult"),
        Some(&JsValueSnapshot::String(
            "module-left,module-left-microtask,module-right,module-right-microtask,dcl".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_script_does_not_wait_for_runtime_inserted_blocking_stylesheet() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-script-waits-for-runtime-inserted-stylesheet"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "dynamicBlockingStylesheetSawStyleLoaded"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicBlockingStylesheetSawLate"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicBlockingStylesheetSawDcl"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicBlockingStylesheetSawLoad"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicBlockingStylesheetFinalOrder"),
        Some(&JsValueSnapshot::String("dcl,dynamic,load".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_async_script_can_overtake_slower_in_order_script() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-script-async-overtakes-in-order"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicScriptTaxonomyOrderResult"),
        Some(&JsValueSnapshot::String(
            "async-fast,in-order-slow".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_in_order_scripts_keep_append_order_when_front_is_slow() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-script-in-order-preserves-order"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicInOrderScriptOrderResult"),
        Some(&JsValueSnapshot::String(
            "in-order-slow,in-order-fast".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicInOrderScriptFinalOrder"),
        Some(&JsValueSnapshot::String(
            "in-order-slow,in-order-fast,window-load".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_dynamic_script_load_waits_until_parser_progress_restores_state() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-dynamic-script-load-after-parser-progress"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "parseTimeDynamicRestored"),
        Some(&JsValueSnapshot::String("restored".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parseTimeDynamicSaw"),
        Some(&JsValueSnapshot::String("restored".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parseTimeDynamicUnsafeMarker"),
        Some(&JsValueSnapshot::String("restored".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parseTimeDynamicRestoreOrder"),
        Some(&JsValueSnapshot::String(
            "clobber-script,restore-inline".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parseTimeDynamicFinalOrder"),
        Some(&JsValueSnapshot::String(
            "clobber-script,restore-inline,dynamic-script,dynamic-load".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parseTimeDynamicLastError"),
        Some(&JsValueSnapshot::String(String::new()))
    );
    assert!(
        !page
            .script_execution()
            .lifecycle_errors()
            .iter()
            .any(|message| message.contains("invokeApps")
                || message.contains("Cannot read properties of undefined")),
        "unexpected lifecycle errors: {:?}",
        page.script_execution().lifecycle_errors()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_dynamic_script_error_waits_until_parser_progress_restores_state() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-dynamic-script-error-after-parser-progress"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "parseTimeDynamicErrorRestored"),
        Some(&JsValueSnapshot::String("restored".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parseTimeDynamicErrorSaw"),
        Some(&JsValueSnapshot::String("restored".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parseTimeDynamicErrorRestoreOrder"),
        Some(&JsValueSnapshot::String(
            "clobber-script,restore-inline".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parseTimeDynamicErrorFinalOrder"),
        Some(&JsValueSnapshot::String(
            "clobber-script,restore-inline,dynamic-error".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_inserted_inline_script_does_not_dispatch_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/runtime-inserted-inline-script-does-not-dispatch-load"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeInlineLoadSaw"),
        Some(&JsValueSnapshot::String("not-fired".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeInlineLoadOrderResult"),
        Some(&JsValueSnapshot::String(
            "script-run,after-append".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_external_script_load_fires_before_later_restore_inline() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-external-script-load-after-page-task-turn"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWritePageTaskRestored"),
        Some(&JsValueSnapshot::String("restored".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWritePageTaskSaw"),
        Some(&JsValueSnapshot::String("missing".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWritePageTaskRestoreOrder"),
        Some(&JsValueSnapshot::String(
            "written-script,written-load,restore-inline".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWritePageTaskFinalOrder"),
        Some(&JsValueSnapshot::String(
            "written-script,written-load".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_external_script_delayed_load_does_not_block_parent_runtime() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page =
        browser
            .fetch(&server.url(
                "/compat/document-write-delayed-external-script-does-not-block-parent-runtime",
            ))
            .await?;

    let Some(JsValueSnapshot::Number(elapsed)) =
        diagnostic_global(&page, "documentWriteDelayedAfterElapsed")
    else {
        panic!("missing documentWriteDelayedAfterElapsed diagnostic");
    };
    assert!(
        *elapsed < 60.0,
        "document.write returned after {elapsed}ms; delayed script load should not block parent JS"
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteDelayedAfterOrder"),
        Some(&JsValueSnapshot::String("outer-after-write".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteDelayedExternalSawOuter"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteDelayedRestoreSawExternal"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteDelayedRestoreOrder"),
        Some(&JsValueSnapshot::String(
            "outer-after-write,external-script,external-load,restore-inline".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_open_during_parser_script_with_pending_written_external_is_ignored() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/document-open-during-parser-script-with-pending-written-external-is-ignored",
        ))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"written-before-ignored-open\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"written-after-ignored-open\"")
    );
    assert_eq!(
        diagnostic_global(&page, "documentOpenParserExternalRan"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "documentOpenParserInlineRan"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "documentOpenParserOrderResult"),
        Some(&JsValueSnapshot::String(
            "after-schedule,external-script,written-inline".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_connected_external_classic_dispatches_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parser-connected-external-classic-dispatches-load"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "parserConnectedLoadSaw"),
        Some(&JsValueSnapshot::String("fired".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserConnectedLoadFinalOrder"),
        Some(&JsValueSnapshot::String("external-load".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserConnectedLoadAfterExternalOrder"),
        Some(&JsValueSnapshot::String(
            "external-load,after-external".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_connected_external_classic_load_document_write_uses_insertion_point() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page =
        browser
            .fetch(&server.url(
                "/compat/parser-connected-external-classic-load-document-write-insertion-point",
            ))
            .await?;

    assert_eq!(
        diagnostic_global(&page, "parserConnectedLoadWriteText"),
        Some(&JsValueSnapshot::String("Some text".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserConnectedLoadWriteReadyState"),
        Some(&JsValueSnapshot::String("loading".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_connected_external_classic_load_document_write_notifies_parent() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page =
        browser
            .fetch(&server.url(
                "/compat/parser-connected-external-classic-load-document-write-parent-callback",
            ))
            .await?;

    assert_eq!(
        diagnostic_global(&page, "parserConnectedFrameLoadWriteObservedText"),
        Some(&JsValueSnapshot::String("Some text".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserConnectedFrameLoadWriteObservedCallbackType"),
        Some(&JsValueSnapshot::String("function".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserConnectedFrameLoadWriteCall"),
        Some(&JsValueSnapshot::String("called".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserConnectedFrameLoadWriteText"),
        Some(&JsValueSnapshot::String("Some text".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_connected_external_classic_error_microtask_precedes_later_inline() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parser-connected-external-classic-error-microtask"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "parserConnectedErrorSaw"),
        Some(&JsValueSnapshot::String("fired".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserConnectedErrorDuring"),
        Some(&JsValueSnapshot::String("error".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserConnectedErrorAfterInline"),
        Some(&JsValueSnapshot::String(
            "error,error-microtask,after-inline".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parserConnectedErrorFinalOrder"),
        Some(&JsValueSnapshot::String(
            "error,error-microtask,after-inline".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_connected_external_classic_unknown_scheme_error_document_write_continues()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page =
        browser
            .fetch(&server.url(
                "/compat/parser-connected-external-classic-unknown-scheme-errors-and-continues",
            ))
            .await?;

    assert_eq!(
        diagnostic_global(&page, "parserConnectedUnknownSchemeFinalOrder"),
        Some(&JsValueSnapshot::String(
            "unknown-error,unknown-written,missing-error,missing-written,after-inline".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parserConnectedUnknownSchemeUnknownSawFlag"),
        Some(&JsValueSnapshot::String("false".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserConnectedUnknownSchemeMissingSawFlag"),
        Some(&JsValueSnapshot::String("true".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserConnectedUnknownSchemeFinalReadyStates"),
        Some(&JsValueSnapshot::String(
            "unknown-error:loading,unknown-written:loading,missing-error:loading,missing-written:loading".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parserConnectedUnknownSchemeScriptCount"),
        Some(&JsValueSnapshot::Number(6.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_connected_inline_classic_does_not_dispatch_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parser-connected-inline-classic-does-not-dispatch-load"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "parserInlineLoadSaw"),
        Some(&JsValueSnapshot::String("fired".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserInlineLoadAttributeTarget"),
        Some(&JsValueSnapshot::String("dynamic".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserInlineLoadFinalTargets"),
        Some(&JsValueSnapshot::String("dynamic".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_external_defer_dispatches_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parser-owned-external-defer-dispatches-load"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "parserOwnedDeferLoadSaw"),
        Some(&JsValueSnapshot::String("fired".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedDeferLoadFinalOrder"),
        Some(&JsValueSnapshot::String(
            "inline-defer,external-load".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedDeferLoadAfterInlineOrder"),
        Some(&JsValueSnapshot::String("inline-defer".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_external_async_dispatches_load_after_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parser-owned-external-async-dispatches-load"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "parserOwnedAsyncLoadSaw"),
        Some(&JsValueSnapshot::String("fired".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedAsyncLoadSawTail"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedAsyncLoadSawDcl"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedAsyncLoadAfterDclOrder"),
        Some(&JsValueSnapshot::String("dcl".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedAsyncLoadFinalOrder"),
        Some(&JsValueSnapshot::String(
            "dcl,async-script,script-load".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedAsyncLoadWindowOrder"),
        Some(&JsValueSnapshot::String(
            "dcl,async-script,script-load,window-load".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_owned_external_in_order_load_dispatches_after_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/runtime-owned-external-in-order-load-after-domcontentloaded"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderLoadAfterDcl"),
        Some(&JsValueSnapshot::String("after-append,dcl".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderLoadDuringLoad"),
        Some(&JsValueSnapshot::String(
            "after-append,dcl,external-script,load".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderLoadFinalOrder"),
        Some(&JsValueSnapshot::String(
            "after-append,dcl,external-script,load,load-microtask,window-load".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_owned_external_in_order_error_dispatches_after_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/runtime-owned-external-in-order-error-after-domcontentloaded"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderErrorAfterDcl"),
        Some(&JsValueSnapshot::String("dcl,after-append".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderErrorDuringError"),
        Some(&JsValueSnapshot::String(
            "dcl,after-append,error".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderErrorFinalOrder"),
        Some(&JsValueSnapshot::String(
            "dcl,after-append,error,error-microtask,window-load".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_owned_external_in_order_with_defer_still_executes_after_domcontentloaded()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page =
        browser
            .fetch(&server.url(
                "/compat/runtime-owned-external-in-order-with-defer-stays-after-domcontentloaded",
            ))
            .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderWithDeferAfterDeferOrder"),
        Some(&JsValueSnapshot::String("after-append,defer".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderWithDeferDclOrder"),
        Some(&JsValueSnapshot::String(
            "after-append,defer,dcl".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderWithDeferLoadOrder"),
        Some(&JsValueSnapshot::String(
            "after-append,defer,dcl,external-script,load".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderWithDeferFinalOrder"),
        Some(&JsValueSnapshot::String(
            "after-append,defer,dcl,external-script,load,window-load".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_owned_external_in_order_from_domcontentloaded_handler_stays_after_domcontentloaded()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/runtime-owned-external-in-order-from-domcontentloaded-handler"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderFromDclAfterDclOrder"),
        Some(&JsValueSnapshot::String("dcl,after-append".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderFromDclLoadOrder"),
        Some(&JsValueSnapshot::String(
            "dcl,after-append,external-script,load,load-microtask".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInOrderFromDclFinalOrder"),
        Some(&JsValueSnapshot::String(
            "dcl,after-append,external-script,load,load-microtask,window-load".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_owned_external_async_does_not_block_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/runtime-owned-external-async-does-not-block-domcontentloaded"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncClassicDclOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,dcl:interactive".to_owned()
        ))
    );
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
async fn runtime_owned_external_async_fast_does_not_overtake_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page =
        browser
            .fetch(&server.url(
                "/compat/runtime-owned-external-async-fast-does-not-overtake-domcontentloaded",
            ))
            .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncFastDclOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,dcl:interactive".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncFastLoadOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,dcl:interactive,external-script:interactive,load:interactive"
                .to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncFastFinalOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,dcl:interactive,external-script:interactive,load:interactive,window-load:complete"
                .to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_owned_external_async_can_run_before_a_delayed_streaming_tail() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/runtime-owned-external-async-fast-streaming-tail"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncFastDclOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,external-script:loading,load:loading,dcl:interactive"
                .to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncFastFinalOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,external-script:loading,load:loading,dcl:interactive,window-load:complete"
                .to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_owned_inline_module_single_line_import_executes() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/runtime-owned-inline-module-single-line-import-executes"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeInlineModuleErrors"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    let final_order = match diagnostic_global(&page, "runtimeInlineModuleFinalOrder") {
        Some(JsValueSnapshot::String(order)) => order.as_str(),
        other => panic!("runtime inline module should publish its final order, got {other:?}"),
    };
    assert!(
        matches!(
            final_order,
            "ready:loading,after-append,dcl:interactive,module:1,module-microtask,window-load:complete"
                | "ready:loading,after-append,module:1,module-microtask,dcl:interactive,window-load:complete"
        ),
        "runtime-created async module and DOMContentLoaded may race, but both must finish before Window load: {final_order}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_owned_inline_module_runs_while_parser_defer_is_blocked() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server.url("/compat/runtime-owned-inline-module-runs-while-parser-defer-is-blocked"),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeModuleBlockedDeferFinalOrder"),
        Some(&JsValueSnapshot::String(
            "ready:loading,after-append,module:interactive,module-microtask,defer,dcl:interactive,window-load:complete"
                .to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_owned_external_async_with_defer_does_not_block_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page =
        browser
            .fetch(&server.url(
                "/compat/runtime-owned-external-async-with-defer-does-not-block-domcontentloaded",
            ))
            .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncWithDeferAfterDeferOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,defer".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncWithDeferDclOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,defer,dcl:interactive".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncWithDeferLoadOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,defer,dcl:interactive,external-script,load".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedAsyncWithDeferFinalOrder"),
        Some(&JsValueSnapshot::String(
            "after-append:async=true,defer,dcl:interactive,external-script,load,window-load:complete"
                .to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_owned_default_async_module_side_effect_races_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page =
        browser
            .fetch(&server.url(
                "/compat/runtime-owned-default-async-module-side-effect-after-domcontentloaded",
            ))
            .await?;

    let final_order = match diagnostic_global(&page, "runtimeOwnedAsyncModuleFinalOrder") {
        Some(JsValueSnapshot::String(order)) => order.as_str(),
        other => panic!("runtime external module should publish its final order, got {other:?}"),
    };
    assert!(
        matches!(
            final_order,
            "ready:loading,after-append:async=true,dcl:interactive,module,module-microtask,window-load:complete"
                | "ready:loading,after-append:async=true,module,module-microtask,dcl:interactive,window-load:complete"
        ),
        "runtime-created async module and DOMContentLoaded may race, but both must finish before Window load: {final_order}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_owned_inline_module_missing_default_export_reports_window_error_after_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/runtime-owned-inline-module-missing-default-export-after-domcontentloaded",
        ))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInlineModuleLinkFailureMessageMatches"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedInlineModuleLinkFailureFinalOrder"),
        Some(&JsValueSnapshot::String(
            "ready:loading,after-append:loading:async=true,dcl:interactive,later-module,window-error,window-error-microtask,window-load:complete".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_owned_external_module_load_failure_reports_script_error_after_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/runtime-owned-external-module-load-failure-after-later-module"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedExternalModuleLoadFailureWindowErrors"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "runtimeOwnedExternalModuleLoadFailureFinalOrder"),
        Some(&JsValueSnapshot::String(
            "ready:loading,after-broken:loading:async=true,after-later:loading:async=true,dcl:interactive,later-module,later-module-microtask,script-error,script-error-microtask,window-load:complete".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_external_script_error_fires_before_later_restore_inline() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-external-script-error-after-page-task-turn"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteErrorPageTaskRestored"),
        Some(&JsValueSnapshot::String("restored".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteErrorPageTaskSaw"),
        Some(&JsValueSnapshot::String("initial".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteErrorPageTaskRestoreOrder"),
        Some(&JsValueSnapshot::String(
            "written-error,restore-inline".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteErrorPageTaskFinalOrder"),
        Some(&JsValueSnapshot::String("written-error".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_script_type_mutation_from_connected_datablock_remains_inert() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-script-type-mutation-remains-inert"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicTypeMutationBefore"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicTypeMutationRuns"),
        Some(&JsValueSnapshot::Number(0.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_script_error_does_not_abort_later_in_order_work() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-script-error-does-not-abort-queue"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicScriptErrorCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicScriptErrorDocumentCapture"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicScriptErrorOrderResult"),
        Some(&JsValueSnapshot::String("error,load".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicInOrderScriptOrderResult"),
        Some(&JsValueSnapshot::String("in-order-fast".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_script_preparation_context_stays_in_old_document() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/dynamic-script-preparation-context-stays-in-old-document"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.dynamicPreparationContextResult === 'replacement-only'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "documentOpenAsyncRan"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "staleDynamicRan"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicPreparationContextResult"),
        Some(&JsValueSnapshot::String("replacement-only".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn implicit_document_write_from_async_keeps_old_defer_work() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-implicit-replace-drops-old-defer"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteImplicitReplaceRan"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteImplicitStaleDeferRan"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteImplicitResult"),
        Some(&JsValueSnapshot::String("initial".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn implicit_document_write_from_async_keeps_old_module_work() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-implicit-replace-drops-old-module"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteImplicitModuleReplaceRan"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteImplicitStaleModuleRan"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteImplicitModuleResult"),
        Some(&JsValueSnapshot::String("initial".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_replacement_async_stays_after_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-replacement-async-stays-after-domcontentloaded"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteReplacementInlineRuns"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteReplacementAsyncDclOrder"),
        Some(&JsValueSnapshot::String("dcl:interactive".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteReplacementAsyncScriptOrder"),
        Some(&JsValueSnapshot::String(
            "dcl:interactive,async-script:interactive".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteReplacementAsyncLoadOrder"),
        Some(&JsValueSnapshot::String(
            "dcl:interactive,async-script:interactive,load:interactive".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteReplacementAsyncFinalOrder"),
        Some(&JsValueSnapshot::String(
            "dcl:interactive,async-script:interactive,load:interactive,window-load:complete"
                .to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_replacement_syncs_style_sources_before_written_scripts() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-replacement-style-source-sync"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteReplacementStyleDisplay"),
        Some(&JsValueSnapshot::String("block".to_owned()))
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-style-display=\"block\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_nested_writer_restores_outer_insertion_point() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-nested-writer-restores-outer-insertion-point"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteNestedWriterResult"),
        Some(&JsValueSnapshot::String(
            "after|outer-start,inner,after-visible".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_nested_external_script_serializes_outer_resume() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-nested-external-script-serializes-outer-resume"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteNestedExternalResult"),
        Some(&JsValueSnapshot::String(
            "parent,parent-after-write,child,outer-after,outer-inline".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_external_split_script_continues_parser_session() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-external-split-script-parser-session"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteExternalSplitSessionResult"),
        Some(&JsValueSnapshot::String(
            "parent,written-inline,tail-hidden,parent-after-write,after-parent".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_inserted_external_script_resumes_chunked_root_input() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = tokio::time::timeout(
        Duration::from_secs(5),
        browser.fetch(&server.url("/compat/document-write-inserted-external-resumes-chunked-root")),
    )
    .await
    .expect("document.write parser continuation must not spin on queued root input")?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteInsertedChunkedResult"),
        Some(&JsValueSnapshot::String(
            "before-write,after-write,inserted-external,tail-script".to_owned()
        ))
    );
    assert!(
        page.serialize_html_async()
            .await?
            .contains("id=\"document-write-inserted-chunked-tail\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn imported_started_child_script_stays_inert_when_appended() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/imported-started-child-script-stays-inert"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "importedStartedChildResult"),
        Some(&JsValueSnapshot::String(
            "child-script,after-import".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_script_sees_parser_visible_dom_boundary() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-parser-visible-dom-boundary"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteParserVisibleDomResult"),
        Some(&JsValueSnapshot::String("ran|yes".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_external_script_stays_parser_blocking_at_insertion_point() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-external-parser-blocking-boundary"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteExternalResult"),
        Some(&JsValueSnapshot::String(
            "ran|before|no-after|after-present".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_external_script_load_microtask_precedes_later_written_inline() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/document-write-external-script-load-microtask-before-later-written-inline",
        ))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteLoadMicrotaskDuringLoad"),
        Some(&JsValueSnapshot::String("external-script,load".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteLoadMicrotaskAfterInline"),
        Some(&JsValueSnapshot::String(
            "external-script,load,load-microtask,after-inline".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteLoadMicrotaskFinal"),
        Some(&JsValueSnapshot::String(
            "external-script,load,load-microtask,after-inline".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_inline_importmap_applies_before_written_module() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-importmap-before-written-module"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteImportMapModuleResult"),
        Some(&JsValueSnapshot::String(
            "classic-after-write,module:1:2".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_inline_importmap_applies_before_written_external_module() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-importmap-before-written-external-module"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteExternalImportMapResult"),
        Some(&JsValueSnapshot::String(
            "classic-after-write,module:1:2".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_invalid_inline_importmap_reports_window_error_before_written_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-invalid-importmap-before-written-module"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteInvalidImportMapErrors"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteInvalidImportMapContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteInvalidImportMapOrderResult"),
        Some(&JsValueSnapshot::String(
            "window-error,later-module".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_invalid_inline_importmap_reports_window_error_before_restore_inline()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-invalid-importmap-before-restore-inline"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteInvalidImportMapProgressErrors"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteInvalidImportMapProgressRestoreOrder"),
        Some(&JsValueSnapshot::String(
            "window-error,restore-inline".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteInvalidImportMapProgressMicrotaskOrder"),
        Some(&JsValueSnapshot::String(
            "window-error,restore-inline,window-error-microtask".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteInvalidImportMapProgressFinalOrder"),
        Some(&JsValueSnapshot::String(
            "window-error,restore-inline,window-error-microtask".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_defer_script_queues_after_later_classic() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-defer-queues-after-later-classic"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteDeferQueueResult"),
        Some(&JsValueSnapshot::String(
            "classic-after-write,defer-script".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_defer_script_runs_before_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-defer-runs-before-domcontentloaded"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteDeferDclAtDcl"),
        Some(&JsValueSnapshot::String(
            "classic-after-write,defer-script,dcl".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteDeferDclFinal"),
        Some(&JsValueSnapshot::String(
            "classic-after-write,defer-script,dcl,load".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_open_after_parent_load_runs_written_data_script() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/child-document-open-after-parent-load-data-script"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "childDocumentOpenAfterLoadDone"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "childDocumentOpenAfterLoadResult"),
        Some(&JsValueSnapshot::String(
            "parent-load,data-script-1,after-close,data-script-2".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_open_after_load_runs_written_external_scripts() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/document-open-after-load-external-scripts"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.documentOpenAfterLoadResult === 'inline,external-1,external-2' && document.readyState === 'complete'",
            Duration::from_secs(2),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"written\"")
    );
    let Some(JsValueSnapshot::String(order)) =
        diagnostic_global(&page, "documentOpenAfterLoadResult")
    else {
        panic!("missing documentOpenAfterLoadResult diagnostic");
    };
    assert_eq!(order, "inline,external-1,external-2");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_multi_level_nested_writer_preserves_stack() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-multi-level-nested-writer"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteMultiLevelResult"),
        Some(&JsValueSnapshot::String(
            "inner-after|middle-after|outer-start,middle-start,inner,inner-after-visible,middle-after-visible".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_module_runs_without_hanging_late_stylesheet_tail() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-late-stylesheet-does-not-block-written-module"))
        .await?;

    let result = diagnostic_global(&page, "documentWriteLateStyleResult");
    let order = diagnostic_global(&page, "documentWriteLateStyleOrder");
    let html = page.serialize_html_async().await?;
    assert_eq!(
        result,
        Some(&JsValueSnapshot::String(
            "classic-after-write,module-before-style|true".to_owned()
        )),
        "unexpected document.write/module order: order={order:?}, html={}",
        html,
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_split_tags_stream_across_multiple_calls() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-split-tags-stream-across-calls"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteSplitStreamResult"),
        Some(&JsValueSnapshot::String(
            "ran|div-visible|after-present".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_split_script_stream_across_multiple_calls() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-split-script-stream-across-calls"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteSplitScriptResult"),
        Some(&JsValueSnapshot::String(
            "inline-script,after-missing|after-present".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_split_external_script_stream_across_multiple_calls() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-split-external-script-stream-across-calls"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteSplitExternalResult"),
        Some(&JsValueSnapshot::String(
            "ran|before|no-after|after-present".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_split_importmap_and_module_stream_across_multiple_calls() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-write-split-importmap-and-module-stream-across-calls"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "documentWriteSplitImportMapModuleResult"),
        Some(&JsValueSnapshot::String("1:2|after-present".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_importmap_applies_before_later_dynamic_module() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-importmap-before-module"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicImportMapGreeting"),
        Some(&JsValueSnapshot::String("initial".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_external_importmap_dispatches_error_before_later_module() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-external-importmap-error-before-module"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicExternalImportMapErrorCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicExternalImportMapOrderResult"),
        Some(&JsValueSnapshot::String("error,module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_module_execution_failure_dispatches_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-module-execution-failure-does-not-abort-queue"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleExecFailureContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleExecFailureErrorCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleExecFailureMessageMentionsFixture"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleExecFailureOrderResult"),
        Some(&JsValueSnapshot::String("later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_module_missing_default_export_dispatches_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-module-missing-default-export-does-not-abort-queue"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleMissingExportContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleMissingExportOrderResult"),
        Some(&JsValueSnapshot::String("later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_module_pending_star_missing_export_dispatches_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server.url("/compat/dynamic-module-pending-star-missing-export-does-not-abort-queue"),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicModulePendingStarMissingExportContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModulePendingStarMissingExportOrderResult"),
        Some(&JsValueSnapshot::String("later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_module_pending_star_link_failure_happens_before_body_and_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page =
        browser
            .fetch(&server.url(
                "/compat/dynamic-module-pending-star-link-failure-before-body-and-later-module",
            ))
            .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicPendingStarLinkContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicPendingStarLinkBodyRan"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicPendingStarLinkOrderResult"),
        Some(&JsValueSnapshot::String("later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_module_pending_star_final_missing_reports_link_failure_instead_of_timeout()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server.url("/compat/dynamic-module-pending-star-final-missing-reports-link-failure"),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicPendingStarFinalMissingOrderResult"),
        Some(&JsValueSnapshot::String("later-module".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicPendingStarFinalMissingWindowError"),
        Some(&JsValueSnapshot::String(format!(
            "ModuleLinkFailed: module `{}` does not export `missingValue`",
            server.url("/assets/module_pending_star_cycle_b.mjs")
        )))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_module_missing_default_export_reports_window_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server.url(
                "/compat/dynamic-module-missing-default-export-reports-window-error-does-not-abort-queue",
            ),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleMissingExportWindowContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleMissingExportWindowErrorCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleMissingExportWindowMessageMatches"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleMissingExportWindowOrderResult"),
        Some(&JsValueSnapshot::String("later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_module_missing_default_export_reports_window_error_payload_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server.url(
                "/compat/dynamic-module-missing-default-export-reports-window-error-payload-does-not-abort-queue",
            ),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleMissingExportPayloadContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(
            &page,
            "dynamicModuleMissingExportPayloadErrorMessageMatches"
        ),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleMissingExportPayloadFilenameMatches"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleMissingExportPayloadOrderResult"),
        Some(&JsValueSnapshot::String("later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_module_tla_rejection_dispatches_error_without_aborting_later_module() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-module-tla-rejection-does-not-abort-queue"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleTlaFailureContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleTlaFailureErrors"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleTlaFailureOrderResult"),
        Some(&JsValueSnapshot::String("error,later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_module_tla_exotic_rejection_reports_window_error_payload_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server.url(
                "/compat/dynamic-module-tla-exotic-rejection-reports-window-error-payload-does-not-abort-queue",
            ),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleTlaPayloadContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleTlaPayloadErrorMessageMatches"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleTlaPayloadFilenameMatches"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleTlaPayloadOrderResult"),
        Some(&JsValueSnapshot::String(
            "window-error,later-module".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_script_reattach_does_not_restart_after_commit() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-script-reattach-stays-started"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicReattachLoads"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicInOrderScriptOrderResult"),
        Some(&JsValueSnapshot::String("in-order-fast".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_script_src_mutation_does_not_restart_after_commit() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-script-src-mutation-stays-started"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicSrcMutationLoads"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicInOrderScriptOrderResult"),
        Some(&JsValueSnapshot::String("in-order-fast".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_script_async_attr_add_remove_clears_force_async() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-script-async-attr-clears-force-async"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicInOrderScriptOrderResult"),
        Some(&JsValueSnapshot::String(
            "in-order-slow,in-order-fast".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_script_src_added_after_connect_starts_once() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-script-src-added-after-connect-starts-once"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicSrcAddedLoads"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicInOrderScriptOrderResult"),
        Some(&JsValueSnapshot::String("in-order-fast".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn importmap_scopes_and_prefixes_override_global_resolution() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/importmap-scopes-and-prefixes"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "importMapScopedGreeting"),
        Some(&JsValueSnapshot::String("scoped".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "importMapScopedPkgLabel"),
        Some(&JsValueSnapshot::String("scoped-pkg".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "importMapScopesAndPrefixesResult"),
        Some(&JsValueSnapshot::String("scoped,scoped-pkg".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn later_importmap_cannot_override_already_resolved_specifier() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/importmap-merge-after-resolution"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "importMapMergeFirstGreeting"),
        Some(&JsValueSnapshot::String("initial".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "importMapMergeSecondGreeting"),
        Some(&JsValueSnapshot::String("initial".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "importMapMergeExtraLabel"),
        Some(&JsValueSnapshot::String("extra".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "importMapMergeAfterResolutionResult"),
        Some(&JsValueSnapshot::String("initial,initial,extra".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn importmap_normalizes_url_like_specifiers_before_resolution() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/importmap-url-like-normalization"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "importMapUrlLikeNormalizationResult"),
        Some(&JsValueSnapshot::String("canonical".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn importmap_registered_after_module_load_adds_unresolved_mapping() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/importmap-after-module-load"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "importMapAcquisitionFirstModule"),
        Some(&JsValueSnapshot::String("initial".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "importMapAcquisitionLateModule"),
        Some(&JsValueSnapshot::String("extra".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "importMapAcquisitionMutationDone"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_async_module_allows_late_importmap() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-async-module-allows-late-importmap"))
        .await?;
    assert_eq!(diagnostic_global(&page, "dynamicAsyncImportMapLoads"), None);
    assert_eq!(
        diagnostic_global(&page, "dynamicAsyncImportMapLabel"),
        Some(&JsValueSnapshot::String("extra".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicAsyncImportMapErrors"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicAsyncModuleReady"),
        None,
        "top-level await must not keep the external module or document load event pending"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_module_allows_late_dynamic_importmap() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parser-owned-module-allows-late-dynamic-importmap"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserModuleAcquisitionFirstGreeting"),
        Some(&JsValueSnapshot::String("initial".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserModuleAcquisitionLateLabel"),
        Some(&JsValueSnapshot::String("extra".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserModuleAcquisitionLateErrors"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "parserModuleAcquisitionMutationDone"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_importmap_applies_after_dynamic_module_prepare() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parser-owned-importmap-applies-after-dynamic-module-prepare"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicModulePrepareBarrierInstalled"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicModuleQueuedBeforeLateParserImportMap"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserImportMapAfterDynamicModuleLabel"),
        Some(&JsValueSnapshot::String("extra".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserImportMapAfterDynamicModuleErrors"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "parserImportMapAfterDynamicModuleDone"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_module_resolution_failure_dispatches_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parser-owned-module-error-before-later-module"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserOwnedModuleError"),
        Some(&JsValueSnapshot::String("blocked".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedModuleContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedModuleOrderResult"),
        Some(&JsValueSnapshot::String("error,later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_module_missing_export_dispatches_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parser-owned-module-missing-export-before-later-module"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportError"),
        Some(&JsValueSnapshot::String("missing-export".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportOrderResult"),
        Some(&JsValueSnapshot::String("error,later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_module_pending_star_missing_export_dispatches_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server
                .url("/compat/parser-owned-module-pending-star-missing-export-before-later-module"),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserOwnedPendingStarMissingExportError"),
        Some(&JsValueSnapshot::String(
            "pending-star-missing-export".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedPendingStarMissingExportContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedPendingStarMissingExportOrderResult"),
        Some(&JsValueSnapshot::String("error,later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_module_pending_star_link_failure_happens_before_body_and_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/parser-owned-module-pending-star-link-failure-before-body-and-later-module",
        ))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserOwnedPendingStarLinkError"),
        Some(&JsValueSnapshot::String(
            "pending-star-link-failure".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedPendingStarLinkContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedPendingStarLinkBodyRan"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedPendingStarLinkOrderResult"),
        Some(&JsValueSnapshot::String("error,later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_module_pending_star_final_missing_reports_link_failure_instead_of_timeout()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server
                .url("/compat/parser-owned-module-pending-star-final-missing-reports-link-failure"),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserOwnedPendingStarFinalMissingOrderResult"),
        Some(&JsValueSnapshot::String("error,later-module".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedPendingStarFinalMissingWindowError"),
        Some(&JsValueSnapshot::String(format!(
            "ModuleLinkFailed: module `{}` does not export `missingValue`",
            server.url("/assets/module_pending_star_cycle_b.mjs")
        )))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_module_missing_export_reports_window_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/parser-owned-module-missing-export-reports-window-error-before-later-module",
        ))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportWindowContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportWindowErrorCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportWindowMessageMatches"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportWindowOrderResult"),
        Some(&JsValueSnapshot::String(
            "window-error,later-module".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_module_missing_export_reports_window_error_after_restore_inline() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/parser-owned-module-missing-export-reports-window-error-after-restore-inline",
        ))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportRestoreErrors"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportRestoreRestoreOrder"),
        Some(&JsValueSnapshot::String("restore-inline".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportRestoreMicrotaskOrder"),
        Some(&JsValueSnapshot::String(
            "restore-inline,window-error,window-error-microtask".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportRestoreFinalOrder"),
        Some(&JsValueSnapshot::String(
            "restore-inline,window-error,window-error-microtask".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_module_missing_export_reports_window_error_payload_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server.url(
                "/compat/parser-owned-module-missing-export-reports-window-error-payload-before-later-module",
            ),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportPayloadContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportPayloadErrorMessageMatches"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportPayloadFilenameMatches"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedMissingExportPayloadOrderResult"),
        Some(&JsValueSnapshot::String(
            "window-error,later-module".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_module_tla_rejection_reports_window_error_after_restore_inline() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/parser-owned-module-tla-rejection-reports-window-error-after-restore-inline",
        ))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserOwnedTlaRestoreErrors"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedTlaRestoreRestoreOrder"),
        Some(&JsValueSnapshot::String("restore-inline".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedTlaRestoreMicrotaskOrder"),
        Some(&JsValueSnapshot::String(
            "restore-inline,window-error,window-error-microtask".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedTlaRestoreFinalOrder"),
        Some(&JsValueSnapshot::String(
            "restore-inline,window-error,window-error-microtask".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_module_tla_rejection_dispatches_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parser-owned-module-tla-rejection-before-later-module"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserOwnedModuleTlaErrors"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedModuleTlaContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedModuleTlaOrderResult"),
        Some(&JsValueSnapshot::String("error,later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_module_missing_export_reports_window_error_after_restore_inline()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/document-write-module-missing-export-reports-window-error-after-restore-inline",
        ))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "documentWriteMissingExportRestoreErrors"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteMissingExportRestoreRestoreOrder"),
        Some(&JsValueSnapshot::String("restore-inline".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteMissingExportRestoreMicrotaskOrder"),
        Some(&JsValueSnapshot::String(
            "restore-inline,window-error,window-error-microtask".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteMissingExportRestoreFinalOrder"),
        Some(&JsValueSnapshot::String(
            "restore-inline,window-error,window-error-microtask".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_module_tla_rejection_reports_window_error_after_restore_inline()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/document-write-module-tla-rejection-reports-window-error-after-restore-inline",
        ))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "documentWriteTlaRestoreErrors"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteTlaRestoreRestoreOrder"),
        Some(&JsValueSnapshot::String("restore-inline".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteTlaRestoreMicrotaskOrder"),
        Some(&JsValueSnapshot::String(
            "restore-inline,window-error,window-error-microtask".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "documentWriteTlaRestoreFinalOrder"),
        Some(&JsValueSnapshot::String(
            "restore-inline,window-error,window-error-microtask".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_module_tla_exotic_rejection_reports_window_error_payload_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server.url(
                "/compat/parser-owned-module-tla-exotic-rejection-reports-window-error-payload-before-later-module",
            ),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserOwnedModuleTlaPayloadContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedModuleTlaPayloadErrorMessageMatches"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedModuleTlaPayloadFilenameMatches"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedModuleTlaPayloadOrderResult"),
        Some(&JsValueSnapshot::String(
            "window-error,later-module".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_importmap_parse_failure_dispatches_error_before_later_module() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parser-owned-importmap-error-before-later-module"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserOwnedImportMapErrors"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedImportMapContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedImportMapOrderResult"),
        Some(&JsValueSnapshot::String(
            "window-error,later-module".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_owned_importmap_parse_failure_waits_until_parser_progress_restores_state()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parser-owned-importmap-error-after-parser-progress"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserOwnedImportMapProgressRestored"),
        Some(&JsValueSnapshot::String("restored".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedImportMapProgressSaw"),
        Some(&JsValueSnapshot::String("not-run".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedImportMapProgressRestoreOrder"),
        Some(&JsValueSnapshot::String(
            "window-error,window-error-microtask,restore-inline".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedImportMapProgressMicrotaskOrder"),
        Some(&JsValueSnapshot::String(
            "window-error,window-error-microtask".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "parserOwnedImportMapProgressFinalOrder"),
        Some(&JsValueSnapshot::String(
            "window-error,window-error-microtask,restore-inline".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_default_and_side_effect_imports_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-default-and-side-effect-imports"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultImportValue"),
        Some(&JsValueSnapshot::String("default-export-ok".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultImportSawSideEffectCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultExportEvalCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultImportOrderResult"),
        Some(&JsValueSnapshot::String(
            "module:default-export-ok,dcl".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_default_reexport_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-default-reexport"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultReexportResult"),
        Some(&JsValueSnapshot::String(
            "default-reexport-ok:default-reexport-ok:42".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultReexportDefaultName"),
        Some(&JsValueSnapshot::String("makeDefaultValue".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultReexportAliasName"),
        Some(&JsValueSnapshot::String("makeDefaultValue".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultReexportFinalOrder"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_string_literal_export_names_decode_js_escapes_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-string-literal-export-names"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleStringLiteralExportNamesResult"),
        Some(&JsValueSnapshot::String("42:string-name-ok".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStringLiteralExportNamesFinalOrder"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_export_default_function_and_class_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-export-default-function-and-class"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultDeclResult"),
        Some(&JsValueSnapshot::String("function-ok:ok".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultDeclFunctionName"),
        Some(&JsValueSnapshot::String("makeValue".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultDeclClassName"),
        Some(&JsValueSnapshot::String("ValueBox".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultDeclDclOrder"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_export_default_anonymous_declarations_preserve_default_name() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-export-default-anonymous-declarations"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultAnonymousResult"),
        Some(&JsValueSnapshot::String(
            "anon-fn-ok:anon-class-ok".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultAnonymousFunctionName"),
        Some(&JsValueSnapshot::String("default".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultAnonymousClassName"),
        Some(&JsValueSnapshot::String("default".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDefaultAnonymousDclOrder"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_export_named_class_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-export-class-named"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleExportClassResult"),
        Some(&JsValueSnapshot::String("class:named-export-ok".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExportClassName"),
        Some(&JsValueSnapshot::String("DerivedAnswer".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExportClassDclOrder"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_export_generator_functions_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-export-generator-functions"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleExportGeneratorResult"),
        Some(&JsValueSnapshot::String(
            "one:two|async-one:async-two|default-one:default-two".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExportGeneratorDefaultName"),
        Some(&JsValueSnapshot::String("default".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExportGeneratorFinalOrder"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_export_const_multiple_bindings_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-export-const-multiple-bindings"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleExportConstMultipleResult"),
        Some(&JsValueSnapshot::String("1:2:1-2".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExportConstMultipleFinalOrder"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_module_completion_keeps_dcl_ahead_of_due_timer_on_owner_loop() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/parser-module-completion-dcl-before-timer"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.parserModuleCompletionFinalOrder === 'module,dcl,timer'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "parserModuleCompletionFinalOrder"),
        Some(&JsValueSnapshot::String("module,dcl,timer".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_export_destructuring_bindings_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-export-destructuring-bindings"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.moduleExportDestructuringFinalOrder === 'module,dcl,timeout'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleExportDestructuringImmediate"),
        Some(&JsValueSnapshot::String(
            "42:destructure-ok:1:2:1:10:20-30".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExportDestructuringLater"),
        Some(&JsValueSnapshot::String("2:11:21-31".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExportDestructuringFinalOrder"),
        Some(&JsValueSnapshot::String("module,dcl,timeout".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_export_nested_destructuring_bindings_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-export-nested-destructuring-bindings"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.moduleNestedDestructuringFinalOrder === 'module,dcl,timeout'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleNestedDestructuringImmediate"),
        Some(&JsValueSnapshot::String(
            "42:lead:tail-a-tail-b:nested-default:1:10:20:30-40".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleNestedDestructuringLater"),
        Some(&JsValueSnapshot::String("2:11:21:31-41".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleNestedDestructuringFinalOrder"),
        Some(&JsValueSnapshot::String("module,dcl,timeout".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_export_nested_initializer_commas_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-export-nested-initializer-commas"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.moduleNestedInitializerFinalOrder === 'module,dcl,timeout'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleNestedInitializerImmediate"),
        Some(&JsValueSnapshot::String(
            "42:1-2-3:multi-comma:1:1".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleNestedInitializerLater"),
        Some(&JsValueSnapshot::String("nested-ok:2:2".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleNestedInitializerFinalOrder"),
        Some(&JsValueSnapshot::String("module,dcl,timeout".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_dependency_fetch_uses_module_credentials_mode() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let _ = browser.fetch(&server.url("/cookie")).await?;
    let page = browser
        .fetch(&server.url("/compat/module-dependency-fetch-uses-module-credentials"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(r#"data-module-dependency-cookie="true""#),
        "same-origin module dependency fetch should carry the document cookie"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_import_export_lists_support_comments_and_trailing_commas_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-import-export-list-comments"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleImportExportListCommentsResult"),
        Some(&JsValueSnapshot::String(
            "default-ok:42:comments-ok".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleImportExportListCommentsFinalOrder"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_multiline_dynamic_import_executes() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-multiline-dynamic-import"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleMultilineDynamicImportIsPromise"),
        Some(&JsValueSnapshot::Bool(true))
    );

    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleMultilineDynamicImportValue === 'multiline-import-ok'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleMultilineDynamicImportValue"),
        Some(&JsValueSnapshot::String("multiline-import-ok".to_owned()))
    );
    let final_order =
        diagnostic_global_string(&page, "moduleMultilineDynamicImportFinalOrder").unwrap_or("");
    assert_order_before(final_order, "module", "dcl");
    assert_order_contains(final_order, "resolved:multiline-import-ok");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_dynamic_import_with_comments_and_trailing_comma_executes() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-dynamic-import-comments-and-trailing-comma"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleDynamicImportCommentResolved === 'comment-ok'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicImportCommentResolved"),
        Some(&JsValueSnapshot::String("comment-ok".to_owned()))
    );
    let final_order =
        diagnostic_global_string(&page, "moduleDynamicImportCommentFinalOrder").unwrap_or("");
    assert_order_before(final_order, "module-start", "module-end");
    assert_order_before(final_order, "module-end", "dcl");
    assert_order_contains(final_order, "resolved:comment-ok");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_dynamic_import_with_static_concat_executes() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-dynamic-import-static-concat"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicImportStaticConcatPromiseLike"),
        Some(&JsValueSnapshot::Bool(true))
    );

    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleDynamicImportStaticConcatAwaitedValue === 'comment-ok'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicImportStaticConcatPromiseValue"),
        Some(&JsValueSnapshot::String("comment-ok".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicImportStaticConcatAwaitedValue"),
        Some(&JsValueSnapshot::String("comment-ok".to_owned()))
    );
    let final_order =
        diagnostic_global_string(&page, "moduleDynamicImportStaticConcatFinalOrder").unwrap_or("");
    assert_order_before(final_order, "module-start", "dcl");
    assert_order_before(final_order, "promise:comment-ok", "awaited:comment-ok");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_dynamic_import_source_rejects_without_hanging() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-dynamic-import-source-rejects"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleDynamicImportSourceRejected === true",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicImportSourceRejected"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicImportSourceErrorName"),
        Some(&JsValueSnapshot::String("SyntaxError".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicImportSourceMessageIncludesSourcePhase"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_import_resolves_after_imported_module_replaces_iframe_document() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/dynamic-import-document-write-iframe"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.dynamicImportDocumentWriteDone === true && window.dynamicImportDocumentWriteResolved === true",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicImportDocumentWriteBody"),
        Some(&JsValueSnapshot::String(
            "document.write body contents\n".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicImportDocumentWriteRejected"),
        None
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_import_attributes_and_dynamic_options_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-import-attributes-and-dynamic-options"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleImportAttributesDynamicValue === 'comment-ok' && window.moduleImportAttributesTextRejected === true",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleImportAttributesPromiseLike"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleImportAttributesDynamicValue"),
        Some(&JsValueSnapshot::String("comment-ok".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleImportAttributesFinalOrder"),
        Some(&JsValueSnapshot::String(
            "static:comment-ok,dcl,dynamic:comment-ok".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleImportAttributesTextErrorName"),
        Some(&JsValueSnapshot::String("TypeError".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_import_assertions_legacy_syntax_reports_error() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-import-assertions-legacy-syntax"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleImportAssertionsPromiseLike"),
        None
    );
    assert_eq!(
        diagnostic_global(&page, "moduleImportAssertionsDynamicValue"),
        None
    );
    let Some(JsValueSnapshot::String(error)) =
        diagnostic_global(&page, "moduleImportAssertionsError")
    else {
        panic!("expected legacy import assertions syntax error");
    };
    assert!(error.contains("Unexpected identifier 'assert'"), "{error}");
    assert_eq!(
        diagnostic_global(&page, "moduleImportAssertionsFinalOrder"),
        Some(&JsValueSnapshot::String("window-error,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_static_json_and_css_imports_use_synthetic_modules() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-static-json-css-import"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleStaticJsonCssValue"),
        Some(&JsValueSnapshot::String("42:json-ok".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStaticJsonCssSheetRules"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStaticJsonCssSheetText"),
        Some(&JsValueSnapshot::String(
            "body { color: rgb(1, 2, 3); }".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStaticJsonCssFinalOrder"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_static_wasm_import_exposes_wasm_exports() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-static-wasm-import"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmExports"),
        Some(&JsValueSnapshot::String("func,glob,mem,tab".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmFuncType"),
        Some(&JsValueSnapshot::String("function".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmMem"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmGlob"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmGlobValue"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmNamedGlob"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmNamedGlobValue"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmTab"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_dynamic_wasm_import_uses_original_instance_constructor() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/module-dynamic-wasm-import-ignores-patched-instance"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicWasmPatchedInstanceDone"),
        Some(&JsValueSnapshot::Bool(true)),
        "dynamic wasm import error: {:?}",
        diagnostic_global(&page, "moduleDynamicWasmPatchedInstanceError")
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicWasmPatchedInstanceFuncType"),
        Some(&JsValueSnapshot::String("function".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicWasmPatchedInstanceMem"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicWasmPatchedInstanceGlob"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicWasmPatchedInstanceGlobValue"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicWasmPatchedInstanceStillPatched"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_wasm_namespace_instance_returns_cached_instance() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/module-wasm-namespace-instance"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleWasmNamespaceInstanceDone"),
        Some(&JsValueSnapshot::Bool(true)),
        "namespaceInstance error: {:?}",
        diagnostic_global(&page, "moduleWasmNamespaceInstanceError")
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmNamespaceInstanceStatic"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmNamespaceInstanceShared"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmNamespaceInstanceFuncType"),
        Some(&JsValueSnapshot::String("function".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmNamespaceInstanceRejectsPlainObject"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmNamespaceInstanceRejectsJsNamespace"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_static_wasm_import_chain_links_wasm_and_js_dependencies() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-static-wasm-import-chain"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmImportResult"),
        Some(&JsValueSnapshot::String("executed".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_static_wasm_import_allows_acyclic_js_dependency_graph() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-static-wasm-import-js-dependency-graph"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmImportJsDependencyResult"),
        Some(&JsValueSnapshot::String("leaf".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_static_wasm_import_preserves_js_dependency_error_object() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-static-wasm-import-throwing-js-dependency"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmThrowingJsDependencyConstructor"),
        Some(&JsValueSnapshot::String("LinkError".to_owned())),
        "message: {:?}",
        diagnostic_global(&page, "moduleStaticWasmThrowingJsDependencyMessage")
    );
    let message = diagnostic_global(&page, "moduleStaticWasmThrowingJsDependencyMessage")
        .and_then(|value| match value {
            JsValueSnapshot::String(message) => Some(message.as_str()),
            _ => None,
        })
        .unwrap_or("");
    assert!(
        message.contains("dependency-link-boom"),
        "message should preserve the dependency error detail: {message:?}"
    );
    assert!(
        !message.contains("TypeError: LinkError"),
        "message should not wrap the original LinkError in a synthetic TypeError: {message:?}"
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmThrowingJsDependencyScriptLoad"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmThrowingJsDependencyScriptError"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStaticWasmThrowingJsDependencyLoaded"),
        None
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_mutable_wasm_global_initial_value_is_unwrapped() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-mutable-wasm-global-initial-value"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalType"),
        Some(&JsValueSnapshot::String("number".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalIsGlobal"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalInitial"),
        Some(&JsValueSnapshot::Number(0.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_wasm_global_unwrap_uses_original_value_getter() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/module-wasm-global-unwrapping-ignores-patched-getter"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleWasmGlobalPatchedGetterDone"),
        Some(&JsValueSnapshot::Bool(true)),
        "wasm global unwrap error: {:?}",
        diagnostic_global(&page, "moduleWasmGlobalPatchedGetterError")
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmGlobalPatchedGetterType"),
        Some(&JsValueSnapshot::String("number".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmGlobalPatchedGetterIsGlobal"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmGlobalPatchedGetterValue"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmGlobalPatchedGetterNamedType"),
        Some(&JsValueSnapshot::String("number".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmGlobalPatchedGetterNamedIsGlobal"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmGlobalPatchedGetterNamedValue"),
        Some(&JsValueSnapshot::Number(0.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires V8 wasm-aware module binding loads for mutable global exports"]
async fn module_mutable_wasm_global_export_is_live_binding() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-mutable-wasm-global-live-binding"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalLiveInitialNamespace"),
        Some(&JsValueSnapshot::Number(100.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalLiveInitialNamed"),
        Some(&JsValueSnapshot::Number(100.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalLiveInitialGetter"),
        Some(&JsValueSnapshot::Number(100.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalLiveGetterAfterSet"),
        Some(&JsValueSnapshot::Number(555.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalLiveNamespaceAfterSet"),
        Some(&JsValueSnapshot::Number(555.0)),
        "namespace access must observe the current wasm global value"
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalLiveNamedAfterSet"),
        Some(&JsValueSnapshot::Number(555.0)),
        "static named imports must observe the current wasm global value"
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalLiveType"),
        Some(&JsValueSnapshot::String("number".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalLiveIsGlobal"),
        Some(&JsValueSnapshot::Bool(false))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires V8 wasm-aware module binding loads for mutable global re-exports"]
async fn module_mutable_wasm_global_dep_reexport_is_live_binding() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-mutable-wasm-global-dep-reexport-live-binding"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalReexportInitialNamespace"),
        Some(&JsValueSnapshot::Number(100.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalReexportInitialNamed"),
        Some(&JsValueSnapshot::Number(100.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalReexportInitialGetter"),
        Some(&JsValueSnapshot::Number(100.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalReexportGetterAfterSet"),
        Some(&JsValueSnapshot::Number(777.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalReexportNamespaceAfterSet"),
        Some(&JsValueSnapshot::Number(777.0)),
        "namespace access must observe the dependency wasm global storage"
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalReexportNamedAfterSet"),
        Some(&JsValueSnapshot::Number(777.0)),
        "static imports must observe the dependency wasm global storage"
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalReexportType"),
        Some(&JsValueSnapshot::String("number".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMutableWasmGlobalReexportIsGlobal"),
        Some(&JsValueSnapshot::Bool(false))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_v128_wasm_global_export_throws_reference_error_on_namespace_access() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/module-wasm-v128-global-export-throws-tdz"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleWasmV128TdzDone"),
        Some(&JsValueSnapshot::Bool(true)),
        "v128 TDZ page error: {:?}",
        diagnostic_global(&page, "moduleWasmV128TdzError")
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmV128TdzMutableValue"),
        Some(&JsValueSnapshot::Number(100.0)),
        "v128 TDZ page error: {:?}",
        diagnostic_global(&page, "moduleWasmV128TdzError")
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmV128TdzThrows"),
        Some(&JsValueSnapshot::Bool(true)),
        "namespace access should throw ReferenceError"
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmV128TdzErrorName"),
        Some(&JsValueSnapshot::String("ReferenceError".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_source_phase_wasm_import_returns_compiled_module_sources() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/module-source-phase-wasm-import"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseStaticIsModule"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseAbstractModuleSourceHidden"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseAbstractModuleSourceName"),
        Some(&JsValueSnapshot::String("AbstractModuleSource".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseModuleConstructorExtendsAbstract"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseModulePrototypeExtendsAbstract"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseStaticIsAbstractModuleSource"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseStaticExports"),
        Some(&JsValueSnapshot::String("func,glob,mem,tab".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseSharedIsModule"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseSharedEvaluationResult"),
        Some(&JsValueSnapshot::String("executed".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseDynamicIsModule"),
        Some(&JsValueSnapshot::Bool(true)),
        "dynamic error: {:?}, unhandled: {:?}",
        diagnostic_global(&page, "moduleSourcePhaseDynamicError"),
        diagnostic_global(&page, "moduleSourcePhaseUnhandled")
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseDynamicLogged"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseDynamicDone"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn audio_worklet_module_imports_wasm_source_phase() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/audio-worklet-wasm-source-phase"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "audioWorkletWasmDone"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "audioWorkletWasmError"),
        Some(&JsValueSnapshot::String(String::new()))
    );
    assert_eq!(
        diagnostic_global(&page, "audioWorkletWasmResult"),
        Some(&JsValueSnapshot::String(
            "42|true|true|func,glob,mem,tab".to_owned()
        ))
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-audio-worklet-wasm=\"42|true|true|func,glob,mem,tab\""),
        "audio worklet wasm result should be reflected in final HTML: {}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_source_phase_imports_share_resolved_module_identity() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/module-source-phase-identity"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseIdentityDone"),
        Some(&JsValueSnapshot::Bool(true)),
        "source-phase identity error: {:?}",
        diagnostic_global(&page, "moduleSourcePhaseIdentityError")
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseIdentityNamespaceShared"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseIdentitySourceShared"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_source_phase_wasm_import_reuses_modulepreload_resource() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/module-source-phase-wasm-modulepreload"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhasePreloadDone"),
        Some(&JsValueSnapshot::Bool(true)),
        "preload error: {:?}",
        diagnostic_global(&page, "moduleSourcePhasePreloadError")
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhasePreloadCountAfterLoad"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhasePreloadTransferPositive"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhasePreloadInitiator"),
        Some(&JsValueSnapshot::String("link".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhasePreloadImportIsModule"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhasePreloadCountAfterImport"),
        Some(&JsValueSnapshot::Number(1.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_source_phase_dynamic_script_reuses_modulepreload_resource() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-source-phase-wasm-dynamic-script-modulepreload"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            r#"
                (globalThis.moduleSourcePhaseDynamicPreloadDone === true &&
                    globalThis.moduleSourcePhaseStaticDone === true) ||
                Boolean(globalThis.moduleSourcePhaseDynamicPreloadError) ||
                Boolean(globalThis.moduleSourcePhaseDynamicScriptError)
            "#,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseDynamicPreloadDone"),
        Some(&JsValueSnapshot::Bool(true)),
        "preload error: {:?}, script error: {:?}",
        diagnostic_global(&page, "moduleSourcePhaseDynamicPreloadError"),
        diagnostic_global(&page, "moduleSourcePhaseDynamicScriptError")
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseStaticDone"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseDynamicBeforeAwait"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseDynamicAfterAwait"),
        Some(&JsValueSnapshot::Bool(true)),
        "script error: {:?}",
        diagnostic_global(&page, "moduleSourcePhaseDynamicScriptError")
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseDynamicScriptLoad"),
        Some(&JsValueSnapshot::Bool(false)),
        "successful inline module scripts must not dispatch load"
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseDynamicPreloadCountAfterLoad"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSourcePhaseDynamicPreloadCountAfterImport"),
        Some(&JsValueSnapshot::Number(1.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_external_wasm_script_executes_start_function() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-external-wasm-script-executes-start"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleExternalWasmScriptErrored"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExternalWasmScriptLog"),
        Some(&JsValueSnapshot::String("executed".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_wasm_csp_blocks_cross_origin_script_element_fetch() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/module-wasm-csp-blocks-cross-origin"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleWasmCspViolationCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmCspViolationText"),
        Some(&JsValueSnapshot::String(
            "script-src-elem|script-src-elem|true|true".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmCspExecuted"),
        Some(&JsValueSnapshot::String(String::new()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wasm_api_csp_blocks_eval_from_response_header() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/wasm-api-csp-blocks-eval-from-response-header"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "wasmCspHeaderResult"),
        Some(&JsValueSnapshot::String("CompileError|true".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "wasmCspHeaderEventText"),
        Some(&JsValueSnapshot::String(
            "script-src|script-src|default-src 'self' 'unsafe-inline'|wasm-eval|true".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn wasm_module_postmessage_into_csp_iframe_fires_violation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch_with_wait_until(
            &server.url("/compat/wasm-module-postmessage-into-csp-iframe"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.wasmPostMessageCspResult !== ''",
            Duration::from_secs(2),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "wasmPostMessageCspResult"),
        Some(&JsValueSnapshot::String(
            "script-src|script-src|default-src 'unsafe-inline'|wasm-eval|true".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "wasmPostMessageCspUnexpectedMessage"),
        Some(&JsValueSnapshot::Bool(false))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_wasm_link_error_reports_typed_window_error() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-wasm-link-error-reports-typed-window-error"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleWasmLinkErrorConstructor"),
        Some(&JsValueSnapshot::String("LinkError".to_owned())),
        "message: {:?}",
        diagnostic_global(&page, "moduleWasmLinkErrorMessage")
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmLinkErrorScriptLoad"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmLinkErrorScriptError"),
        Some(&JsValueSnapshot::Bool(false))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_wasm_js_cycle_reports_guard_without_crashing() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-wasm-js-cycle-reports-guard"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleConstructor"),
        Some(&JsValueSnapshot::String("Error".to_owned())),
        "message: {:?}",
        diagnostic_global(&page, "moduleWasmJsCycleMessage")
    );
    let message = diagnostic_global(&page, "moduleWasmJsCycleMessage")
        .and_then(|value| match value {
            JsValueSnapshot::String(message) => Some(message.as_str()),
            _ => None,
        })
        .unwrap_or("");
    assert!(
        message.contains(
            "cyclic WebAssembly module evaluation through JavaScript dependencies is not supported yet"
        ),
        "message should expose the cycle guard: {message:?}"
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleUnexpected"),
        Some(&JsValueSnapshot::Bool(false))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires wasm module records to participate in the V8 module evaluation SCC"]
async fn module_wasm_js_cycle_evaluates_js_dependency_initializers() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/module-wasm-js-cycle-future-acceptance"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleFutureDone"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleFutureError"),
        Some(&JsValueSnapshot::String(String::new())),
        "cycle error: {:?}",
        diagnostic_global(&page, "moduleWasmJsCycleFutureError")
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleFutureGlobalType"),
        Some(&JsValueSnapshot::String("number".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleFutureGlobalIsGlobal"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleFutureGlobalValue"),
        Some(&JsValueSnapshot::Number(24.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleFutureFunction"),
        Some(&JsValueSnapshot::Number(43.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleFutureIncrementGlobal"),
        Some(&JsValueSnapshot::Number(43.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleFutureMemoryBefore"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleFutureMutateMemory"),
        Some(&JsValueSnapshot::Number(42.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleFutureMemoryAfter"),
        Some(&JsValueSnapshot::Number(42.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleFutureTableBeforeNull"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleFutureTableRefIsFunction"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleWasmJsCycleFutureTableAfterSame"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_js_wasm_cycle_function_import_captures_initial_binding() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/module-js-wasm-cycle-function-import"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleJsWasmCycleConstructor"),
        Some(&JsValueSnapshot::String(String::new())),
        "message: {:?}",
        diagnostic_global(&page, "moduleJsWasmCycleMessage")
    );
    assert_eq!(
        diagnostic_global(&page, "moduleJsWasmCycleInitialRun"),
        Some(&JsValueSnapshot::Number(42.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleJsWasmCycleAfterReassignRun"),
        Some(&JsValueSnapshot::Number(42.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleJsWasmCycleImportedBinding"),
        Some(&JsValueSnapshot::Number(24.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_reserved_wasm_names_reject_with_link_error() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/module-reserved-wasm-names-reject-with-link-error"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleReservedWasmNameDone"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleReservedWasmNameResults"),
        Some(&JsValueSnapshot::String(
            [
                "import-name:LinkError:true",
                "export-name:LinkError:true",
                "import-module:LinkError:true",
                "patched-link-error:LinkError:true:true:false",
            ]
            .join("|")
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleReservedWasmNameUnhandled"),
        Some(&JsValueSnapshot::String(String::new()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_import_meta_resolve_uses_import_map_and_static_concat() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-import-meta-resolve"))
        .await?;
    let expected_target = server.url("/assets/module-dynamic-import-comments-target.mjs");
    assert_eq!(
        diagnostic_global(&page, "moduleImportMetaResolveMapped"),
        Some(&JsValueSnapshot::String(expected_target.clone()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleImportMetaResolveConcat"),
        Some(&JsValueSnapshot::String(expected_target))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleImportMetaResolveThrew"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleImportMetaResolveFinalOrder"),
        Some(&JsValueSnapshot::String("module,resolved,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_import_meta_resolve_supports_comments_and_trailing_comma() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-import-meta-resolve-comments-and-trailing-comma"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleImportMetaResolveCommentValue"),
        Some(&JsValueSnapshot::String(
            server.url("/assets/module-dynamic-import-comments-target.mjs"),
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleImportMetaResolveCommentFinalOrder"),
        Some(&JsValueSnapshot::String("module,resolved,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_dynamic_import_with_template_literal_executes() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-dynamic-import-template-literal"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicImportTemplatePromiseLike"),
        Some(&JsValueSnapshot::Bool(true))
    );

    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleDynamicImportTemplateResult === 'template-ok:template-ok'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicImportTemplateResult"),
        Some(&JsValueSnapshot::String(
            "template-ok:template-ok".to_owned()
        ))
    );
    let final_order =
        diagnostic_global_string(&page, "moduleDynamicImportTemplateFinalOrder").unwrap_or("");
    assert_order_before(final_order, "dcl", "module");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_dynamic_import_eval_and_function_use_active_script_base_url() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-dynamic-import-string-compilation/base/page"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleDynamicImportStringCompilationDone === true",
            Duration::from_secs(2),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "moduleDynamicImportStringCompilationError"),
        None
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicImportStringCompilationEval"),
        Some(&JsValueSnapshot::String("dynamic-base-ok:eval".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicImportStringCompilationFunction"),
        Some(&JsValueSnapshot::String(
            "dynamic-base-ok:function".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleDynamicImportStringCompilationFinalOrder"),
        Some(&JsValueSnapshot::String("script,eval,function".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_string_literal_specifiers_decode_js_escapes() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-escaped-string-literal-specifiers"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleEscapedSpecifierResult === 'escaped-default:escaped-value:escaped-value:escaped-default'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleEscapedSpecifierResult"),
        Some(&JsValueSnapshot::String(
            "escaped-default:escaped-value:escaped-value:escaped-default".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleEscapedSpecifierOrderAtDcl"),
        Some(&JsValueSnapshot::String("dcl".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleEscapedSpecifierFinalOrder"),
        Some(&JsValueSnapshot::String("dcl,module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_export_variable_live_bindings_update_across_turns() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-export-variable-live-bindings"))
        .await?;
    browser
        .wait_for_page_delay(&mut page, Duration::from_millis(250))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleExportVariableInitial"),
        Some(&JsValueSnapshot::String("0:init".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExportVariableAfterMicrotask"),
        Some(&JsValueSnapshot::String("0:init".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExportVariableAfterTimeout"),
        Some(&JsValueSnapshot::String("2:timeout".to_owned()))
    );
    let final_order =
        diagnostic_global(&page, "moduleExportVariableFinalOrder").and_then(|value| match value {
            JsValueSnapshot::String(value) => Some(value.as_str()),
            _ => None,
        });
    assert!(
        matches!(
            final_order,
            Some(
                "module:0:init,microtask:0:init,dcl,timeout:2:timeout"
                    | "module:0:init,microtask:0:init,timeout:2:timeout,dcl"
            )
        ),
        "unexpected export-variable order: {final_order:?}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_self_bare_dynamic_import_returns_promise_and_waits_for_own_evaluation() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-self-bare-dynamic-import-resolves-after-own-evaluation"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleSelfBareImportResolvedValue === 'ready'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleSelfBareImportIsPromise"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSelfBareImportResolvedValue"),
        Some(&JsValueSnapshot::String("ready".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSelfBareImportFinalOrder"),
        Some(&JsValueSnapshot::String(
            "module-start,module-end,dcl,resolved:ready".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_self_bare_dynamic_import_after_settle_reuses_existing_promise() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-self-bare-dynamic-import-after-settle-resolves"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.moduleSelfBareImportAfterSettleFinalOrder === 'module-start,module-end,dcl,timeout-start,resolved:ready-after-settle'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleSelfBareImportAfterSettleIsPromise"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSelfBareImportAfterSettleResolvedValue"),
        Some(&JsValueSnapshot::String("ready-after-settle".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSelfBareImportAfterSettleFinalOrder"),
        Some(&JsValueSnapshot::String(
            "module-start,module-end,dcl,timeout-start,resolved:ready-after-settle".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_runtime_helper_calls_are_not_shadowed_by_module_lexicals() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-runtime-helper-shadowing"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleRuntimeHelperShadowValue"),
        Some(&JsValueSnapshot::String("shadow-safe".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleRuntimeHelperShadowLexicalType"),
        Some(&JsValueSnapshot::String("object".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleRuntimeHelperShadowFinalOrder"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_multiline_import_and_export_list_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-multiline-import-and-export-list"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleMultilineResult"),
        Some(&JsValueSnapshot::String("42:multiline-ok".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleMultilineOrderResult"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_export_star_and_namespace_reexport_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-export-star-and-namespace-reexport"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleExportStarResult"),
        Some(&JsValueSnapshot::String(
            "42:ok:42:ok:default-ok".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExportStarOrderResult"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_string_literal_export_names_decode_surrogate_pairs_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-string-literal-export-names-surrogate-pairs"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleStringLiteralExportNamesSurrogatePairsLaunch"),
        Some(&JsValueSnapshot::Number(42.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleStringLiteralExportNamesSurrogatePairsRelay"),
        Some(&JsValueSnapshot::String("orbital".to_owned()))
    );
    assert_eq!(
        diagnostic_global(
            &page,
            "moduleStringLiteralExportNamesSurrogatePairsOrderResult",
        ),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_export_star_string_literal_namespace_decodes_js_escapes_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-export-star-string-literal-namespace"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleExportStarStringLiteralNsResult"),
        Some(&JsValueSnapshot::String("42:ok:default-ok".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExportStarStringLiteralNsFinalOrder"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_escaped_identifier_names_execute() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-escaped-identifier-names"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleEscapedIdentifierNamesResult"),
        Some(&JsValueSnapshot::String("default-ok:41:42:box".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleEscapedIdentifierNamesFinalOrder"),
        Some(&JsValueSnapshot::String("module,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_export_star_ambiguous_name_dispatches_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-export-star-ambiguous-before-later-module"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleExportStarAmbiguousError"),
        Some(&JsValueSnapshot::String("ambiguous".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExportStarAmbiguousContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleExportStarAmbiguousOrderResult"),
        Some(&JsValueSnapshot::String("error,later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_cycle_missing_export_dispatches_error_without_aborting_later_module() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-cycle-missing-export-before-later-module"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleCycleMissingExportError"),
        Some(&JsValueSnapshot::String("missing-export".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleCycleMissingExportContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleCycleMissingExportOrderResult"),
        Some(&JsValueSnapshot::String("error,later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_cycle_initializing_missing_export_dispatches_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-cycle-initializing-missing-export-before-later-module"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleCycleInitializingMissingError"),
        Some(&JsValueSnapshot::String("missing-export".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleCycleInitializingMissingContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleCycleInitializingMissingOrderResult"),
        Some(&JsValueSnapshot::String("error,later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_cycle_default_missing_through_export_star_dispatches_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server
                .url("/compat/module-cycle-default-missing-from-export-star-before-later-module"),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleCycleDefaultMissingError"),
        Some(&JsValueSnapshot::String("missing-default".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleCycleDefaultMissingContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleCycleDefaultMissingOrderResult"),
        Some(&JsValueSnapshot::String("error,later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_import_in_cycle_waits_for_target_module_evaluation_before_resolving() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-cycle-dynamic-import-waits-for-target-evaluation"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleCycleSawAValue === 'a-ready'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleCycleAfterAEndOrder"),
        Some(&JsValueSnapshot::String(
            "b-start,b-after-import,a-start,a-end".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleCycleSawAValue"),
        Some(&JsValueSnapshot::String("a-ready".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleCycleFinalOrder"),
        Some(&JsValueSnapshot::String(
            "b-start,b-after-import,a-start,a-end,dcl,b-dynamic-resolved".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleCycleDclOrder"),
        Some(&JsValueSnapshot::String(
            "b-start,b-after-import,a-start,a-end,dcl".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_cycle_export_star_late_binding_becomes_visible_after_target_evaluation()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-cycle-export-star-late-binding"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleCycleExportStarLateBindingValue"),
        Some(&JsValueSnapshot::Number(41.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_cycle_export_star_multihop_late_binding_becomes_visible_after_target_evaluation()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-cycle-export-star-multihop-late-binding"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleCycleExportStarMultihopLateBindingValue"),
        Some(&JsValueSnapshot::Number(41.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_cycle_export_star_late_ambiguity_dispatches_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-cycle-export-star-late-ambiguous-before-later-module"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleCycleExportStarLateAmbiguousError"),
        Some(&JsValueSnapshot::String("ambiguous".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleCycleExportStarLateAmbiguousContinuation"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleCycleExportStarLateAmbiguousOrderResult"),
        Some(&JsValueSnapshot::String("error,later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_cycle_export_star_late_ambiguity_omits_export_from_namespace_object() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(
            &server.url("/compat/module-cycle-export-star-late-ambiguous-namespace-omits-export"),
        )
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleCycleExportStarLateAmbiguousNamespaceImported === true",
            Duration::from_secs(2),
        )
        .await?;
    let imported = diagnostic_global(&page, "moduleCycleExportStarLateAmbiguousNamespaceImported");
    if imported != Some(&JsValueSnapshot::Bool(true)) {
        panic!(
            "expected namespace dynamic import to resolve, imported={imported:?}, error={:?}, order={:?}",
            diagnostic_global(&page, "moduleCycleExportStarLateAmbiguousNamespaceError"),
            diagnostic_global(
                &page,
                "moduleCycleExportStarLateAmbiguousNamespaceOrderResult"
            )
        );
    }
    assert_eq!(
        diagnostic_global(
            &page,
            "moduleCycleExportStarLateAmbiguousNamespaceHasShared"
        ),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(
            &page,
            "moduleCycleExportStarLateAmbiguousNamespaceDescriptor"
        ),
        Some(&JsValueSnapshot::Bool(true))
    );
    let final_order = diagnostic_global_string(
        &page,
        "moduleCycleExportStarLateAmbiguousNamespaceOrderResult",
    )
    .unwrap_or("");
    assert_order_before(final_order, "dcl", "import-resolved");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_cycle_export_star_multihop_late_ambiguity_omits_export_from_namespace_object()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page =
        browser
            .fetch(&server.url(
                "/compat/module-cycle-export-star-multihop-late-ambiguous-namespace-omits-export",
            ))
            .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleCycleExportStarMultihopLateAmbiguousNamespaceImported === true",
            Duration::from_secs(2),
        )
        .await?;
    let imported = diagnostic_global(
        &page,
        "moduleCycleExportStarMultihopLateAmbiguousNamespaceImported",
    );
    if imported != Some(&JsValueSnapshot::Bool(true)) {
        panic!(
            "expected multihop namespace dynamic import to resolve, imported={imported:?}, error={:?}, order={:?}",
            diagnostic_global(
                &page,
                "moduleCycleExportStarMultihopLateAmbiguousNamespaceError"
            ),
            diagnostic_global(
                &page,
                "moduleCycleExportStarMultihopLateAmbiguousNamespaceOrderResult"
            )
        );
    }
    assert_eq!(
        diagnostic_global(
            &page,
            "moduleCycleExportStarMultihopLateAmbiguousNamespaceHasShared",
        ),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(
            &page,
            "moduleCycleExportStarMultihopLateAmbiguousNamespaceDescriptor",
        ),
        Some(&JsValueSnapshot::Bool(true))
    );
    let final_order = diagnostic_global_string(
        &page,
        "moduleCycleExportStarMultihopLateAmbiguousNamespaceOrderResult",
    )
    .unwrap_or("");
    assert_order_before(final_order, "dcl", "import-resolved");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_cycle_export_star_multihop_late_ambiguity_dispatches_error_without_aborting_later_module()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page =
        browser
            .fetch(&server.url(
                "/compat/module-cycle-export-star-multihop-late-ambiguous-before-later-module",
            ))
            .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleCycleExportStarMultihopLateAmbiguousError"),
        Some(&JsValueSnapshot::String("ambiguous".to_owned()))
    );
    assert_eq!(
        diagnostic_global(
            &page,
            "moduleCycleExportStarMultihopLateAmbiguousContinuation",
        ),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(
            &page,
            "moduleCycleExportStarMultihopLateAmbiguousOrderResult",
        ),
        Some(&JsValueSnapshot::String("error,later-module".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn static_import_waits_for_initializing_shared_non_cycle_dependency() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(
            &server.url("/compat/module-static-import-waits-for-initializing-non-cycle-dependency"),
        )
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleSharedInitializingParentAValue === 'ready' && window.moduleSharedInitializingParentBValue === 'ready'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleSharedInitializingParentAValue"),
        Some(&JsValueSnapshot::String("ready".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSharedInitializingParentBValue"),
        Some(&JsValueSnapshot::String("ready".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSharedInitializingParentAObservedDepEnd"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSharedInitializingParentBObservedDepEnd"),
        Some(&JsValueSnapshot::Bool(true))
    );
    let dcl_order = diagnostic_global(&page, "moduleSharedInitializingDclOrder").and_then(
        |value| match value {
            JsValueSnapshot::String(value) => Some(value.as_str()),
            _ => None,
        },
    );
    assert!(
        matches!(dcl_order, Some("dcl")),
        "unexpected shared-initializing order: {dcl_order:?}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn static_import_waits_for_initializing_non_cycle_dependency_to_settle() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(
            &server.url("/compat/module-static-import-waits-for-initializing-non-cycle-dependency"),
        )
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleSharedInitializingParentAValue === 'ready' && window.moduleSharedInitializingParentBValue === 'ready'",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleSharedInitializingParentAValue"),
        Some(&JsValueSnapshot::String("ready".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSharedInitializingParentBValue"),
        Some(&JsValueSnapshot::String("ready".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSharedInitializingParentAObservedDepEnd"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSharedInitializingParentBObservedDepEnd"),
        Some(&JsValueSnapshot::Bool(true))
    );
    let dcl_order = diagnostic_global(&page, "moduleSharedInitializingDclOrder");
    assert!(
        matches!(dcl_order, Some(JsValueSnapshot::String(order)) if order == "dcl"),
        "unexpected moduleSharedInitializingDclOrder: {dcl_order:?}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_failed_module_dependency_is_not_reexecuted_across_later_parser_owned_importers()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-shared-failed-dependency-is-not-reexecuted"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleSharedFailedEvalCountDuringErrors"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSharedFailedEvalCountAtDcl"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSharedFailedOrderResult"),
        Some(&JsValueSnapshot::String(
            "module-shared-failed-a:error,module-shared-failed-b:error,dcl".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_unsupported_module_dependency_is_not_retried_across_later_parser_owned_importers()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let mut tight_config = AppConfig::default();
    tight_config.fetch_mut().set_request_timeout_ms(100);
    let browser = Browser::new(tight_config)?;

    let page = browser
        .fetch(&server.url("/compat/module-shared-unsupported-dependency-is-not-retried"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleSharedUnsupportedOrderResult"),
        Some(&JsValueSnapshot::String(
            "module-shared-unsupported-a:error,module-shared-unsupported-b:error,dcl".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleSharedUnsupportedReady"),
        None
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_top_level_await_does_not_delay_domcontentloaded_past_initial_evaluation()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-top-level-await-delays-domcontentloaded"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleTlaDclSawAwait"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleTlaOrderResult"),
        Some(&JsValueSnapshot::String("module-start,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_top_level_await_over_fifty_ms_does_not_delay_domcontentloaded_to_completion()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/module-top-level-await-over-fifty-ms-delays-domcontentloaded"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleLongTlaDclSawAwait"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleLongTlaWaitedPastLegacyCap"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleLongTlaOrderResult"),
        Some(&JsValueSnapshot::String("module-start,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_tla_dependency_completion_does_not_delay_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/module-tla-dependency-delays-parent-and-domcontentloaded"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.moduleTlaDependencyRootSawReady === true",
            Duration::from_secs(2),
        )
        .await?;
    assert_eq!(
        diagnostic_global(&page, "moduleTlaDependencyRootSawReady"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleTlaDependencyDclSawReady"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleTlaDependencyOrderResult"),
        Some(&JsValueSnapshot::String("dep-start,dcl".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_nomodule_script_commits_skip_and_stays_inert_after_mutation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/dynamic-script-nomodule-commits-skip"))
        .await?;
    assert_eq!(
        diagnostic_global(&page, "dynamicNomoduleRuns"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "dynamicNomoduleAfterMutation"),
        Some(&JsValueSnapshot::Number(0.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parse_time_lifecycle_tasks_keep_task_turn_boundaries_across_defer_dcl_async_and_load()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-lifecycle-tasks"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskFinalOrder"),
        Some(&JsValueSnapshot::String(
            "defer-script,defer-microtask,dcl,dcl-microtask,async-script,load".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskAfterLoadMicrotaskOrder"),
        Some(&JsValueSnapshot::String(
            "defer-script,defer-microtask,dcl,dcl-microtask,async-script,load,load-microtask"
                .to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskDclMicrotaskSeen"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskLoadMicrotaskSeen"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn parse_time_lifecycle_async_phase_stays_outside_defer_and_dcl_turns() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/parse-time-lifecycle-tasks"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskDclOrder"),
        Some(&JsValueSnapshot::String(
            "defer-script,defer-microtask,dcl".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskDclAfterMicrotaskOrder"),
        Some(&JsValueSnapshot::String(
            "defer-script,defer-microtask,dcl,dcl-microtask".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskFinalOrder"),
        Some(&JsValueSnapshot::String(
            "defer-script,defer-microtask,dcl,dcl-microtask,async-script,load".to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn parse_time_lifecycle_queue_can_stop_cleanly_at_domcontentloaded_stage() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/parse-time-lifecycle-tasks"),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskDclOrder"),
        Some(&JsValueSnapshot::String(
            "defer-script,defer-microtask,dcl".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskDclSeen"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskDclMicrotaskSeen"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskDclAfterMicrotaskOrder"),
        Some(&JsValueSnapshot::String(
            "defer-script,defer-microtask,dcl,dcl-microtask".to_owned()
        ))
    );
    assert_eq!(diagnostic_global(&page, "lifecycleTaskAsyncSeen"), None);
    assert_eq!(diagnostic_global(&page, "lifecycleTaskLoadSeen"), None);
    assert_eq!(diagnostic_global(&page, "lifecycleTaskFinalOrder"), None);
    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskLoadMicrotaskSeen"),
        None
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskAfterLoadMicrotaskOrder"),
        None
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn parse_time_lifecycle_queue_can_stop_cleanly_at_load_stage_after_load_microtask()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/parse-time-lifecycle-tasks"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskLoadSeen"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskLoadMicrotaskSeen"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleTaskAfterLoadMicrotaskOrder"),
        Some(&JsValueSnapshot::String(
            "defer-script,defer-microtask,dcl,dcl-microtask,async-script,load,load-microtask"
                .to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}
