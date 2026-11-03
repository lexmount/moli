use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig};
use moli_test_support::FixtureServer;
use serde_json::{Value, json};
use tokio::time::Duration;
use url::Url;

#[tokio::test(flavor = "multi_thread")]
async fn parser_inline_and_connect_csp_violations_preserve_check_order() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    for (operations, expected) in [
        (
            "inline(); connect();",
            json!([
                ["script-src-elem", "SCRIPT", true, true, false],
                ["connect-src", "#document", true, true, false]
            ]),
        ),
        (
            "connect(); inline();",
            json!([
                ["connect-src", "#document", true, true, false],
                ["script-src-elem", "SCRIPT", true, true, false]
            ]),
        ),
    ] {
        let markup = r#"<!doctype html>
<meta http-equiv="Content-Security-Policy" content="script-src 'nonce-self'; connect-src 'none'">
<body><script nonce="self">
const seen = [];
let finish;
let duringCall = true;
window.cspFinished = new Promise(resolve => {finish = resolve;});
addEventListener('securitypolicyviolation', event => {
  seen.push([event.effectiveDirective, event.target.nodeName,
    event.isTrusted, event instanceof SecurityPolicyViolationEvent, duringCall]);
  if (seen.length === 2) finish(seen);
});
function inline() {
  const script = document.createElement('script');
  script.textContent = 'globalThis.inlineCspExecuted = true';
  document.body.appendChild(script);
}
function connect() {
  const socket = new WebSocket('ws://forbidden.invalid/');
  socket.onerror = () => {};
}
__CHECK_ORDER__
duringCall = false;
</script>"#
            .replace("__CHECK_ORDER__", operations);
        let mut url = Url::parse(&server.url("/compat/child-dynamic-markup-document"))?;
        url.query_pairs_mut().append_pair("markup", &markup);
        let browser = Browser::new(BrowserConfig::default())?;
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            let mut page = browser.fetch(url.as_str()).await?;
            page.evaluate_runtime_expression_with_await_async(
                "cspFinished.then(events => JSON.stringify({events, executed: globalThis.inlineCspExecuted === true}))",
                true,
            ).await
        }).await??;
        let observed: Value = serde_json::from_str(
            result["value"]
                .as_str()
                .expect("CSP reporting order result"),
        )?;
        assert_eq!(observed["events"], expected, "{operations}: {observed}");
        assert_eq!(observed["executed"], false, "{operations}: {observed}");
    }
    server.shutdown().await;
    Ok(())
}
