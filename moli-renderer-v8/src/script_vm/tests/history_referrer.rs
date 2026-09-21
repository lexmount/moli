use super::*;

const HISTORY_NAVIGATION_REFERRER: &str =
    include_str!("../../../tests/fixtures/history-navigation-referrer.js");

#[tokio::test]
async fn child_history_navigation_referrer_uses_the_committed_document_url() {
    for mode in [
        "push",
        "replace",
        "base",
        "pending",
        "sibling",
        "currententrychange",
    ] {
        let server = StaticHttpServer::spawn(if mode == "sibling" { 3 } else { 2 }).await;
        let base = server.base_url().origin().ascii_serialization();
        let loader = static_http_loader([]);
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(&format!("{base}/parent"), &loader);
        vm.eval(&format!(
            "{HISTORY_NAVIGATION_REFERRER}\n\
             globalThis.historyReferrerResult = 'pending';\n\
             historyNavigationReferrer({base:?}, {mode:?}).then(\n\
               result => historyReferrerResult = result,\n\
               error => historyReferrerResult = String(error));"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(historyReferrerResult !== 'pending')",
            "true",
            mode,
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(historyReferrerResult)").unwrap())
                .unwrap();
        let source_url = format!("{base}/changed/{mode}?q=1#fragment");
        let referrer = format!("{base}/changed/{mode}?q=1");
        let source_base = if mode == "base" {
            format!("{base}/assets/")
        } else {
            source_url.clone()
        };
        assert_eq!(
            result,
            serde_json::json!({
                "observed": [source_url, source_base],
                "referrer": referrer,
                "destination": format!("{base}/destination?case={mode}"),
            }),
            "{mode}"
        );
        let requests = server.finish().await;
        let request = requests.last().unwrap();
        assert_eq!(request.target, format!("/destination?case={mode}"));
        assert_eq!(
            request.header_value("referer"),
            Some(referrer.as_str()),
            "{mode}"
        );
    }
}
