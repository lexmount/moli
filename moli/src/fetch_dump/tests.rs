use std::sync::Arc;

use anyhow::Result;
use axum::{Router, response::Html, routing::get};
use moli_core::{
    LayoutPolicy,
    page::Page,
    runtime::{Browser, BrowserConfig},
};
use serde_json::{Value, json};
use tokio::{net::TcpListener, task::JoinHandle};

use super::{
    render_page_dump_async, render_page_dump_with_options_async, render_page_output_async,
};
use crate::{
    cli::{DumpFormat, StripOptions},
    config::FetchCommandConfig,
};

async fn load_page(html: &str) -> Result<(Browser, Page, JoinHandle<()>)> {
    load_page_with_config(html, BrowserConfig::default()).await
}

async fn load_page_with_config(
    html: &str,
    config: BrowserConfig,
) -> Result<(Browser, Page, JoinHandle<()>)> {
    let browser = Browser::new(config)?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let body = Arc::new(html.to_owned());
    let server_body = Arc::clone(&body);
    let http_server = tokio::spawn(async move {
        let app = Router::new().route(
            "/",
            get(move || {
                let body = Arc::clone(&server_body);
                async move { Html((*body).clone()) }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });
    let page = browser.fetch(&format!("http://{addr}/")).await?;
    Ok((browser, page, http_server))
}

fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(&bytes[12..16], b"IHDR");
    (
        u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
        u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
    )
}

#[tokio::test]
async fn render_full_page_screenshot_extends_beyond_viewport() -> Result<()> {
    let config = BrowserConfig::default().with_layout_policy(LayoutPolicy::OnDemand);
    let (_browser, mut page, http_server) = load_page_with_config(
        concat!(
            "<!doctype html><style>html,body{margin:0}",
            "main{height:1300px;background:linear-gradient(red,blue)}</style>",
            "<main></main>",
        ),
        config,
    )
    .await?;

    // Full-document output must work before any other capture publishes layout.
    let full_page = render_page_output_async(
        &mut page,
        &FetchCommandConfig {
            dump_mode: Some(DumpFormat::ScreenshotFull),
            ..FetchCommandConfig::default()
        },
    )
    .await?;
    let viewport = render_page_output_async(
        &mut page,
        &FetchCommandConfig {
            dump_mode: Some(DumpFormat::Screenshot),
            ..FetchCommandConfig::default()
        },
    )
    .await?;

    let viewport_dimensions = png_dimensions(&viewport);
    let full_page_dimensions = png_dimensions(&full_page);
    assert_eq!(full_page_dimensions.1, 1300);
    assert_eq!(full_page_dimensions.0, viewport_dimensions.0);
    assert!(full_page_dimensions.1 > viewport_dimensions.1);

    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn render_page_dump_default_html_uses_renderer_live_serialize() -> Result<()> {
    let (_browser, mut page, http_server) =
        load_page(r#"<!doctype html><html><body><main id="target">old</main></body></html>"#)
            .await?;

    let mutation = json!({
        "id": 17,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.getElementById('target').textContent = 'live'; 'done';",
            "returnByValue": true
        }
    });
    let pending = page.start_runtime_protocol_message(serde_json::to_string(&mutation)?)?;
    let completion = pending.wait().await?;

    let rendered = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::Html,
        StripOptions::default(),
        false,
        false,
        false,
        None,
    )
    .await?;

    assert!(rendered.contains(r#"<main id="target">live</main>"#));
    assert!(!rendered.contains(r#"<main id="target">old</main>"#));

    let _ = page.finish_runtime_protocol_message(completion)?;
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn strip_ui_keeps_content_inside_details() -> Result<()> {
    let (_browser, mut page, http_server) = load_page(
        "<!doctype html><html><body><nav><details><summary>Menu</summary><p>Navigation entry</p></details></nav><main><details><summary>Product details</summary><p>Critical product description</p><button>Buy now</button></details><details open><summary>Specifications</summary><p>Material: cotton</p></details></main></body></html>",
    )
    .await?;
    let rendered = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::Markdown,
        StripOptions {
            js: false,
            css: false,
            ui: true,
        },
        false,
        false,
        false,
        None,
    )
    .await?;
    assert!(!rendered.contains("Menu"));
    assert!(!rendered.contains("Navigation entry"));
    assert!(!rendered.contains("Buy now"));
    assert!(rendered.contains("Product details"));
    assert!(rendered.contains("Critical product description"));
    assert!(rendered.contains("Specifications"));
    assert!(rendered.contains("Material: cotton"));
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn render_page_dump_postprocessed_html_uses_renderer_live_dump() -> Result<()> {
    let (_browser, mut page, http_server) = load_page(
            r#"<!doctype html><html><body><script>window.old=true;</script><main id="target" style="color:red" onclick="old()">old</main></body></html>"#,
        )
        .await?;

    let mutation = json!({
        "id": 19,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.getElementById('target').textContent = 'live'; 'done';",
            "returnByValue": true
        }
    });
    let pending = page.start_runtime_protocol_message(serde_json::to_string(&mutation)?)?;
    let completion = pending.wait().await?;

    let rendered = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::Html,
        StripOptions {
            js: true,
            css: true,
            ui: false,
        },
        true,
        false,
        false,
        None,
    )
    .await?;

    assert!(rendered.contains(r#"<main id="target">live</main>"#));
    assert!(rendered.contains("<base href="));
    assert!(
        !rendered.contains(r#"<main id="target" style="color:red" onclick="old()">old</main>"#)
    );
    assert!(!rendered.contains("<script>"));
    assert!(!rendered.contains("onclick="));
    assert!(!rendered.contains("style="));

    let _ = page.finish_runtime_protocol_message(completion)?;
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn render_markdown_dump_uses_renderer_live_dump() -> Result<()> {
    let (_browser, mut page, http_server) =
        load_page(r#"<!doctype html><html><body><main id="target">old</main></body></html>"#)
            .await?;

    let mutation = json!({
        "id": 20,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.getElementById('target').textContent = 'live'; 'done';",
            "returnByValue": true
        }
    });
    let pending = page.start_runtime_protocol_message(serde_json::to_string(&mutation)?)?;
    let completion = pending.wait().await?;

    let rendered = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::Markdown,
        StripOptions::default(),
        false,
        false,
        false,
        None,
    )
    .await?;

    assert_eq!(rendered, "live");
    assert!(!rendered.contains("old"));

    let _ = page.finish_runtime_protocol_message(completion)?;
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn markdown_dump_reads_dom_after_explicit_layout_observer_update() -> Result<()> {
    let config = BrowserConfig::default().with_layout_policy(LayoutPolicy::OnDemand);
    let (_browser, mut page, http_server) = load_page_with_config(
        r#"<!doctype html><main id="viewport" style="width:520px;height:80px"></main><script>
            const viewport = document.getElementById('viewport');
            const rows = [
                ['1', 'Ada', '31', 'London', 'active'],
                ['2', 'Grace', '28', 'New York', 'active'],
                ['3', 'Linus', '35', 'Helsinki', 'away'],
                ['4', 'Margaret', '33', 'Toronto', 'active'],
            ];
            new ResizeObserver(([entry]) => {
                const columns = entry.contentRect.width >= 500
                    ? ['ID', 'Name', 'Age', 'City', 'Status']
                    : ['ID', 'Name'];
                viewport.replaceChildren();
                const header = document.createElement('div');
                header.textContent = columns.join(' | ');
                viewport.appendChild(header);
                for (const row of rows) {
                    const item = document.createElement('div');
                    item.textContent = row.slice(0, columns.length).join(' | ');
                    viewport.appendChild(item);
                }
            }).observe(viewport);
        </script>"#,
        config,
    )
    .await?;

    // Layout is an explicit operation. Once its ResizeObserver callback has
    // updated the live DOM, Markdown reads that DOM without publishing layout.
    page.publish_layout_async().await?;
    page.evaluate_runtime_expression_with_await_async(
        "new Promise((resolve,reject)=>{const end=Date.now()+2000;const poll=()=>document.getElementById('viewport').childElementCount===5?resolve():Date.now()>end?reject(new Error('layout-driven DOM update missing')):setTimeout(poll,1);poll()})",
        true,
    )
    .await?;
    assert_eq!(
        page.evaluate_runtime_expression_async(
            "document.getElementById('viewport').textContent.includes('Ada')",
        )
        .await?["value"],
        true
    );

    let rendered = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::Markdown,
        StripOptions::default(),
        false,
        false,
        false,
        None,
    )
    .await?;

    assert!(
        rendered.contains("ID | Name | Age | City | Status"),
        "{rendered}"
    );
    assert!(
        rendered.contains("1 | Ada | 31 | London | active"),
        "{rendered}"
    );
    assert!(
        rendered.contains("4 | Margaret | 33 | Toronto | active"),
        "{rendered}"
    );
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn fresh_layout_resize_observer_updates_virtual_children_and_deduplicates_sizes() -> Result<()>
{
    let config = BrowserConfig::default().with_layout_policy(LayoutPolicy::OnDemand);
    let (_browser, mut page, server) = load_page_with_config(r#"<div id="box" style="width:120px;height:40px"></div><script>
        window.widths=[];const box=document.getElementById('box');
        new ResizeObserver(es=>{const width=es[0].contentRect.width;widths.push(width);if(width>300&&!box.querySelector('span')){const item=document.createElement('span');item.id='virtual-item';item.textContent='Virtual child';box.appendChild(item)}}).observe(box);
    </script>"#, config).await?;
    let screenshot = FetchCommandConfig {
        dump_mode: Some(DumpFormat::Screenshot),
        ..Default::default()
    };
    render_page_output_async(&mut page, &screenshot).await?;
    page.evaluate_runtime_expression_with_await_async("new Promise((resolve,reject)=>{const end=Date.now()+2000;const poll=()=>widths.includes(120)?resolve():Date.now()>end?reject(new Error('initial resize notification missing')):setTimeout(poll,1);poll()})", true).await?;
    page.evaluate_runtime_expression_async("document.getElementById('box').style.width='420px'")
        .await?;
    render_page_output_async(&mut page, &screenshot).await?;
    page.evaluate_runtime_expression_with_await_async("new Promise((resolve,reject)=>{const end=Date.now()+2000;const poll=()=>document.getElementById('virtual-item')?resolve():Date.now()>end?reject(new Error('updated resize notification missing')):setTimeout(poll,1);poll()})", true).await?;
    let widths = page
        .evaluate_runtime_expression_async("JSON.stringify(widths)")
        .await?;
    // Real-layout observers receive the geometry produced by rendering
    // updates. The non-layout fallback must not race them with a zero-size
    // callback from the earlier DOM-only checkpoint path.
    assert_eq!(widths["value"], "[120,420]");
    render_page_output_async(&mut page, &screenshot).await?;
    assert_eq!(
        page.evaluate_runtime_expression_async("JSON.stringify(widths)")
            .await?["value"],
        "[120,420]"
    );
    server.abort();
    Ok(())
}

#[tokio::test]
async fn resize_observer_same_depth_feedback_is_bounded() -> Result<()> {
    let config = BrowserConfig::default().with_layout_policy(LayoutPolicy::OnDemand);
    let (_browser, mut page, server) = load_page_with_config(r#"<div id="box" style="width:120px;height:20px"></div><script>window.calls=0;window.recoveryCalls=0;window.feedback=false;window.errors=[];window.onerrorCalls=0;const box=document.getElementById('box');window.observer=new ResizeObserver(es=>{calls++;if(feedback)box.style.width=(es[0].contentRect.width+1)+'px'});addEventListener('error',e=>{errors.push({message:e.message,instance:e instanceof ErrorEvent,target:e.target===window,currentTarget:e.currentTarget===window,trusted:e.isTrusted,bubbles:e.bubbles,cancelable:e.cancelable,error:e.error,filename:e.filename,lineno:e.lineno,colno:e.colno});observer.disconnect();window.recoveryObserver=new ResizeObserver(()=>recoveryCalls++);recoveryObserver.observe(box);e.preventDefault()});window.onerror=()=>{onerrorCalls++;return true};observer.observe(box)</script>"#, config).await?;
    render_page_output_async(
        &mut page,
        &FetchCommandConfig {
            dump_mode: Some(DumpFormat::Screenshot),
            ..Default::default()
        },
    )
    .await?;
    page.evaluate_runtime_expression_async(
        "calls=0;feedback=true;document.getElementById('box').style.width='140px'",
    )
    .await?;
    page.evaluate_runtime_expression_with_await_async(
        "new Promise((resolve,reject)=>{const end=Date.now()+2000;const poll=()=>calls&&recoveryCalls?resolve():Date.now()>end?reject(new Error('resize recovery callback missing')):setTimeout(poll,1);poll()})",
        true,
    )
    .await?;
    page.evaluate_runtime_expression_async("document.getElementById('box').style.width='200px'")
        .await?;
    render_page_output_async(
        &mut page,
        &FetchCommandConfig {
            dump_mode: Some(DumpFormat::Screenshot),
            ..Default::default()
        },
    )
    .await?;
    page.evaluate_runtime_expression_with_await_async(
        "new Promise((resolve,reject)=>{const end=Date.now()+2000;const poll=()=>recoveryCalls>=2?resolve():Date.now()>end?reject(new Error('second resize recovery callback missing')):setTimeout(poll,1);poll()})",
        true,
    )
    .await?;
    assert_eq!(
        page.evaluate_runtime_expression_async("String(calls)")
            .await?["value"],
        "1"
    );
    let result = page
        .evaluate_runtime_expression_async("JSON.stringify({errors,onerrorCalls,recoveryCalls})")
        .await?;
    let result: Value = serde_json::from_str(result["value"].as_str().unwrap())?;
    assert_eq!(
        result,
        json!({"errors":[{"message":"ResizeObserver loop completed with undelivered notifications.","instance":true,"target":true,"currentTarget":true,"trusted":true,"bubbles":false,"cancelable":true,"error":null,"filename":"","lineno":0,"colno":0}],"onerrorCalls":1,"recoveryCalls":2})
    );
    server.abort();
    Ok(())
}

#[tokio::test]
async fn render_semantic_tree_dump_uses_renderer_live_accessibility_tree() -> Result<()> {
    let (_browser, mut page, http_server) =
        load_page(r#"<!doctype html><html><body><button id="target">old</button></body></html>"#)
            .await?;

    let mutation = json!({
        "id": 18,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.getElementById('target').textContent = 'live'; 'done';",
            "returnByValue": true
        }
    });
    let pending = page.start_runtime_protocol_message(serde_json::to_string(&mutation)?)?;
    let completion = pending.wait().await?;

    let rendered = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::SemanticTree,
        StripOptions::default(),
        false,
        false,
        false,
        None,
    )
    .await?;

    assert!(rendered.contains(r#""value": "live""#));
    assert!(!rendered.contains(r#""value": "old""#));

    let _ = page.finish_runtime_protocol_message(completion)?;
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn semantic_dump_preserves_protocol_json_bytes_and_text_fields() -> Result<()> {
    let (_browser, mut page, http_server) = load_page(
        r#"<!doctype html><html><head><title>AX serialization</title></head><body>
          <ul><li>List item</li></ul>
          <input aria-label="Text field" value="field value">
          <input type="range" aria-label="Numeric field" value="42">
          <button disabled>Disabled action</button>
          <div aria-hidden="true"><span>Ignored text</span></div>
          <div hidden>Not rendered</div>
        </body></html>"#,
    )
    .await?;
    let protocol = page
        .accessibility_tree_payloads_for_document_async(None)
        .await?;
    let json = super::semantic::render_json(&mut page, false).await?;
    assert_eq!(json, serde_json::to_string_pretty(&protocol)?);
    let nodes: Vec<serde_json::Value> = serde_json::from_str(&json)?;
    let field_names = |node: &serde_json::Value| {
        node.as_object()
            .expect("AX object")
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        field_names(&nodes[0]),
        [
            "nodeId",
            "backendDOMNodeId",
            "ignored",
            "role",
            "name",
            "properties",
            "childIds",
        ]
    );
    let marker = nodes
        .iter()
        .find(|node| node["role"]["value"] == "ListMarker")
        .expect("list marker");
    assert_eq!(
        field_names(marker),
        [
            "nodeId",
            "ignored",
            "role",
            "name",
            "properties",
            "parentId",
            "childIds",
        ]
    );
    let text = super::semantic::render_text(&mut page, false).await?;
    assert!(text.contains("textbox: Text field = field value [backendNodeId="));
    assert!(text.contains("textbox: Numeric field = 42 [backendNodeId="));
    assert!(
        !text.contains("ListMarker"),
        "markers have no child reference in the protocol tree"
    );
    assert!(!text.contains("Not rendered"));
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn render_semantic_tree_dumps_include_child_frames_only_when_requested() -> Result<()> {
    let (_browser, mut page, http_server) = load_page(
            r#"<!doctype html><html><body><iframe srcdoc="<button aria-label='Child action'>inside</button>"></iframe></body></html>"#,
        )
        .await?;

    let without_frames = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::SemanticTree,
        StripOptions::default(),
        false,
        false,
        false,
        None,
    )
    .await?;
    let with_frames = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::SemanticTree,
        StripOptions::default(),
        false,
        true,
        false,
        None,
    )
    .await?;

    assert!(!without_frames.contains("Child action"));
    assert!(with_frames.contains("Child action"));

    let payloads: Vec<Value> = serde_json::from_str(&with_frames)?;
    let child_root = payloads
        .iter()
        .filter(|payload| payload["role"]["value"] == "RootWebArea")
        .nth(1)
        .expect("child frame RootWebArea");
    assert_eq!(
        child_root
            .as_object()
            .expect("child root object")
            .keys()
            .next_back()
            .map(String::as_str),
        Some("parentId"),
        "frame attachment must preserve the existing JSON field order"
    );
    let child_root_id = child_root["nodeId"].as_str().expect("child root nodeId");
    let owner_id = child_root["parentId"]
        .as_str()
        .expect("child root should be attached to its iframe owner");
    let owner = payloads
        .iter()
        .find(|payload| payload["nodeId"] == owner_id)
        .expect("iframe owner AX node");
    assert_eq!(owner["role"]["value"], "Iframe");
    assert!(
        owner["childIds"]
            .as_array()
            .expect("iframe childIds")
            .iter()
            .any(|child_id| child_id == child_root_id)
    );

    let text_without_frames = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::SemanticTreeText,
        StripOptions::default(),
        false,
        false,
        false,
        None,
    )
    .await?;
    let text_with_frames = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::SemanticTreeText,
        StripOptions::default(),
        false,
        true,
        false,
        None,
    )
    .await?;

    assert!(!text_without_frames.contains("Child action"));
    assert!(text_with_frames.contains("Child action"));

    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn render_semantic_tree_with_frames_recurses_into_nested_frames() -> Result<()> {
    let (_browser, mut page, http_server) = load_page(
            r#"<!doctype html><html><body><iframe srcdoc="<iframe srcdoc='&lt;button aria-label=&quot;Nested action&quot;&gt;inside&lt;/button&gt;'></iframe>"></iframe></body></html>"#,
        )
        .await?;

    let rendered = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::SemanticTreeText,
        StripOptions::default(),
        false,
        true,
        false,
        None,
    )
    .await?;

    assert!(rendered.contains("Nested action"));
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn render_page_dump_with_options_async_inlines_child_frames() -> Result<()> {
    let (_browser, mut page, http_server) = load_page(
            r#"<!doctype html><html><body><iframe id="child" srcdoc="<p>child frame</p>"></iframe></body></html>"#,
        )
        .await?;

    let rendered = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::Html,
        StripOptions::default(),
        false,
        true,
        false,
        None,
    )
    .await?;

    assert!(rendered.contains("data-moli-frame-url="));
    assert!(rendered.contains("child frame"));
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn render_page_dump_async_includes_network_trace_config_summary() -> Result<()> {
    let (_browser, mut page, http_server) =
        load_page(r#"<!doctype html><html><body>ok</body></html>"#).await?;

    let rendered = render_page_dump_async(
        &mut page,
        &FetchCommandConfig {
            dump_mode: Some(DumpFormat::Json),
            trace_network: true,
            network_trace_config: Some(crate::network_trace::NetworkTraceConfigSummary {
                explicit_http_proxy: true,
                env_proxy_fallback: false,
                http_no_proxy: true,
                proxy_bearer_token: true,
                tls_verify_host: true,
                obey_robots: false,
                http_cache: false,
                connect_timeout_ms: Some(2500),
                request_timeout_ms: 5000,
                max_concurrent: Some(16),
                max_host_open: Some(4),
                max_host_connections: Some(6),
                effective_max_host_connections: Some(6),
                max_total_connections: Some(64),
                http2_max_concurrent_streams: Some(100),
                max_response_size: Some(1024),
                block_private_networks: false,
                block_cidr_count: 0,
            }),
            ..FetchCommandConfig::default()
        },
    )
    .await?;
    let payload: Value = serde_json::from_str(&rendered)?;

    assert_eq!(payload["network"]["config"]["explicit_http_proxy"], true);
    assert_eq!(payload["network"]["config"]["env_proxy_fallback"], false);
    assert_eq!(payload["network"]["config"]["proxy_bearer_token"], true);
    assert_eq!(payload["network"]["config"]["connect_timeout_ms"], 2500);
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn resize_observer_registered_in_child_during_parent_delivery_receives_initial_size()
-> Result<()> {
    let config = BrowserConfig::default().with_layout_policy(LayoutPolicy::OnDemand);
    let (_browser, mut page, server) = load_page_with_config(r#"<div id="box" style="width:120px;height:20px"></div><iframe id="child" srcdoc="&lt;div id='target' style='width:80px;height:20px'&gt;&lt;/div&gt;"></iframe>"#, config).await?;
    page.publish_layout_async().await?;
    page.evaluate_runtime_expression_async(r#"window.parentCalls=0;window.childWidths=[];new ResizeObserver(()=>{if(++parentCalls===1){const w=document.getElementById('child').contentWindow;window.childObserver=new w.ResizeObserver(es=>childWidths.push(es[0].contentRect.width));childObserver.observe(w.document.getElementById('target'));}}).observe(document.getElementById('box'))"#).await?;
    page.evaluate_runtime_expression_with_await_async(
        "new Promise((resolve,reject)=>{const end=Date.now()+2000;const poll=()=>childWidths.length?resolve():Date.now()>end?reject(new Error('child resize notification missing')):setTimeout(poll,1);poll()})",
        true,
    )
    .await?;
    let result = page
        .evaluate_runtime_expression_async("JSON.stringify({parentCalls,childWidths})")
        .await?;
    server.abort();
    assert_eq!(result["value"], r#"{"parentCalls":1,"childWidths":[80]}"#);
    Ok(())
}
