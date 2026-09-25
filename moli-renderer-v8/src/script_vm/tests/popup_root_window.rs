use super::*;

#[tokio::test]
async fn popup_root_window_preserves_nested_parent_top_identity_and_origin_checks() {
    const PROBE: &str = include_str!("../../../tests/fixtures/popup-root-window.js");
    const A: &str = "www.example.test";
    const B: &str = "remote.example.test";
    for hosts in [
        vec![B, B],
        vec![A, B],
        vec![A, B, B],
        vec![A, B, A],
        vec![B, A, B],
        vec![B, B, A],
        vec![A, A, A],
        vec![A, B, A, B],
    ] {
        let bodies = (0..hosts.len())
            .map(|index| {
                format!(
                    "<!doctype html><body><script>{PROBE}\npopupRootFrame({}, {index});</script>",
                    serde_json::json!(hosts),
                )
            })
            .collect();
        let server = StaticHttpServer::spawn_with_bodies(bodies).await;
        let loader = static_http_loader([server.resolve_entry(A), server.resolve_entry(B)]);
        let opener_url = server.url_for_host(A, "/opener");
        let popup_url = server.url_for_host(hosts[0], "/popup");
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(opener_url.as_str(), &loader);
        vm.eval(&format!(
            "globalThis.popupRootResult = null;\n\
             onmessage = event => popupRootResult = event.data;\n\
             globalThis.rootPopup = open({:?});",
            popup_url.as_str(),
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(popupRootResult !== null)",
            "true",
            &format!("nested popup roots: {hosts:?}"),
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(popupRootResult)").unwrap()).unwrap();
        let child = hosts.last().unwrap();
        let parent_path = if hosts.len() == 2 {
            "/popup".to_owned()
        } else {
            format!("/frame-{}", hosts.len() - 2)
        };
        let top_path = if *child == hosts[0] {
            "/popup"
        } else {
            "SecurityError"
        };
        assert_eq!(
            result,
            serde_json::json!({
                "topHasOpener": true,
                "topIsParent": hosts.len() == 2,
                "parentTopIsTop": true,
                "topParentIsTop": true,
                "topSelfIsTop": true,
                "topDocumentPath": top_path,
                "parentDocumentPath": if *child == hosts[hosts.len() - 2] {
                    parent_path.as_str()
                } else {
                    "SecurityError"
                },
                "openerDocumentPath": if *child == A { "/opener" } else { "SecurityError" },
            }),
            "{hosts:?}",
        );
        vm.eval("rootPopup.close()").unwrap();
        assert_eq!(server.finish_targets().await.len(), hosts.len());
    }
}
