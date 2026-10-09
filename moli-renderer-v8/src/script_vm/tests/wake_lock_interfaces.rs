use super::*;

#[test]
fn wake_lock_interfaces_validate_receivers_and_secure_realm_exposure() {
    for url in ["https://wake-lock.test/", "http://wake-lock.test/"] {
        let mut vm = new_storage_page_task_executor_test_vm(url);
        vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
            .unwrap();
        vm.eval(&format!(
            "({}).then(value => globalThis.__wakeLockDone = value, error => globalThis.__wakeLockDone = String(error));",
            include_str!("wake_lock_interfaces.js")
        ))
        .unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__wakeLockDone").unwrap(), "true", "{url}");
        if url.starts_with("https:") {
            assert_eq!(
                vm.eval("__nodeReplacementResults.platformRequests.length === 6 && __nodeReplacementResults.platformRequests.every(row => row.result === 'denied' && row.name === 'NotAllowedError' && row.code === 0)")
                    .unwrap(),
                "true",
                "No platform backend must not produce an acquired sentinel"
            );
        }
    }
}
