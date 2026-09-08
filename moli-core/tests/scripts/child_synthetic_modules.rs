use super::*;

async fn evaluate_child_synthetic_expression(expression: &str) -> Result<serde_json::Value> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let result = page
        .evaluate_runtime_expression_with_await_async(expression, true)
        .await?;
    server.shutdown().await;
    Ok(result)
}

async fn child_synthetic_probe(source: &str) -> Result<serde_json::Value> {
    let html = format!(
        r#"<!doctype html><script>
        window.result = 'not evaluated';
        window.onerror = (message, url, line, column, error) => {{
            window.result = error.name + ': ' + error.message;
        }};
        </script><script type="module">{source}</script>"#
    );
    let expression = format!(
        r#"new Promise(resolve => {{
            const frame = document.createElement('iframe');
            frame.onload = () => resolve(frame.contentWindow.result);
            frame.srcdoc = {};
            document.body.appendChild(frame);
        }})"#,
        serde_json::to_string(&html)?
    );
    evaluate_child_synthetic_expression(&expression).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_synthetic_json_module_uses_its_own_document_source() -> Result<()> {
    let result = child_synthetic_probe(
        r#"import value from 'data:application/json,{"answer":42}' with { type: 'json' };
        window.result = value.answer + ':' + (Object.getPrototypeOf(value) === Object.prototype);"#,
    )
    .await?;
    assert_eq!(result["value"], "42:true", "{result:?}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_synthetic_css_module_uses_its_own_document_source() -> Result<()> {
    let result = child_synthetic_probe(
        r#"import sheet from 'data:text/css,body%20%7B%20color%3A%20red%3B%20%7D' with { type: 'css' };
        window.result = (sheet instanceof CSSStyleSheet) + ':' + sheet.cssRules.length;"#,
    )
    .await?;
    assert_eq!(result["value"], "true:1", "{result:?}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_synthetic_json_parse_error_is_not_a_missing_source_error() -> Result<()> {
    let result = child_synthetic_probe(
        r#"import value from 'data:application/json,not-json' with { type: 'json' };
        window.result = 'must not execute';"#,
    )
    .await?;
    assert!(
        result["value"]
            .as_str()
            .is_some_and(|value| value.starts_with("SyntaxError: ")),
        "{result:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_synthetic_dynamic_import_after_load_uses_its_realm() -> Result<()> {
    let result = evaluate_child_synthetic_expression(
        r#"(async () => {
          const frame = document.createElement('iframe');
          const loaded = new Promise(resolve => frame.onload = resolve);
          frame.srcdoc = `<script>
            window.probe = async () => {
              await new Promise(resolve => setTimeout(resolve, 0));
              const jsonUrl = 'data:application/json,{"answer":42}';
              const cssUrl = 'data:text/css,body%7Bcolor%3Ared%7D';
              const json = await import(jsonUrl, {with: {type: 'json'}});
              const css = await import(cssUrl, {with: {type: 'css'}});
              const again = await import(jsonUrl, {with: {type: 'json'}});
              let failure;
              try { await import('data:application/json,invalid', {with: {type: 'json'}}); }
              catch (error) { failure = error; }
              return [json.default.answer === 42,
                Object.getPrototypeOf(json.default) === Object.prototype,
                css.default instanceof CSSStyleSheet, css.default.cssRules.length === 1,
                json === again, failure instanceof SyntaxError].join(':');
            };
          </script>`;
          document.body.appendChild(frame);
          await loaded;
          return await frame.contentWindow.probe();
        })()"#,
    )
    .await?;
    assert_eq!(
        result["value"], "true:true:true:true:true:true",
        "{result:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_synthetic_module_values_are_cached_per_realm_not_per_url() -> Result<()> {
    let result = evaluate_child_synthetic_expression(
        r#"(async () => {
          const jsonUrl = 'data:application/json,{"answer":42}';
          const cssUrl = 'data:text/css,body%7Bcolor%3Ared%7D';
          const rootJson = (await import(jsonUrl, {with: {type: 'json'}})).default;
          const rootSheet = (await import(cssUrl, {with: {type: 'css'}})).default;
          const makeFrame = async () => {
            const frame = document.createElement('iframe');
            const loaded = new Promise(resolve => frame.onload = resolve);
            frame.srcdoc = `<script type="module">
              import value from '${jsonUrl}' with {type: 'json'};
              import sheet from '${cssUrl}' with {type: 'css'};
              window.value = value;
              window.sheet = sheet;
              window.readAgain = async () => {
                const json = await import('${jsonUrl}', {with: {type: 'json'}});
                const css = await import('${cssUrl}', {with: {type: 'css'}});
                return json.default === value && css.default === sheet;
              };
            </script>`;
            document.body.appendChild(frame);
            await loaded;
            return frame.contentWindow;
          };
          const [left, right] = await Promise.all([makeFrame(), makeFrame()]);
          left.value.answer = 7;
          left.sheet.insertRule('p { color: blue; }', 1);
          return [rootJson.answer === 42, right.value.answer === 42,
            left.value.answer === 7, rootSheet.cssRules.length === 1,
            right.sheet.cssRules.length === 1, left.sheet.cssRules.length === 2,
            left.value !== right.value, left.value !== rootJson,
            left.sheet !== right.sheet, left.sheet !== rootSheet,
            Object.getPrototypeOf(left.value) === left.Object.prototype,
            Object.getPrototypeOf(right.value) === right.Object.prototype,
            left.sheet instanceof left.CSSStyleSheet,
            right.sheet instanceof right.CSSStyleSheet,
            await left.readAgain(), await right.readAgain()].join(':');
        })()"#,
    )
    .await?;
    assert_eq!(result["value"], ["true"; 16].join(":"), "{result:?}");
    Ok(())
}
