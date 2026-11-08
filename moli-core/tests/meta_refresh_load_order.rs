use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig, RenderedDomWaitUntil};
use moli_test_support::FixtureServer;
use std::time::Duration;
use url::Url;

#[tokio::test]
async fn zero_delay_child_meta_refresh_preserves_initial_owner_load_url() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let child_markup = r##"<!doctype html><meta charset="utf-8">
<section id="section">section</section>
<script>
addEventListener('load', () => parent.messages.push('window:' + location.hash));
addEventListener('hashchange', () => parent.messages.push('hash:' + location.hash));
const meta = document.createElement('meta');
meta.httpEquiv = 'refresh';
meta.content = '0;url=#section';
document.documentElement.appendChild(meta);
</script>"##;
    let mut child = Url::parse(&server.url("/compat/child-dynamic-markup-document"))?;
    child.query_pairs_mut().append_pair("markup", child_markup);
    let child_json = serde_json::to_string(child.as_str())?;
    let parent_markup = format!(
        r#"<!doctype html><meta charset="utf-8"><body><script>
globalThis.messages = [];
const frame = document.createElement('iframe');
frame.onload = () => {{
  messages.push('owner:' + frame.contentWindow.location.hash);
}};
frame.src = {child_json};
document.body.appendChild(frame);
</script>"#,
    );
    let mut parent = Url::parse(&server.url("/compat/child-dynamic-markup-document"))?;
    parent
        .query_pairs_mut()
        .append_pair("markup", &parent_markup);
    let browser = Browser::new(BrowserConfig::default())?;
    let mut page = browser
        .fetch_with_wait_until(
            parent.as_str(),
            RenderedDomWaitUntil::DomContentLoaded,
            Duration::from_secs(3),
        )
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "messages.includes('hash:#section')",
            Duration::from_secs(3),
        )
        .await?;
    let state = page
        .evaluate_runtime_expression_async("JSON.stringify(messages)")
        .await?;
    assert_eq!(
        state.get("value").and_then(serde_json::Value::as_str),
        Some(r#"["window:","owner:","hash:#section"]"#),
        "first iframe load must precede its zero-delay refresh: {state}",
    );
    server.shutdown().await;
    Ok(())
}
