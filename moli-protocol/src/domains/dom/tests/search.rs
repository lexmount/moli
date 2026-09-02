use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn get_search_results_no_bc_error() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 8, "method": "DOM.getSearchResults",
                           "params": {"searchId": "Nope", "fromIndex": 0, "toIndex": 10}}))
        .await;
    ctx.expect_error(8, -31998, "BrowserContextNotLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn search_flow_stub_returns_zero_results() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    ctx.process_async(json!({"id": 12, "method": "DOM.performSearch",
                           "params": {"query": "p"}}))
        .await;
    // The stub reports a numeric result count until search indexing is implemented.
    let msg = ctx.take_one();
    assert_eq!(msg["result"]["searchId"], "0");
    assert!(msg["result"]["resultCount"].is_number());
}

#[tokio::test(flavor = "multi_thread")]
async fn query_selector_no_page_returns_error() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    ctx.process_async(json!({"id": 9, "method": "DOM.querySelector",
                           "params": {"nodeId": 99, "selector": ""}}))
        .await;
    ctx.expect_error(9, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn query_selector_all_no_page_returns_error() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    ctx.process_async(json!({"id": 9, "method": "DOM.querySelectorAll",
                           "params": {"nodeId": 99, "selector": ""}}))
        .await;
    ctx.expect_error(9, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn search_flow_with_real_dom() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><p>one</p><p>two</p></body></html>",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.performSearch",
        "params": { "query": "p" }
    }))
    .await;
    let search_msg = take_response_by_id(&mut ctx, 2);
    let search_id = search_msg["result"]["searchId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert_eq!(search_msg["id"], 2);
    assert_eq!(search_id, "0");
    assert_eq!(search_msg["result"]["resultCount"], 2);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.getSearchResults",
        "params": {
            "searchId": search_id,
            "fromIndex": 0,
            "toIndex": 2
        }
    }))
    .await;
    let res = take_response_by_id(&mut ctx, 3);
    assert_eq!(res["id"], 3);
    assert_eq!(
        res["result"]["nodeIds"].as_array().map(|a| a.len()),
        Some(2)
    );

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.getSearchResults",
        "params": {
            "searchId": search_id,
            "fromIndex": 1,
            "toIndex": 2
        }
    }))
    .await;
    ctx.expect_result(
        4,
        json!({ "nodeIds": [res["result"]["nodeIds"][1].clone()] }),
        None,
    );

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.discardSearchResults",
        "params": { "searchId": "0" }
    }))
    .await;
    ctx.expect_result(5, json!({}), None);

    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.getSearchResults",
        "params": {
            "searchId": "0",
            "fromIndex": 0,
            "toIndex": 1
        }
    }))
    .await;
    ctx.expect_error(6, -32000, "No search session with given id found");
}

async fn perform_search_and_get_all_results(
    ctx: &mut TestContext,
    command_id: u64,
    query: &str,
    expected_count: usize,
) -> Vec<serde_json::Value> {
    ctx.process_async(json!({
        "id": command_id,
        "method": "DOM.performSearch",
        "params": { "query": query }
    }))
    .await;
    let search = take_response_by_id(ctx, command_id);
    assert_eq!(
        search["result"]["resultCount"],
        json!(expected_count),
        "unexpected search result count for {query:?}: {search:?}"
    );
    let search_id = search["result"]["searchId"]
        .as_str()
        .expect("search id")
        .to_owned();

    ctx.process_async(json!({
        "id": command_id + 1,
        "method": "DOM.getSearchResults",
        "params": {
            "searchId": search_id,
            "fromIndex": 0,
            "toIndex": expected_count
        }
    }))
    .await;
    take_response_by_id(ctx, command_id + 1)["result"]["nodeIds"]
        .as_array()
        .expect("search node ids")
        .clone()
}

async fn publish_document_for_search(
    ctx: &mut TestContext,
    command_id: u64,
    session_id: Option<&str>,
) {
    let mut command = json!({
        "id": command_id,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    });
    if let Some(session_id) = session_id {
        command["sessionId"] = json!(session_id);
    }
    ctx.process_async(command).await;
    let response = take_response_by_id(ctx, command_id);
    assert!(
        response["result"]["root"]["nodeId"]
            .as_u64()
            .is_some_and(|node_id| node_id > 0),
        "DOM.getDocument must publish the search session root: {response:?}"
    );
    if let Some(session_id) = session_id {
        assert_eq!(response["sessionId"], json!(session_id));
    }
    ctx.sent.clear();
}

