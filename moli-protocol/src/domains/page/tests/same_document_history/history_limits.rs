use super::*;

const PROBE: &str = include_str!("history_limits.js");

#[tokio::test(flavor = "multi_thread")]
async fn history_update_limits_preserve_conversion_order_and_pending_navigation() {
    let mut page = SameDocumentPage::new().await;
    let result = page
        .evaluate(&format!("{PROBE}\nprobeHistoryUpdateAdmission()"))
        .await;
    assert_eq!(result["admitted"], 200);
    assert_eq!(result["conversion"], json!(["title", "url", "state"]));
    assert_eq!(
        result["errors"],
        json!(["marker", "DataCloneError", "SecurityError", "marker"])
    );
    assert_eq!(result["unchanged"], true);
    assert_eq!(result["events"], 0);
    assert_eq!(result["aborted"], false);
    assert_eq!(result["finished"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn history_update_limits_follow_the_receiver_browsing_context() {
    let mut page = SameDocumentPage::new().await;
    let result = page
        .evaluate(&format!("{PROBE}\nprobeHistoryUpdateOwners()"))
        .await;
    assert_eq!(
        result,
        json!({
            "childLimited": true, "parentAllowed": true, "siblingAllowed": true,
            "popupAllowed": true, "retainedAcrossNavigation": true, "freshAfterRemoval": true,
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn history_update_limits_bound_reentrant_callbacks_and_release_on_return() {
    let mut page = SameDocumentPage::new().await;
    let result = page
        .evaluate(&format!("{PROBE}\nprobeHistoryUpdateRecursion()"))
        .await;
    let rows = result.as_array().unwrap();
    assert_eq!(rows.len(), 5);
    for row in rows {
        let count = row["count"].as_u64().unwrap();
        assert!((2..=65).contains(&count), "{row}");
        assert_eq!(row["errors"], json!([]), "{row}");
        assert_eq!(row["recovered"], true, "{row}");
        if row["mode"] == "cross-window" {
            assert_eq!(row["outcomes"], json!(["AbortError", "AbortError"]));
        }
    }
}
