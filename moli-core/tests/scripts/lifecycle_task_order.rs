use super::*;

async fn observe_lifecycle_markup(child: bool, markup: &str) -> Result<Vec<String>> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut target = url::Url::parse(&server.url("/compat/child-dynamic-markup-document"))?;
    target.query_pairs_mut().append_pair("markup", markup);
    if child {
        let parent = format!(
            "<!doctype html><iframe id=target src=\"{}\"></iframe>",
            target.as_str().replace('&', "&amp;")
        );
        target.set_query(None);
        target.query_pairs_mut().append_pair("markup", &parent);
    }
    let mut page = browser.fetch(target.as_str()).await?;
    let result = page
        .evaluate_runtime_expression_with_await_async(
            "JSON.stringify((document.getElementById('target')?.contentWindow || window).events)",
            true,
        )
        .await?;
    server.shutdown().await;
    Ok(serde_json::from_str(
        result["value"].as_str().expect("lifecycle observations"),
    )?)
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_dom_tasks_precede_lifecycle_events() -> Result<()> {
    let markup = r#"<!doctype html><details id=earlier></details><script>
      window.events = [];
      document.addEventListener('readystatechange', () => events.push(document.readyState));
      document.addEventListener('DOMContentLoaded', () => events.push('DOMContentLoaded'));
      addEventListener('load', () => events.push('load'));
      earlier.addEventListener('toggle', () => events.push('earlier DOM task'));
      earlier.open = true;
    </script><p>parser tail</p>"#;
    for child in [false, true] {
        let result = observe_lifecycle_markup(child, markup).await?;
        assert_eq!(
            result,
            [
                "interactive",
                "earlier DOM task",
                "DOMContentLoaded",
                "complete",
                "load"
            ],
            "child={child}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn deferred_scripts_keep_dom_task_positions_around_domcontentloaded() -> Result<()> {
    for attribute in ["defer", "type=module"] {
        let markup = format!(
            r#"<!doctype html><details id=earlier></details><details id=later></details>
    <script>
      window.events = [];
      document.addEventListener('DOMContentLoaded', () => events.push('DOMContentLoaded'));
      addEventListener('load', () => events.push('load'));
      earlier.addEventListener('toggle', () => {{
        events.push('earlier DOM task');
        later.open = true;
      }});
      later.addEventListener('toggle', () => events.push('later DOM task'));
    </script><script {attribute} src="data:text/javascript,document.getElementById('earlier').open=true"></script>
    <p>parser tail</p>"#,
        );
        for child in [false, true] {
            let events = observe_lifecycle_markup(child, &markup).await?;
            let at = |name: &str| events.iter().position(|event| event == name).unwrap();
            assert!(
                at("earlier DOM task") < at("DOMContentLoaded"),
                "{child}, {events:?}"
            );
            assert!(
                at("DOMContentLoaded") < at("later DOM task"),
                "{child}, {events:?}"
            );
            assert!(at("later DOM task") < at("load"), "{child}, {events:?}");
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn earlier_dom_task_retires_queued_domcontentloaded() -> Result<()> {
    let markup = r#"<!doctype html><details id=earlier></details><script>
      window.events = [];
      document.addEventListener('DOMContentLoaded', () => events.push('old DCL'));
      earlier.addEventListener('toggle', () => {
        document.open();
        document.write("<!doctype html><script>document.addEventListener('DOMContentLoaded', () => events.push('new DCL'))<\/script><p>replacement</p>");
        document.close();
      }, {once: true});
      earlier.open = true;
    </script><p>old parser tail</p>"#;
    for child in [false, true] {
        assert_eq!(
            observe_lifecycle_markup(child, markup).await?,
            ["new DCL"],
            "child={child}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn earlier_dom_task_can_add_a_document_load_delay() -> Result<()> {
    let markup = r#"<!doctype html><details id=earlier></details><script>
      window.events = [];
      document.addEventListener('DOMContentLoaded', () => {
        events.push('DOMContentLoaded');
        earlier.open = true;
      });
      earlier.addEventListener('toggle', () => {
        events.push('earlier DOM task');
        const script = document.createElement('script');
        script.async = true;
        script.src = 'data:text/javascript,' + encodeURIComponent("events.push('async')");
        document.body.append(script);
      });
      addEventListener('load', () => events.push('load'));
    </script><p>parser tail</p>"#;
    for child in [false, true] {
        assert_eq!(
            observe_lifecycle_markup(child, markup).await?,
            ["DOMContentLoaded", "earlier DOM task", "async", "load"],
            "child={child}"
        );
    }
    Ok(())
}
