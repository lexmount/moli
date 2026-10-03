//! Behavioral ports of Chromium 2d184faab9fb11e1070fb1eeafebf291f6e85923:
//! third_party/blink/web_tests/inspector-protocol/webmcp/invoke-tool.js,
//! cancel-invocation.js, list-tools.js, track-autosubmit.js and
//! invoke-tool-cross-document.js.
//! These adapters use the production CDP scheduler and renderer output fences.

use crate::testing::TestContext;
use serde_json::{Value, json};

async fn page(script: &str) -> TestContext {
    page_at(script, url::Url::parse("https://webmcp.test/").unwrap()).await
}

async fn page_at(script: &str, source_url: url::Url) -> TestContext {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_and_wait_for_response_async(
        json!({"id":1,"method":"Target.createTarget","params":{"url":"about:blank"}}),
    )
    .await;
    assert!(ctx.take_response_by_id(1)["result"]["targetId"].is_string());
    ctx.install_buffered_navigation_fixture_for_session_owner(
        source_url,
        format!("<!doctype html><body><script>{script}</script>"),
        None,
    )
    .await;
    ctx.sent.clear();
    ctx
}

async fn command(ctx: &mut TestContext, id: u64, method: &str, params: Value) -> Value {
    session_command(ctx, id, method, params, None).await
}

async fn session_command(
    ctx: &mut TestContext,
    id: u64,
    method: &str,
    params: Value,
    session: Option<&str>,
) -> Value {
    let mut message = json!({"id":id,"method":method,"params":params});
    if let Some(session) = session {
        message["sessionId"] = json!(session);
    }
    ctx.process_and_wait_for_response_async(message).await;
    ctx.sent
        .iter()
        .find(|message| message["id"] == id)
        .expect("command response")
        .clone()
}

async fn event(ctx: &mut TestContext, method: &str) -> Value {
    ctx.wait_for_scheduler_message(method, |message| message["method"] == method)
        .await
}

async fn frame_id(ctx: &mut TestContext) -> String {
    command(ctx, 2, "Page.getFrameTree", json!({})).await["result"]["frameTree"]["frame"]["id"]
        .as_str()
        .unwrap()
        .into()
}

