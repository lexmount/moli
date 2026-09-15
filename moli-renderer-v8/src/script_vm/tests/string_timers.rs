use super::*;
use crate::script_provenance::CompiledStringProvenance;

fn execute_nonce_script(vm: &mut ScriptVm, source: &str, url: &Url, nonce: Option<&str>) {
    vm.exec_in_enclosing_script_turn_with_provenance(
        source,
        &CompiledStringProvenance::at_url(url.clone()),
        0,
        nonce,
        true,
    )
    .map_err(|error| error.into_anyhow())
    .expect("the initiating script should execute");
}

fn consume_string_timer(vm: &mut ScriptVm) {
    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("string timer should execute"),
        crate::host::HostTimeoutRunResult::Consumed
    ));
}

fn assert_import_nonce(vm: &mut ScriptVm, base_url: &Url, nonce: Option<&str>) {
    let request = vm
        .document_runtime
        .take_next_native_dynamic_module_import()
        .expect("the timer should queue a dynamic import")
        .into_dynamic_import_request();
    assert_eq!(request.specifier(), "./dependency.mjs");
    assert_eq!(request.base_url(), base_url);
    assert_eq!(request.fetch_metadata().nonce.as_deref(), nonce);
    assert!(!request.fetch_metadata().parser_inserted);
    assert!(request.fetch_metadata().integrity.is_none());
}

#[test]
fn string_timers_preserve_nonce_for_timeout_interval_and_nested_source() {
    for (source, timer_turns) in [
        (r#"setTimeout("import('./dependency.mjs')", 0);"#, 1),
        (
            r#"globalThis.__nonceInterval = setInterval("clearInterval(__nonceInterval); import('./dependency.mjs')", 0);"#,
            1,
        ),
        (
            r#"setTimeout("setTimeout(\"import('./dependency.mjs')\", 0)", 0);"#,
            2,
        ),
    ] {
        let mut vm = new_storage_test_vm("https://timer-nonce.test/page.html");
        let script_url = Url::parse("https://timer-nonce.test/scripts/initiator.js").unwrap();
        execute_nonce_script(&mut vm, source, &script_url, Some("initiating-nonce"));
        for _ in 0..timer_turns {
            consume_string_timer(&mut vm);
        }
        assert_import_nonce(&mut vm, &script_url, Some("initiating-nonce"));
    }
}

#[test]
fn string_timers_keep_nonce_snapshots_separate_for_identical_source_and_url() {
    let mut vm = new_storage_test_vm("https://timer-nonce.test/page.html");
    let script_url = Url::parse("https://timer-nonce.test/scripts/same.js").unwrap();
    let nonces = [Some("first-nonce"), None, Some("last-nonce")];
    for nonce in nonces {
        execute_nonce_script(
            &mut vm,
            r#"setTimeout("import('./dependency.mjs')", 0);"#,
            &script_url,
            nonce,
        );
    }
    for nonce in nonces {
        consume_string_timer(&mut vm);
        assert_import_nonce(&mut vm, &script_url, nonce);
    }
}

#[test]
fn string_timers_use_the_active_function_nonce_without_borrowing_the_callers_nonce() {
    for initiating_nonce in [Some("function-nonce"), None] {
        let mut vm = new_storage_test_vm("https://timer-nonce.test/page.html");
        let script_url = Url::parse("https://timer-nonce.test/scripts/function.js").unwrap();
        execute_nonce_script(
            &mut vm,
            r#"globalThis.scheduleFromInitiator = () => setTimeout("import('./dependency.mjs')", 0);"#,
            &script_url,
            initiating_nonce,
        );
        execute_nonce_script(
            &mut vm,
            "scheduleFromInitiator();",
            &Url::parse("https://timer-nonce.test/other/caller.js").unwrap(),
            Some("caller-nonce"),
        );
        consume_string_timer(&mut vm);
        assert_import_nonce(&mut vm, &script_url, initiating_nonce);
    }
}
