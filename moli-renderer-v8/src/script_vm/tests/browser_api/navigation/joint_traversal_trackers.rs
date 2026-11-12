use super::*;

async fn observe_joint_traversal_trackers(
    classic: &str,
    api: &str,
    mode: &str,
) -> serde_json::Value {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://joint-navigation-trackers.test/",
        &loader,
    );
    vm.eval(
        r#"
        const root = document.documentElement ||
          document.appendChild(document.createElement('html'));
        const frame = document.createElement('iframe');
        frame.srcdoc = '<p>child</p>';
        root.appendChild(frame);
        'queued'
        "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    vm.eval(&format!(
        "globalThis.jointTrackerResult = null; const probe = {}; probe({classic:?}, {api:?}, {mode:?}).then(value => {{jointTrackerResult = value;}}, error => {{jointTrackerResult = {{error: String(error)}};}});",
        include_str!("joint_traversal_trackers.js"),
    ))
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    if mode == "held" {
        let pending = if api == "parent" {
            r#"{"committed":true,"finished":true}"#
        } else {
            r#"{"committed":false,"finished":true}"#
        };
        assert_eq!(
            vm.eval("JSON.stringify(jointTrackerPending)").unwrap(),
            pending,
            "{classic}/{api}/{mode} before releasing precommit"
        );
        vm.eval("releaseJointTrackerPrecommit(); 'released'")
            .unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
            .await
            .unwrap();
    }
    serde_json::from_str(&vm.eval("JSON.stringify(jointTrackerResult)").unwrap()).unwrap()
}

#[tokio::test]
async fn joint_history_traversals_promote_each_windows_upcoming_tracker() {
    for mode in ["plain", "held"] {
        for classic in ["parent", "child"] {
            for api in ["parent", "child"] {
                let result = observe_joint_traversal_trackers(classic, api, mode).await;
                for kind in ["committed", "finished"] {
                    assert_eq!(
                        result[kind],
                        serde_json::json!({"status":"fulfilled", "identity":true, "realm":true}),
                        "{classic}/{api}/{mode}/{kind}: {result}"
                    );
                }
                let events = result["events"].as_array().unwrap();
                assert_eq!(events.len(), 2, "{classic}/{api}/{mode}: {result}");
                for event in events {
                    assert_eq!(event["ownsInfo"], event["owner"] == api, "{result}");
                    assert_eq!(event["emptyInfo"], event["owner"] != api, "{result}");
                }
                if mode == "held" && api == "child" {
                    assert_eq!(
                        result["blockedPrecommit"],
                        serde_json::json!({"name":"InvalidStateError", "realm":true, "called":false}),
                        "{result}"
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn joint_history_promoted_trackers_preserve_handler_errors_and_reentrant_calls() {
    for api in ["parent", "child"] {
        let classic = if api == "parent" { "child" } else { "parent" };
        let result = observe_joint_traversal_trackers(classic, api, "reject").await;
        for kind in ["committed", "finished"] {
            let expected = if api == "child" && kind == "committed" {
                serde_json::json!({"status":"fulfilled", "identity":true, "realm":true})
            } else {
                serde_json::json!({"status":"rejected", "marker":true, "name":"Error"})
            };
            assert_eq!(result[kind], expected, "{classic}/{api}/{kind}: {result}");
        }
        let result = observe_joint_traversal_trackers(classic, api, "reentrant").await;
        for kind in ["committed", "finished"] {
            assert_eq!(
                result[kind],
                serde_json::json!({"status":"fulfilled", "identity":true, "realm":true}),
                "{classic}/{api}/{kind}: {result}"
            );
            assert_eq!(
                result["reentrant"][kind]["name"], "InvalidStateError",
                "{classic}/{api}/{kind}: {result}"
            );
        }
        assert_eq!(result["distinctPromises"], true, "{result}");
    }
}
