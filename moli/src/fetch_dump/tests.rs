use std::sync::Arc;

use anyhow::Result;
use axum::{Router, routing::get};
use moli_core::{
    LayoutPolicy,
    page::Page,
    runtime::{Browser, BrowserConfig},
};
use moli_renderer_v8::{
    RendererInspectorCommandEnvelope, RendererInspectorCommandRoute, RendererInspectorIngressTicket,
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
                async move { (*body).clone() }
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

// A renderer-only mutation is deliberate: dumps must read live state before
// the Browser observes this frozen inspection completion.
async fn mutate_renderer_without_page_observation(
    page: &Page,
    message: Value,
) -> Result<moli_core::page::CompletedPageCommand> {
    let route = page.renderer_inspection_endpoint().enqueue_main_command(
        RendererInspectorCommandEnvelope::new_main_protocol_on_page_owner(
            RendererInspectorIngressTicket::new(
                None,
                None,
                RendererInspectorCommandRoute::MainThread,
            ),
            None,
            message.to_string(),
            None,
        ),
    )?;
    moli_core::page::PendingPageCommand::from_inspector_main_route(route)
        .wait()
        .await
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

    let viewport = render_page_output_async(
        &mut page,
        &FetchCommandConfig {
            dump_mode: Some(DumpFormat::Screenshot),
            ..FetchCommandConfig::default()
        },
    )
    .await?;
    let full_page = render_page_output_async(
        &mut page,
        &FetchCommandConfig {
            dump_mode: Some(DumpFormat::ScreenshotFull),
            ..FetchCommandConfig::default()
        },
    )
    .await?;

    let viewport_dimensions = png_dimensions(&viewport);
    let full_page_dimensions = png_dimensions(&full_page);
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
    let completion = mutate_renderer_without_page_observation(&page, mutation).await?;

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

    page.observe_renderer_page_state(completion.page_state());
    let _ = completion.into_runtime_protocol_message_command_turn()?;
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
    let completion = mutate_renderer_without_page_observation(&page, mutation).await?;

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

    page.observe_renderer_page_state(completion.page_state());
    let _ = completion.into_runtime_protocol_message_command_turn()?;
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
    let completion = mutate_renderer_without_page_observation(&page, mutation).await?;

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

    page.observe_renderer_page_state(completion.page_state());
    let _ = completion.into_runtime_protocol_message_command_turn()?;
    http_server.abort();
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
    let completion = mutate_renderer_without_page_observation(&page, mutation).await?;

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

    page.observe_renderer_page_state(completion.page_state());
    let _ = completion.into_runtime_protocol_message_command_turn()?;
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
                libcurl_env_proxy_fallback: false,
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
    assert_eq!(
        payload["network"]["config"]["libcurl_env_proxy_fallback"],
        false
    );
    assert_eq!(payload["network"]["config"]["proxy_bearer_token"], true);
    assert_eq!(payload["network"]["config"]["connect_timeout_ms"], 2500);
    http_server.abort();
    Ok(())
}
