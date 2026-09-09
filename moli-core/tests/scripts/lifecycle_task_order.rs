use super::*;

async fn parser_rejection_notification_precedes_lifecycle_events(child: bool) -> Result<()> {
    let markup = r#"<!doctype html><body><script>
      window.events = [];
      document.addEventListener('readystatechange', () => events.push(document.readyState));
      document.addEventListener('DOMContentLoaded', () => events.push('DOMContentLoaded'));
      addEventListener('load', () => events.push('load'));
      addEventListener('unhandledrejection', event => {
        events.push(event.reason);
        event.preventDefault();
      });
      Promise.reject('rejection');
    </script><p id=tail>parser tail</p>"#;
    let result = observe_lifecycle_markup(child, markup).await?;
    assert_eq!(
        result["value"].as_str(),
        Some(r#"["interactive","rejection","DOMContentLoaded","complete","load"]"#),
        "child={child}"
    );
    Ok(())
}

async fn observe_lifecycle_markup(child: bool, markup: &str) -> Result<serde_json::Value> {
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
    Ok(result)
}

#[tokio::test(flavor = "multi_thread")]
async fn main_parser_rejection_notification_precedes_lifecycle_events() -> Result<()> {
    parser_rejection_notification_precedes_lifecycle_events(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_rejection_notification_precedes_lifecycle_events() -> Result<()> {
    parser_rejection_notification_precedes_lifecycle_events(true).await
}

async fn deferred_rejection_keeps_dom_fifo_position(child: bool, module: bool) -> Result<()> {
    let attribute = if module { "type=module" } else { "defer" };
    let markup = format!(
        r#"<!doctype html><body><script>
      window.events = [];
      document.addEventListener('readystatechange', () => events.push(document.readyState));
      document.addEventListener('DOMContentLoaded', () => events.push('DOMContentLoaded'));
      addEventListener('load', () => events.push('load'));
      addEventListener('unhandledrejection', event => {{
        event.preventDefault();
        events.push(event.reason);
        const later = document.createElement('script');
        later.src = '';
        later.onerror = () => events.push('later DOM task');
        document.body.append(later);
      }});
    </script><script {attribute} src="data:text/javascript,Promise.reject('rejection')"></script>
    <p id=tail>parser tail</p>"#
    );
    let result = observe_lifecycle_markup(child, &markup).await?;
    let events: Vec<String> = serde_json::from_str(result["value"].as_str().unwrap())?;
    let at = |name: &str| events.iter().position(|event| event == name).unwrap();
    assert!(at("interactive") < at("rejection"), "{events:?}");
    assert!(at("rejection") < at("DOMContentLoaded"), "{events:?}");
    assert!(at("DOMContentLoaded") < at("later DOM task"), "{events:?}");
    assert!(at("rejection") < at("complete"), "{events:?}");
    assert!(at("complete") < at("load"), "{events:?}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_defer_dom_tasks_keep_their_positions_around_domcontentloaded() -> Result<()> {
    deferred_rejection_keeps_dom_fifo_position(false, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_defer_dom_tasks_keep_their_positions_around_domcontentloaded() -> Result<()> {
    deferred_rejection_keeps_dom_fifo_position(true, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn main_module_dom_tasks_keep_their_positions_around_domcontentloaded() -> Result<()> {
    deferred_rejection_keeps_dom_fifo_position(false, true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_module_dom_tasks_keep_their_positions_around_domcontentloaded() -> Result<()> {
    deferred_rejection_keeps_dom_fifo_position(true, true).await
}

async fn earlier_dom_task_replaces_document_before_queued_dcl(child: bool) -> Result<()> {
    let markup = r#"<!doctype html><body><script>
      window.events = [];
      document.addEventListener('DOMContentLoaded', () => events.push('old DCL'));
      addEventListener('unhandledrejection', event => {
        event.preventDefault();
        document.open();
        document.write("<!doctype html><script>document.addEventListener('DOMContentLoaded', () => events.push('new DCL'))<\/script><p>replacement</p>");
        document.close();
      }, {once: true});
      Promise.reject('replace');
    </script><p>old parser tail</p>"#;
    let result = observe_lifecycle_markup(child, markup).await?;
    assert_eq!(result["value"].as_str(), Some(r#"["new DCL"]"#));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_earlier_dom_task_retires_queued_domcontentloaded() -> Result<()> {
    earlier_dom_task_replaces_document_before_queued_dcl(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_earlier_dom_task_retires_queued_domcontentloaded() -> Result<()> {
    earlier_dom_task_replaces_document_before_queued_dcl(true).await
}

async fn dom_callback_before_load_does_not_lose_load(child: bool) -> Result<()> {
    let markup = r#"<!doctype html><body><script>
      window.events = [];
      document.addEventListener('DOMContentLoaded', () => {
        events.push('DOMContentLoaded');
        Promise.reject('before load');
      });
      addEventListener('unhandledrejection', event => {
        event.preventDefault();
        events.push('rejection');
        const script = document.createElement('script');
        script.async = true;
        script.src = 'data:text/javascript,' + encodeURIComponent("events.push('async')");
        document.body.append(script);
      });
      addEventListener('load', () => events.push('load'));
    </script><p>parser tail</p>"#;
    let result = observe_lifecycle_markup(child, markup).await?;
    let events: Vec<String> = serde_json::from_str(result["value"].as_str().unwrap())?;
    for expected in ["DOMContentLoaded", "rejection", "async", "load"] {
        assert_eq!(
            events
                .iter()
                .filter(|event| event.as_str() == expected)
                .count(),
            1,
            "child={child}, {events:?}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_earlier_dom_callback_does_not_lose_load_after_starting_async_script() -> Result<()> {
    dom_callback_before_load_does_not_lose_load(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_earlier_dom_callback_does_not_lose_load_after_starting_async_script() -> Result<()> {
    dom_callback_before_load_does_not_lose_load(true).await
}

async fn parser_import_map_error_keeps_dom_fifo_position(
    child: bool,
    document_write: bool,
) -> Result<()> {
    let import_map = r#"<script type=importmap src="data:application/json,%7B%7D"
      onerror="events.push('parser error')"></script>"#;
    let parser_markup = if document_write {
        format!(
            "<script>document.write({})</script>",
            serde_json::to_string(import_map)?.replace("</script>", "<\\/script>")
        )
    } else {
        import_map.to_owned()
    };
    let markup = format!(
        r#"<!doctype html><head><script>
      window.events = [];
      document.addEventListener('DOMContentLoaded', () => events.push('DOMContentLoaded'));
    </script>{parser_markup}<script>
      const later = document.createElement('script');
      later.type = 'importmap';
      later.src = 'data:application/json,%7B%7D';
      later.onerror = () => events.push('dynamic error');
      document.head.append(later);
    </script></head><body>parser tail"#
    );
    let result = observe_lifecycle_markup(child, &markup).await?;
    assert_eq!(
        result["value"].as_str(),
        Some(r#"["parser error","dynamic error","DOMContentLoaded"]"#),
        "child={child}, document_write={document_write}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_parser_import_map_error_precedes_later_dom_tasks() -> Result<()> {
    parser_import_map_error_keeps_dom_fifo_position(false, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_import_map_error_precedes_later_dom_tasks() -> Result<()> {
    parser_import_map_error_keeps_dom_fifo_position(true, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn main_written_import_map_error_precedes_later_dom_tasks() -> Result<()> {
    parser_import_map_error_keeps_dom_fifo_position(false, true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_written_import_map_error_precedes_later_dom_tasks() -> Result<()> {
    parser_import_map_error_keeps_dom_fifo_position(true, true).await
}
