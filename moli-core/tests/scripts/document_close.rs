use super::*;

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

async fn written_child_script_close_finishes_load(
    character_chunks: bool,
    plaintext: bool,
    external: bool,
) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let inserted = if plaintext {
        "<table><plaintext>Filler "
    } else {
        "<p id=inserted>inserted</p>"
    };
    let source = format!(
        r#"
          document.addEventListener('DOMContentLoaded', () => parent.events.push('child-DCL'));
          addEventListener('load', () => parent.events.push('child-load:' + document.readyState));
          document.write({});
          document.close();
          parent.events.push('close-return:' + document.readyState);
        "#,
        serde_json::to_string(inserted)?
    );
    let mut written = if external {
        format!(
            "<script src=\"data:text/javascript,{}\"></script>",
            url::form_urlencoded::byte_serialize(source.as_bytes())
                .collect::<String>()
                .replace('+', "%20")
        )
    } else {
        format!("<script>{source}</script>")
    };
    if !character_chunks {
        written.push_str("<main id=tail>tail</main>");
    }
    let parent = format!(
        r#"<!doctype html><body><script>
          window.events = [];
          addEventListener('load', () => events.push('parent-load:' + document.readyState));
          const frame = document.createElement('iframe');
          frame.id = 'target';
          document.body.append(frame);
          const written = {};
          if ({character_chunks}) {{
            for (const character of written) frame.contentDocument.write(character);
          }} else {{
            frame.contentDocument.write(written);
          }}
        </script>"#,
        serde_json::to_string(&written)?.replace("</script>", "<\\/script>")
    );
    // The written script is the only caller of close(). Fetch must finish both
    // the child and its load-blocked parent without another close from outside.
    let mut page = tokio::time::timeout(
        Duration::from_secs(10),
        browser.fetch(&markup_url(&server, &parent)),
    )
    .await??;
    let result = page
        .evaluate_runtime_expression_with_await_async(
            r#"(() => {
              const child = document.getElementById('target').contentDocument;
              return JSON.stringify({events, readyState: child.readyState,
                inserted: child.getElementById('inserted')?.textContent ?? null,
                tail: child.getElementById('tail')?.textContent ?? null,
                children: Array.from(child.body.children, node => [node.localName, node.textContent])});
            })()"#,
            true,
        )
        .await?;
    let result: serde_json::Value =
        serde_json::from_str(result["value"].as_str().expect("child close observation"))?;
    assert_eq!(result["readyState"], "complete");
    assert_eq!(
        result["events"],
        serde_json::json!([
            "close-return:loading",
            "child-DCL",
            "child-load:complete",
            "parent-load:complete"
        ])
    );
    if plaintext {
        assert_eq!(
            result["children"],
            serde_json::json!([["plaintext", "Filler "], ["table", ""]])
        );
    } else {
        assert_eq!(result["inserted"], "inserted");
    }
    if !character_chunks {
        assert_eq!(result["tail"], "tail");
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_close_in_character_chunked_script_finishes_load() -> Result<()> {
    written_child_script_close_finishes_load(true, false, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_close_in_plaintext_writer_finishes_load() -> Result<()> {
    written_child_script_close_finishes_load(true, true, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_close_in_written_script_drains_tail_and_finishes_load() -> Result<()> {
    written_child_script_close_finishes_load(false, false, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_close_in_external_writer_drains_tail_and_finishes_load() -> Result<()> {
    written_child_script_close_finishes_load(false, false, true).await
}
