use super::*;
use std::fmt::Write;

async fn evaluate_preparation_error_document(html: &str) -> Result<serde_json::Value> {
    let mut document_url = String::from("data:text/html,");
    for byte in html.bytes() {
        write!(&mut document_url, "%{byte:02X}")?;
    }
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&document_url).await?;
    let result = page
        .evaluate_runtime_expression_with_await_async(
            "preparationDone.then(() => JSON.stringify(probe))",
            true,
        )
        .await?;
    serde_json::from_str(result["value"].as_str().expect("serialized probe")).map_err(Into::into)
}

#[tokio::test(flavor = "multi_thread")]
async fn script_preparation_errors_target_elements_without_reporting_window_exceptions()
-> Result<()> {
    let result = evaluate_preparation_error_document(
        r#"<!doctype html><body><script>
        globalThis.probe = {elements: [], exceptions: [], captures: [], bubbles: [], checkpoints: [], reexecuted: 0};
        globalThis.preparationDone = new Promise(resolve => globalThis.preparationComplete = resolve);
        const OriginalEvent = Event;
        const OriginalErrorEvent = ErrorEvent;
        globalThis.Event = function() { throw new Error('must use intrinsic Event'); };
        addEventListener('error', e => {
          if (e.target === window) probe.exceptions.push(e.message.includes('author sentinel'));
          else probe.bubbles.push(e.target.id);
        });
        addEventListener('error', e => {
          if (e.target !== window) probe.captures.push(e.target.id);
        }, true);
        function failed(e) {
          probe.elements.push([this.id, e.type, e.bubbles, e.cancelable, e.composed,
            e.isTrusted, e instanceof OriginalEvent, e instanceof OriginalErrorEvent,
            e.target === this, e.currentTarget === this]);
          Promise.resolve().then(() => {
            probe.checkpoints.push(this.id);
            if (probe.checkpoints.length === 8) preparationComplete();
          });
          this.src = 'data:text/javascript,probe.reexecuted%2B%2B';
        }
        </script>
        <script id="classic-empty" src="" onerror="failed.call(this, event)"></script>
        <script id="classic-invalid" src="http://[" onerror="failed.call(this, event)"></script>
        <script id="async-empty" async src="" onerror="failed.call(this, event)"></script>
        <script id="defer-invalid" defer src="http://[" onerror="failed.call(this, event)"></script>
        <script id="module-empty" type="module" src="" onerror="failed.call(this, event)"></script>
        <script id="module-invalid" type="module" src="http://[" onerror="failed.call(this, event)"></script>
        <script>
        document.write('<script id="write-empty" src="" onerror="failed.call(this, event)"><\/script>');
        document.write('<script id="write-module" type="module" src="http://[" onerror="failed.call(this, event)"><\/script>');
        throw new Error('author sentinel');
        </script>"#,
    )
    .await?;

    let mut elements = result["elements"].as_array().unwrap().clone();
    elements.sort_by(|left, right| left[0].as_str().cmp(&right[0].as_str()));
    let mut expected = [
        "classic-empty",
        "classic-invalid",
        "async-empty",
        "defer-invalid",
        "module-empty",
        "module-invalid",
        "write-empty",
        "write-module",
    ]
    .map(|id| {
        serde_json::json!([
            id, "error", false, false, false, true, true, false, true, true
        ])
    });
    expected.sort_by(|left, right| left[0].as_str().cmp(&right[0].as_str()));
    assert_eq!(elements, expected, "{result}");
    assert_eq!(result["exceptions"], serde_json::json!([true]), "{result}");
    assert_eq!(result["bubbles"], serde_json::json!([]), "{result}");
    assert_eq!(
        result["reexecuted"], 0,
        "failed preparation must still mark the script already started"
    );
    let mut captures = result["captures"].as_array().unwrap().clone();
    captures.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    assert_eq!(
        captures,
        expected
            .iter()
            .map(|row| row[0].clone())
            .collect::<Vec<_>>()
    );
    let mut checkpoints = result["checkpoints"].as_array().unwrap().clone();
    checkpoints.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    assert_eq!(
        checkpoints, captures,
        "each error listener must complete its microtasks"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn script_preparation_errors_are_delivered_in_child_parser_documents() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let result = page.evaluate_runtime_expression_with_await_async(
        r#"new Promise(resolve => {
          const frame = document.createElement('iframe');
          globalThis.childPreparationErrorsReady = result => {
            delete globalThis.childPreparationErrorsReady;
            resolve(JSON.stringify(result));
          };
          frame.srcdoc = `<script>
            globalThis.probe = {elements: [], exceptions: []};
            addEventListener('error', e => { if (e.target === window) probe.exceptions.push(e.message); });
            function failed(e) {
              probe.elements.push([this.id, e instanceof Event,
                e instanceof ErrorEvent, e.isTrusted, e.bubbles, e.target === this]);
              if (probe.elements.length === 3) parent.childPreparationErrorsReady(probe);
            }
          <\/script>
          <script id="child-empty" src="" onerror="failed.call(this, event)"><\/script>
          <script id="child-module" type="module" src="http://[" onerror="failed.call(this, event)"><\/script>
          <script>
            document.write('<script id="child-write" src="" onerror="failed.call(this, event)"><' + '/script>');
          <\/script>`;
          document.body.append(frame);
        })"#, true,
    ).await?;
    server.shutdown().await;
    let result: serde_json::Value =
        serde_json::from_str(result["value"].as_str().expect("child probe"))?;
    assert_eq!(
        result,
        serde_json::json!({
            "elements": [
                ["child-empty", true, false, true, false, true],
                ["child-module", true, false, true, false, true],
                ["child-write", true, false, true, false, true],
            ],
            "exceptions": [],
        })
    );
    Ok(())
}
