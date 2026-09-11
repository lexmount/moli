use super::*;

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

pub(super) async fn run_probe(
    browser: &Browser,
    server: &FixtureServer,
    target: &str,
    source: &str,
) -> Result<serde_json::Value> {
    let markup = if target == "worker" {
        let source = serde_json::to_string(&format!(
            "self.finish = value => postMessage(value);\n{source}"
        ))?;
        format!(
            r#"<!doctype html><script>
              const worker = new Worker(URL.createObjectURL(new Blob([{source}], {{type: 'text/javascript'}})));
              window.done = new Promise(resolve => {{
                worker.onmessage = event => {{ resolve(event.data); worker.terminate(); }};
              }});
              worker.postMessage('go');
            </script>"#
        )
    } else {
        format!(
            "<!doctype html><script>self.done = new Promise(resolve => self.finish = resolve);\n{source}</script>"
        )
    };
    let url = markup_url(server, &markup);
    let url = if target == "child" {
        markup_url(
            server,
            &format!(
                "<!doctype html><iframe id=target src=\"{}\"></iframe>",
                url.replace('&', "&amp;")
            ),
        )
    } else {
        url
    };
    let mut page = browser.fetch(&url).await?;
    let expression = if target == "child" {
        "document.getElementById('target').contentWindow.done.then(JSON.stringify)"
    } else {
        "done.then(JSON.stringify)"
    };
    let value = page
        .evaluate_runtime_expression_with_await_async(expression, true)
        .await?;
    Ok(serde_json::from_str(
        value["value"].as_str().expect("callback probe result"),
    )?)
}

#[tokio::test(flavor = "multi_thread")]
async fn native_message_events_keep_dispatch_state_and_clear_propagation_flags() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = r#"
      const log = [];
      addEventListener('message', event => {
        log.push(['during', event.currentTarget === self, event.eventPhase === 2,
          typeof document === 'undefined' || window.event === event]);
        event.initEvent('mutated', true, true);
        log.push(event.type);
        event.stopImmediatePropagation();
        setTimeout(() => {
          log.push(['after', event.currentTarget === null, event.eventPhase === 0,
            typeof document === 'undefined' || window.event === undefined]);
          dispatchEvent(event);
          finish(log);
        }, 0);
      }, {once: true});
      addEventListener('message', () => log.push('second'));
      if (typeof document !== 'undefined') postMessage('go', '*');
    "#;
    for target in ["window", "child", "worker"] {
        let observed = run_probe(&browser, &server, target, source).await?;
        assert_eq!(
            observed,
            serde_json::json!([
                ["during", true, true, true],
                "message",
                ["after", true, true, true],
                "second"
            ]),
            "target={target}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn native_message_listeners_observe_once_and_removal_during_nested_dispatch() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = r#"
      const log = [];
      let calls = 0;
      const removed = () => log.push('unexpected removed listener');
      addEventListener('message', () => {
        log.push('once');
        if (++calls > 1) return;
        removeEventListener('message', removed);
        dispatchEvent(new Event('message'));
      }, {once: true});
      addEventListener('message', removed);
      addEventListener('message', () => {
        log.push('retained');
        setTimeout(() => finish(log), 0);
      });
      if (typeof document !== 'undefined') postMessage('go', '*');
    "#;
    for target in ["window", "child", "worker"] {
        let observed = run_probe(&browser, &server, target, source).await?;
        assert_eq!(
            observed,
            serde_json::json!(["once", "retained", "retained"]),
            "target={target}"
        );
    }
    server.shutdown().await;
    Ok(())
}
