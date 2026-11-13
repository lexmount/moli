use super::*;

#[tokio::test(flavor = "current_thread")]
async fn intersection_observers_track_native_documents_through_adoption() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://observer-detached-targets.test/");
    let count: usize = vm
        .eval(include_str!(
            "../../../tests/fixtures/observer-detached-targets.js"
        ))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(count, 2);
    for index in 0..count {
        let name = vm
            .eval(&format!("__observerDetached.start({index})"))
            .unwrap();
        vm.advance_timers_until_deadline_for_test(&loader)
            .await
            .unwrap();
        for _ in 0..2 {
            match vm.eval("__observerDetached.step").unwrap().as_str() {
                "adopt" => {}
                "layout" => vm.publish_layout_for_test().unwrap(),
                "" => break,
                step => panic!("unexpected observer fixture step: {step}"),
            }
            vm.eval("__observerDetached.advance()").unwrap();
            vm.advance_timers_until_deadline_for_test(&loader)
                .await
                .unwrap();
        }
        assert_eq!(
            vm.eval("__observerDetached.result").unwrap(),
            vm.eval("__observerDetached.expected").unwrap(),
            "{name}"
        );
    }
}
