use moli_test_support as support;

use anyhow::{Context, Result};
use moli_core::{
    LayoutPolicy,
    page::{DocumentNodeSnapshot, Page},
    runtime::{Browser, BrowserConfig as AppConfig},
    testing::{JsValueSnapshot, ScriptRunOutcome},
};
use support::FixtureServer;
use tokio::time::Duration;

async fn live_document_snapshot(page: &mut Page) -> Result<DocumentNodeSnapshot> {
    let endpoint = page.renderer_inspection_endpoint();
    let route = endpoint
        .dom_inspection(
            moli_renderer_v8::RendererAgentAttachmentId::allocate(),
            None,
        )
        .start_document_node_snapshot_for_document(true, -1, true)?;
    let pending = moli_core::page::PendingPageCommand::from_inspector_main_route(route);
    let completion = pending.wait().await?;
    page.observe_renderer_page_state(completion.page_state());
    let snapshot = completion
        .finish_document_node_snapshot_for_document()?
        .context("renderer live document snapshot should exist")?;
    Ok(snapshot.snapshot)
}

fn find_snapshot_node<'a, F>(
    snapshot: &'a DocumentNodeSnapshot,
    predicate: &mut F,
) -> Option<&'a DocumentNodeSnapshot>
where
    F: FnMut(&DocumentNodeSnapshot) -> bool,
{
    if predicate(snapshot) {
        return Some(snapshot);
    }

    for child in &snapshot.children {
        if let Some(found) = find_snapshot_node(child, predicate) {
            return Some(found);
        }
    }

    for shadow_root in &snapshot.shadow_roots {
        if let Some(found) = find_snapshot_node(shadow_root, predicate) {
            return Some(found);
        }
    }

    None
}

fn count_snapshot_nodes<F>(snapshot: &DocumentNodeSnapshot, predicate: &mut F) -> usize
where
    F: FnMut(&DocumentNodeSnapshot) -> bool,
{
    let self_count = usize::from(predicate(snapshot));
    self_count
        + snapshot
            .children
            .iter()
            .map(|child| count_snapshot_nodes(child, predicate))
            .sum::<usize>()
        + snapshot
            .shadow_roots
            .iter()
            .map(|shadow_root| count_snapshot_nodes(shadow_root, predicate))
            .sum::<usize>()
}

fn snapshot_attribute<'a>(snapshot: &'a DocumentNodeSnapshot, name: &str) -> Option<&'a str> {
    snapshot
        .attributes
        .iter()
        .find(|attribute| attribute.local_name == name)
        .map(|attribute| attribute.value.as_str())
}

fn find_snapshot_by_element_id<'a>(
    snapshot: &'a DocumentNodeSnapshot,
    expected_id: &str,
) -> Option<&'a DocumentNodeSnapshot> {
    find_snapshot_node(snapshot, &mut |node| {
        node.is_element && snapshot_attribute(node, "id") == Some(expected_id)
    })
}

fn diagnostic_global<'a>(page: &'a Page, name: &str) -> Option<&'a JsValueSnapshot> {
    page.script_execution().global(name)
}

