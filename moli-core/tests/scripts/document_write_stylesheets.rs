use super::*;

const SCRIPT: &str = include_str!("../fixtures/runtime/document_write_stylesheet_probe.js");
const INITIALIZE: &str = "window.styleNestingEvents = []; window.styleNestingRan = false;";
const OBSERVE_AND_RELEASE: &str = r#"
    window.styleNestingRanAtReturn = styleNestingRan;
    window.styleNestingRestoredScript = document.currentScript?.id ?? null;
    styleNestingEvents.push('after-write');
    fetch('/assets/dynamic_blocking_stylesheet_script_executed');
"#;

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

fn script_markup(id: &str, source: &str, external: bool) -> String {
    if external {
        let source = url::form_urlencoded::byte_serialize(source.as_bytes())
            .collect::<String>()
            .replace('+', "%20");
        format!("<script id={id} src=\"data:text/javascript,{source}\"></script>")
    } else {
        format!("<script id={id}>{source}</script>")
    }
}

async fn stylesheet_wait_respects_parser_script_nesting(
    child: bool,
    script_created: bool,
    nested: bool,
    external: bool,
    outer_external: bool,
) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let script = script_markup("inner", SCRIPT, external);
    let written = format!(
        "<link id=gate rel=stylesheet href=/assets/dynamic_blocking_stylesheet_gated.css>{script}"
    );
    let fixture = if nested {
        let written = serde_json::to_string(&written)?.replace('<', "\\u003c");
        let outer = script_markup(
            "outer",
            &format!("{INITIALIZE}document.write({written});{OBSERVE_AND_RELEASE}"),
            outer_external,
        );
        format!("<!doctype html><head>{outer}</head><body><main id=tail>tail</main>")
    } else {
        format!("<!doctype html><head>{written}</head><body><main id=tail>tail</main>")
    };
    let initial = if script_created {
        "<!doctype html><body>"
    } else {
        &fixture
    };
    let initial = if child {
        format!(
            "<!doctype html><body><iframe id=target src=\"{}\"></iframe>",
            markup_url(&server, initial)
        )
    } else {
        initial.to_owned()
    };
    let mut page = tokio::time::timeout(
        Duration::from_secs(10),
        browser.fetch(&markup_url(&server, &initial)),
    )
    .await??;
    let receiver = if child {
        "document.getElementById('target').contentWindow"
    } else {
        "window"
    };
    if script_created {
        let initialize = if nested { "" } else { INITIALIZE };
        let observe = if nested { "" } else { OBSERVE_AND_RELEASE };
        let expression = format!(
            r#"new Promise(resolve => {{
              const win = {receiver};
              const doc = win.document;
              doc.open();
              win.eval({});
              doc.addEventListener('DOMContentLoaded', () => resolve(true), {{once: true}});
              doc.write({});
              win.eval({});
              doc.close();
            }})"#,
            serde_json::to_string(initialize)?,
            serde_json::to_string(&fixture)?,
            serde_json::to_string(observe)?,
        );
        tokio::time::timeout(
            Duration::from_secs(10),
            page.evaluate_runtime_expression_with_await_async(&expression, true),
        )
        .await??;
    }
    let observed = page
        .evaluate_runtime_expression_with_await_async(
            &format!(
                r#"(() => {{
                  const win = {receiver};
                  return JSON.stringify({{
                    events: win.styleNestingEvents,
                    ranAtReturn: win.styleNestingRanAtReturn,
                    sheetAtRun: win.styleNestingSheetAtRun,
                    currentScript: win.styleNestingCurrentScript,
                    restoredScript: win.styleNestingRestoredScript,
                    written: win.document.getElementById('written')?.textContent,
                    tail: win.document.getElementById('tail')?.textContent
                  }});
                }})()"#
            ),
            true,
        )
        .await?;
    let observed: serde_json::Value = serde_json::from_str(
        observed["value"]
            .as_str()
            .expect("stylesheet nesting observation"),
    )?;
    let immediate = nested && !external;
    assert_eq!(
        observed,
        serde_json::json!({
            "events": if immediate { ["script", "after-write"] } else { ["after-write", "script"] },
            "ranAtReturn": immediate,
            "sheetAtRun": !immediate,
            "currentScript": "inner",
            "restoredScript": if nested { Some("outer") } else { None },
            "written": "ok",
            "tail": "tail"
        }),
        "child={child}, script_created={script_created}, nested={nested}, external={external}, outer_external={outer_external}"
    );
    server.shutdown().await;
    Ok(())
}

macro_rules! stylesheet_nesting_test {
    ($name:ident, $child:expr, $created:expr, $nested:expr, $external:expr) => {
        stylesheet_nesting_test!($name, $child, $created, $nested, $external, false);
    };
    ($name:ident, $child:expr, $created:expr, $nested:expr, $external:expr, $outer_external:expr) => {
        #[tokio::test(flavor = "multi_thread")]
        async fn $name() -> Result<()> {
            stylesheet_wait_respects_parser_script_nesting(
                $child,
                $created,
                $nested,
                $external,
                $outer_external,
            )
            .await
        }
    };
}

stylesheet_nesting_test!(
    main_nested_inline_ignores_stylesheet_wait,
    false,
    false,
    true,
    false
);
stylesheet_nesting_test!(
    child_nested_inline_ignores_stylesheet_wait,
    true,
    false,
    true,
    false
);
stylesheet_nesting_test!(
    main_open_nested_inline_ignores_stylesheet_wait,
    false,
    true,
    true,
    false
);
stylesheet_nesting_test!(
    child_open_nested_inline_ignores_stylesheet_wait,
    true,
    true,
    true,
    false
);
stylesheet_nesting_test!(
    main_nested_external_keeps_stylesheet_wait,
    false,
    false,
    true,
    true
);
stylesheet_nesting_test!(
    child_nested_external_keeps_stylesheet_wait,
    true,
    false,
    true,
    true
);
stylesheet_nesting_test!(
    main_open_nested_external_keeps_stylesheet_wait,
    false,
    true,
    true,
    true
);
stylesheet_nesting_test!(
    child_open_nested_external_keeps_stylesheet_wait,
    true,
    true,
    true,
    true
);
stylesheet_nesting_test!(
    main_open_direct_inline_keeps_stylesheet_wait,
    false,
    true,
    false,
    false
);
stylesheet_nesting_test!(
    child_open_direct_inline_keeps_stylesheet_wait,
    true,
    true,
    false,
    false
);

stylesheet_nesting_test!(
    main_external_outer_inline_ignores_stylesheet_wait,
    false,
    false,
    true,
    false,
    true
);
stylesheet_nesting_test!(
    child_external_outer_inline_ignores_stylesheet_wait,
    true,
    false,
    true,
    false,
    true
);
stylesheet_nesting_test!(
    main_open_external_outer_inline_ignores_stylesheet_wait,
    false,
    true,
    true,
    false,
    true
);
stylesheet_nesting_test!(
    child_open_external_outer_inline_ignores_stylesheet_wait,
    true,
    true,
    true,
    false,
    true
);