#[tokio::test(flavor = "multi_thread")]
async fn perform_search_unions_plain_text_tag_attribute_and_xpath_matches() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        20,
        "<!doctype html><html><body><p id='first' data-label='Needle Value'>first paragraph</p><p id='second'>second paragraph</p></body></html>",
    )
    .await;
    ctx.sent.clear();
    publish_document_for_search(&mut ctx, 21, None).await;

    let text_nodes = perform_search_and_get_all_results(&mut ctx, 30, "paragraph", 2).await;
    for (offset, node_id) in text_nodes.into_iter().enumerate() {
        ctx.process_async(json!({
            "id": 40 + offset,
            "method": "DOM.describeNode",
            "params": { "nodeId": node_id }
        }))
        .await;
        let description = take_response_by_id(&mut ctx, 40 + offset as u64);
        assert_eq!(description["result"]["node"]["nodeType"], json!(3));
        assert!(
            description["result"]["node"]["nodeValue"]
                .as_str()
                .is_some_and(|value| value.contains("paragraph")),
            "plain-text search should return matching text nodes: {description:?}"
        );
    }

    let tag_nodes = perform_search_and_get_all_results(&mut ctx, 50, "<p>", 2).await;
    assert_eq!(tag_nodes.len(), 2);

    let attribute_nodes =
        perform_search_and_get_all_results(&mut ctx, 60, "\"Needle Value\"", 1).await;
    assert_eq!(attribute_nodes.len(), 1);

    let xpath_nodes =
        perform_search_and_get_all_results(&mut ctx, 70, "//p[@id='second']", 1).await;
    assert_eq!(xpath_nodes.len(), 1);

    let document_nodes = perform_search_and_get_all_results(&mut ctx, 80, "/", 1).await;
    ctx.process_async(json!({
        "id": 82,
        "method": "DOM.describeNode",
        "params": { "nodeId": document_nodes[0] }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 82);
    assert_eq!(document["result"]["node"]["nodeType"], json!(9));
}

#[tokio::test(flavor = "multi_thread")]
async fn search_results_keep_hidden_whitespace_positions_session_local() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .attach_active_session("SID-whitespace-default");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body>\n  <main id='target'>value</main>\n</body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 90,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-1" }
    }))
    .await;
    let all_session_id = take_response_by_id(&mut ctx, 90)["result"]["sessionId"]
        .as_str()
        .expect("attached session id")
        .to_owned();
    ctx.sent.clear();

    for (id, session_id, params) in [
        (91, "SID-whitespace-default", json!({})),
        (
            92,
            all_session_id.as_str(),
            json!({ "includeWhitespace": "all" }),
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "sessionId": session_id,
            "method": "DOM.enable",
            "params": params,
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert_eq!(response["result"], json!({}));
        assert_eq!(response["sessionId"], json!(session_id));
        ctx.sent.clear();
    }

    publish_document_for_search(&mut ctx, 910, Some("SID-whitespace-default")).await;
    publish_document_for_search(&mut ctx, 920, Some(all_session_id.as_str())).await;

    let mut searches = Vec::new();
    for (id, session_id) in [
        (93, "SID-whitespace-default"),
        (94, all_session_id.as_str()),
    ] {
        ctx.process_async(json!({
            "id": id,
            "sessionId": session_id,
            "method": "DOM.performSearch",
            "params": { "query": "/html/body/text()" }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert_eq!(response["sessionId"], json!(session_id));
        let result_count = response["result"]["resultCount"]
            .as_u64()
            .and_then(|count| usize::try_from(count).ok())
            .expect("search result count");
        assert!(
            result_count > 0,
            "the fixture must retain at least one whitespace text node: {response:?}"
        );
        searches.push((
            session_id.to_owned(),
            response["result"]["searchId"]
                .as_str()
                .expect("search id")
                .to_owned(),
            result_count,
        ));
        ctx.sent.clear();
    }

    assert_eq!(
        searches[0].2, searches[1].2,
        "whitespace projection must not change search result positions"
    );

    for (offset, (session_id, search_id, result_count)) in searches.into_iter().enumerate() {
        let id = 95 + offset as u64;
        ctx.process_async(json!({
            "id": id,
            "sessionId": session_id,
            "method": "DOM.getSearchResults",
            "params": {
                "searchId": search_id,
                "fromIndex": 0,
                "toIndex": result_count,
            }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        let node_ids = response["result"]["nodeIds"]
            .as_array()
            .expect("search node ids");
        assert_eq!(node_ids.len(), result_count);
        if session_id == "SID-whitespace-default" {
            assert!(
                node_ids.iter().all(|node_id| node_id == &json!(0)),
                "default search results must keep hidden positions as zero ids: {response:?}"
            );
        } else {
            assert!(
                node_ids
                    .iter()
                    .all(|node_id| node_id.as_u64().is_some_and(|node_id| node_id > 0)),
                "includeWhitespace=all must publish each whitespace search result: {response:?}"
            );
        }
        ctx.sent.clear();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn perform_search_traverses_author_shadow_roots_by_default() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    ctx.process_async(json!({
        "id": 90,
        "method": "Page.navigate",
        "params": {
            "url": "data:text/html,<!doctype html><html><body><div id='host'></div><script>const root=document.getElementById('host').attachShadow({mode:'open'});const span=document.createElement('span');span.id='shadow-hit';span.textContent=['shadow','phrase'].join(' ');root.appendChild(span)</script></body></html>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 90);
    let loader_id = navigation["result"]["loaderId"]
        .as_str()
        .expect("navigation loader id");
    crate::testing::wait_until_renderer_document_load(&mut ctx, None, "TID-1", loader_id).await;
    let _ = ctx.take_all();
    publish_document_for_search(&mut ctx, 91, None).await;

    let element_nodes = perform_search_and_get_all_results(&mut ctx, 100, "#shadow-hit", 1).await;
    ctx.process_async(json!({
        "id": 102,
        "method": "DOM.describeNode",
        "params": { "nodeId": element_nodes[0] }
    }))
    .await;
    let element = take_response_by_id(&mut ctx, 102);
    assert_eq!(element["result"]["node"]["nodeName"], json!("SPAN"));

    let text_nodes = perform_search_and_get_all_results(&mut ctx, 110, "shadow phrase", 1).await;
    ctx.process_async(json!({
        "id": 112,
        "method": "DOM.describeNode",
        "params": { "nodeId": text_nodes[0] }
    }))
    .await;
    let text = take_response_by_id(&mut ctx, 112);
    assert_eq!(text["result"]["node"]["nodeType"], json!(3));
    assert_eq!(text["result"]["node"]["nodeValue"], json!("shadow phrase"));
}

#[tokio::test(flavor = "multi_thread")]
async fn perform_search_selectors_cover_all_frame_documents_and_author_shadow_roots() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    navigate_to_data_html_async(
        &mut ctx,
        120,
        r#"<!doctype html><html><body>
            <div class='find-me' id='main'></div>
            <div id='main-host'></div>
            <iframe srcdoc="<!doctype html><html><body>
                <div class='find-me' id='child-main'></div>
                <div id='child-host'></div>
                <script>
                    const root = document.getElementById('child-host').attachShadow({mode:'closed'});
                    const hit = document.createElement('div');
                    hit.className = 'find-me';
                    hit.id = 'child-shadow';
                    root.appendChild(hit);
                </script>
            </body></html>"></iframe>
            <script>
                const root = document.getElementById('main-host').attachShadow({mode:'open'});
                const hit = document.createElement('div');
                hit.className = 'find-me';
                hit.id = 'main-shadow';
                root.appendChild(hit);
            </script>
        </body></html>"#,
    )
    .await;
    publish_document_for_search(&mut ctx, 119, None).await;

    let node_ids = perform_search_and_get_all_results(&mut ctx, 121, ".find-me", 4).await;
    let mut element_ids = Vec::new();
    for (offset, node_id) in node_ids.into_iter().enumerate() {
        let command_id = 130 + offset as u64;
        ctx.process_async(json!({
            "id": command_id,
            "method": "DOM.describeNode",
            "params": { "nodeId": node_id }
        }))
        .await;
        let description = take_response_by_id(&mut ctx, command_id);
        let node = &description["result"]["node"];
        assert_eq!(node["nodeName"], json!("DIV"));
        element_ids.push(
            node_attribute_value(node, "id")
                .expect("search result element id")
                .to_owned(),
        );
    }

    assert_eq!(
        element_ids,
        ["main", "main-shadow", "child-main", "child-shadow"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_search_targets_loaded_background_owner_without_activation() {
    let mut ctx = TestContext::new();
    let background = PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut bc = BrowserContext::new("BID-A".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<!doctype html><html><body><span>one</span><span>two</span></body></html>",
        Some("SID-background"),
    )
    .await;

    ctx.process_async(json!({
        "id": 301,
        "sessionId": "SID-background",
        "method": "DOM.performSearch",
        "params": { "query": "span" }
    }))
    .await;
    let search_msg = take_response_by_id(&mut ctx, 301);
    let search_id = search_msg["result"]["searchId"]
        .as_str()
        .expect("search id")
        .to_owned();
    assert_eq!(search_msg["sessionId"], "SID-background");
    assert_eq!(search_msg["result"]["resultCount"], 2);
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(BrowserContext::active_target_id),
        Some("TID-active")
    );

    ctx.process_async(json!({
        "id": 302,
        "sessionId": "SID-background",
        "method": "DOM.getSearchResults",
        "params": {
            "searchId": search_id,
            "fromIndex": 0,
            "toIndex": 2
        }
    }))
    .await;
    let results = take_response_by_id(&mut ctx, 302);
    assert_eq!(results["sessionId"], "SID-background");
    assert_eq!(
        results["result"]["nodeIds"].as_array().map(Vec::len),
        Some(2)
    );

    ctx.process_async(json!({
        "id": 303,
        "sessionId": "SID-background",
        "method": "DOM.discardSearchResults",
        "params": { "searchId": "0" }
    }))
    .await;
    ctx.expect_result(303, json!({}), Some("SID-background"));

    ctx.process_async(json!({
        "id": 304,
        "sessionId": "SID-background",
        "method": "DOM.getSearchResults",
        "params": {
            "searchId": "0",
            "fromIndex": 0,
            "toIndex": 1
        }
    }))
    .await;
    ctx.expect_error(304, -32000, "No search session with given id found");
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_search_targets_inactive_loaded_owner_without_activation() {
    let mut ctx = TestContext::new();
    let mut active = BrowserContext::new("BID-active".to_owned());
    active.set_active_target_id("TID-active".to_owned());
    active.attach_active_session("SID-active".to_owned());
    ctx.conn.browser_context = Some(active);

    let mut inactive = BrowserContext::new("BID-inactive".to_owned());
    inactive.set_active_target_id("TID-inactive".to_owned());
    inactive.set_target_url("about:blank".to_owned());
    inactive.attach_active_session("SID-inactive".to_owned());
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<!doctype html><html><body><article>one</article><article>two</article></body></html>",
        Some("SID-inactive"),
    )
    .await;

    ctx.process_async(json!({
        "id": 311,
        "sessionId": "SID-inactive",
        "method": "DOM.performSearch",
        "params": { "query": "article" }
    }))
    .await;
    let search_msg = take_response_by_id(&mut ctx, 311);
    let search_id = search_msg["result"]["searchId"]
        .as_str()
        .expect("search id")
        .to_owned();
    assert_eq!(search_msg["sessionId"], "SID-inactive");
    assert_eq!(search_msg["result"]["resultCount"], 2);
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-active")
    );

    ctx.process_async(json!({
        "id": 312,
        "sessionId": "SID-inactive",
        "method": "DOM.getSearchResults",
        "params": {
            "searchId": search_id,
            "fromIndex": 0,
            "toIndex": 2
        }
    }))
    .await;
    let results = take_response_by_id(&mut ctx, 312);
    assert_eq!(results["sessionId"], "SID-inactive");
    assert_eq!(
        results["result"]["nodeIds"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-active")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn perform_search_without_shadow_dom_uses_live_renderer_dispatch() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><p class='hit'>one</p><p class='hit'>two</p></body></html>",
    )
    .await;

    let perform_search_raw = json!({
        "id": 2,
        "method": "DOM.performSearch",
        "params": { "query": "p.hit" }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&perform_search_raw)
        .expect("DOM.performSearch should dispatch to renderer when a page is loaded");
    assert_eq!(pending.kind_name(), "DOM");
    let messages = complete_pending_command_task_for_test(&mut ctx, pending).await;
    assert_eq!(
        messages[0]["result"],
        json!({ "searchId": "0", "resultCount": 2 })
    );
    assert!(
        messages
            .iter()
            .all(|message| message["method"] != json!("DOM.setChildNodes")),
        "performSearch without includeUserAgentShadowDOM should not emit child-node snapshots"
    );

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "0", "fromIndex": 0, "toIndex": 2 }
    }))
    .await;
    let results = take_response_by_id(&mut ctx, 3);
    assert_eq!(
        results["result"]["nodeIds"],
        json!([0, 0]),
        "search preserves result positions but does not publish frontend ids before DOM.getDocument"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn perform_search_include_user_agent_shadow_dom_keeps_author_shadow_search_event_free() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        201,
        "<!doctype html><html><body><div id='host'></div><script>const root=document.getElementById('host').attachShadow({mode:'open'});const hit=document.createElement('p');hit.id='needle';root.appendChild(hit)</script></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 204,
        "method": "DOM.performSearch",
        "params": {
            "query": "#needle",
            "includeUserAgentShadowDOM": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 204);
    assert_eq!(
        response["result"],
        json!({ "searchId": "0", "resultCount": 1 })
    );
    assert!(
        ctx.take_all()
            .iter()
            .all(|message| message["method"] != json!("DOM.setChildNodes")),
        "includeUserAgentShadowDOM should not turn performSearch into an ancestry snapshot command"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn perform_search_include_user_agent_shadow_dom_projects_generated_control_nodes() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        212,
        "<!doctype html><html><body><input id='control' type='search' value='needle'></body></html>",
    )
    .await;

    for (id, query, include_user_agent_shadow_dom, expected_count) in [
        (213, "#editing-view-port", false, 0),
        (214, "#editing-view-port", true, 1),
        (215, "needle", false, 1),
        (216, "needle", true, 2),
        (217, "//*[@id='editing-view-port']", true, 0),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "DOM.performSearch",
            "params": {
                "query": query,
                "includeUserAgentShadowDOM": include_user_agent_shadow_dom,
            }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert_eq!(
            response["result"]["resultCount"],
            json!(expected_count),
            "unexpected generated-tree search projection for {query:?}: {response:?}"
        );
        assert!(
            ctx.take_all()
                .iter()
                .all(|message| message["method"] != json!("DOM.setChildNodes")),
            "search must bind generated results without publishing ancestry"
        );
    }

    ctx.process_async(json!({
        "id": 218,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "1", "fromIndex": 0, "toIndex": 1 }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 218)["result"]["nodeIds"],
        json!([0]),
        "generated search results remain unbound before this session publishes its document"
    );

    ctx.process_async(json!({
        "id": 219,
        "method": "DOM.enable"
    }))
    .await;
    ctx.expect_result(219, json!({}), None);
    ctx.process_async(json!({
        "id": 220,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    assert!(
        take_response_by_id(&mut ctx, 220)["result"]["root"]["nodeId"]
            .as_u64()
            .is_some_and(|node_id| node_id > 0),
        "DOM.getDocument must publish this session's Inspector tree"
    );

    ctx.process_async(json!({
        "id": 221,
        "method": "DOM.performSearch",
        "params": {
            "query": "#editing-view-port",
            "includeUserAgentShadowDOM": true,
        }
    }))
    .await;
    ctx.expect_result(221, json!({ "searchId": "5", "resultCount": 1 }), None);
    ctx.process_async(json!({
        "id": 222,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "5", "fromIndex": 0, "toIndex": 1 }
    }))
    .await;
    let generated_node_id = take_response_by_id(&mut ctx, 222)["result"]["nodeIds"][0]
        .as_u64()
        .expect("generated UA search result frontend node id");
    assert!(generated_node_id > 0);

    ctx.process_async(json!({
        "id": 223,
        "method": "DOM.describeNode",
        "params": { "nodeId": generated_node_id }
    }))
    .await;
    let description = take_response_by_id(&mut ctx, 223);
    assert_eq!(description["result"]["node"]["nodeName"], json!("DIV"));
    assert_eq!(
        description["result"]["node"]["attributes"],
        json!(["id", "editing-view-port"])
    );

    ctx.process_async(json!({
        "id": 224,
        "method": "DOM.performSearch",
        "params": {
            "query": "#editing-view-port",
            "includeUserAgentShadowDOM": true,
        }
    }))
    .await;
    ctx.expect_result(224, json!({ "searchId": "6", "resultCount": 1 }), None);
    ctx.process_async(json!({
        "id": 225,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "6", "fromIndex": 0, "toIndex": 1 }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 225)["result"]["nodeIds"][0],
        json!(generated_node_id),
        "repeated search must reuse the generated Inspector identity"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn perform_search_includes_child_frame_documents_without_snapshot_side_effects() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        205,
        "<!doctype html><html><body><iframe id='child' srcdoc=\"<p id='hit'></p><script>document.getElementById('hit').textContent=['child','search','needle'].join(' ')</script>\"></iframe></body></html>",
    )
    .await;
    publish_document_for_search(&mut ctx, 206, None).await;

    let perform_search_raw = json!({
        "id": 209,
        "method": "DOM.performSearch",
        "params": {
            "query": "child search needle",
            "includeUserAgentShadowDOM": true
        }
    })
    .to_string();
    let perform_search_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&perform_search_raw)
        .expect("DOM.performSearch should start as a renderer command");
    let perform_search_messages =
        complete_pending_command_task_for_test(&mut ctx, perform_search_pending).await;
    let response = perform_search_messages
        .iter()
        .find(|message| message["id"] == json!(209))
        .expect("DOM.performSearch response");
    assert_eq!(
        response["result"],
        json!({ "searchId": "0", "resultCount": 1 })
    );
    assert!(
        perform_search_messages
            .iter()
            .all(|message| message["method"] != json!("DOM.setChildNodes"))
    );

    ctx.process_async(json!({
        "id": 210,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "0", "fromIndex": 0, "toIndex": 1 }
    }))
    .await;
    let node_id = take_response_by_id(&mut ctx, 210)["result"]["nodeIds"][0]
        .as_u64()
        .expect("child document search node id");
    ctx.process_async(json!({
        "id": 211,
        "method": "DOM.describeNode",
        "params": { "nodeId": node_id }
    }))
    .await;
    let description = take_response_by_id(&mut ctx, 211);
    assert_eq!(description["result"]["node"]["nodeType"], json!(3));
    assert_eq!(
        description["result"]["node"]["nodeValue"],
        json!("child search needle")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn search_flow_reuses_registered_node_ids_across_multiple_searches() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(&mut ctx, 20, "<a id='a1'></a><a id='a2'></a>").await;
    publish_document_for_search(&mut ctx, 27, None).await;

    ctx.process_async(json!({
        "id": 21,
        "method": "DOM.performSearch",
        "params": { "query": "a[id]" }
    }))
    .await;
    ctx.expect_result(21, json!({ "searchId": "0", "resultCount": 2 }), None);

    ctx.process_async(json!({
        "id": 22,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "0", "fromIndex": 0, "toIndex": 2 }
    }))
    .await;
    let first_result = ctx.take_one();
    let first_node_ids = first_result["result"]["nodeIds"]
        .as_array()
        .expect("nodeIds")
        .iter()
        .filter_map(|value| value.as_u64())
        .collect::<Vec<_>>();
    assert_eq!(first_node_ids.len(), 2);

    ctx.process_async(json!({
        "id": 23,
        "method": "DOM.performSearch",
        "params": { "query": "#a1" }
    }))
    .await;
    ctx.expect_result(23, json!({ "searchId": "1", "resultCount": 1 }), None);

    ctx.process_async(json!({
        "id": 24,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "1", "fromIndex": 0, "toIndex": 1 }
    }))
    .await;
    ctx.expect_result(24, json!({ "nodeIds": [first_node_ids[0]] }), None);

    ctx.process_async(json!({
        "id": 25,
        "method": "DOM.performSearch",
        "params": { "query": "#a2" }
    }))
    .await;
    ctx.expect_result(25, json!({ "searchId": "2", "resultCount": 1 }), None);

    ctx.process_async(json!({
        "id": 26,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "2", "fromIndex": 0, "toIndex": 1 }
    }))
    .await;
    ctx.expect_result(26, json!({ "nodeIds": [first_node_ids[1]] }), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_search_results_preserves_renderer_frontend_binding_for_node_consumers() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    ctx.process_async(json!({
        "id": 30,
        "method": "Page.navigate",
        "params": {
            "url": "data:text/html,<a id='a1'></a><a id='a2'></a>"
        }
    }))
    .await;
    ctx.expect_result(
        30,
        json!({
            "frameId": "TID-1",
            "loaderId": "LID-0000000001",
        }),
        None,
    );
    crate::testing::wait_until_renderer_document_load(&mut ctx, None, "TID-1", "LID-0000000001")
        .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 31,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 31);
    let anchor = find_dom_node_by_attribute(&document["result"]["root"], "id", "a1")
        .unwrap_or_else(|| panic!("expected #a1 in getDocument response: {document:?}"));
    let frontend_node_id = anchor["nodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("#a1 frontend nodeId");
    let backend_node_id = anchor["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("#a1 backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "getDocument should register renderer-owned backend ids: {document:?}"
    );

    ctx.process_async(json!({
        "id": 32,
        "method": "DOM.performSearch",
        "params": { "query": "#a1" }
    }))
    .await;
    ctx.expect_result(32, json!({ "searchId": "0", "resultCount": 1 }), None);

    ctx.conn
        .clear_runtime_remote_object_tracking_for_session_owner(None);

    ctx.process_async(json!({
        "id": 33,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "0", "fromIndex": 0, "toIndex": 1 }
    }))
    .await;
    ctx.expect_result(33, json!({ "nodeIds": [frontend_node_id] }), None);

    ctx.conn
        .clear_runtime_remote_object_tracking_for_session_owner(None);

    ctx.process_async(json!({
        "id": 34,
        "method": "DOM.getAttributes",
        "params": { "nodeId": frontend_node_id }
    }))
    .await;
    ctx.expect_result(34, json!({ "attributes": ["id", "a1"] }), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_search_results_validates_index_ranges() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><p>one</p><p>two</p></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.performSearch",
        "params": { "query": "p" }
    }))
    .await;
    ctx.expect_result(2, json!({ "searchId": "0", "resultCount": 2 }), None);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "0", "fromIndex": 1, "toIndex": 1 }
    }))
    .await;
    ctx.expect_error(3, -32000, "Invalid search result range");

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "0", "fromIndex": 2, "toIndex": 3 }
    }))
    .await;
    ctx.expect_error(4, -32000, "Invalid search result range");

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "0", "fromIndex": 0, "toIndex": 3 }
    }))
    .await;
    ctx.expect_error(5, -32000, "Invalid search result range");

    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "0", "fromIndex": -1, "toIndex": 1 }
    }))
    .await;
    ctx.expect_error(6, -32000, "Invalid search result range");
}