#[tokio::test]
async fn builds_dom_structure_and_exposes_script_nodes() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser.fetch(&server.url("/script")).await?;
    let document = live_document_snapshot(&mut page).await?;

    assert_eq!(document.node_name, "#document");
    let html = find_snapshot_node(&document, &mut |node| node.local_name == "html")
        .expect("document element should exist");
    let head = find_snapshot_node(&document, &mut |node| node.local_name == "head")
        .expect("head node should exist");
    let body = find_snapshot_node(&document, &mut |node| node.local_name == "body")
        .expect("body node should exist");
    assert_eq!(html.parent_id, Some(document.node_id));
    assert_eq!(head.parent_id, Some(html.node_id));
    assert_eq!(body.parent_id, Some(html.node_id));
    assert_eq!(
        count_snapshot_nodes(&document, &mut |node| node.local_name == "script"),
        1
    );
    let script = find_snapshot_node(&document, &mut |node| {
        node.local_name == "script" && snapshot_attribute(node, "src") == Some("/assets/app.js")
    });
    assert!(
        script.is_some(),
        "script src should be visible in renderer live document snapshot"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn range_exposes_constructor_create_range_and_clone_contents_basics() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/compat/range-basic")).await?;

    assert_eq!(
        diagnostic_global(&page, "rangeCtorCollapsed"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rangeCtorStartOffset"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "rangeCtorEndOffset"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "rangeCtorStartIsDocument"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rangeCtorEndIsDocument"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rangeCreateRangeInstance"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rangeCreateRangeCollapsed"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rangeCommentStartOffset"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "rangeCommentEndOffset"),
        Some(&JsValueSnapshot::Number(13.0))
    );
    assert_eq!(
        diagnostic_global(&page, "rangeNodeStartOffset"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "rangeNodeEndOffset"),
        Some(&JsValueSnapshot::Number(2.0))
    );
    assert_eq!(
        diagnostic_global(&page, "rangeCloneCallsAtLeastTwo"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rangeCloneFragmentChildren"),
        Some(&JsValueSnapshot::Number(1.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn range_internal_algorithms_ignore_page_tampered_methods() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default().with_layout_policy(LayoutPolicy::OnDemand))?;

    let mut page = browser
        .fetch(&server.url("/compat/range-internal-algorithms-ignore-page-tampered-methods"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-split-text-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-insert-before-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-append-child-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-clone-node-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-remove-child-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-delete-data-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-create-text-node-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-create-comment-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-create-document-fragment-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-create-element-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-get-bounding-client-rect-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-get-client-rects-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-insert-first-text=\"hello\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-insert-middle-tag=\"STRONG\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-insert-middle-id=\"inserted\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-insert-last-text=\" world\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-surround-wrapper-tag=\"EM\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-surround-wrapper-id=\"wrapped\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-surround-wrapper-child-count=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-surround-first-child-id=\"wrap-a\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-surround-last-child-id=\"wrap-b\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-clone-child-count=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-clone-first-tag=\"SPAN\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-clone-first-id=\"clone-a\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-clone-second-id=\"clone-b\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-extract-child-count=\"3\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-extract-first-text=\"hello\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-extract-middle-tag=\"STRONG\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-extract-middle-id=\"extract-mid\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-extract-last-text=\"tail\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-extract-host-text=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-delete-fragment-text=\"bcd\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-delete-host-text=\"aef\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-comment-clone-text=\"mmenta\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-context-child-count=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-context-first-tag=\"B\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-context-second-tag=\"I\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    let geometry = page
        .evaluate_runtime_expression_async(
            r#"JSON.stringify({
                width: Number(document.body.dataset.rectWidth),
                height: Number(document.body.dataset.rectHeight),
                rectsLength: Number(document.body.dataset.rectsLength),
                firstWidth: Number(document.body.dataset.rectsFirstWidth)
            })"#,
        )
        .await?;
    let geometry: serde_json::Value = serde_json::from_str(
        geometry["value"]
            .as_str()
            .context("range geometry probe should stringify")?,
    )?;
    assert!(geometry["width"].as_f64().is_some_and(|value| value > 0.0));
    assert!(geometry["height"].as_f64().is_some_and(|value| value > 0.0));
    assert_eq!(geometry["rectsLength"].as_u64(), Some(2));
    assert!(
        geometry["firstWidth"]
            .as_f64()
            .is_some_and(|value| value > 0.0)
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn selection_exposes_get_selection_and_basic_range_mutations() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/selection-basic"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "selectionInitialRangeCount"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionInitialType"),
        Some(&JsValueSnapshot::String("None".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionInitialIsCollapsed"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionInitialAnchorNodeIsNull"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionInitialDirection"),
        Some(&JsValueSnapshot::String("none".to_owned()))
    );

    assert_eq!(
        diagnostic_global(&page, "selectionAfterCollapseRangeCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterCollapseType"),
        Some(&JsValueSnapshot::String("Caret".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterCollapseIsCollapsed"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterCollapseAnchorOffset"),
        Some(&JsValueSnapshot::Number(4.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterCollapseFocusOffset"),
        Some(&JsValueSnapshot::Number(4.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterCollapseDirection"),
        Some(&JsValueSnapshot::String("none".to_owned()))
    );

    assert_eq!(
        diagnostic_global(&page, "selectionAfterExtendRangeCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterExtendType"),
        Some(&JsValueSnapshot::String("Range".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterExtendIsCollapsed"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterExtendAnchorOffset"),
        Some(&JsValueSnapshot::Number(4.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterExtendFocusOffset"),
        Some(&JsValueSnapshot::Number(9.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterExtendDirection"),
        Some(&JsValueSnapshot::String("forward".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionRangeAtZeroIsRange"),
        Some(&JsValueSnapshot::Bool(true))
    );

    assert_eq!(
        diagnostic_global(&page, "selectionAfterDeleteText"),
        Some(&JsValueSnapshot::String("The  brown fox".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterDeleteType"),
        Some(&JsValueSnapshot::String("Caret".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterDeleteAnchorOffset"),
        Some(&JsValueSnapshot::Number(4.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterDeleteFocusOffset"),
        Some(&JsValueSnapshot::Number(4.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectionAfterDeleteDirection"),
        Some(&JsValueSnapshot::String("none".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn selectionchange_ignores_page_tampered_document_dispatch_event() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/selectionchange-ignores-page-tampered-document-dispatch-event"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-selectionchange-fired') === 'yes'",
            Duration::from_secs(2),
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
            .contains("data-selectionchange-fired=\"yes\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-selectionchange-count=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-selection-anchor-offset=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-selection-focus-offset=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn selection_contains_node_ignores_page_tampered_node_contains() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/selection-contains-node-ignores-page-tampered-node-contains"))
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
            .contains("data-contains-partial-s1=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-contains-partial-s2=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-contains-partial-nested=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-contains-full-nested=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-contains-partial-outside=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn selection_set_base_and_extent_ignores_page_tampered_compare_document_position()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/selection-set-base-and-extent-ignores-page-tampered-compare-document-position",
        ))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-tampered-compare-document-position-called=\"no\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-selection-direction=\"forward\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-anchor-parent-id=\"first\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-focus-parent-id=\"second\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-range-start-parent-id=\"first\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-range-start-offset=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-range-end-parent-id=\"second\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-range-end-offset=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn resolves_url_constructor_properties_in_classic_and_module_scripts() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser.fetch(&server.url("/url-binding")).await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            &format!(
                "globalThis.moduleUrlHref === {} && \
                 globalThis.documentReadyStateAtLoad === 'complete'",
                serde_json::to_string(&server.url("/assets/page-mod1.js?mode=test#frag"))?
            ),
            Duration::from_secs(2),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "urlHref"),
        Some(&JsValueSnapshot::String(
            server.url("/path/to/resource?x=1#frag")
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "urlOrigin"),
        Some(&JsValueSnapshot::String(format!(
            "http://{}",
            server.url("").trim_start_matches("http://")
        )))
    );
    assert_eq!(
        diagnostic_global(&page, "urlHost"),
        Some(&JsValueSnapshot::String(
            server.url("").trim_start_matches("http://").to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "urlHostname"),
        Some(&JsValueSnapshot::String("127.0.0.1".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "urlPort"),
        Some(&JsValueSnapshot::String(
            server
                .url("")
                .trim_start_matches("http://127.0.0.1:")
                .to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "urlPathname"),
        Some(&JsValueSnapshot::String("/path/to/resource".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "urlSearch"),
        Some(&JsValueSnapshot::String("?x=1".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "urlHash"),
        Some(&JsValueSnapshot::String("#frag".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "urlRelativeHref"),
        Some(&JsValueSnapshot::String(server.url("/child.js")))
    );
    assert_eq!(
        diagnostic_global(&page, "urlStringifyWorks"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "urlObjectToStringValue"),
        Some(&JsValueSnapshot::String(
            "https://example.test/from-object?value=1".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "urlObjectStringValue"),
        Some(&JsValueSnapshot::String(
            "https://example.test/from-object?value=1".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "urlObjectHref"),
        Some(&JsValueSnapshot::String(
            "https://example.test/from-object?value=1".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "urlAnchorPropertyHref"),
        Some(&JsValueSnapshot::String(
            server.url("/anchor-target?q=1#ok")
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "urlAnchorStringValue"),
        Some(&JsValueSnapshot::String(
            server.url("/anchor-target?q=1#ok")
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "urlAnchorStringifyWorks"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "urlAnchorHref"),
        Some(&JsValueSnapshot::String(
            server.url("/anchor-target?q=1#ok")
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "urlBaseAnchorHref"),
        Some(&JsValueSnapshot::String(server.url("/child")))
    );
    assert_eq!(
        diagnostic_global(&page, "urlCanParseAbsolute"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "urlCanParseRelativeWithBase"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "urlCanParseRejectsInvalidBaseForAbsolute"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "documentUrlValue"),
        Some(&JsValueSnapshot::String(server.url("/url-binding")))
    );
    assert_eq!(
        diagnostic_global(&page, "documentDocumentUriValue"),
        Some(&JsValueSnapshot::String(server.url("/url-binding")))
    );
    assert_eq!(
        diagnostic_global(&page, "documentBaseUriValue"),
        Some(&JsValueSnapshot::String(server.url("/url-binding")))
    );
    assert_eq!(
        diagnostic_global(&page, "documentReadyStateDuringScript"),
        Some(&JsValueSnapshot::String("loading".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "documentReadyStateAtDomContentLoaded"),
        Some(&JsValueSnapshot::String("interactive".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "documentReadyStateAtLoad"),
        Some(&JsValueSnapshot::String("complete".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "documentCookieValue"),
        Some(&JsValueSnapshot::String(String::new()))
    );
    assert_eq!(
        diagnostic_global(&page, "urlInvalidThrows"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleUrlHref"),
        Some(&JsValueSnapshot::String(
            server.url("/assets/page-mod1.js?mode=test#frag")
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleUrlPathname"),
        Some(&JsValueSnapshot::String("/assets/page-mod1.js".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleUrlSearch"),
        Some(&JsValueSnapshot::String("?mode=test".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "moduleUrlHash"),
        Some(&JsValueSnapshot::String("#frag".to_owned()))
    );
    assert!(
        !page
            .script_execution()
            .runs()
            .iter()
            .any(|run| matches!(run.outcome(), ScriptRunOutcome::Failed(_)))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn supports_selector_corner_cases_for_attributes_and_syntax_errors() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/selector-corner-cases")).await?;

    assert_eq!(
        diagnostic_global(&page, "selectorCornerUppercaseAttributeName"),
        Some(&JsValueSnapshot::String("case-target".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerWhitespaceAroundOperator"),
        Some(&JsValueSnapshot::String("case-target".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerExactEmptyValue"),
        Some(&JsValueSnapshot::String("case-target".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerBooleanExactEmptyValue"),
        Some(&JsValueSnapshot::String("checked-box".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerMultiAttributeCompound"),
        Some(&JsValueSnapshot::String("case-target".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerCompoundWithTagAndAttribute"),
        Some(&JsValueSnapshot::String("case-target".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerClassWordWhitespace"),
        Some(&JsValueSnapshot::String("case-target".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerPrefixDash"),
        Some(&JsValueSnapshot::String("case-target".to_owned()))
    );

    assert_eq!(
        diagnostic_global(&page, "selectorCornerStartsWithEmptyLength"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerEndsWithEmptyLength"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerContainsEmptyLength"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerWordEmptyLength"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerPrefixDashEmptyLength"),
        Some(&JsValueSnapshot::Number(0.0))
    );

    assert_eq!(
        diagnostic_global(&page, "selectorCornerInvalidMissingBracket"),
        Some(&JsValueSnapshot::String("SyntaxError:12".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerInvalidMissingValue"),
        Some(&JsValueSnapshot::String("SyntaxError:12".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerInvalidUnsupportedOperator"),
        Some(&JsValueSnapshot::String("SyntaxError:12".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerDuplicateIdNoMatch"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerDuplicateIdAllLength"),
        Some(&JsValueSnapshot::Number(0.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerInvalidMatchesAttribute"),
        Some(&JsValueSnapshot::String("SyntaxError:12".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerMatchesDuplicateIdNoMatch"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorCornerMatchesDuplicateSameId"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn routes_live_dom_selectors_through_rust_after_dom_mutation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/selector-host-bridge")).await?;

    assert_eq!(
        diagnostic_global(&page, "selectorHostAlphaId"),
        Some(&JsValueSnapshot::String("alpha".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorHostItemCount"),
        Some(&JsValueSnapshot::Number(2.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorHostLeafMatches"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorHostClosestId"),
        Some(&JsValueSnapshot::String("scope-root".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorHostDynamicId"),
        Some(&JsValueSnapshot::String("dynamic".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorHostDynamicGetElementById"),
        Some(&JsValueSnapshot::String("dynamic".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorHostUsedQuerySelector"),
        Some(&JsValueSnapshot::Number(3.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorHostUsedQuerySelectorAll"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorHostUsedMatches"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "selectorHostUsedClosest"),
        Some(&JsValueSnapshot::Number(1.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn exposes_hidden_native_bridge_host_objects() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/native-bridge")).await?;

    assert_eq!(
        diagnostic_global(&page, "nativeBridgeError"),
        Some(&JsValueSnapshot::Null)
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeHasBridge"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeHasWindow"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeHasDocument"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeHasBody"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeHasDocumentElement"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeWindowIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgePublicWindowIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeBridgeWindowResolvesToPublicWindow"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeBridgeSelfResolvesToPublicWindow"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeWindowDocumentIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeDocumentIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeBodyIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeDocumentElementIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeAppendReturnsSameWrapper"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeAttrBeforeRemove"),
        Some(&JsValueSnapshot::String("native".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeAttrAfterRemove"),
        Some(&JsValueSnapshot::Null)
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeIdAttr"),
        Some(&JsValueSnapshot::String("native-child".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeBodyText"),
        Some(&JsValueSnapshot::String("seednative-child".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeTextInstance"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeTextConstructor"),
        Some(&JsValueSnapshot::String("Text".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeCommentInstance"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeCommentConstructor"),
        Some(&JsValueSnapshot::String("Comment".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgePiInstance"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgePiConstructor"),
        Some(&JsValueSnapshot::String("ProcessingInstruction".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeFragmentInstance"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "nativeBridgeFragmentConstructor"),
        Some(&JsValueSnapshot::String("DocumentFragment".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn bridges_event_target_and_public_collections() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/event-collections-bridge"))
        .await?;
    // The fixture intentionally validates zero-delay Window timers. `load`
    // does not guarantee that a separate timer task has run, so wait for the
    // exact observable instead of depending on incidental task order.
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.windowTimeoutCalls === 'window|bridge'",
            Duration::from_secs(1),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "eventCollectionsError"),
        Some(&JsValueSnapshot::Null)
    );
    assert_eq!(
        diagnostic_global(&page, "windowIsEventTarget"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "windowIsWindow"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "publicWindowPrototypeIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "publicWindowIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "publicWindowDocumentIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "publicWindowLocationIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "publicWindowConsoleIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "bridgeWindowIsEventTarget"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "bridgeWindowIsWindow"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "bridgeWindowDocumentIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "bridgeWindowResolvesToPublicWindow"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "bridgeSelfResolvesToPublicWindow"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "bridgeWindowSetTimeoutIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "documentDefaultViewIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "documentParentWindowMissing"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "windowEventCalls"),
        Some(&JsValueSnapshot::String(
            "custom-window:true:true".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "windowEventInside"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "bridgeWindowEventInside"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "windowEventOutsideUndefined"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "bridgeWindowEventOutsideUndefined"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "windowCancelDispatchReturn"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "windowCancelDefaultPrevented"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "plainWindowDispatchError"),
        Some(&JsValueSnapshot::String("TypeError:true".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "windowTimeoutCalls"),
        Some(&JsValueSnapshot::String("window|bridge".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "windowConsoleCalls"),
        Some(&JsValueSnapshot::String("undefined|undefined".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "documentEventCalls"),
        Some(&JsValueSnapshot::Number(21.0))
    );
    assert_eq!(
        diagnostic_global(&page, "childNodesIsNodeList"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "childNodesLength"),
        Some(&JsValueSnapshot::Number(4.0))
    );
    assert_eq!(
        diagnostic_global(&page, "childNodesNames"),
        Some(&JsValueSnapshot::String(
            "SPAN|#comment|SPAN|#text".to_owned()
        ))
    );
    assert_eq!(
        diagnostic_global(&page, "childNodesEntries"),
        Some(&JsValueSnapshot::String("0:1|1:8|2:1|3:3".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "childrenIsHtmlCollection"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "childrenLength"),
        Some(&JsValueSnapshot::Number(2.0))
    );
    assert_eq!(
        diagnostic_global(&page, "childrenIds"),
        Some(&JsValueSnapshot::String("alpha|beta".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "childrenNamedItem"),
        Some(&JsValueSnapshot::String("beta".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "childrenNamedProperty"),
        Some(&JsValueSnapshot::String("alpha".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn routes_document_lifecycle_and_script_load_through_rust_host_dispatch() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/lifecycle-bridge")).await?;

    assert_eq!(
        diagnostic_global(&page, "lifecycleBridgeError"),
        Some(&JsValueSnapshot::Null)
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleDispatchHelpersMissing"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleDomContentLoadedCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleDocumentElementLoadCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleScriptLoadCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleScriptLoadTargetMatched"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleScriptLoadCurrentTargetMatched"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleDocumentElementLoadTargetMatched"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleDocumentElementLoadCurrentTargetMatched"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "lifecycleInlineReplayCount"),
        Some(&JsValueSnapshot::Number(1.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn bridges_tree_navigation_and_mutation_through_public_dom_api() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/tree-bridge")).await?;

    assert_eq!(
        diagnostic_global(&page, "treeBridgeError"),
        Some(&JsValueSnapshot::Null)
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeOwnerDocument"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeDocumentOwnerNull"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeParentNode"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeFirstChild"),
        Some(&JsValueSnapshot::String("alpha".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeLastChild"),
        Some(&JsValueSnapshot::String("beta".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeAlphaNextSibling"),
        Some(&JsValueSnapshot::String("#comment".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeBetaPreviousSibling"),
        Some(&JsValueSnapshot::String("#comment".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeContainsSelf"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeContainsAlpha"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeContainsBody"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeRemovedIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeRemovedParentNode"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeAfterRemoveLastChild"),
        Some(&JsValueSnapshot::String("#comment".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeInsertedIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeInsertedParentNode"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeInsertedPreviousSibling"),
        Some(&JsValueSnapshot::String("alpha".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeInsertedNextSibling"),
        Some(&JsValueSnapshot::String("#comment".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "treeBridgeChildNodeOrder"),
        Some(&JsValueSnapshot::String("alpha|gamma|#comment".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn keeps_live_collections_in_sync_with_dom_mutation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/live-collections")).await?;

    assert_eq!(
        diagnostic_global(&page, "liveCollectionsError"),
        Some(&JsValueSnapshot::Null)
    );
    assert_eq!(
        diagnostic_global(&page, "liveChildNodesSameInstance"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "liveChildNodesLengthAfterRemove"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "liveChildNodesFirstAfterRemove"),
        Some(&JsValueSnapshot::String("beta".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "liveChildNodesLengthAfterRestore"),
        Some(&JsValueSnapshot::Number(2.0))
    );
    assert_eq!(
        diagnostic_global(&page, "liveChildNodesFirstAfterRestore"),
        Some(&JsValueSnapshot::String("alpha".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "liveChildrenLengthAfterAppend"),
        Some(&JsValueSnapshot::Number(3.0))
    );
    assert_eq!(
        diagnostic_global(&page, "liveChildrenLastId"),
        Some(&JsValueSnapshot::String("gamma".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "liveTagLengthInitial"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "liveTagLengthAfterAppend"),
        Some(&JsValueSnapshot::Number(2.0))
    );
    assert_eq!(
        diagnostic_global(&page, "liveTagLengthAfterRemove"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "liveClassLengthInitial"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "liveClassLengthAfterAdd"),
        Some(&JsValueSnapshot::Number(2.0))
    );
    assert_eq!(
        diagnostic_global(&page, "liveClassLengthAfterRemove"),
        Some(&JsValueSnapshot::Number(1.0))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn prefers_rust_live_dom_over_js_shadow_state_for_public_dom_reads() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/rust-dom-source-of-truth"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "rustDomSourceError"),
        Some(&JsValueSnapshot::Null)
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceChildNodesSameInstance"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceChildrenSameInstance"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceChildNodesLength"),
        Some(&JsValueSnapshot::Number(3.0))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceChildrenLength"),
        Some(&JsValueSnapshot::Number(3.0))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceChildOrder"),
        Some(&JsValueSnapshot::String("alpha|beta|field".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceChildrenIds"),
        Some(&JsValueSnapshot::String("alpha|beta|field".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceAttribute"),
        Some(&JsValueSnapshot::String("rust".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceHasAttribute"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceBetaById"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceFieldById"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceParentNode"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceOwnerDocument"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceTextNodeTextContent"),
        Some(&JsValueSnapshot::String("BETA".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceRootTextContent"),
        Some(&JsValueSnapshot::String("alphaBETA".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceDocumentSpans"),
        Some(&JsValueSnapshot::String("alpha|beta".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceRootSpans"),
        Some(&JsValueSnapshot::String("alpha|beta".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceDocumentItems"),
        Some(&JsValueSnapshot::String("alpha|beta".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceRootItems"),
        Some(&JsValueSnapshot::String("alpha|beta".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceNamedLength"),
        Some(&JsValueSnapshot::Number(2.0))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomSourceNamedTags"),
        Some(&JsValueSnapshot::String("SPAN|INPUT".to_owned()))
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"beta\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"field\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-state=\"rust\"")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("data-state=\"seed\"")
    );

    let document = live_document_snapshot(&mut page).await?;
    let root = find_snapshot_by_element_id(&document, "rust-root").expect("root node should exist");
    let beta = find_snapshot_by_element_id(&document, "beta").expect("beta node should exist");
    let field = find_snapshot_by_element_id(&document, "field").expect("field node should exist");

    assert_eq!(snapshot_attribute(root, "data-state"), Some("rust"));
    assert_eq!(
        beta.parent_id,
        Some(root.node_id),
        "beta should be attached to the final exported root"
    );
    assert_eq!(
        field.parent_id,
        Some(root.node_id),
        "field should be attached to the final exported root"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn lazily_hydrates_traversal_wrappers_for_rust_backed_nodes() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/rust-dom-lazy-hydration"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.rustDomLazyHydrationError === null",
            Duration::from_secs(2),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "rustDomLazyHydrationError"),
        Some(&JsValueSnapshot::Null)
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomLazyHydrationParentTag"),
        Some(&JsValueSnapshot::String("DIV".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomLazyHydrationOuterTag"),
        Some(&JsValueSnapshot::String("SECTION".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomLazyHydrationOwnerDocument"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomLazyHydrationFirstChildIsInner"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomLazyHydrationLastChildId"),
        Some(&JsValueSnapshot::String("tail".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomLazyHydrationNextSiblingId"),
        Some(&JsValueSnapshot::String("tail".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomLazyHydrationPreviousSiblingId"),
        Some(&JsValueSnapshot::String("inner".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomLazyHydrationChildOrder"),
        Some(&JsValueSnapshot::String("inner|tail".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn keeps_tree_mutation_and_script_connection_in_sync_after_shadow_drift() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/rust-dom-mutation-sync"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "rustDomMutationSyncError"),
        Some(&JsValueSnapshot::Null)
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomMutationSyncExecuted"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomMutationSyncScriptConnected"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomMutationSyncScriptParent"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomMutationSyncGammaParent"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomMutationSyncRemovedIdentity"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomMutationSyncRemovedParent"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomMutationSyncRemovedConnected"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomMutationSyncChildOrder"),
        Some(&JsValueSnapshot::String("alpha|gamma|SCRIPT".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn executes_connected_scripts_inserted_via_document_fragment() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let page = browser
        .fetch(&server.url("/rust-dom-fragment-script-sync"))
        .await?;

    assert_eq!(
        diagnostic_global(&page, "fragmentScriptRuns"),
        Some(&JsValueSnapshot::Number(1.0))
    );
    assert_eq!(
        diagnostic_global(&page, "fragmentScriptConnected"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "fragmentScriptParentIsBody"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "fragmentFragmentEmpty"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn keeps_document_open_detach_state_in_sync_after_shadow_drift() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/rust-dom-document-open-sync"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "globalThis.rustDomDocumentOpenSyncError === null",
            Duration::from_secs(2),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "rustDomDocumentOpenSyncError"),
        Some(&JsValueSnapshot::Null)
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomDocumentOpenSyncOldParentPreserved"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomDocumentOpenSyncOldConnected"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomDocumentOpenSyncShellReady"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomDocumentOpenSyncNewId"),
        Some(&JsValueSnapshot::String("new".to_owned()))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomDocumentOpenSyncOldGone"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("<main id=\"new\">new</main>")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("id=\"old\"")
    );

    let document = live_document_snapshot(&mut page).await?;
    assert!(find_snapshot_by_element_id(&document, "old").is_none());
    let new_main = find_snapshot_by_element_id(&document, "new").expect("new main should exist");
    assert_eq!(
        new_main.local_name, "main",
        "new main should be visible in renderer live document snapshot"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn document_open_multiwrite_replaces_live_document_synchronously() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/rust-dom-document-open-multiwrite-sync"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "window.rustDomDocumentOpenMultiwriteError === null",
            Duration::from_secs(2),
        )
        .await?;

    assert_eq!(
        diagnostic_global(&page, "rustDomDocumentOpenMultiwriteError"),
        Some(&JsValueSnapshot::Null)
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomDocumentOpenMultiwriteOldGone"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "rustDomDocumentOpenMultiwriteNewId"),
        Some(&JsValueSnapshot::String("new".to_owned()))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn parses_inline_scripts_template_contents_and_comments() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser.fetch(&server.url("/inline-script")).await?;
    let document = live_document_snapshot(&mut page).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("window.inlineReady"),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("window.templateReady"),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        find_snapshot_node(&document, &mut |node| {
            node.node_name == "#comment" && node.node_value == "head"
        })
        .is_some()
    );
    assert!(
        find_snapshot_node(&document, &mut |node| {
            node.local_name == "template" && snapshot_attribute(node, "id") == Some("tpl")
        })
        .is_some()
    );
    browser
        .wait_for_script_truthy(
            &mut page,
            "(() => { const template = document.getElementById('tpl'); return Boolean(template && template.childNodes.length === 0 && template.content && template.content.querySelector('script') && template.content.querySelector('script').textContent.includes('window.templateReady')); })()",
            Duration::from_secs(2),
        )
        .await?;

    server.shutdown().await;
    Ok(())
}
