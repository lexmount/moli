use super::*;

#[tokio::test(flavor = "current_thread")]
async fn removing_shadow_subtrees_retires_nested_windows_before_reinsertion() {
    for shadow in [false, true] {
        for mode in ["remove", "replace", "ancestor", "fragment"] {
            let server = StaticHttpServer::spawn_with_bodies(vec![
                "<!doctype html><body>Window removal target</body>".to_owned(); 3
            ])
            .await;
            let loader = static_http_loader([server.resolve_entry("www.example.test")]);
            let parent_url = server.url_for_host("www.example.test", "/page.html");
            let mut vm =
                new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
            let script = include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/shadow-frame-retirement.js"
            ));
            vm.exec(
                &format!(
                    r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__removedWindowResult = null;
({script})({{mode: {mode:?}, shadow: {shadow}}}).then(
  result => {{ __removedWindowResult = result; }},
  error => {{ __removedWindowResult = {{error: String(error)}}; }}
);
"#
                ),
                None,
            )
            .expect("Window removal probe should start");
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(__removedWindowResult !== null)",
                "true",
                "Window removal probe should finish",
            )
            .await;
            let result: serde_json::Value = serde_json::from_str(
                &vm.eval("JSON.stringify(__removedWindowResult)")
                    .expect("Window removal observations"),
            )
            .unwrap();
            assert_eq!(
                result,
                serde_json::json!({
                    "after": [true, true], "reinserted": [true, true],
                    "fresh": {"top": true, "parent": true, "frameElement": true,
                        "closed": false, "different": true},
                }),
                "{mode}/shadow={shadow}: {result}"
            );
            assert_eq!(
                server.finish_targets().await,
                vec![
                    "/history.html?root",
                    "/history.html?child",
                    "/history.html?replacement"
                ],
                "{mode}/shadow={shadow}"
            );
        }
    }
}
