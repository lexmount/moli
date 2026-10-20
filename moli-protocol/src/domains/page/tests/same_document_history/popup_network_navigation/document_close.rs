use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn document_close_completes_the_target_page_owner_lifecycle() {
    const SCRIPT: &str = include_str!("../../../../../../tests/fixtures/document-close-owner.js");
    const SOURCE: &str = "<!doctype html><script src='/document-close-owner.js'></script><script>installDocumentCloseSource();</script>";
    run_history_test(async {
        for source in ["root", "iframe", "popup", "self"] {
            for operation in ["same-turn", "later-close", "reopen"] {
                let routes = axum::Router::new()
                    .route(
                        "/close-source",
                        axum::routing::get(|| async { axum::response::Html(SOURCE) }),
                    )
                    .route(
                        "/close-target",
                        axum::routing::get(|| async {
                            axum::response::Html("<!doctype html><body>Target")
                        }),
                    )
                    .route(
                        "/document-close-owner.js",
                        axum::routing::get(|| async {
                            (
                                [(axum::http::header::CONTENT_TYPE, "text/javascript")],
                                SCRIPT,
                            )
                        }),
                    );
                let mut page = SameDocumentPage::with_routes(routes).await;
                let result = page
                    .evaluate(&format!(
                        "{SCRIPT}\ndocumentCloseOwner(location.origin, {source:?}, {operation:?})"
                    ))
                    .await;
                assert!(
                    result["checks"]
                        .as_object()
                        .unwrap()
                        .values()
                        .all(|value| value == &json!(true)),
                    "{result}"
                );
            }
        }
    })
    .await;
}
