use super::*;

fn measurement_vm() -> StandaloneScriptVmHarness {
    let mut vm = new_storage_html_test_vm("https://resize-measurement.test/");
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(
        r#"
        for (const id of ['content', 'border', 'device']) {
            const target = document.createElement('div');
            target.id = id;
            target.style.cssText = 'width:80px;height:30px;padding:5px;border:2px solid;box-sizing:content-box';
            document.body.append(target);
        }
        window.deliveries=[];
        "#,
    )
    .unwrap();
    vm.publish_layout_for_test().unwrap();
    vm
}

fn observe_in_current_turn<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let source = crate::util::v8str(
        scope,
        r#"
        window.observer = new ResizeObserver(entries => deliveries.push(entries.map(e =>
            [e.target.id, e.contentBoxSize[0].inlineSize, e.borderBoxSize[0].inlineSize]
        )));
        observer.observe(document.getElementById('content'));
        observer.observe(document.getElementById('border'), {box:'border-box'});
        observer.observe(document.getElementById('device'), {box:'device-pixel-content-box'});
        observer
        "#,
    );
    let value = v8::Script::compile(scope, source, None)
        .and_then(|script| crate::script_execution::execute_compiled_script(scope, script))
        .expect("register ResizeObserver without completing the current JS turn");
    v8::Local::<v8::Object>::try_from(value).unwrap()
}

#[test]
fn resize_observer_sampling_does_not_consume_pending_delivery() {
    let mut vm = measurement_vm();
    let builds = vm.layout_pass_observability_for_test().1;
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let observer = observe_in_current_turn(scope);
        let expected = vec![(80.0, 30.0), (94.0, 44.0), (80.0, 30.0)];
        for _ in 0..2 {
            assert_eq!(
                crate::context_bootstrap::resize_observer_measurements_for_test(
                    scope,
                    observer,
                    &[],
                )?,
                expected,
                "gather must leave observations active until broadcast"
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(deliveries)").unwrap(),
        r#"[[["content",80,94],["border",80,94],["device",80,94]]]"#,
        "native sampling must not consume the pending legacy callback"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    assert_eq!(vm.eval("observer.takeRecords().length").unwrap(), "0");
}

#[test]
fn resize_observer_reporting_commits_only_selected_measurements() {
    let mut vm = measurement_vm();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let observer = observe_in_current_turn(scope);
        assert_eq!(
            crate::context_bootstrap::resize_observer_measurements_for_test(scope, observer, &[1])?,
            vec![(80.0, 30.0), (94.0, 44.0), (80.0, 30.0)]
        );
        assert_eq!(
            crate::context_bootstrap::resize_observer_measurements_for_test(scope, observer, &[])?,
            vec![(80.0, 30.0), (80.0, 30.0)],
            "unbroadcast observations must remain active"
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(deliveries)").unwrap(),
        r#"[[["content",80,94],["device",80,94]]]"#
    );
    assert_eq!(vm.eval("observer.takeRecords().length").unwrap(), "0");
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_entries_preserve_webidl_snapshots_and_callback_realms() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://resize-entries.test/");
    let count: usize = vm
        .eval(include_str!(
            "../../../tests/fixtures/resize-observer-entry-webidl.js"
        ))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(count, 12);
    vm.eval(
        "__resizeEntryChecks.resumeAfterLayout = null;
         __resizeEntryChecks.publishLayout = () => new Promise(resolve => {
           __resizeEntryChecks.resumeAfterLayout = resolve;
         });",
    )
    .unwrap();
    for index in 0..count {
        let name = vm
            .eval(&format!("__resizeEntryChecks.start({index})"))
            .unwrap();
        let mut publications = 0;
        loop {
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(__resizeEntryChecks.result !== null || __resizeEntryChecks.resumeAfterLayout !== null)",
                "true",
                &name,
            )
            .await;
            if vm.eval("__resizeEntryChecks.result !== null").unwrap() == "true" {
                break;
            }
            publications += 1;
            assert!(publications <= 2, "{name}: unexpected layout request");
            vm.publish_layout_for_test().unwrap();
            vm.eval(
                "(() => {
                   const resume = __resizeEntryChecks.resumeAfterLayout;
                   __resizeEntryChecks.resumeAfterLayout = null;
                   resume();
                 })()",
            )
            .unwrap();
        }
        let result = vm.eval("__resizeEntryChecks.result").unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(result["total"].as_u64().unwrap() > 0, "{name}");
        assert_eq!(result["failures"], serde_json::json!([]), "{name}");
    }
}