fn find_dom_node_by_attribute<'a>(
    node: &'a serde_json::Value,
    name: &str,
    value: &str,
) -> Option<&'a serde_json::Value> {
    if node
        .get("attributes")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|attributes| {
            attributes.chunks(2).any(|pair| {
                pair.first().and_then(serde_json::Value::as_str) == Some(name)
                    && pair.get(1).and_then(serde_json::Value::as_str) == Some(value)
            })
        })
    {
        return Some(node);
    }
    for field in ["children", "shadowRoots"] {
        if let Some(found) = node
            .get(field)
            .and_then(serde_json::Value::as_array)
            .and_then(|children| {
                children
                    .iter()
                    .find_map(|child| find_dom_node_by_attribute(child, name, value))
            })
        {
            return Some(found);
        }
    }
    None
}

#[tokio::test(flavor = "multi_thread")]
async fn query_selector_invalid_node_errors_with_loaded_page() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><p>one</p><p>two</p></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.querySelector",
        "params": { "nodeId": 99, "selector": "" }
    }))
    .await;
    ctx.expect_error(2, -32000, "Could not find node with given id");

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelectorAll",
        "params": { "nodeId": 99, "selector": "" }
    }))
    .await;
    ctx.expect_error(3, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn query_selector_no_match_returns_chromium_empty_result() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div><p>one</p><p>two</p></div></body></html>",
    )
    .await;
    publish_document_for_search(&mut ctx, 10, None).await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.performSearch",
        "params": { "query": "div" }
    }))
    .await;
    ctx.expect_result(2, json!({ "searchId": "0", "resultCount": 1 }), None);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "0", "fromIndex": 0, "toIndex": 1 }
    }))
    .await;
    let div_node_id = ctx.take_one()["result"]["nodeIds"][0].as_u64().unwrap_or(0);
    assert!(div_node_id > 0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.querySelector",
        "params": { "nodeId": div_node_id, "selector": "a" }
    }))
    .await;
    assert_eq!(
        ctx.take_all(),
        vec![json!({ "id": 4, "result": { "nodeId": 0 } })],
        "a query without a result must not publish an unrelated child snapshot"
    );

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.querySelectorAll",
        "params": { "nodeId": div_node_id, "selector": "a" }
    }))
    .await;
    assert_eq!(
        ctx.take_all(),
        vec![json!({ "id": 5, "result": { "nodeIds": [] } })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn query_selector_nodes_found() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div><p id='a'>x</p></div></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let doc = ctx.take_one();
    let root = &doc["result"]["root"];
    let root_id = root["nodeId"].as_u64().unwrap_or(0);
    assert!(root_id > 0);
    let html = child_element_by_node_name(root, "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let body_node_id = body["nodeId"].as_u64().expect("body node id");
    assert_eq!(
        html["children"]
            .as_array()
            .expect("default document HTML children")
            .iter()
            .filter_map(|node| node["nodeName"].as_str())
            .collect::<Vec<_>>(),
        vec!["HEAD", "BODY"],
        "Chromium DOM.getDocument defaults to depth 2"
    );

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": {
            "nodeId": root_id,
            "selector": "p"
        }
    }))
    .await;
    let messages = ctx.take_all();
    let body_event_position = messages
        .iter()
        .position(|message| {
            message["method"] == json!("DOM.setChildNodes")
                && message["params"]["parentId"] == json!(body_node_id)
        })
        .expect("querySelector should publish the missing BODY children");
    let body_event = &messages[body_event_position];
    let div = node_array_element_by_node_name(&body_event["params"]["nodes"], "DIV");
    let div_node_id = div["nodeId"].as_u64().expect("div node id");
    let div_event_position = messages
        .iter()
        .position(|message| {
            message["method"] == json!("DOM.setChildNodes")
                && message["params"]["parentId"] == json!(div_node_id)
        })
        .expect("querySelector should publish the selected node's direct siblings");
    let response_position = messages
        .iter()
        .position(|message| message["id"] == json!(3))
        .expect("querySelector response");
    assert!(body_event_position < div_event_position);
    assert!(div_event_position < response_position);
    let selected = &messages[response_position];
    assert_eq!(selected["id"], 3);
    let selected_node_id = selected["result"]["nodeId"].as_u64().unwrap_or(0);
    assert!(selected_node_id > 0);
    assert_eq!(
        node_array_element_by_node_name(&messages[div_event_position]["params"]["nodes"], "P")["nodeId"],
        json!(selected_node_id)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn query_selector_document_root_uses_live_renderer_dispatch() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><p class='item'>a</p><p class='item'>b</p></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = take_response_by_id(&mut ctx, 2)["result"]["root"]["nodeId"]
        .as_u64()
        .expect("root node id");

    let query_raw = json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": ".item" }
    })
    .to_string();
    let query_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&query_raw)
        .expect("document-root DOM.querySelector should dispatch to renderer");
    assert_eq!(query_pending.kind_name(), "DOM");
    let query_messages = complete_pending_command_task_for_test(&mut ctx, query_pending).await;
    let query_response_position = query_messages
        .iter()
        .position(|message| message["id"] == json!(3))
        .expect("querySelector response");
    assert!(query_response_position > 0);
    assert!(
        query_messages[..query_response_position]
            .iter()
            .all(|message| message["method"] == json!("DOM.setChildNodes")),
        "querySelector must publish only node-path events before its response: {query_messages:?}"
    );
    let query_response = &query_messages[query_response_position];
    let first_node_id = query_response["result"]["nodeId"]
        .as_u64()
        .expect("selected node id");
    assert!(first_node_id > 0);

    let query_all_raw = json!({
        "id": 4,
        "method": "DOM.querySelectorAll",
        "params": { "nodeId": root_id, "selector": ".item" }
    })
    .to_string();
    let query_all_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&query_all_raw)
        .expect("document-root DOM.querySelectorAll should dispatch to renderer");
    assert_eq!(query_all_pending.kind_name(), "DOM");
    let query_all_messages =
        complete_pending_command_task_for_test(&mut ctx, query_all_pending).await;
    assert_eq!(
        query_all_messages.len(),
        1,
        "a repeated query must not republish an already requested node path"
    );
    let query_all_response = &query_all_messages[0];
    assert_eq!(query_all_response["id"], json!(4));
    let node_ids = query_all_response["result"]["nodeIds"]
        .as_array()
        .expect("node id array");
    assert_eq!(node_ids.len(), 2);
    assert_eq!(node_ids[0], json!(first_node_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn query_selector_nodes_found_emits_set_child_nodes_once_like_chromium() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div><p id='a'>x</p></div></body></html>",
    )
    .await;

    // Chromium returns nodeId 0 from getSearchResults until the frontend has
    // requested the document. A shallow request establishes the frontend root
    // while leaving the DIV children unpushed for the setChildNodes assertion.
    ctx.process_async(json!({
        "id": 10,
        "method": "DOM.getDocument",
        "params": { "depth": 0 }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10);

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.performSearch",
        "params": { "query": "div" }
    }))
    .await;
    ctx.expect_result(2, json!({ "searchId": "0", "resultCount": 1 }), None);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "0", "fromIndex": 0, "toIndex": 1 }
    }))
    .await;
    let div_node_id = ctx.take_one()["result"]["nodeIds"][0].as_u64().unwrap_or(0);
    assert!(div_node_id > 0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.querySelector",
        "params": { "nodeId": div_node_id, "selector": "p" }
    }))
    .await;
    let selected = take_response_by_id(&mut ctx, 4);
    let p_node_id = selected["result"]["nodeId"].as_u64().unwrap_or(0);
    assert!(p_node_id > 0);
    ctx.expect_event(
        "DOM.setChildNodes",
        Some(&json!({
            "parentId": div_node_id,
            "nodes": [{
                "nodeName": "P"
            }]
        })),
    );

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.querySelectorAll",
        "params": { "nodeId": div_node_id, "selector": "p" }
    }))
    .await;
    let selected_all = take_response_by_id(&mut ctx, 5);
    assert_eq!(selected_all["result"]["nodeIds"], json!([p_node_id]));
    assert!(
        ctx.take_all().is_empty(),
        "a repeated query must not republish the DIV children"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn query_selector_with_child_frame_completes_through_pending_set_child_nodes() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='container'><iframe id='child' srcdoc=\"<p>child</p>\"></iframe><p id='target'>x</p></div></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "Page.getFrameTree"}))
        .await;
    let child_frame_id = take_response_by_id(&mut ctx, 2)["result"]["frameTree"]["childFrames"][0]
        ["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned();

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.getDocument",
        "params": { "depth": 3 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 3);
    let html = child_element_by_node_name(&document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let container = child_element_by_node_name(body, "DIV");
    let container_node_id = container["nodeId"].as_u64().expect("container node id");

    let query_raw = json!({
        "id": 4,
        "method": "DOM.querySelector",
        "params": { "nodeId": container_node_id, "selector": "p" }
    })
    .to_string();
    let query_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&query_raw)
        .expect("DOM.querySelector with child frame siblings should start as pending command");
    let query_messages = complete_pending_command_task_for_test(&mut ctx, query_pending).await;
    assert_eq!(query_messages[0]["method"], json!("DOM.setChildNodes"));
    assert_eq!(query_messages[1]["id"], json!(4));
    let set_child_nodes = &query_messages[0];
    assert_eq!(
        set_child_nodes["params"]["parentId"],
        json!(container_node_id)
    );
    let iframe = node_array_element_by_node_name(&set_child_nodes["params"]["nodes"], "IFRAME");
    let p = node_array_element_by_node_name(&set_child_nodes["params"]["nodes"], "P");
    assert_eq!(iframe["frameId"], json!(child_frame_id));
    assert_eq!(query_messages[1]["result"]["nodeId"], p["nodeId"]);

    let query_all_raw = json!({
        "id": 5,
        "method": "DOM.querySelectorAll",
        "params": { "nodeId": container_node_id, "selector": "p" }
    })
    .to_string();
    let query_all_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&query_all_raw)
        .expect("DOM.querySelectorAll with child frame siblings should start as pending command");
    let query_all_messages =
        complete_pending_command_task_for_test(&mut ctx, query_all_pending).await;
    assert_eq!(query_all_messages.len(), 1);
    assert_eq!(query_all_messages[0]["id"], json!(5));
    assert_eq!(
        query_all_messages[0]["result"]["nodeIds"],
        json!([p["nodeId"].clone()])
    );
}