#[tokio::test]
async fn web_mcp_permissions_header_bounds_cross_origin_iframe_delegation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let cases = std::sync::Arc::new([
        (None, false, false, "allowed"),
        (
            Some("tools=(self)".to_owned()),
            false,
            false,
            "NotAllowedError",
        ),
        (Some("tools=()".to_owned()), false, false, "NotAllowedError"),
        (Some("tools=*".to_owned()), false, false, "allowed"),
        (
            Some(format!("tools=(self \"http://{address}\")")),
            false,
            false,
            "allowed",
        ),
        (
            Some(format!("tools=(\"http://{address}\")")),
            false,
            false,
            "NotAllowedError",
        ),
        (Some("tools=*".to_owned()), false, true, "NotAllowedError"),
        (Some("tools=(self)".to_owned()), true, false, "allowed"),
    ]);
    let parent_cases = cases.clone();
    let child_cases = cases.clone();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route("/parent/{case}", axum::routing::get(move |axum::extract::Path(index): axum::extract::Path<usize>| {
                let cases = parent_cases.clone();
                async move {
                    let mut headers = axum::http::HeaderMap::new();
                    if let Some(header) = &cases[index].0 {
                        headers.insert("permissions-policy", header.parse().unwrap());
                    }
                    (headers, axum::response::Html(format!(r#"<!doctype html><body><script>
                        globalThis.childResult = new Promise(resolve => addEventListener('message', event => resolve(event.data), {{once:true}}));
                        const frame = document.createElement('iframe');
                        frame.allow = 'tools *'; frame.src = 'http://{address}/child/{index}'; document.body.append(frame);
                    </script>"#)))
                }
            }))
            .route("/child/{case}", axum::routing::get(move |axum::extract::Path(index): axum::extract::Path<usize>| {
                let cases = child_cases.clone();
                async move {
                    let mut headers = axum::http::HeaderMap::new();
                    if cases[index].2 {
                        headers.insert("permissions-policy", "tools=()".parse().unwrap());
                    }
                    (headers, axum::response::Html(r#"<!doctype html><script>
                        (async () => {
                            try {
                                await document.modelContext.registerTool({name:'child', description:'Child', execute:()=>42});
                                parent.postMessage('allowed', '*');
                            } catch (error) { parent.postMessage(error.name, '*'); }
                        })();
                    </script>"#))
                }
            }));
        axum::serve(listener, app).await.unwrap();
    });
    for (index, (header, same_origin, _, expected)) in cases.iter().enumerate() {
        let mut ctx = page("").await;
        command(&mut ctx, 2, "Page.enable", json!({})).await;
        let host = if *same_origin {
            "127.0.0.1"
        } else {
            "localhost"
        };
        let url = format!("http://{host}:{}/parent/{index}", address.port());
        let reply = command(&mut ctx, 3, "Page.navigate", json!({"url":url})).await;
        assert!(reply["result"].get("errorText").is_none(), "{reply}");
        event(&mut ctx, "Page.loadEventFired").await;
        let result = command(
            &mut ctx,
            4,
            "Runtime.evaluate",
            json!({
                "expression":"childResult", "awaitPromise":true, "returnByValue":true
            }),
        )
        .await;
        assert_eq!(
            result["result"]["result"]["value"], *expected,
            "{header:?}, same origin: {same_origin}: {result}"
        );
    }
    server.abort();
}

#[tokio::test]
async fn web_mcp_invocation_requires_an_enabled_domain_and_reports_parameter_errors() {
    let mut ctx = page(
        "document.modelContext.registerTool({name:'echo',description:'Echo',execute:()=>42});",
    )
    .await;
    let root = frame_id(&mut ctx).await;
    let input = json!({"frameId":root,"toolName":"echo","input":{}});
    assert_eq!(
        command(&mut ctx, 3, "WebMCP.invokeTool", input.clone()).await["error"],
        json!({"code":-32000,"message":"WebMCP domain is not enabled"})
    );
    command(&mut ctx, 4, "WebMCP.enable", json!({})).await;
    event(&mut ctx, "WebMCP.toolsAdded").await;
    for (id, params, message) in [
        (
            5,
            json!({"frameId":root,"toolName":"missing","input":{}}),
            "Tool not found",
        ),
        (
            6,
            json!({"frameId":"missing","toolName":"echo","input":{}}),
            "No frame for given id found",
        ),
    ] {
        assert_eq!(
            command(&mut ctx, id, "WebMCP.invokeTool", params).await["error"],
            json!({"code":-32602,"message":message})
        );
    }
    assert_eq!(
        command(
            &mut ctx,
            7,
            "WebMCP.cancelInvocation",
            json!({"invocationId":"18446744073709551615"})
        )
        .await["error"],
        json!({"code":-32602,"message":"No pending execution for invocation id"})
    );
    command(&mut ctx, 8, "WebMCP.disable", json!({})).await;
    assert_eq!(
        command(&mut ctx, 9, "WebMCP.invokeTool", input).await["error"]["code"],
        -32000
    );
}

#[tokio::test]
async fn web_mcp_invocation_uses_the_callback_realm_for_arguments() {
    let mut ctx = page(
        r#"
        const frame = document.createElement('iframe');
        document.body.append(frame);
        document.modelContext.registerTool({
            name: 'realm', description: 'Callback in another realm',
            execute: frame.contentWindow.eval(`async (input, options) => ({
                root: Object.getPrototypeOf(input) === Object.prototype,
                nested: Object.getPrototypeOf(input.nested) === Object.prototype,
                array: Object.getPrototypeOf(input.items) === Array.prototype,
                element: Object.getPrototypeOf(input.items[0]) === Object.prototype,
                options: Object.getPrototypeOf(options) === Object.prototype,
                signal: options.signal instanceof parent.AbortSignal
            })`)
        });
    "#,
    )
    .await;
    let root = frame_id(&mut ctx).await;
    command(&mut ctx, 3, "WebMCP.enable", json!({})).await;
    event(&mut ctx, "WebMCP.toolsAdded").await;
    let reply = command(
        &mut ctx,
        4,
        "WebMCP.invokeTool",
        json!({"frameId":root,"toolName":"realm","input":{"nested":{},"items":[{}]}}),
    )
    .await;
    assert!(reply.get("error").is_none(), "{reply}");
    let response = event(&mut ctx, "WebMCP.toolResponded").await;
    assert_eq!(
        response["params"]["invocationId"],
        reply["result"]["invocationId"]
    );
    assert_eq!(response["params"]["status"], "Completed");
    assert_eq!(
        response["params"]["output"],
        json!({"root":true,"nested":true,"array":true,"element":true,"options":true,"signal":true})
    );
    assert_eq!(
        command(
            &mut ctx,
            5,
            "WebMCP.cancelInvocation",
            json!({"invocationId":reply["result"]["invocationId"]})
        )
        .await["error"]["message"],
        "No pending execution for invocation id"
    );
}

#[tokio::test]
async fn web_mcp_result_serialization_exception_remains_inspectable() {
    let mut ctx = page("document.modelContext.registerTool({name:'serialize',description:'Throw from toJSON',execute:()=>({toJSON(){throw new TypeError('serialization failed')}})});").await;
    let root = frame_id(&mut ctx).await;
    command(&mut ctx, 3, "WebMCP.enable", json!({})).await;
    event(&mut ctx, "WebMCP.toolsAdded").await;
    let reply = command(
        &mut ctx,
        4,
        "WebMCP.invokeTool",
        json!({"frameId":root,"toolName":"serialize","input":{}}),
    )
    .await;
    let response = event(&mut ctx, "WebMCP.toolResponded").await;
    assert_eq!(
        response["params"]["invocationId"],
        reply["result"]["invocationId"]
    );
    assert_eq!(response["params"]["status"], "Error");
    assert!(response["params"].get("output").is_none());
    let exception = &response["params"]["exception"];
    assert_eq!(exception["className"], "TypeError");
    assert!(
        exception["description"]
            .as_str()
            .unwrap()
            .contains("serialization failed")
    );
    let properties = command(
        &mut ctx,
        5,
        "Runtime.getProperties",
        json!({"objectId":exception["objectId"],"ownProperties":true}),
    )
    .await;
    assert!(properties.get("error").is_none(), "{properties}");
    assert!(
        properties["result"]["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|property| property["name"] == "message"
                && property["value"]["value"] == "serialization failed")
    );
}

#[tokio::test]
async fn web_mcp_registration_stack_trace_is_retained_for_later_enable() {
    let script = r#"
        function registerFromCallsite() {
            document.modelContext.registerTool({name:'located',description:'Registration location',execute:()=>42});
        }
        registerFromCallsite();
        //# sourceURL=https://webmcp.test/tools.js
    "#;
    let mut ctx = page(script).await;
    command(&mut ctx, 3, "WebMCP.enable", json!({})).await;
    let added = event(&mut ctx, "WebMCP.toolsAdded").await;
    let frames = added["params"]["tools"][0]["stackTrace"]["callFrames"]
        .as_array()
        .expect("registration stack");
    assert!(frames.len() >= 2, "{frames:?}");
    assert_eq!(frames[0]["functionName"], "registerFromCallsite");
    assert_eq!(frames[0]["url"], "https://webmcp.test/tools.js");
    assert_eq!(
        frames[0]["lineNumber"],
        script
            .lines()
            .position(|line| line.contains("document.modelContext.registerTool"))
            .unwrap()
    );
    assert!(frames[0]["columnNumber"].is_u64());
    assert!(
        frames[0]["scriptId"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

async fn assert_navigation_result(topology: &str, destination: &str) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, axum::Router::new().route("/result", axum::routing::any(|| async {
            axum::response::Html(r#"<!doctype html><body>
              <script type="application/ld+json">{"cross-document":"success"}</script>
              <script type="application/ld+json">malformed</script>
              <script type="application/ld+json">/* comment */ {"url":"https://example.test/","escaped":"\\\"//"}</script>
              <script type="APPLICATION/LD+JSON">{"ignored":true}</script>
              <script>document.addEventListener('DOMContentLoaded',()=>{
                document.querySelector('script').textContent='{"too-late":true}';
              });</script>"#)
        })).route("/replace", axum::routing::get(|| async {
            axum::response::Html(r#"<!doctype html><script defer src="/hold.js"></script><script>
              const channel=new MessageChannel();
              channel.port1.onmessage=()=>{
                document.open();document.write('<script type="application/ld+json">{"wrong-document":true}<\/script>');document.close();
              };
              channel.port2.postMessage('replace');
            </script>"#)
        })).route("/blocked_result", axum::routing::get(|| async {
            axum::response::Html(r#"<!doctype html><script defer src="/hold.js"></script>
              <script type="application/ld+json">{"must-not-return":true}</script>"#)
        })).route("/hold.js", axum::routing::get(|| async {
            // DCL remains blocked. Cross-origin tool results must fail at
            // commit without waiting for this resource; replacement retires it.
            std::future::pending::<axum::response::Html<&'static str>>().await
        })).route("/sandbox", axum::routing::get(|| async {
            ([("Content-Security-Policy", "sandbox allow-scripts")], axum::response::Html(r#"<!doctype html><script type="application/ld+json">{"sandboxed":true}</script>"#))
        })).route("/empty", axum::routing::get(|| async { axum::http::StatusCode::NO_CONTENT }))
        .route("/same_redirect", axum::routing::get(|| async {axum::response::Redirect::temporary("/result")}))
        .route("/cross_redirect", axum::routing::get(move || async move {axum::response::Redirect::temporary(&format!("http://localhost:{}/result", address.port()))}))
        .route("/reset", axum::routing::get(|| async { axum::http::StatusCode::RESET_CONTENT }))).await.unwrap();
    });
    let target = match topology {
        "named_child" => "target=result",
        "named_child_post" => "target=result method=post",
        "top" => "target=_top",
        "parent" | "child_parent" | "nested_parent" => "target=_parent",
        "parent_post" | "child_parent_post" | "nested_parent_post" => "target=_PARENT method=post",
        "child_self_post" => "target=_self method=post",
        _ => "",
    };
    let action = if destination == "cross_blocked" {
        format!("http://localhost:{}/blocked_result", address.port())
    } else if destination == "cross_origin" {
        format!("http://localhost:{}/result", address.port())
    } else {
        format!("http://{address}/{destination}")
    };
    let form = json!(format!(
        r#"<form toolname=navigate_tool tooldescription=Navigate toolautosubmit {target} action="{action}"><input name=text></form>"#
    ));
    let source = if matches!(topology, "nested_parent" | "nested_parent_post") {
        format!(
            "const outer=document.createElement('iframe');document.body.append(outer);const child=outer.contentDocument.createElement('iframe');child.srcdoc={form};outer.contentDocument.body.append(child);"
        )
    } else if matches!(
        topology,
        "child_self" | "child_parent" | "child_self_post" | "child_parent_post"
    ) {
        format!(
            "const child=document.createElement('iframe');child.srcdoc={form};document.body.append(child);"
        )
    } else {
        format!(
            "document.body.innerHTML={form};{}",
            if matches!(topology, "named_child" | "named_child_post") {
                "const child=document.createElement('iframe');child.name='result';document.body.append(child);"
            } else {
                ""
            }
        )
    };
    let source_url = format!("http://{address}/source");
    let mut ctx = page_at(&source, url::Url::parse(&source_url).unwrap()).await;
    command(
        &mut ctx,
        2,
        "Runtime.evaluate",
        json!({"expression":"globalThis.retainedDocument=document"}),
    )
    .await;
    command(&mut ctx, 3, "WebMCP.enable", json!({})).await;
    let added = event(&mut ctx, "WebMCP.toolsAdded").await;
    let frame = added["params"]["tools"][0]["frameId"].as_str().unwrap();
    let reply = command(
        &mut ctx,
        4,
        "WebMCP.invokeTool",
        json!({"frameId":frame,"toolName":"navigate_tool","input":{"text":"unused"}}),
    )
    .await;
    let invocation = reply["result"]["invocationId"].as_str().unwrap();
    let response = event(&mut ctx, "WebMCP.toolResponded").await;
    if matches!(destination, "result" | "same_redirect") {
        assert_eq!(
            response["params"],
            json!({"invocationId":invocation,"status":"Completed","output":[
                {"cross-document":"success"},{"url":"https://example.test/","escaped":"\\\"//"}
            ]})
        );
    } else {
        assert_eq!(response["params"]["invocationId"], invocation);
        assert_eq!(response["params"]["status"], "Error");
        assert!(response["params"].get("output").is_none());
        if matches!(
            destination,
            "cross_origin" | "cross_blocked" | "cross_redirect" | "sandbox"
        ) {
            assert_eq!(
                response["params"]["errorText"],
                "Cannot return tool results after a cross-origin navigation"
            );
        }
    }
    if matches!(destination, "empty" | "reset") {
        let retained = command(&mut ctx, 5, "Runtime.evaluate", json!({
            "expression": format!("document===retainedDocument && location.href==='{source_url}' && \
                document.querySelector('form input').value==='unused' && \
                !document.querySelector('form').matches(':tool-form-active')"),
            "returnByValue":true
        })).await;
        assert_eq!(retained["result"]["result"]["value"], true);
        assert!(
            !ctx.sent
                .iter()
                .any(|message| message["method"] == "WebMCP.toolsRemoved")
        );
    }
    assert_eq!(
        ctx.sent
            .iter()
            .filter(|message| message["method"] == "WebMCP.toolResponded"
                && message["params"]["invocationId"] == invocation)
            .count(),
        0
    );
    server.abort();
}

#[tokio::test]
async fn web_mcp_chromium_cross_document_navigation_returns_destination_json_ld() {
    assert_navigation_result("root", "result").await;
}

#[tokio::test]
async fn web_mcp_named_child_navigation_returns_destination_json_ld() {
    assert_navigation_result("named_child", "result").await;
}

#[tokio::test]
async fn web_mcp_parsed_child_form_navigation_returns_destination_json_ld() {
    assert_navigation_result("child_self", "result").await;
}

#[tokio::test]
async fn web_mcp_top_target_navigation_returns_destination_json_ld() {
    assert_navigation_result("top", "result").await;
}

#[tokio::test]
async fn web_mcp_navigation_rejects_replaced_destination_document() {
    assert_navigation_result("root", "replace").await;
    assert_navigation_result("named_child", "replace").await;
}

#[tokio::test]
async fn web_mcp_navigation_without_destination_document_reports_error() {
    for destination in ["empty", "reset"] {
        assert_navigation_result("root", destination).await;
        assert_navigation_result("named_child", destination).await;
    }
}

#[tokio::test]
async fn web_mcp_form_navigation_binds_resolved_targets_for_get_and_post() {
    for topology in [
        "parent",
        "child_parent",
        "nested_parent",
        "parent_post",
        "child_parent_post",
        "nested_parent_post",
        "child_self_post",
        "named_child_post",
    ] {
        assert_navigation_result(topology, "result").await;
    }
}

#[tokio::test]
async fn web_mcp_navigation_results_require_the_original_origin_after_redirects() {
    for topology in ["root", "named_child", "child_self", "parent"] {
        for destination in [
            "cross_blocked",
            "cross_origin",
            "cross_redirect",
            "same_redirect",
            "sandbox",
        ] {
            assert_navigation_result(topology, destination).await;
        }
    }
}

#[tokio::test]
async fn web_mcp_chromium_declarative_autosubmit_changes_and_backend_node() {
    let mut ctx = page(r#"
      document.body.innerHTML = '<form id="declarative" toolname="declarative_tool" tooldescription="A declarative WebMCP tool"><input name="text"></form>';
    "#).await;
    let root = frame_id(&mut ctx).await;
    command(&mut ctx, 3, "WebMCP.enable", json!({})).await;
    let added = event(&mut ctx, "WebMCP.toolsAdded").await;
    let tool = &added["params"]["tools"][0];
    assert_eq!(tool["name"], "declarative_tool");
    assert_eq!(tool["frameId"], root);
    assert_eq!(
        tool["inputSchema"],
        json!({"type":"object","properties":{"text":{"type":"string"}},"required":[]})
    );
    assert!(tool.get("annotations").is_none());
    let backend = tool["backendNodeId"]
        .as_u64()
        .expect("native form backend node");
    let resolved = command(
        &mut ctx,
        4,
        "DOM.resolveNode",
        json!({"backendNodeId":backend}),
    )
    .await;
    let object_id = resolved["result"]["object"]["objectId"].as_str().unwrap();
    let description = command(&mut ctx,5,"Runtime.callFunctionOn",json!({"objectId":object_id,"functionDeclaration":"function(){return this.tagName+'|'+this.id}","returnByValue":true})).await;
    assert_eq!(description["result"]["result"]["value"], "FORM|declarative");
    for (id, expression, autosubmit) in [
        (
            6,
            "document.getElementById('declarative').setAttribute('toolautosubmit','')",
            true,
        ),
        (
            7,
            "document.getElementById('declarative').removeAttribute('toolautosubmit')",
            false,
        ),
    ] {
        command(
            &mut ctx,
            id,
            "Runtime.evaluate",
            json!({"expression":expression}),
        )
        .await;
        assert_eq!(
            event(&mut ctx, "WebMCP.toolsRemoved").await["params"]["tools"],
            json!([{"name":"declarative_tool","frameId":root}])
        );
        let added = event(&mut ctx, "WebMCP.toolsAdded").await;
        let tool = &added["params"]["tools"][0];
        assert_eq!(tool["backendNodeId"], backend);
        assert_eq!(
            tool["annotations"]["autosubmit"].as_bool().unwrap_or(false),
            autosubmit
        );
    }
}

#[tokio::test]
async fn web_mcp_declarative_cdp_invocation_fills_native_controls_and_returns_response() {
    let mut ctx = page(r#"
      document.body.innerHTML = '<form toolautosubmit toolname="echo_form" tooldescription="Echo form"><input name="text" required></form>';
      document.querySelector('form').addEventListener('submit',e=>{
        e.preventDefault();e.respondWith(Promise.resolve({agent:e.agentInvoked,text:e.target.elements.text.value}));
      });
    "#).await;
    let root = frame_id(&mut ctx).await;
    command(&mut ctx, 3, "WebMCP.enable", json!({})).await;
    event(&mut ctx, "WebMCP.toolsAdded").await;
    let reply = command(
        &mut ctx,
        4,
        "WebMCP.invokeTool",
        json!({"frameId":root,"toolName":"echo_form","input":{"text":"hello"}}),
    )
    .await;
    let invocation = reply["result"]["invocationId"].as_str().unwrap();
    assert_eq!(
        event(&mut ctx, "WebMCP.toolInvoked").await["params"]["invocationId"],
        invocation
    );
    assert_eq!(
        event(&mut ctx, "WebMCP.toolResponded").await["params"],
        json!({
            "invocationId":invocation,"status":"Completed","output":{"agent":true,"text":"hello"}
        })
    );
    let reply = command(
        &mut ctx,
        5,
        "WebMCP.invokeTool",
        json!({"frameId":root,"toolName":"echo_form","input":{"unknown":"invalid"}}),
    )
    .await;
    let invocation = reply["result"]["invocationId"].as_str().unwrap();
    event(&mut ctx, "WebMCP.toolInvoked").await;
    let response = event(&mut ctx, "WebMCP.toolResponded").await;
    assert_eq!(response["params"]["invocationId"], invocation);
    assert_eq!(response["params"]["status"], "Error");
    assert!(response["params"].get("exception").is_none());
    assert!(
        response["params"]["errorText"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
}

#[tokio::test]
async fn web_mcp_chromium_list_imperative_tools_replays_and_tracks_changes() {
    let mut ctx = page(r#"
      globalThis.initialController = new AbortController();
      document.modelContext.registerTool({name:'initial_imperative_tool',description:'An imperative WebMCP tool',execute:obj=>obj.text,annotations:{readOnlyHint:true}}, {signal:initialController.signal});
    "#).await;
    let root = frame_id(&mut ctx).await;
    assert_eq!(
        command(&mut ctx, 3, "WebMCP.enable", json!({})).await["result"],
        json!({})
    );
    let added = event(&mut ctx, "WebMCP.toolsAdded").await;
    let mut tools = added["params"]["tools"].clone();
    let stack = tools[0]
        .as_object_mut()
        .unwrap()
        .remove("stackTrace")
        .expect("registration stack");
    assert!(!stack["callFrames"].as_array().unwrap().is_empty());
    assert_eq!(
        tools,
        json!([{"name":"initial_imperative_tool","description":"An imperative WebMCP tool","frameId":root,"annotations":{"readOnly":true,"consequential":false,"untrustedContent":false,"debugging":false}}])
    );
    command(&mut ctx, 4, "Runtime.evaluate", json!({"expression":"document.modelContext.registerTool({name:'new_imperative_tool',description:'Another imperative tool',execute: obj => obj.text,inputSchema:{type:'object',properties:{text:{type:'string'}},required:['text']}})","awaitPromise":true})).await;
    let added = event(&mut ctx, "WebMCP.toolsAdded").await;
    assert_eq!(added["params"]["tools"][0]["name"], "new_imperative_tool");
    assert_eq!(
        added["params"]["tools"][0]["inputSchema"]["required"],
        json!(["text"])
    );
    assert!(added["params"]["tools"][0].get("annotations").is_none());
    command(
        &mut ctx,
        5,
        "Runtime.evaluate",
        json!({"expression":"initialController.abort()"}),
    )
    .await;
    assert_eq!(
        event(&mut ctx, "WebMCP.toolsRemoved").await["params"],
        json!({"tools":[{"name":"initial_imperative_tool","frameId":root}]})
    );
    command(&mut ctx, 6, "WebMCP.disable", json!({})).await;
    command(&mut ctx, 7, "Runtime.evaluate", json!({"expression":"document.modelContext.registerTool({name:'newer_imperative_tool',description:'Another imperative tool',execute:()=>{}})","awaitPromise":true})).await;
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != "WebMCP.toolsAdded")
    );
    command(&mut ctx, 8, "WebMCP.enable", json!({})).await;
    let replay = event(&mut ctx, "WebMCP.toolsAdded").await;
    assert_eq!(replay["params"]["tools"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn web_mcp_chromium_invoke_returns_id_before_events_and_preserves_failure_exception() {
    let mut ctx = page(r#"
      document.modelContext.registerTool({name:'test_tool',description:'A test WebMCP tool',execute:async obj=>obj.text});
      document.modelContext.registerTool({name:'failing_tool',description:'A failing WebMCP tool',execute:async()=>{throw new Error('This tool always fails')}});
    "#).await;
    let root = frame_id(&mut ctx).await;
    command(&mut ctx, 3, "WebMCP.enable", json!({})).await;
    event(&mut ctx, "WebMCP.toolsAdded").await;
    let reply = command(
        &mut ctx,
        4,
        "WebMCP.invokeTool",
        json!({"frameId":root,"toolName":"test_tool","input":{"text":"hello world"}}),
    )
    .await;
    let invocation = reply["result"]["invocationId"].as_str().unwrap();
    let response_position = ctx
        .sent
        .iter()
        .position(|message| message["id"] == 4)
        .unwrap();
    for method in ["WebMCP.toolInvoked", "WebMCP.toolResponded"] {
        if let Some(position) = ctx
            .sent
            .iter()
            .position(|message| message["method"] == method)
        {
            assert!(
                response_position < position,
                "invoke response must precede {method}: {:?}",
                ctx.sent
            );
        }
    }
    let invoked = event(&mut ctx, "WebMCP.toolInvoked").await;
    assert_eq!(invoked["params"]["invocationId"], invocation);
    assert_eq!(invoked["params"]["input"], r#"{"text":"hello world"}"#);
    assert!(
        ctx.sent.iter().any(|message| message["id"] == 4),
        "response is delivered before invocation event"
    );
    let result = event(&mut ctx, "WebMCP.toolResponded").await;
    assert_eq!(
        result["params"],
        json!({"invocationId":invocation,"status":"Completed","output":"hello world"})
    );
    assert_eq!(
        command(
            &mut ctx,
            5,
            "WebMCP.invokeTool",
            json!({"frameId":root,"toolName":"nonexistent_tool","input":{}})
        )
        .await["error"]["message"],
        "Tool not found"
    );
    assert_eq!(
        command(
            &mut ctx,
            6,
            "WebMCP.invokeTool",
            json!({"frameId":"invalid_frame_id","toolName":"test_tool","input":{}})
        )
        .await["error"]["message"],
        "No frame for given id found"
    );
    let reply = command(
        &mut ctx,
        7,
        "WebMCP.invokeTool",
        json!({"frameId":root,"toolName":"failing_tool","input":{}}),
    )
    .await;
    let result = event(&mut ctx, "WebMCP.toolResponded").await;
    assert_eq!(
        result["params"]["invocationId"],
        reply["result"]["invocationId"]
    );
    assert_eq!(result["params"]["status"], "Error");
    assert_eq!(result["params"]["exception"]["className"], "Error");
    assert!(
        result["params"]["exception"]["description"]
            .as_str()
            .unwrap()
            .contains("This tool always fails")
    );
    let object = result["params"]["exception"]["objectId"].as_str().unwrap();
    let properties = command(
        &mut ctx,
        8,
        "Runtime.getProperties",
        json!({"objectId":object,"ownProperties":true}),
    )
    .await;
    assert!(
        properties.get("error").is_none(),
        "exception remains inspectable: {properties}"
    );
}

#[tokio::test]
async fn web_mcp_chromium_cancel_rejects_invalid_and_repeated_ids_and_ignores_late_result() {
    let mut ctx = page(r#"
      globalThis.aborted = 0;
      document.modelContext.registerTool({name:'test_tool',description:'A test WebMCP tool',execute:(input,options)=>new Promise(resolve=>{options.signal.onabort=()=>{aborted++;resolve('late result')}})});
    "#).await;
    let root = frame_id(&mut ctx).await;
    command(&mut ctx, 3, "WebMCP.enable", json!({})).await;
    let reply = command(
        &mut ctx,
        4,
        "WebMCP.invokeTool",
        json!({"frameId":root,"toolName":"test_tool","input":{}}),
    )
    .await;
    let id = reply["result"]["invocationId"].as_str().unwrap();
    event(&mut ctx, "WebMCP.toolInvoked").await;
    assert_eq!(
        command(
            &mut ctx,
            5,
            "WebMCP.cancelInvocation",
            json!({"invocationId":"invalid-id"})
        )
        .await["error"]["message"],
        "Invalid invocation id"
    );
    assert_eq!(
        command(
            &mut ctx,
            6,
            "WebMCP.cancelInvocation",
            json!({"invocationId":id})
        )
        .await["result"],
        json!({})
    );
    assert_eq!(
        command(
            &mut ctx,
            7,
            "WebMCP.cancelInvocation",
            json!({"invocationId":id})
        )
        .await["error"]["message"],
        "No pending execution for invocation id"
    );
    assert_eq!(
        event(&mut ctx, "WebMCP.toolResponded").await["params"],
        json!({"invocationId":id,"status":"Canceled","errorText":""})
    );
    assert_eq!(
        command(
            &mut ctx,
            8,
            "Runtime.evaluate",
            json!({"expression":"aborted","returnByValue":true})
        )
        .await["result"]["result"]["value"],
        1
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != "WebMCP.toolResponded")
    );
}

#[tokio::test]
async fn web_mcp_enable_survives_document_replacement_and_ids_are_not_reused() {
    let script = "document.modelContext.registerTool({name:'wait',description:'pending',execute:()=>new Promise(()=>{})});";
    let mut ctx = page(script).await;
    let root = frame_id(&mut ctx).await;
    command(&mut ctx, 3, "WebMCP.enable", json!({})).await;
    let first = command(
        &mut ctx,
        4,
        "WebMCP.invokeTool",
        json!({"frameId":root,"toolName":"wait","input":{}}),
    )
    .await;
    event(&mut ctx, "WebMCP.toolInvoked").await;
    ctx.sent.clear();
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url::Url::parse("https://webmcp.test/next").unwrap(),
        format!("<!doctype html><script>{script}</script>"),
        None,
    )
    .await;
    let replay = event(&mut ctx, "WebMCP.toolsAdded").await;
    let root = replay["params"]["tools"][0]["frameId"].as_str().unwrap();
    let second = command(
        &mut ctx,
        5,
        "WebMCP.invokeTool",
        json!({"frameId":root,"toolName":"wait","input":{}}),
    )
    .await;
    assert_ne!(
        first["result"]["invocationId"],
        second["result"]["invocationId"]
    );
    assert_eq!(
        command(
            &mut ctx,
            6,
            "WebMCP.cancelInvocation",
            json!({"invocationId":first["result"]["invocationId"]})
        )
        .await["error"]["message"],
        "No pending execution for invocation id"
    );
    assert_eq!(
        command(
            &mut ctx,
            7,
            "WebMCP.cancelInvocation",
            json!({"invocationId":second["result"]["invocationId"]})
        )
        .await["result"],
        json!({})
    );
}

#[tokio::test]
async fn web_mcp_document_open_removes_tools_and_finishes_pending_invocations_once() {
    let mut ctx = page(r#"
      document.modelContext.registerTool({name:'wait',description:'pending',execute:(input,{signal})=>{
        globalThis.savedSignal = signal;
        return new Promise(resolve=>signal.addEventListener('abort',()=>resolve('late')));
      }});
    "#).await;
    let root = frame_id(&mut ctx).await;
    command(&mut ctx, 3, "WebMCP.enable", json!({})).await;
    event(&mut ctx, "WebMCP.toolsAdded").await;
    let reply = command(
        &mut ctx,
        4,
        "WebMCP.invokeTool",
        json!({"frameId":root,"toolName":"wait","input":{}}),
    )
    .await;
    let invocation = &reply["result"]["invocationId"];
    event(&mut ctx, "WebMCP.toolInvoked").await;
    command(&mut ctx, 5, "Runtime.evaluate", json!({"expression":"document.open(); document.write('<!doctype html><body>new</body>'); document.close();"})).await;
    assert_eq!(
        event(&mut ctx, "WebMCP.toolsRemoved").await["params"],
        json!({"tools":[{"name":"wait","frameId":root}]})
    );
    assert_eq!(
        event(&mut ctx, "WebMCP.toolResponded").await["params"],
        json!({"invocationId":invocation,"status":"Error","errorText":"Tool document is no longer active"})
    );
    // Retirement emits the protocol result before its browser task aborts the
    // signal. Wait for that task's effect rather than racing Runtime.evaluate.
    assert_eq!(
        command(
            &mut ctx,
            6,
            "Runtime.evaluate",
            json!({"expression":"savedSignal.aborted ? true : new Promise(resolve => savedSignal.addEventListener('abort', () => resolve(savedSignal.aborted), {once:true}))","returnByValue":true,"awaitPromise":true})
        )
        .await["result"]["result"]["value"],
        true
    );
    assert_eq!(
        command(
            &mut ctx,
            7,
            "WebMCP.cancelInvocation",
            json!({"invocationId":invocation})
        )
        .await["error"]["message"],
        "No pending execution for invocation id"
    );
    command(&mut ctx, 8, "Runtime.evaluate", json!({"expression":"document.modelContext.registerTool({name:'wait',description:'new',execute:()=> 'new'})","awaitPromise":true})).await;
    assert_eq!(
        event(&mut ctx, "WebMCP.toolsAdded").await["params"]["tools"][0]["description"],
        "new"
    );
    let reply = command(
        &mut ctx,
        9,
        "WebMCP.invokeTool",
        json!({"frameId":root,"toolName":"wait","input":{}}),
    )
    .await;
    event(&mut ctx, "WebMCP.toolInvoked").await;
    assert_eq!(
        event(&mut ctx, "WebMCP.toolResponded").await["params"],
        json!({"invocationId":reply["result"]["invocationId"],"status":"Completed","output":"new"})
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != "WebMCP.toolResponded")
    );
}

#[tokio::test]
async fn web_mcp_sessions_receive_only_their_enabled_events_and_own_replays() {
    let mut ctx = page("").await;
    let targets = command(&mut ctx, 2, "Target.getTargets", json!({})).await;
    let target = targets["result"]["targetInfos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["type"] == "page")
        .unwrap()["targetId"]
        .clone();
    let first = command(
        &mut ctx,
        3,
        "Target.attachToTarget",
        json!({"targetId":target,"flatten":true}),
    )
    .await;
    let first = first["result"]["sessionId"].as_str().unwrap();
    let second = command(
        &mut ctx,
        4,
        "Target.attachToTarget",
        json!({"targetId":target,"flatten":true}),
    )
    .await;
    let second = second["result"]["sessionId"].as_str().unwrap();
    assert_ne!(first, second);
    ctx.sent.clear();
    assert_eq!(
        session_command(&mut ctx, 5, "WebMCP.enable", json!({}), Some(first)).await["result"],
        json!({})
    );
    session_command(&mut ctx, 6, "Runtime.evaluate", json!({"expression":"document.modelContext.registerTool({name:'first',description:'first',execute:()=>{}})","awaitPromise":true}), Some(second)).await;
    assert_eq!(
        event(&mut ctx, "WebMCP.toolsAdded").await["sessionId"],
        first
    );
    assert_eq!(
        session_command(&mut ctx, 7, "WebMCP.enable", json!({}), Some(second)).await["result"],
        json!({})
    );
    let replay = event(&mut ctx, "WebMCP.toolsAdded").await;
    assert_eq!(replay["sessionId"], second);
    assert_eq!(replay["params"]["tools"][0]["name"], "first");
    session_command(&mut ctx, 8, "WebMCP.disable", json!({}), Some(first)).await;
    session_command(&mut ctx, 9, "Runtime.evaluate", json!({"expression":"document.modelContext.registerTool({name:'second',description:'second',execute:()=>{}})","awaitPromise":true}), Some(first)).await;
    let added = event(&mut ctx, "WebMCP.toolsAdded").await;
    assert_eq!(added["sessionId"], second);
    assert_eq!(added["params"]["tools"][0]["name"], "second");
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != "WebMCP.toolsAdded")
    );
}
