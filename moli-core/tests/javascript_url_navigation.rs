use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig};
use moli_test_support::FixtureServer;
use serde_json::Value;
use tokio::time::Duration;
use url::Url;

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

async fn child_javascript_url_navigation(action: &str, other_target: bool) -> Result<Value> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let source = format!(
        r#"<!doctype html><body><form action='/net/echo'>
          <input name='q' value='submitted'></form>
          <a id='go' href='javascript:run()'>go</a><script>
          function run() {{
            {action}
            parent.scriptContinued = true;
            return '<body id="completion">replacement';
          }}
        </script>"#
    );
    let parent = format!(
        r#"<!doctype html><body><script>
          let finish;
          const loaded = new Promise(resolve => finish = resolve);
          let otherLoaded = Promise.resolve(null);
          if ({other_target}) {{
            const other = document.createElement('iframe');
            other.name = 'other';
            otherLoaded = new Promise(resolve => {{
              other.onload = () => {{
                if (other.contentWindow.location.pathname === '/net/echo')
                  resolve(JSON.parse(other.contentDocument.body.textContent));
              }};
            }});
            document.body.append(other);
          }}
          window.finished = Promise.all([loaded, otherLoaded]);
          const frame = document.createElement('iframe');
          frame.name = 'source';
          frame.onload = () => {{
            const originalDocument = frame.contentDocument;
            frame.onload = () => finish({{
              path: frame.contentWindow.location.pathname,
              search: frame.contentWindow.location.search,
              body: frame.contentDocument.body.textContent,
              bodyId: frame.contentDocument.body.id,
              documentChanged: frame.contentDocument !== originalDocument,
              scriptContinued
            }});
            setTimeout(() => frame.contentDocument.getElementById('go').click(), 0);
          }};
          frame.src = {};
          document.body.append(frame);
        </script>"#,
        serde_json::to_string(&markup_url(&server, &source))?.replace("</script>", "<\\/script>")
    );
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let mut page = browser.fetch(&markup_url(&server, &parent)).await?;
        page.evaluate_runtime_expression_with_await_async(
            "finished.then(value => JSON.stringify(value))",
            true,
        )
        .await
    })
    .await??;
    let result: Value = serde_json::from_str(result["value"].as_str().unwrap())?;
    server.shutdown().await;
    assert_eq!(result[0]["documentChanged"], true, "{action}: {result}");
    assert_eq!(result[0]["scriptContinued"], true, "{action}: {result}");
    Ok(result)
}

#[tokio::test(flavor = "multi_thread")]
async fn child_javascript_url_string_does_not_overtake_its_form_submission() -> Result<()> {
    for method in ["get", "post"] {
        for target in ["", "_self", "source"] {
            let action = format!(
                "document.forms[0].method = {method:?}; \
                 document.forms[0].target = {target:?}; document.forms[0].submit();"
            );
            let result = child_javascript_url_navigation(&action, false).await?;
            // Observe the first load after the javascript URL. A temporary
            // replacement document would fire an extra load at the old URL.
            assert_eq!(result[0]["path"], "/net/echo", "{action}: {result}");
            let response: Value = serde_json::from_str(result[0]["body"].as_str().unwrap())?;
            assert_eq!(response["method"], method.to_uppercase());
            if method == "post" {
                assert_eq!(result[0]["search"], "");
            } else {
                assert_eq!(result[0]["search"], "?q=submitted");
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_javascript_url_string_does_not_overtake_its_location_navigation() -> Result<()> {
    let result =
        child_javascript_url_navigation("location.href = '/net/echo?q=location';", false).await?;
    assert_eq!(result[0]["path"], "/net/echo", "{result}");
    assert_eq!(result[0]["search"], "?q=location");

    let result = child_javascript_url_navigation(
        "location.href = \"javascript:'<body id=successor>successor'\";",
        false,
    )
    .await?;
    assert_eq!(result[0]["bodyId"], "successor", "{result}");
    assert_eq!(result[0]["body"], "successor");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_javascript_url_string_survives_navigation_to_another_frame() -> Result<()> {
    for method in ["get", "post"] {
        let action = format!(
            "document.forms[0].method = {method:?}; \
             document.forms[0].target = 'other'; document.forms[0].submit();"
        );
        let result = child_javascript_url_navigation(&action, true).await?;
        assert_eq!(result[0]["bodyId"], "completion", "{action}: {result}");
        assert_eq!(result[0]["body"], "replacement");
        assert_eq!(result[1]["method"], method.to_uppercase());
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_javascript_url_string_survives_without_a_new_cross_document_navigation() -> Result<()>
{
    for action in [
        "document.body.dataset.changed = 'yes';",
        "location.hash = 'fragment';",
        "navigation.onnavigate = event => event.preventDefault(); document.forms[0].submit();",
    ] {
        let result = child_javascript_url_navigation(action, false).await?;
        assert_eq!(result[0]["bodyId"], "completion", "{action}: {result}");
        assert_eq!(result[0]["body"], "replacement");
    }
    Ok(())
}
