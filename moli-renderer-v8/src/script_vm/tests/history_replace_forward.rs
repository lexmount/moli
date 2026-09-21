use super::*;

const HISTORY_REPLACE_FORWARD: &str =
    include_str!("../../../tests/fixtures/history-replace-forward.js");

#[tokio::test]
async fn cross_document_replace_preserves_forward_entries_and_states() {
    for method in ["location", "navigation"] {
        let server = StaticHttpServer::spawn(7).await;
        let base = server.base_url().origin().ascii_serialization();
        let loader = static_http_loader([]);
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(&format!("{base}/parent"), &loader);
        vm.eval(&format!(
            "{HISTORY_REPLACE_FORWARD}\n\
             globalThis.replaceForwardResult = 'pending';\n\
             historyReplaceForward({base:?}, {method:?}).then(\n\
               value => replaceForwardResult = value,\n\
               error => replaceForwardResult = String(error));"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(replaceForwardResult !== 'pending')",
            "true",
            method,
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(replaceForwardResult)").unwrap())
                .unwrap();
        assert_eq!(
            result,
            serde_json::json!({
                "paths": ["/a", "/replaced", "/c"],
                "backwardIdentity": true,
                "replacementIdentity": true,
                "forwardIdentity": true,
                "savedState": {"navigation": "forward"},
                "forwardPath": "/c",
                "classicState": {"classic": "forward"},
                "navigationState": {"navigation": "forward"},
                "replacementPath": "/replaced",
            }),
            "{method}"
        );
        assert_eq!(
            server.finish_targets().await,
            ["/a", "/b", "/c", "/b", "/replaced", "/c", "/replaced"],
            "{method}"
        );
    }
}
