use super::*;

#[test]
fn navigator_scheduling_exposes_native_idle_capability_contract() {
    for url in ["https://scheduling.test/", "http://scheduling.test/"] {
        let mut vm = new_storage_test_vm(url);
        assert_eq!(
            vm.eval(include_str!(
                "../../../../tests/fixtures/navigator-scheduling.js"
            ))
            .expect("Scheduling idle and WebIDL contract should hold"),
            "window-ok"
        );
    }
}

#[test]
fn navigator_scheduling_materializes_in_its_navigator_realm() {
    let mut vm = new_storage_test_vm("https://scheduling-realm.test/");
    vm.eval(
        r#"
        const frame = document.createElement('iframe');
        (document.body || document.documentElement || document).appendChild(frame);
        globalThis.__schedulingFrame = frame;
        "#,
    )
    .expect("Scheduling child setup");
    materialize_single_child_default_realm_for_test(&mut vm, "Scheduling child realm");
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const child = __schedulingFrame.contentWindow;
              const getter = Object.getOwnPropertyDescriptor(Navigator.prototype, 'scheduling').get;
              const scheduling = getter.call(child.navigator);
              return JSON.stringify({
                ownRealm: Object.getPrototypeOf(scheduling) === child.Scheduling.prototype,
                stable: scheduling === child.navigator.scheduling,
                separate: scheduling !== navigator.scheduling,
                idle: Scheduling.prototype.isInputPending.call(scheduling)
              });
            })()
            "#,
        )
        .expect("borrowed Scheduling getter should use the Navigator realm"),
        r#"{"ownRealm":true,"stable":true,"separate":true,"idle":false}"#
    );
}
