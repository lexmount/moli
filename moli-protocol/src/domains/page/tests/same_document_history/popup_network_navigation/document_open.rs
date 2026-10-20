use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn document_open_uses_the_entry_document_across_page_hosts() {
    const SCRIPT: &str = include_str!("../../../../../../tests/fixtures/document-open-entry.js");
    const SOURCE: &str = "<!doctype html><body>Source<script src='/document-open-entry.js'></script><script>installDocumentOpenEntrySource();</script>";
    run_history_test(async {
        let mut cases = Vec::new();
        for source in ["root", "iframe", "popup", "popup-microtask", "self"] {
            for method in ["open", "borrowed", "write", "writeln"] {
                cases.push((source, method, json!({})));
            }
        }
        cases.push(("root", "borrowed", json!({"poison": true})));
        cases.push(("popup", "open", json!({"poison": true})));
        for (source, method, options) in cases {
            let routes = axum::Router::new()
                .route(
                    "/entry-source",
                    axum::routing::get(|| async { axum::response::Html(SOURCE) }),
                )
                .route(
                    "/entry-target",
                    axum::routing::get(|| async {
                        axum::response::Html("<!doctype html><body>Target")
                    }),
                )
                .route(
                    "/document-open-entry.js",
                    axum::routing::get(|| async {
                        (
                            [(axum::http::header::CONTENT_TYPE, "text/javascript")],
                            SCRIPT,
                        )
                    }),
                );
            let mut page = SameDocumentPage::with_routes(routes).await;
            let script = format!(
                "{SCRIPT}\ndocumentOpenEntry(location.origin, {source:?}, {method:?}, {options})"
            );
            let result = page.evaluate(&script).await;
            assert_eq!(result["total"], json!(8));
            assert_eq!(result["passed"], result["total"], "{result}");
        }
    })
    .await;
}
