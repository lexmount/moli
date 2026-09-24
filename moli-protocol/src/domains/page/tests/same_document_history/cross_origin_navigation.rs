use super::*;

const PROBE: &str = include_str!("cross_origin_navigation.js");
const CHILD: &str = include_str!("cross_origin_navigation.html");

async fn check_cross_origin_navigation(api: &str) {
    for cross_origin in [false, true] {
        for destination in ["fragment", "document", "cross-origin-document"] {
            for action in ["cancel", "proceed", "intercept"] {
                if destination == "cross-origin-document" && action == "intercept" {
                    continue;
                }
                let routes = axum::Router::new()
                    .route(
                        "/cross-origin-navigation.html",
                        axum::routing::any(|| async { axum::response::Html(CHILD) }),
                    )
                    .route(
                        "/cross-origin-navigation-destination.html",
                        axum::routing::any(|| async { axum::response::Html(CHILD) }),
                    );
                let mut page = SameDocumentPage::with_routes(routes).await;
                let result = page.evaluate(&format!(
                    "{PROBE}\ncrossOriginNavigationProbe({api:?}, {cross_origin}, {destination:?}, {action:?})"
                )).await;
                let fragment = destination == "fragment" && api != "post";
                let fires = !cross_origin || fragment;
                let canceled = fires && action == "cancel";
                let retained = canceled || fragment || (fires && action == "intercept");
                let form = matches!(api, "submit" | "requestSubmit" | "post");
                let records = if fires {
                    json!([{
                        "kind": "navigate", "type": if api == "replace" { "replace" } else { "push" },
                        "cancelable": true, "canIntercept": destination != "cross-origin-document",
                        "sameDocument": fragment, "hashChange": fragment,
                        "sourceNull": cross_origin || !form,
                        "formData": if api == "post" { json!([["secret", "source-only"]]) } else { json!(null) },
                        "key": "", "id": "", "index": -1,
                    }])
                } else {
                    json!([])
                };
                assert_eq!(
                    result,
                    json!({
                        "terminal": if canceled { "navigate" } else if retained { "success" } else { "load" },
                        "records": records, "sourceEvents": [],
                        "returnedExisting": if api == "open" { json!(true) } else { json!(null) },
                        "unchanged": canceled, "reached": !canceled, "retained": retained,
                        "entriesDelta": if canceled || api == "replace" || destination == "cross-origin-document" { 0 } else { 1 },
                        "currentEntryMatches": true,
                    }),
                    "{api}/{cross_origin}/{destination}/{action}"
                );
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_origin_navigation_location_href() {
    check_cross_origin_navigation("href").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_origin_navigation_window_location() {
    check_cross_origin_navigation("window-location").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_origin_navigation_location_replace() {
    check_cross_origin_navigation("replace").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_origin_navigation_window_open() {
    check_cross_origin_navigation("open").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_origin_navigation_form_submit() {
    check_cross_origin_navigation("submit").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_origin_navigation_form_request_submit() {
    check_cross_origin_navigation("requestSubmit").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_origin_navigation_form_post_stays_cross_document() {
    check_cross_origin_navigation("post").await;
}
