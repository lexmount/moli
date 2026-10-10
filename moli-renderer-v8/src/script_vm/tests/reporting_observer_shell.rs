use super::*;

#[test]
fn reporting_observer_supports_registration_records_and_callee_error_realms() {
    for url in [
        "https://shell-interfaces.test/",
        "http://shell-interfaces.test/",
        "http://localhost/",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm(url);
        vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
            .unwrap();
        assert_eq!(
            vm.eval(include_str!("reporting_observer_shell.js"))
                .unwrap(),
            "ok",
            "{url}"
        );
    }
}

#[test]
fn reporting_observer_registration_is_idempotent_and_uses_the_observers_relevant_global() {
    let mut vm = new_storage_page_task_executor_test_vm("https://reporting-registration.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<iframe id=child></iframe>';
        const other = document.getElementById('child').contentWindow;
        globalThis.ownObserver = new ReportingObserver(() => {});
        globalThis.otherObserver = new other.ReportingObserver(() => {});
    "#,
    )
    .unwrap();
    for (script, own_expected, other_expected) in [
        ("void 0", 0, 0),
        ("ownObserver.observe(); ownObserver.observe()", 1, 0),
        (
            "ReportingObserver.prototype.observe.call(otherObserver)",
            1,
            1,
        ),
        ("ownObserver.disconnect(); ownObserver.disconnect()", 0, 1),
        (
            "ReportingObserver.prototype.disconnect.call(otherObserver)",
            0,
            0,
        ),
    ] {
        vm.eval(script).unwrap();
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            let global = scope.get_current_context().global(scope);
            for (name, expected) in [
                ("ownObserver", own_expected),
                ("otherObserver", other_expected),
            ] {
                let key = v8::String::new(scope, name).unwrap();
                let observer =
                    v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap())
                        .unwrap();
                let context = observer.get_creation_context(scope).unwrap();
                let scope = &mut v8::ContextScope::new(scope, context);
                let owner = context.global(scope);
                let registered = crate::util::get_private_value(
                    scope,
                    owner,
                    "__moliRegisteredReportingObservers",
                )
                .and_then(|value| v8::Local::<v8::Set>::try_from(value).ok());
                assert_eq!(
                    registered.map_or(0, |set| set.size()),
                    expected,
                    "{name}: {script}"
                );
            }
            Ok(())
        })
        .unwrap();
    }
}
