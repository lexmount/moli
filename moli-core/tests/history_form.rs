use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig};
use moli_test_support::FixtureServer;
use tokio::time::Duration;
use url::Url;

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

async fn child_form_history(phase: &str, method: &str, target: &str) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(BrowserConfig::default())?;
    let source = format!(
        r#"<!doctype html><body><script>
          function go() {{
            if (phase === 'reopen') document.open();
            const owner = phase === 'foreign' ? parent.document : document;
            const form = owner.createElement('form');
            form.action = '/net/echo';
            form.method = {};
            form.target = {};
            const input = owner.createElement('input');
            input.name = 'q'; input.value = 'submitted';
            form.append(input);
            (owner.body || owner.documentElement || owner).append(form);
            if (phase === 'same-url') history.replaceState(null, '',
              form.method === 'get' ? '/net/echo?q=submitted' : '/net/echo');
            navigation.addEventListener('navigate', e => {{
              parent.navigationType = e.navigationType;
              parent.formDataValue = e.formData && e.formData.get('q');
            }});
            parent.readiness = document.readyState;
            const oldURL = location.href;
            const oldLength = history.length;
            form.submit();
            parent.oldDocumentUnchanged = location.href === oldURL && history.length === oldLength;
          }}
          const phase = {};
          if (phase === 'parser' || phase === 'foreign') go();
          else if (phase === 'DOMContentLoaded') document.addEventListener(phase, go, {{once: true}});
          else if (phase === 'load' || phase === 'pageshow') addEventListener(phase, go, {{once: true}});
          else addEventListener('load', () => setTimeout(go, 0), {{once: true}});
        </script>"#,
        serde_json::to_string(method)?,
        serde_json::to_string(target)?,
        serde_json::to_string(phase)?
    );
    let parent = format!(
        r#"<!doctype html><body><script>
          window.finished = new Promise(resolve => window.finish = resolve);
          const initialLength = history.length;
          const frame = document.createElement('iframe');
          frame.name = 'form-frame';
          frame.onload = () => {{
            if (frame.contentWindow.location.pathname === '/net/echo') setTimeout(() => finish({{
              delta: history.length - initialLength,
              index: frame.contentWindow.navigation.currentEntry.index,
              navigationType, formDataValue, oldDocumentUnchanged, readiness,
              response: JSON.parse(frame.contentDocument.body.textContent)
            }}), 0);
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
    let result: serde_json::Value = serde_json::from_str(result["value"].as_str().unwrap())?;
    let replaces = !matches!(phase, "after-load" | "reopen" | "foreign");
    let delta = if replaces { 0 } else { 1 };
    assert_eq!(
        result["delta"], delta,
        "{phase}/{method}/{target}: {result}"
    );
    assert_eq!(
        result["index"], delta,
        "{phase}/{method}/{target}: {result}"
    );
    assert_eq!(
        result["navigationType"],
        if replaces { "replace" } else { "push" }
    );
    assert_eq!(result["oldDocumentUnchanged"], true);
    assert_eq!(result["response"]["method"], method.to_uppercase());
    assert_eq!(
        result["formDataValue"],
        if method == "post" {
            serde_json::json!("submitted")
        } else {
            serde_json::Value::Null
        }
    );
    assert_eq!(
        result["readiness"],
        match phase {
            "parser" | "reopen" | "foreign" => "loading",
            "DOMContentLoaded" => "interactive",
            _ => "complete",
        }
    );
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn self_form_navigation_replaces_until_load_callbacks_finish() -> Result<()> {
    for method in ["get", "post"] {
        for phase in ["parser", "DOMContentLoaded", "load", "pageshow"] {
            child_form_history(phase, method, "_self").await?;
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn named_self_form_navigation_uses_the_target_document_load_state() -> Result<()> {
    for method in ["get", "post"] {
        for phase in ["parser", "pageshow", "after-load"] {
            child_form_history(phase, method, "form-frame").await?;
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn self_form_navigation_pushes_after_load_even_after_document_open() -> Result<()> {
    for method in ["get", "post"] {
        for phase in ["after-load", "reopen"] {
            child_form_history(phase, method, "_self").await?;
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn form_navigation_auto_replaces_the_same_url_after_load() -> Result<()> {
    for method in ["get", "post"] {
        for target in ["_self", "form-frame"] {
            child_form_history("same-url", method, target).await?;
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn form_navigation_from_another_document_pushes_even_while_target_loads() -> Result<()> {
    for method in ["get", "post"] {
        child_form_history("foreign", method, "form-frame").await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_form_navigation_events_use_submission_time_load_state() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    for method in ["get", "post"] {
        for phase in ["parser", "load", "pageshow", "after-load"] {
            let browser = Browser::new(BrowserConfig::default())?;
            let markup = format!(
                r#"<!doctype html><body><form action='/net/echo' method='{}'>
                  <input name='q' value='submitted'></form><script>
                  window.finished = new Promise(resolve => window.finish = resolve);
                  navigation.onnavigate = e => {{
                    e.preventDefault();
                    finish({{type: e.navigationType, data: e.formData && e.formData.get('q')}});
                  }};
                  function go() {{ document.forms[0].submit(); }}
                  const phase = {};
                  if (phase === 'parser') go();
                  else if (phase === 'after-load') addEventListener('load', () => setTimeout(go, 0));
                  else addEventListener(phase, go);
                </script>"#,
                method,
                serde_json::to_string(phase)?
            );
            let result = tokio::time::timeout(Duration::from_secs(10), async {
                let mut page = browser.fetch(&markup_url(&server, &markup)).await?;
                page.evaluate_runtime_expression_with_await_async(
                    "finished.then(value => JSON.stringify(value))",
                    true,
                )
                .await
            })
            .await??;
            let result: serde_json::Value =
                serde_json::from_str(result["value"].as_str().unwrap())?;
            assert_eq!(
                result,
                serde_json::json!({
                    "type": if phase == "after-load" { "push" } else { "replace" },
                    "data": if method == "post" { Some("submitted") } else { None },
                }),
                "{phase}/{method}"
            );
        }
    }
    server.shutdown().await;
    Ok(())
}
