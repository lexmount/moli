use super::*;

pub(super) async fn assert_page_fixture(source: &str, total: u64) {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body><iframe></iframe>").await;
    ctx.process_async(json!({
        "id": 210_263,
        "method": "Runtime.evaluate",
        "params": {
            "expression": format!("{source}; JSON.stringify(__uiEventResults)"),
            "returnByValue": true,
            "userGesture": true
        }
    }))
    .await;
    let response = wait_for_response_by_id_async(&mut ctx, None, 210_263).await;
    assert!(response["error"].is_null(), "{response}");
    assert!(
        response["result"]["exceptionDetails"].is_null(),
        "{response}"
    );
    let facts: serde_json::Value = serde_json::from_str(
        response["result"]["result"]["value"]
            .as_str()
            .expect("fixture returns JSON"),
    )
    .unwrap();
    let failed: Vec<_> = facts["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["passed"] != true)
        .collect();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(facts["total"], total);
    assert_eq!(facts["passed"], total);
}
