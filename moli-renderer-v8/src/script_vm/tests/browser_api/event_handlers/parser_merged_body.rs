use super::*;

const MERGED_BODY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/parser-merged-body-handlers.html"
));

fn expected_snapshot() -> serde_json::Value {
    serde_json::json!({
        "trace": ["before", "attribute:true:true", "after",
            "before", "attribute:true:true", "after", "before", "after", "focus:true"],
        "sameHandler": true,
        "cleared": true,
        "attributeRetained": true,
        "loadHandler": true
    })
}

#[tokio::test]
async fn child_and_popup_parser_merged_body_handlers_are_registered_once() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://merged-body-handlers.test/",
        "<!doctype html><body></body>",
        &loader,
    );
    vm.eval(&format!(
        r#"
const mergedMarkup = {};
globalThis.mergedFrame = document.createElement('iframe');
mergedFrame.srcdoc = mergedMarkup;
document.body.append(mergedFrame);
const mergedURL = URL.createObjectURL(new Blob([mergedMarkup], {{type:'text/html'}}));
globalThis.mergedPopup = open(mergedURL, 'merged-body');
"#,
        serde_json::to_string(MERGED_BODY).unwrap()
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(!!mergedFrame.contentWindow.mergedBodyScriptDone && !!mergedPopup.mergedBodyScriptDone)",
        "true",
        "merged body parser scripts",
    )
    .await;
    let snapshots: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify([mergedFrame.contentWindow.mergedBodySnapshot, mergedPopup.mergedBodySnapshot])")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        snapshots,
        serde_json::json!([expected_snapshot(), expected_snapshot()])
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(!!mergedFrame.contentWindow.mergedBodyLoaded && !!mergedPopup.mergedBodyLoaded)",
        "true",
        "merged body load handlers",
    )
    .await;
    vm.eval("mergedPopup.close(); mergedFrame.remove(); URL.revokeObjectURL(mergedURL)")
        .unwrap();
}

#[test]
fn document_write_parser_merged_body_handlers_are_registered_once() {
    let mut vm = new_storage_test_vm("https://merged-body-handlers.test/");
    vm.eval(&format!(
        "document.open(); document.write({}); document.close();",
        serde_json::to_string(MERGED_BODY).unwrap()
    ))
    .unwrap();
    let snapshot: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(mergedBodySnapshot)").unwrap()).unwrap();
    assert_eq!(snapshot, expected_snapshot());
}
