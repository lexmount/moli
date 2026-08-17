use moli_test_support as support;

use anyhow::Result;
use moli_core::{
    LayoutPolicy,
    runtime::{Browser, BrowserConfig as AppConfig, RenderedDomWaitUntil},
    testing::JsValueSnapshot,
};
use moli_fetch::FetchConfig;
use support::FixtureServer;
use tokio::time::Duration;

fn diagnostic_global<'a>(
    page: &'a moli_core::page::Page,
    name: &str,
) -> Option<&'a JsValueSnapshot> {
    page.script_execution().global(name)
}

fn evaluated_string(value: serde_json::Value) -> Option<String> {
    value
        .get("value")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

#[tokio::test]
async fn browser_surface_compat_supports_storage_navigator_arrays_and_history_state() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/browser-surface"))
        .await?;

    assert_eq!(page.final_url().path(), "/compat/history-replaced");
    assert_eq!(page.final_url().query(), Some("from=replace"));
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
            .contains("data-storage-type=\"function\"")
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
            .contains("data-window-add-event-listener=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-remove-event-listener=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-dispatch-event=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-global-add-event-listener=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-global-remove-event-listener=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-global-dispatch-event=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-length-after-push=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state-after-push=\"{&quot;step&quot;:1}\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state-after-replace=\"{&quot;step&quot;:2}\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-location-after-push=\"/compat/history-pushed?from=push\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-location-after-replace=\"/compat/history-replaced?from=replace\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn canvas_to_data_url_exists_and_handles_zero_size() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/canvas-to-data-url-exists-and-handles-zero-size"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-typeof=\"function\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-nonzero-prefix=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-zero=\"data:,\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn event_handler_accessors_cover_attribute_property_and_body_onload_reflection() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/event-handler-accessors"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-event-handler=\"function:click:true:ok:target:custom:true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-body-onload-reflection=\"function:true:true:true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn html_content_accessors_cover_inner_outer_html_and_inner_text() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/html-content-accessors"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-inner-text=\"Alpha Beta\nGamma\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-script-inner-text=\"{&quot;ignored&quot;:true}\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-inner-text=\"false:false:true\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap()
            .contains("data-inner-html=\"&lt;p id=&quot;p1&quot;&gt;Hello &lt;em&gt;World&lt;/em&gt;&lt;/p&gt;:World\"")
    );
    assert!(page.serialize_html_async().await.unwrap().contains(
        "data-inner-text-setter=\"hello &lt;b&gt;literal&lt;/b&gt;:hello &amp;lt;b&amp;gt;literal&amp;lt;/b&amp;gt;\""
    ));
    assert!(page.serialize_html_async().await.unwrap().contains(
        "data-outer-html=\"&lt;div id=&quot;old-node&quot;&gt;old&lt;/div&gt;:true:&lt;section id=&quot;new-node&quot;&gt;&lt;strong&gt;New&lt;/strong&gt;&lt;/section&gt;\""
    ));

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn details_dialog_accessors_cover_open_return_value_and_close_event() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/details-dialog-accessors"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-details=\"true:true:group-a:group-a\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dialog-initial=\"false:\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dialog-show=\"true:true:seed:null\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dialog-close=\"false:false:done:null\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dialog-show-modal=\"true:true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dialog-close-count-after-call=\"0\""),
        "the close event must not run synchronously inside dialog.close()"
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dialog-close-count=\"1\""),
        "the queued close event must run exactly once"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn html_element_reflected_accessors_cover_simple_tag_specific_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/html-element-reflected-accessors"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-li-value=\"0:7:7\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-ol-start=\"1:3:3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-ol-reversed=\"true:true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-ol-type=\":A:A\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-optgroup=\"true:true:Group A\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-quote-cite=\"https://example.test/source:https://example.test/source\""
        )
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-table-cell=\"1:1:1000:1000:0:0:65534:65534\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-time-datetime=\"2026-04-18T00:00:00Z:2026-04-18T00:00:00Z\"")
    );
    assert!(page.serialize_html_async().await.unwrap().contains(
        "data-meta-content=\"::keywords:alpha beta:alpha beta:refresh:refresh:undefined\""
    ));
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-track-kind=\"subtitles:captions:captions:metadata:bad-kind\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-global-attributes=\"alpha:alpha:one two:one two:after:before:rtl:rtl\""
        )
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-hidden-tabindex=\"true:true:-1:5:5:0:-1\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn script_state_snapshot_handles_throwing_to_primitive() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let page = browser
        .fetch(&server.url("/compat/script-state-snapshot-handles-throwing-to-primitive"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("<main id=\"after\">after</main>")
    );
    assert_eq!(
        page.script_execution().global("throwingPrimitive"),
        Some(&JsValueSnapshot::Unsupported("[object]".to_owned()))
    );
    assert_eq!(
        page.script_execution().global("throwingAccessor"),
        Some(&JsValueSnapshot::Unsupported("[accessor]".to_owned()))
    );
    assert_eq!(
        page.script_execution().global("revokedArrayProxy"),
        Some(&JsValueSnapshot::Unsupported("[object]".to_owned()))
    );
    assert_eq!(
        page.script_execution().global("aSnapshotOrder"),
        Some(&JsValueSnapshot::String("a".to_owned()))
    );
    assert_eq!(
        page.script_execution().global("zSnapshotOrder"),
        Some(&JsValueSnapshot::String("z".to_owned()))
    );
    assert_eq!(
        page.script_execution().global("快照键"),
        Some(&JsValueSnapshot::String("unicode".to_owned()))
    );
    assert_eq!(page.script_execution().global("17"), None);
    assert_eq!(page.script_execution().global("Object"), None);

    let snapshot_order = page
        .script_execution()
        .globals()
        .keys()
        .filter(|name| name.ends_with("SnapshotOrder") || name.as_str() == "快照键")
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        snapshot_order,
        vec!["aSnapshotOrder", "zSnapshotOrder", "快照键"]
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn script_state_snapshot_ignores_set_prototype_tamper() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let page = browser
        .fetch(&server.url("/compat/script-state-snapshot-ignores-set-prototype-tamper"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("<main id=\"after\">after</main>")
    );
    assert_eq!(
        page.script_execution()
            .global("snapshotAfterSetPrototypeTamper"),
        Some(&JsValueSnapshot::String("survived".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn style_link_stylesheet_accessors_keep_link_sheet_null_without_source() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/style-link-stylesheet-accessors"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-style-accessors=\"text/css:text/css:screen:render:true:false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-style-sheet=\"true:true:true:true\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-link-accessors=\"preload stylesheet:use-credentials:use-credentials:anonymous:invalid-token\""
        )
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-link-sheet=\"true:true:true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn failed_linked_stylesheet_exposes_an_empty_sheet_after_error() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;

    let result = page
        .evaluate_runtime_expression_with_await_async(
            r#"
new Promise(resolve => {
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = '/missing-linked-stylesheet.css';
  link.onerror = () => resolve(JSON.stringify({
    sheet: link.sheet instanceof CSSStyleSheet,
    owner: link.sheet && link.sheet.ownerNode === link,
    rules: link.sheet && link.sheet.cssRules.length
  }));
  document.head.appendChild(link);
})
"#,
            true,
        )
        .await?;

    let evaluated = evaluated_string(result.clone());
    assert_eq!(
        evaluated.as_deref(),
        Some(r#"{"sheet":true,"owner":true,"rules":0}"#),
        "{result:?}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn shadow_dom_slot_template_accessors_cover_wrapper_identity_and_assignment() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/shadow-dom-slot-template-accessors"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-shadow-root=\"true:true:open\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-slot-reflection=\"named:named:true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-slot-assignment=\"assigned:assigned\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-template-content=\"11:1:true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn navigator_extended_exposes_connection_send_beacon_and_stubs() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/navigator-extended"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-connection-type=\"object\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-connection-effective-type=\"4g\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-connection-downlink=\"10\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-connection-rtt=\"50\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-connection-save-data=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-send-beacon-type=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-device-memory=\"8\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-do-not-track=\"null\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-get-battery-type=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-permissions-type=\"object\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-permissions-query-type=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-storage-type=\"object\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-storage-estimate-type=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-media-devices-type=\"object\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-service-worker-type=\"object\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-clipboard-type=\"object\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-clipboard-read-text-type=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-clipboard-write-text-type=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-clipboard-roundtrip=\"moli clipboard\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn dom_content_loaded_event_has_correct_bubbles_and_cancelable() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/compat/event-bubbles")).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dcl-bubbles=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dcl-cancelable=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dcl-type=\"DOMContentLoaded\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn event_listener_exceptions_do_not_abort_dispatch() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/event-listener-exception-dispatch"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dispatch-continued=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dispatch-error=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-listener-order=\"first|second\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-handler-order=\"handler|listener\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-stop-immediate-order=\"first\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn custom_element_callback_exceptions_do_not_abort_reactions() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/custom-element-callback-exception"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-connected-continued=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-connected-error=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-connected-order=\"throw-connected|good-connected\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-attribute-continued=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-attribute-error=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-attribute-order=\"one|two\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn local_event_target_callback_exceptions_do_not_abort_dispatch() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/local-event-target-callback-exception"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-font-continued=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-font-error=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-font-order=\"first|second|handler\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mql-continued=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mql-error=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mql-order=\"handler|first|second\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-simple-continued=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-simple-error=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-simple-order=\"handler|first|second\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn sync_foreach_callback_exceptions_propagate_and_stop_iteration() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/sync-foreach-callback-exception"))
        .await?;

    for prefix in [
        "headers",
        "usp",
        "formdata",
        "fontfaceset",
        "collection",
        "classlist",
    ] {
        assert!(
            page.serialize_html_async()
                .await
                .unwrap()
                .contains(&format!("data-{prefix}-count=\"1\"")),
            "missing count for {prefix}: {}",
            page.serialize_html_async().await.unwrap()
        );
        assert!(
            page.serialize_html_async()
                .await
                .unwrap()
                .contains(&format!("data-{prefix}-caught=\"true\"")),
            "missing caught flag for {prefix}: {}",
            page.serialize_html_async().await.unwrap()
        );
        assert!(
            page.serialize_html_async()
                .await
                .unwrap()
                .contains(&format!("data-{prefix}-error=\"{prefix} boom\"")),
            "missing error for {prefix}: {}",
            page.serialize_html_async().await.unwrap()
        );
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn message_channel_and_message_port_deliver_messages_in_order() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/message-channel"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            r#"
document.body.dataset.onmessage === "hello" &&
document.body.dataset.startedOrder === "first,second,third" &&
document.body.dataset.duplex === "1:1" &&
document.body.dataset.objectType === "test" &&
document.body.dataset.objectValue === "42"
"#,
            std::time::Duration::from_secs(3),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-channel-tag=\"[object MessageChannel]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-port-instance=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-port-tag=\"[object MessagePort]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-port-distinct=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-onmessage=\"hello\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-started-order=\"first,second,third\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-duplex=\"1:1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-object-type=\"test\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-object-value=\"42\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn shared_worker_iframe_performance_now_keeps_worker_clock_ahead_of_later_iframe()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/shared-worker-iframe-performance-owner"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.__sharedWorkerPerfResult && window.__sharedWorkerPerfResult.done",
            std::time::Duration::from_secs(3),
        )
        .await?;

    let result_value = page
        .evaluate_runtime_expression_async("JSON.stringify(window.__sharedWorkerPerfResult)")
        .await?;
    let result_string = evaluated_string(result_value)
        .ok_or_else(|| anyhow::anyhow!("missing shared worker performance result"))?;
    let result: serde_json::Value = serde_json::from_str(&result_string)?;
    assert_eq!(
        result.get("error").and_then(serde_json::Value::as_str),
        Some(""),
        "{result_string}"
    );
    let events = result
        .get("events")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("missing shared worker performance events"))?;
    assert_eq!(events.len(), 3, "{result_string}");
    assert_eq!(
        events[0].get("phase").and_then(serde_json::Value::as_str),
        Some("top-callback"),
        "{result_string}"
    );
    assert_eq!(
        events[1].get("phase").and_then(serde_json::Value::as_str),
        Some("child-before-post"),
        "{result_string}"
    );
    assert_eq!(
        events[2].get("phase").and_then(serde_json::Value::as_str),
        Some("child-callback"),
        "{result_string}"
    );
    assert_eq!(
        events[2]
            .get("workerAfterIframeStart")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "{result_string}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn window_queue_microtask_ignores_promise_tamper() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/queue-microtask-ignores-promise-tamper"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-promise-resolve-used=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-queue-microtask-fired=\"yes\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn post_message_ignores_page_tampered_queue_microtask() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/post-message-ignores-page-tampered-queue-microtask"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.dataset.postMessageDelivered === 'yes'",
            Duration::from_secs(2),
        )
        .await?;

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
            .contains("data-post-message-delivered=\"yes\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn message_port_ignores_page_tampered_queue_microtask() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/message-port-ignores-page-tampered-queue-microtask"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.dataset.messagePortDelivered === 'yes'",
            Duration::from_secs(2),
        )
        .await?;

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
            .contains("data-message-port-delivered=\"yes\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn mutation_observer_ignores_page_tampered_queue_microtask() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/mutation-observer-ignores-page-tampered-queue-microtask"))
        .await?;

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
            .contains("data-mutation-observer-delivered=\"yes\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mutation-record-count=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn xhr_ignores_page_tampered_queue_microtask() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/xhr-ignores-page-tampered-queue-microtask"),
            RenderedDomWaitUntil::NetworkIdle,
            std::time::Duration::from_secs(5),
        )
        .await?;

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
            .contains("data-xhr-delivered=\"yes\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-xhr-status=\"200\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn mutation_observer_delivers_multiple_matching_observers_in_creation_order() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/mutation-observer-ordering"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-callback-order=\"first,second\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-first-records=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-second-records=\"2\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn performance_measure_entries_are_observable_and_clearable() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/performance-measure-observer"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "performanceMarkInstance"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceMeasureInstance"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceGetEntriesCount"),
        Some(&JsValueSnapshot::Number(6.0))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceGetEntriesByTypeCount"),
        Some(&JsValueSnapshot::Number(3.0))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceGetEntriesByNameCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceTimingSameObject"),
        Some(&JsValueSnapshot::Bool(true))
    );
    match diagnostic_global(&page, "performanceTimingNavigationStart") {
        Some(JsValueSnapshot::Number(value)) => assert!(
            *value > 0.0,
            "expected performance.timing.navigationStart to be positive, got {value}"
        ),
        other => panic!("expected numeric performance.timing.navigationStart, got {other:?}"),
    }
    assert_eq!(
        diagnostic_global(&page, "performanceNavigationType"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceLegacyMeasureDuration"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceMeasureName"),
        Some(&JsValueSnapshot::String("duration".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceMeasureStartTime"),
        Some(&JsValueSnapshot::Number(10.0))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceMeasureEntryType"),
        Some(&JsValueSnapshot::String("measure".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceMeasureDuration"),
        Some(&JsValueSnapshot::Number(32.0))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceMeasureDetailKind"),
        Some(&JsValueSnapshot::String("demo".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceMeasureObservedCount"),
        Some(&JsValueSnapshot::Number(2.0))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceMeasureObservedName"),
        Some(&JsValueSnapshot::String("duration".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceMeasureObservedType"),
        Some(&JsValueSnapshot::String("measure".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceMeasureObservedByTypeCount"),
        Some(&JsValueSnapshot::Number(2.0))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceMeasureObservedByNameCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceMeasureObservedByNameType"),
        Some(&JsValueSnapshot::String("measure".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceEntryTypesBufferedCount"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceEntryTypesBufferedTypes"),
        Some(&JsValueSnapshot::String("[]".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceEntryTypesBufferedNames"),
        Some(&JsValueSnapshot::String("[]".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceBufferedMarkCount"),
        Some(&JsValueSnapshot::Number(3.0))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceBufferedMarkTypes"),
        Some(&JsValueSnapshot::String(
            "[\"mark\",\"mark\",\"mark\"]".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceBufferedMarkNames"),
        Some(&JsValueSnapshot::String(
            "[\"start\",\"end\",\"named-mark\"]".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "performanceMeasureBufferedAfterClear"),
        Some(&JsValueSnapshot::Number(0.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn secondary_webapis_cover_file_file_list_and_resize_observer_basics() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default().with_layout_policy(LayoutPolicy::OnDemand))?;

    let page = browser
        .fetch(&server.url("/compat/secondary-webapis"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-file-instance=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-file-blob-instance=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-file-name=\"note.txt\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-file-type=\"text/plain\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-file-last-modified=\"123\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-file-size=\"3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-file-list-tag=\"[object FileList]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-file-list-length=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-file-list-item-same=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-resize-target=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-resize-entry-count=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-resize-observer-instance=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-resize-rect-width=\"37\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-resize-rect-height=\"19\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-resize-content-box-size=\"1:37:19\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-resize-border-box-size=\"1:37:19\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-resize-device-pixel-content-box-size=\"1:37:19\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-resize-take-records-count=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-resize-take-records-target=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn url_search_params_and_form_data_cover_native_constructor_and_iteration_basics()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/compat/url-form-data")).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-usp-instance=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-usp-tag=\"[object URLSearchParams]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-usp-first=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-usp-all=\"[&quot;1&quot;,&quot;3&quot;]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-usp-keys=\"b,a,a\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-usp-values=\"2,1,3\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap()
            .contains("data-usp-entries=\"[[&quot;b&quot;,&quot;2&quot;],[&quot;a&quot;,&quot;1&quot;],[&quot;a&quot;,&quot;3&quot;]]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-usp-sorted=\"a=1&amp;a=3&amp;b=2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-usp-usv-surrogate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-url-search-sync=\"?x=1&amp;y=2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-url-usv-surrogate=\"/%EF%BF%BD\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-formdata-tag=\"[object FormData]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-formdata-get=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-formdata-all-count=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-formdata-second=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-formdata-keys=\"a,a,b\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-formdata-entry-count=\"3\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap()
            .contains("data-formdata-submitter=\"[[&quot;alpha&quot;,&quot;1&quot;],[&quot;action&quot;,&quot;save&quot;]]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-formdata-usv-surrogate=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn form_data_ignores_page_tampered_node_contains() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/form-data-ignores-page-tampered-node-contains"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-node-contains-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-formdata-in-legend=\"exempt\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-formdata-blocked=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-formdata-entries=\"[[&quot;in-legend&quot;,&quot;exempt&quot;]]\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_user_agent_suffix_updates_navigator_user_agent_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let mut config = AppConfig::default();
    config.fetch_mut().set_user_agent_suffix("internal-tester");
    let browser = Browser::new(config)?;

    let page = browser
        .fetch(&server.url("/compat/window-host-globals"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-navigator-user-agent=\"{} internal-tester\"",
                FetchConfig::DEFAULT_USER_AGENT
            ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn browser_surface_details_expose_array_like_tags_and_storage_methods() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/browser-surface-details"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mime-tag=\"[object MimeTypeArray]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-plugin-tag=\"[object PluginArray]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-storage-tag=\"[object Storage]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mime-item-hit=\"application/pdf\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mime-named-item-hit=\"application/pdf\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-plugin-item-hit=\"PDF Viewer\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-plugin-named-item-hit=\"PDF Viewer\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mime-item-null=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mime-named-item-null=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-plugin-item-null=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-plugin-named-item-null=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-plugin-refresh-undefined=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-storage-prototype=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-storage-roundtrip=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-storage-length-after-set=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-storage-key0=\"alpha\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-storage-length-after-remove=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-scroll-restoration=\"auto\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn window_crypto_exposes_random_values_random_uuid_and_subtle() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/window-crypto"),
            RenderedDomWaitUntil::DomStable,
            std::time::Duration::from_secs(5),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-crypto=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-crypto-instance=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-crypto-tag=\"[object Crypto]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-subtle-instance=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-subtle-tag=\"[object SubtleCrypto]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-get-random-values-returns-same=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-get-random-values-mutates=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-uuid-valid=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-uuid-distinct=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-quota-error=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-crypto-key-instance=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-exported-length=\"128\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sign-buffer=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-verified=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-x25519-private=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-x25519-public=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-derived-length=\"16\"")
    );
    assert!(page.serialize_html_async().await.unwrap().contains(
        "data-digest-sha256=\"1bc375bb92459685194dda18a4b835f4e2972ec1bde6d9ab3db53fcc584a6580\""
    ));
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-crypto-done=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-crypto-error=\"\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn window_named_access_exposes_elements_by_id_without_shadowing_builtins() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-named-access"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-global-id=\"i\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-id=\"testDiv\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-shadowed=\"100\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-location-function=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-location-shadowed=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-location-stringifier=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-location-stringifier=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn window_match_media_exposes_media_query_list_shape() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-match-media"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "matchMediaBasicType"),
        Some(&JsValueSnapshot::String("object".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "matchMediaBasicMedia"),
        Some(&JsValueSnapshot::String("(min-width: 600px)".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "matchMediaBasicMatches"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "matchMediaEventTargetShape"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "matchMediaChangeEventCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "matchMediaLegacyListenerCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "matchMediaOnchangeCount"),
        Some(&JsValueSnapshot::Number(2.0))
    );
    assert_eq!(
        diagnostic_global(&page, "matchMediaCancelableDispatchReturn"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "matchMediaSecondDispatchReturn"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn window_screen_and_orientation_expose_minimal_event_target_semantics() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-screen-events"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "screenEventListenerCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "screenDispatchReturn"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "screenOrientationListenerCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "screenOrientationOnchangeCount"),
        Some(&JsValueSnapshot::Number(2.0))
    );
    assert_eq!(
        diagnostic_global(&page, "screenOrientationFirstDispatchReturn"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "screenOrientationSecondDispatchReturn"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn window_css_exposes_escape_and_supports() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/compat/window-css")).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-type=\"object\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-escape-basic=\"hello\\ world\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-escape-leading-digit=\"\\30 abc\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-escape-special=\"a\\(b\\)\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-escape-null=\"�abc\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-supports-pair=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-supports-condition=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-style-container-in=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-style-container-type-in=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-style-container-name-in=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-computed-container-default=\"none\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-computed-container-type-default=\"normal\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-computed-container-name-default=\"none\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-supports-container-type=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-supports-container-name=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-supports-container-name-double-hyphen=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-supports-container-name-hyphen=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-supports-container-name-hyphen-digit=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-supports-container-shorthand=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-supports-container-shorthand-normal=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-supports-container-shorthand-multi-name=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-supports-container-invalid=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-supports-container-name-invalid=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-supports-container-shorthand-missing-name=\"false\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn document_fonts_remove_event_listener_keeps_other_event_types_intact() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-fonts-events"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-loading-a=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-loading-b=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-loadingdone=\"1\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_match_media_parsing_cases_serialize_like_wpt_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-match-media-parsing"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-empty=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-all=\"all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-list=\"all, all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-feature=\"(color)\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-open-feature=\"(color)\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-close=\"not all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-empty-items=\"not all, not all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-trailing=\"foo, not all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-case-all=\"all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-case-feature=\"(height)\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-case-min-width=\"(min-width: 0cm)\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-list=\"not all, (color)\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-double-empty=\"not all, not all, not all\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_style_attr_braces_drop_only_malformed_declarations() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-style-attr-braces"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-case1-color=\"green\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-case1-background=\"lime\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-case2-color=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-case2-background=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-case3-color=\"green\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-case3-background=\"lime\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_style_attr_urls_preserve_url_tokens_across_base_variants() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-style-attr-urls"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-no-base=\"url(&quot;support/swatch-lime.png&quot;)\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-with-base=\"url(&quot;swatch-lime.png&quot;)\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-with-xml-base=\"url(&quot;swatch-red.png&quot;)\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_query_is_supports_complex_selector_list_arguments() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/compat/servo-query-is")).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-simple=\"b1,c1,d,b2,b3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-compound=\"d,e1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-complex=\"f1,b2,h1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-nested=\"e2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-nested-where=\"c1,d\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-nested-not=\"h1\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_query_where_supports_complex_selector_list_arguments() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-query-where"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-simple=\"b1,c1,d,b2,b3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-compound=\"d,e1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-complex=\"f1,b2,h1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-nested=\"e2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-nested-is=\"c1,d\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-nested-not=\"h1\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_match_media_case_insensitive_rules_apply_and_non_ascii_stays_invalid() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-match-media-case-insensitive"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-media-all=\"all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-media-feature=\"(height)\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-media-width=\"(min-width: 0cm)\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-matches-all=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_match_media_invalid_media_type_keywords_serialize_to_not_all() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-match-media-invalid-types"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-and=\"not all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-or=\"not all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-not=\"not all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-only=\"not all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-not-and=\"not all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-not-or=\"not all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-not-not=\"not all\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-not-only=\"not all\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_match_media_feature_states_follow_default_browser_like_assumptions() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-match-media-feature-states"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-update-bool=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-update-fast=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-update-slow=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-update-none=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-scripting-bool=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-scripting-enabled=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-scripting-initial-only=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-scripting-none=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-display-mode-bool=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-display-mode-browser=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-display-mode-standalone=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dynamic-range-bool=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dynamic-range-standard=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dynamic-range-high=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-video-dynamic-range-bool=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-video-dynamic-range-standard=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-video-dynamic-range-high=\"false\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_match_media_aspect_ratio_serialization_adds_ratio_spacing() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-match-media-aspect-ratio-serialization"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-aspect-ratio=\"(aspect-ratio: 1 / 3)\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-min-aspect-ratio=\"(min-aspect-ratio: 59 / 79)\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-device-aspect-ratio=\"(device-aspect-ratio: 1280 / 720)\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_match_media_preferences_follow_fixed_default_profile() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-match-media-preferences"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-color-scheme-bool=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-color-scheme-light=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-color-scheme-dark=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reduced-motion-bool=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reduced-motion-no-preference=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reduced-motion-reduce=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reduced-data-bool=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reduced-data-no-preference=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reduced-data-reduce=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reduced-transparency-bool=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reduced-transparency-no-preference=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reduced-transparency-reduce=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-prefers-contrast-bool=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-prefers-contrast-no-preference=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-prefers-contrast-more=\"false\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_media_query_list_event_target_surface_matches_wpt_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-media-query-list-event-target"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-add-listener-optional=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-add-listener-dedup=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-add-listener-order=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-remove-listener=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-onchange=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-handle-event=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-handle-event-remove=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-once=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dispatch-default-prevented=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_css_supports_conditions_cover_roundtrip_shaped_subsets() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-css-supports-conditions"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-pair=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-not-custom-prop=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-and-future=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-future-escaped=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-double-parens=\"false\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_cssfloat_cssom_exposes_cssfloat_on_inline_and_computed_style() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-cssfloat-cssom"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-has-cssfloat=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after=\"right\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_css_escape_dom_api_covers_identifier_escaping_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-css-escape-dom-api"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-no-arg-throws=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-symbol-throws=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-stringify-throws=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-undefined-stringify=\"undefined\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-null-replacement=\"a�b\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-leading-digit=\"\\30 a\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-leading-hyphen-digit=\"-\\30 a\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-lone-hyphen=\"\\-\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-space-bang=\"\\ \\!xy\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-control-chars=\"\\1 \\2 \\1e \\1f \"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-bool-stringify=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-null-stringify=\"null\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_css_supports_dom_api_covers_condition_and_property_value_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-css-supports-dom-api"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-simple-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-simple-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-and-or=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-not=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-semicolon=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-display-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-display-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-top-percent=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-top-number=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-background-url=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-background-invalid=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-color-rgba=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-z-index-number=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-important=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-property-empty=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-unsupported-text-decoration-style=\"false\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_css_supports_shorthands_and_wide_keywords_cover_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-css-supports-shorthands"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-border-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-border-inherit=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-border-mixed-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-border-side-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-border-side-inherit=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-border-side-mixed-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-border-radius-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-border-radius-inherit=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-border-radius-mixed-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-border-spacing-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-border-spacing-inherit=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-border-spacing-mixed-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-font-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-font-inherit=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-font-mixed-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-list-style-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-list-style-inherit=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-list-style-mixed-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-margin-inherit=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-margin-mixed-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-outline-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-outline-inherit=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-outline-mixed-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-outline-color-invert=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-overflow-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-overflow-inherit=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-overflow-mixed-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-overflow-overlay=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-transform-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-transform-inherit=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-transform-mixed-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-transition-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-transition-inherit=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-transition-mixed-false=\"false\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_css_supports_boolean_syntax_covers_negation_and_operator_rules() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-css-supports-syntax"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-not-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-double-not-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-triple-not-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-and-chain-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-and-chain-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-or-chain-true=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-or-chain-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mixed-operators-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-not-mixed-operators-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-not-no-space-false=\"false\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_css_supports_coercion_and_invalid_value_cases_cover_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-css-supports-coercion"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-display-semicolon-garbage=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-display-semicolon-leading-garbage=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-two-arg-undefined-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-true-empty-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-array-none-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-display-empty-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-display-colon-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-content-array-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-content-important-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-margin-internal-unit-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-background-spaced-property-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-font-family-newline-false=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-four-nots-false=\"false\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_fontfaceset_basic_set_operations_track_size_and_type_errors() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-fontfaceset-basic"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial-size=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-add-returns-self=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-size-after-add=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-has-after-add=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-second-add-size=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-delete-added=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-size-after-delete=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-delete-again=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-has-after-delete=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-clear-size=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-add=\"TypeError\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-has=\"TypeError\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-delete=\"TypeError\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_fontfaceset_iteration_exposes_set_like_iteration_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-fontfaceset-iteration"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-for-each-families=\"Font1,Font2,Font3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-for-each-arg-pair=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-for-each-self=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-for-each-this=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-keys-families=\"Font1,Font2,Font3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-values-families=\"Font1,Font2,Font3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-entries-pair=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-iterator-families=\"Font1,Font2,Font3\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_fontfaceset_check_returns_true_for_platform_font_queries_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-fontfaceset-platform-fonts"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-arial=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-nonexistent=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sans-serif=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-fallback-list=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-plain-list=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_fontfaceset_detached_document_surface_does_not_crash_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-fontfaceset-unattached-document"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-ok=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-detached-ok=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_webfont_insert_rule_path_handles_multiple_font_face_rules_without_crashing()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-webfont-insert-rule-no-crash"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-ok=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-span-count=\"2\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_media_query_list_add_and_remove_listener_cover_wpt_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-media-query-list-add-remove-listener"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-optional-ok=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dedupe-count=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-order=\"first,second\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-handle-event-count=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-shared-count=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-removed-count=\"0\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_fontfaceset_event_handlers_fire_in_loading_then_done_order_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-fontfaceset-events-subset"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-order=\"loading,loadingdone\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-count=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-ready-status=\"loaded\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_cssstyledeclaration_set_property_preserves_priority_override_subset() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-important-js-override"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-set-non-important=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-prop-non-important=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-set-to-important=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-important-to-non-important=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-prop-important-to-non-important=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-important-to-important=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_cssstylesheet_rule_mutation_keeps_cssrules_list_operable_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-cssstylesheet-rule-mutation"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-ok=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-insert-0=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-insert-1=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-group-insert-0=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-group-insert-1=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-group-length-after-insert=\"3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-group-first-parent=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-group-first-sheet=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-group-length-after-delete=\"2\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-group-css-text-after-style=\"@media screen {\n  main { display: block; color: green; }\n  div { color: red; }\n}\""
        )
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-keyframes-rule-brand=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-keyframes-length=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-keyframe-rule-brand=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-keyframe-find=\"0%\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-keyframe-parent=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-keyframe-sheet=\"true\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap()
            .contains("data-keyframes-after-append=\"@keyframes fade {\n0% { opacity: 0; }\n100% { opacity: 1; }\n50% { opacity: 0.5; transform: scale(1); }\n}\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap()
            .contains("data-keyframes-after-keytext=\"@keyframes fade {\n0% { opacity: 0; }\n100% { opacity: 1; }\n75% { opacity: 0.5; transform: scale(1); }\n}\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap()
            .contains("data-keyframes-after-style=\"@keyframes fade {\n0% { opacity: 0; }\n100% { opacity: 1; }\n75% { transform: scale(1); opacity: 0.75; }\n}\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap()
            .contains("data-keyframes-after-csstext=\"@keyframes fade {\n0% { opacity: 0; }\n100% { opacity: 1; }\n80% { opacity: 0.8; transform: scale(1); }\n}\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-keyframe-type-after-csstext=\"8\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-keyframe-invalid-preserved=\"80%\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap()
            .contains("data-keyframes-after-delete=\"@keyframes fade {\n0% { opacity: 0; }\n100% { opacity: 1; }\n}\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-keyframes-missing=\"true\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap()
            .contains("data-fontface-css-text-after-style=\"@font-face { font-family: Test; src: url(&quot;test.woff2&quot;); font-style: italic; }\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_delete_rule_path_does_not_crash_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-delete-rule-no-crash"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-ok=\"true\"")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("data-length=\"-1\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_box_sizing_aliases_match_prefixed_and_unprefixed_surface_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-box-sizing-backwards-compat"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-prefixed-get-box=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-prefixed-get-webkit=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-prefixed-prop-box=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-prefixed-prop-webkit-upper=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-prefixed-prop-webkit-lower=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-unprefixed-get-box=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-unprefixed-get-webkit=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-unprefixed-prop-box=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-unprefixed-prop-webkit-upper=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-unprefixed-prop-webkit-lower=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_fontfaceset_ready_on_detached_frame_document_does_not_crash_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-fontfaceset-detached-frame-ready"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-ok=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_fontfaceset_ready_keeps_stable_promise_until_loading_starts_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-fontfaceset-ready-basic"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-ok=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-same=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-resolved-self=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-same-after-add=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-status-after-add=\"loaded\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_fontfaceset_ignores_invalid_css_connected_generic_family_names_subset() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-fontfaceset-invalid-family-names"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-size=\"0\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_fontfaceset_set_operations_cover_connected_and_manual_faces_subset() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-fontfaceset-set-operations-subset"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-size=\"3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-for-each-order=\"Font1,Font2,Font3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-for-each-this=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-keys-order=\"Font1,Font2,Font3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-values-order=\"Font1,Font2,Font3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-entries-pairs=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-has-font1=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-manual-add-size=\"4\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-manual-has=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-manual-dedup-size=\"4\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-manual-delete-size=\"3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-manual-delete-has=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-clear-size=\"3\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_fontfaceset_historical_rejects_constructing_with_iterable() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-fontfaceset-historical"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-constructor-throws=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_fontfaceset_connected_has_tracks_style_removal_and_manual_faces() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-fontfaceset-connected"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial-size=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial-has-connected=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial-has-manual=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-add-size=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-add-has-connected=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-add-has-manual=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-remove-style-size=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-remove-style-has-connected=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-remove-style-has-manual=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-delete-manual-size=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-delete-manual-has-connected=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-delete-manual-has-manual=\"false\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_fontfaceset_connected_ignores_page_tampered_style_queries() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server.url("/compat/servo-fontfaceset-connected-ignore-page-tampered-style-queries"),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-get-elements-by-tag-name-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-is-connected-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-disabled-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-type-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-text-content-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-size=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-has-connected=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-connected-family=\"TamperFace\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_fontfaceset_clear_and_delete_keep_css_connected_faces() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-fontfaceset-connected-clear-delete"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial-size=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-clear-size=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-delete-size=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-delete-has-connected=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_fontfaceset_has_tracks_css_connected_and_manual_faces() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-fontfaceset-has"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial-has-connected=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial-has-manual=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-add-has-connected=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-add-has-manual=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-remove-style-has-connected=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-remove-style-has-manual=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-delete-manual-has-connected=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-delete-manual-has-manual=\"false\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_fontfaceset_delete_and_clear_do_not_remove_css_connected_faces() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-fontfaceset-delete-clear-css-connected"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-before-size=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-delete-size=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-delete-has=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-clear-size=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-clear-has=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_fontfaceset_load_replaces_ready_and_rejects_css_wide_keywords() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-fontfaceset-load-ready"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-ready-stable=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-ready-replaced=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-load-promise=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-ready-status=\"loaded\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-css-wide-keyword=\"SyntaxError\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-css-wide-keyword-prefixed=\"SyntaxError\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_fontfaceset_empty_family_load_resolves_to_array() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-fontfaceset-empty-family-load"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-result-is-array=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_fontfaceset_load_after_removing_document_element_does_not_crash() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-fontfaceset-no-root-element"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-load-called=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_fontfaceset_update_after_stylesheet_change_updates_document_fonts_size() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-fontfaceset-update-after-stylesheet-change"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-size-before=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-size-after=\"0\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn servo_fontfaceset_load_uses_updated_css_connected_faces() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/servo-fontfaceset-load-css-connected"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-result-length=\"0\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_stylesheetlist_style_only_keeps_connected_style_nodes_when_disabled() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-stylesheetlist-style-only"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial-length=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-disabled-length=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-owner-ids=\"s1,s2\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_stylesheetlist_item_returns_entries_and_null_out_of_bounds() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-stylesheetlist-item"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-length=\"3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-item0-owner=\"s1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-item1-owner=\"s2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-item2-owner=\"s3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-item0-eq-index=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-oob-null=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-oob-index-undefined=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-iterator-type=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-spread-owner-ids=\"s1,s2,s3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-iter-error=\"\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_stylesheetlist_mixed_link_and_style_entries_follow_disabled_rules() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-stylesheetlist-mixed-disabled"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial-length=\"5\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial-owner-ids=\"s1,s2,s3,s4,s5\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-initial-spread-owner-ids=\"s1,s2,s3,s4,s5\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-disable-length=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-disable-owner-ids=\"s4,s5\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-disable-spread-owner-ids=\"s4,s5\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-iter-error=\"\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_cssom_missing_arguments_throw_on_css_style_declaration_methods() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-cssom-missing-arguments"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-style-type=\"CSSStyleProperties\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-item-throws=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-get-property-value-throws=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-get-property-priority-throws=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-set-property-no-args-throws=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-set-property-name-only-throws=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-remove-property-throws=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chrome_overflow_property_serializes_shorthand_and_longhands_like_blink_subset()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/chrome-overflow-property"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test0-overflow=\"auto\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test0-overflow-x=\"auto\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test0-overflow-y=\"auto\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test1-overflow=\"auto\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test1-overflow-x=\"auto\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test1-overflow-y=\"auto\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test2-overflow=\"scroll\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test2-overflow-x=\"scroll\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test2-overflow-y=\"scroll\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test3-overflow=\"overlay hidden\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test3-overflow-x=\"overlay\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test3-overflow-y=\"hidden\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test4-overflow=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test4-overflow-x=\"auto\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test4-overflow-y=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test5-overflow=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test5-overflow-x=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test5-overflow-y=\"auto\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test6-overflow=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test6-overflow-x=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test6-overflow-y=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test7-overflow=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test7-overflow-x=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-test7-overflow-y=\"\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn abort_signal_any_aborts_from_first_source_and_preserves_reason() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/abort-signal-any"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-any-type=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-empty-aborted=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-empty-reason=\"undefined\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-before-aborted=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-before-reason=\"undefined\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-aborted=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-reason=\"second\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-event-count=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-late-reason=\"second\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-after-late-event-count=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-pre-aborted=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-pre-reason=\"ready\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn blob_urls_cover_create_object_url_fetch_xhr_and_revoke() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/compat/blob-urls")).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-create-prefix=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-create-unique=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-fetch-status=\"200\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-fetch-ok=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-fetch-url=\"blob:")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-fetch-type=\"text/plain\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-fetch-text=\"Hello from blob!\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-xhr-status=\"200\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-xhr-url=\"blob:")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-xhr-text=\"Hello from blob!\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-revoke-safe=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn dom_rect_constructor_exposes_mutable_core_fields_and_derived_edges() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/compat/dom-rect")).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-basic=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-derived=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mutated=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-negative-derived=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn image_data_exposes_constructor_dimensions_and_mutable_clamped_storage() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/compat/image-data")).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-width=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-height=\"3\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-color-space=\"srgb\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-pixel-format=\"rgba-unorm8\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-data-len=\"24\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-zeroes=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mutable=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-too-large=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn web_streams_cover_response_body_readers_pipe_and_text_codec_wrappers() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/compat/web-streams")).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-response-body=\"hello world\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-response-text=\"hello world\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-pipe-through=\"HELLO\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-pipe-to=\"ab\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-text-encoder-stream=\"104,105\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-text-decoder-stream=\"hello\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn intersection_observer_options_reflect_root_margin_and_thresholds() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/intersection-observer-options"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "intersectionObserverRootMatches"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "intersectionObserverRootMargin"),
        Some(&JsValueSnapshot::String("10px 20px 10px 20px".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "intersectionObserverThresholds"),
        Some(&JsValueSnapshot::String("[0.25,0.75]".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "intersectionObserverEntryPrototypeShape"),
        Some(&JsValueSnapshot::String(
            r#"{"time":true,"rootBounds":true,"boundingClientRect":true,"intersectionRect":true,"isIntersecting":true,"isVisible":true,"intersectionRatio":true,"target":true,"getter":"function","value":0.5}"#.to_owned()
        ))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn rootless_intersection_observer_uses_deep_mock_flow() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default().with_layout_policy(LayoutPolicy::Mock))?;

    let mut page = browser.fetch(&server.url("/static")).await?;

    let value = page
        .evaluate_runtime_expression_with_await_async(
            r#"
            (async () => {
                document.body.textContent = "";

                const early = document.createElement("div");
                early.id = "early";
                document.body.appendChild(early);

                const layout = document.createElement("div");
                document.body.appendChild(layout);
                for (let i = 0; i < 18; i++) {
                    const section = document.createElement("section");
                    section.className = "story-section";
                    for (let j = 0; j < 8; j++) {
                        const item = document.createElement("div");
                        item.className = "story-item";
                        section.appendChild(item);
                    }
                    layout.appendChild(section);
                }

                const bottom = document.createElement("div");
                bottom.id = "bottom-ad";
                bottom.className = "index_bottomDbox_SAmA4";
                layout.appendChild(bottom);

                const observe = (target) => new Promise((resolve) => {
                    let done = false;
                    const observer = new IntersectionObserver((entries) => {
                        if (done) return;
                        done = true;
                        const entry = entries[0];
                        observer.disconnect();
                        resolve({
                            isIntersecting: entry && entry.isIntersecting,
                            ratio: entry && entry.intersectionRatio,
                            top: entry && entry.boundingClientRect && entry.boundingClientRect.top
                        });
                    });
                    observer.observe(target);
                    setTimeout(() => {
                        if (done) return;
                        done = true;
                        observer.disconnect();
                        resolve({ timeout: true });
                    }, 100);
                });

                const [earlyEntry, bottomEntry] = await Promise.all([
                    observe(early),
                    observe(bottom)
                ]);
                return JSON.stringify({
                    earlyEntry,
                    bottomEntry,
                    publicBottomTop: bottom.getBoundingClientRect().top
                });
            })()
            "#,
            true,
        )
        .await?;
    let snapshot = value.get("value").and_then(|value| value.as_str());

    assert_eq!(
        snapshot,
        Some(
            r#"{"earlyEntry":{"isIntersecting":true,"ratio":1,"top":0},"bottomEntry":{"isIntersecting":true,"ratio":1,"top":0},"publicBottomTop":3912}"#
        )
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn intersection_observer_root_scopes_delivery_to_descendants_only() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/intersection-observer-root-scope"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.intersectionObserverRootScopedIds === 'inside'",
            Duration::from_secs(2),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "intersectionObserverRootScopedIds"),
        Some(&JsValueSnapshot::String("inside".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "intersectionObserverRootScopedCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn root_client_metrics_track_window_surface_profile() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default().with_layout_policy(LayoutPolicy::OnDemand))?;

    let mut page = browser.fetch(&server.url("/static")).await?;

    let value = page
        .evaluate_runtime_expression_async(
            r#"JSON.stringify({
                innerWidth: window.innerWidth,
                innerHeight: window.innerHeight,
                documentElementClientWidth: document.documentElement.clientWidth,
                documentElementClientHeight: document.documentElement.clientHeight,
                bodyClientWidth: document.body.clientWidth,
                bodyClientHeight: document.body.clientHeight,
                documentElementRectWidth: document.documentElement.getBoundingClientRect().width,
                documentElementRectHeight: document.documentElement.getBoundingClientRect().height
            })"#,
        )
        .await?;
    let snapshot = value.get("value").and_then(|value| value.as_str());

    assert_eq!(
        snapshot,
        Some(
            r#"{"innerWidth":1920,"innerHeight":1080,"documentElementClientWidth":1920,"documentElementClientHeight":1080,"bodyClientWidth":1904,"bodyClientHeight":19,"documentElementRectWidth":1920,"documentElementRectHeight":35}"#
        )
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn react_native_web_fill_classes_inherit_parent_geometry() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default().with_layout_policy(LayoutPolicy::Mock))?;

    let mut page = browser.fetch(&server.url("/static")).await?;

    let value = page
        .evaluate_runtime_expression_async(
            r#"(() => {
                const parent = document.createElement("div");
                parent.style.width = "640px";
                parent.style.height = "480px";
                document.body.appendChild(parent);
                const child = document.createElement("div");
                child.className = "css-175oi2r r-13qz1uu r-1pi2tsx";
                parent.appendChild(child);
                const rect = child.getBoundingClientRect();
                return JSON.stringify({
                    clientWidth: child.clientWidth,
                    clientHeight: child.clientHeight,
                    rectWidth: rect.width,
                    rectHeight: rect.height
                });
            })()"#,
        )
        .await?;
    let snapshot = value.get("value").and_then(|value| value.as_str());

    assert_eq!(
        snapshot,
        Some(r#"{"clientWidth":100,"clientHeight":20,"rectWidth":100,"rectHeight":20}"#)
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn react_native_web_geometry_classes_do_not_overfill_without_fill_class() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default().with_layout_policy(LayoutPolicy::Mock))?;

    let mut page = browser.fetch(&server.url("/static")).await?;

    let value = page
        .evaluate_runtime_expression_async(
            r#"(() => {
                const snapshot = (className) => {
                    const parent = document.createElement("div");
                    parent.style.width = "640px";
                    parent.style.height = "480px";
                    document.body.appendChild(parent);
                    const child = document.createElement("div");
                    child.className = className;
                    parent.appendChild(child);
                    const rect = child.getBoundingClientRect();
                    return {
                        clientWidth: child.clientWidth,
                        clientHeight: child.clientHeight,
                        rectWidth: rect.width,
                        rectHeight: rect.height
                    };
                };
                return JSON.stringify({
                    baseView: snapshot("css-175oi2r"),
                    widthOnly: snapshot("r-13qz1uu"),
                    heightOnly: snapshot("r-1pi2tsx"),
                    flexFill: snapshot("r-13awgt0"),
                    hairline: snapshot("r-109y4c4")
                });
            })()"#,
        )
        .await?;
    let snapshot = value.get("value").and_then(|value| value.as_str());

    assert_eq!(
        snapshot,
        Some(
            r#"{"baseView":{"clientWidth":100,"clientHeight":20,"rectWidth":100,"rectHeight":20},"widthOnly":{"clientWidth":100,"clientHeight":20,"rectWidth":100,"rectHeight":20},"heightOnly":{"clientWidth":100,"clientHeight":20,"rectWidth":100,"rectHeight":20},"flexFill":{"clientWidth":100,"clientHeight":20,"rectWidth":100,"rectHeight":20},"hairline":{"clientWidth":100,"clientHeight":20,"rectWidth":100,"rectHeight":20}}"#
        )
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn inline_geometry_uses_synthetic_box_for_authored_widths_and_percentages() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default().with_layout_policy(LayoutPolicy::Mock))?;

    let mut page = browser.fetch(&server.url("/static")).await?;

    let value = page
        .evaluate_runtime_expression_async(
            r#"(() => {
                const parent = document.createElement("div");
                parent.style.width = "640px";
                parent.style.height = "480px";
                document.body.appendChild(parent);

                const percent = document.createElement("div");
                percent.setAttribute("style", "min-width: 999px; width: 50%; min-height: 999px; height: 25%;");
                parent.appendChild(percent);

                const minOnly = document.createElement("div");
                minOnly.setAttribute("style", "min-width: 999px; min-height: 999px;");
                parent.appendChild(minOnly);

                return JSON.stringify({
                    percentWidth: percent.clientWidth,
                    percentHeight: percent.clientHeight,
                    minOnlyWidth: minOnly.clientWidth,
                    minOnlyHeight: minOnly.clientHeight
                });
            })()"#,
        )
        .await?;
    let snapshot = value.get("value").and_then(|value| value.as_str());

    assert_eq!(
        snapshot,
        Some(r#"{"percentWidth":100,"percentHeight":20,"minOnlyWidth":100,"minOnlyHeight":20}"#)
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn intersection_observer_root_geometry_reflects_root_and_target_rects() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default().with_layout_policy(LayoutPolicy::OnDemand))?;

    let mut page = browser
        .fetch(&server.url("/compat/intersection-observer-root-geometry"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.ioRootMatchesRect === true",
            Duration::from_secs(2),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "ioRootMatchesRect"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "ioTargetMatchesRect"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "ioIntersectionWithinTarget"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "ioPartialRatioIsGeometric"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "ioMarginExpandsRootBounds"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "ioMarginExpandsIntersection"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "ioMarginIncreasesRatio"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn intersection_observer_threshold_crossings_report_exit_and_reentry() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/intersection-observer-thresholds"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.intersectionThresholdCount === 3 && globalThis.intersectionThresholdBurstCount === 2",
            Duration::from_secs(2),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "intersectionThresholdCount"),
        Some(&JsValueSnapshot::Number(3.0))
    );
    assert_eq!(
        diagnostic_global(&page, "intersectionThresholdCapture"),
        Some(&JsValueSnapshot::String("true:1|false:0|true:1".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "intersectionThresholdBurstCount"),
        Some(&JsValueSnapshot::Number(2.0))
    );
    assert_eq!(
        diagnostic_global(&page, "intersectionThresholdBurstCapture"),
        Some(&JsValueSnapshot::String("false:false|true:true".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn mutation_observer_option_validation_and_auto_enable_follow_spec_basics() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/mutation-observer-options"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "moInvalidNoSignals"),
        Some(&JsValueSnapshot::String("TypeError".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moInvalidAttributeOldValue"),
        Some(&JsValueSnapshot::String("TypeError".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moInvalidAttributeFilter"),
        Some(&JsValueSnapshot::String("TypeError".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moInvalidCharacterDataOldValue"),
        Some(&JsValueSnapshot::String("TypeError".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moAutoAttributes"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moAutoCharacterData"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn window_host_globals_expose_aliases_performance_and_visual_viewport() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-host-globals"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-self=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-parent-self=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-frames-self=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-length=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-frames-length=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-performance-instance=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-performance-same-object=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-performance-now-type=\"function\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-performance-now-monotonic=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-performance-time-origin-number=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-performance-tag=\"[object Performance]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-visual-viewport-same-object=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-visual-viewport-tag=\"[object VisualViewport]\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-visual-viewport-offset-left=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-visual-viewport-offset-top=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-visual-viewport-page-left=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-visual-viewport-page-top=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-visual-viewport-width=\"1920\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-visual-viewport-height=\"1080\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-visual-viewport-scale=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-navigator-user-agent=\"{}\"",
                FetchConfig::DEFAULT_USER_AGENT
            ))
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-navigator-platform=\"{}\"",
                moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.platform
            ))
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigator-language=\"en-US\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigator-hardware-concurrency=\"4\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigator-max-touch-points=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-inner-width=\"1920\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-inner-height=\"1080\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-device-pixel-ratio=\"1\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-screen-width=\"1920\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-screen-height=\"1080\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-screen-avail-width=\"1920\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-screen-avail-height=\"1080\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-screen-orientation-angle=\"0\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-screen-orientation-type=\"landscape-primary\"")
    );

    server.shutdown().await;
    Ok(())
}
