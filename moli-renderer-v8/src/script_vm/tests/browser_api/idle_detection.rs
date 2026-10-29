use super::*;

fn idle_permission_override() -> crate::protocol_types::PermissionOverrideRegistration {
    crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::Value::String("idleDetection".to_owned()),
        setting: "granted".to_owned(),
        origin: None,
        embedded_origin: None,
    }
}

fn assert_idle_fixture(vm: &mut StandaloneScriptVmHarness, source: &str) {
    vm.eval(&format!(
        "globalThis.idleFixtureResult='pending'; ({}).then(value=>idleFixtureResult=value,error=>idleFixtureResult=String(error.stack||error));",
        source,
    ))
    .unwrap();
    vm.eval("0").unwrap();
    assert_eq!(vm.eval("String(idleFixtureResult)").unwrap(), "true");
}

#[test]
fn idle_options_convert_before_state_permission_and_abort_algorithms() {
    let mut vm = new_storage_test_vm("https://idle-options.test/");
    assert_idle_fixture(
        &mut vm,
        include_str!("../../../../tests/fixtures/idle-options-dictionary.js"),
    );
}

#[test]
fn idle_abort_stops_observation_and_preserves_other_registered_detectors() {
    let mut vm = new_storage_test_vm("https://idle-abort.test/");
    vm.set_permission_overrides(&[idle_permission_override()]);
    assert_idle_fixture(
        &mut vm,
        include_str!("../../../../tests/fixtures/idle-abort-runtime.js"),
    );
    vm.set_idle_override_and_sync_surface(Some(crate::protocol_types::EmulatedIdleOverride {
        is_user_active: false,
        is_screen_unlocked: false,
    }))
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([idleStopped.userState,idleStopped.screenState,idleActive.userState,idleActive.screenState,idleStoppedChanges,idleActiveChanges])").unwrap(),
        r#"[null,null,"idle","locked",0,1]"#
    );
    vm.eval("idleActiveController.abort()").unwrap();
    vm.set_idle_override_and_sync_surface(None).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([idleStopped.userState,idleActive.userState,idleStoppedChanges,idleActiveChanges])").unwrap(),
        "[null,null,0,1]"
    );
}

#[test]
fn idle_observer_and_signal_algorithm_retire_with_the_detector_realm() {
    let mut vm = new_storage_html_test_vm("https://idle-realm.test/");
    vm.set_permission_overrides(&[idle_permission_override()]);
    assert_idle_fixture(
        &mut vm,
        include_str!("../../../../tests/fixtures/idle-realm-lifetime.js"),
    );
}

#[test]
fn idle_detector_observes_devtools_override_and_clear() {
    let mut vm = new_storage_test_vm("https://idle-detector.test/");
    vm.set_permission_overrides(&[idle_permission_override()]);

    let initial = vm
        .eval(
            r#"
globalThis.idleEvents = [];
globalThis.idleDetector = new IdleDetector();
idleDetector.addEventListener('change', () => {
  idleEvents.push(`${idleDetector.userState}/${idleDetector.screenState}`);
});
const before = [idleDetector.userState, idleDetector.screenState];
const started = idleDetector.start();
JSON.stringify({
  before,
  promise: Object.prototype.toString.call(started),
  state: [idleDetector.userState, idleDetector.screenState],
  events: idleEvents,
});
"#,
        )
        .expect("IdleDetector should start with a granted CDP permission override");
    assert_eq!(
        initial,
        r#"{"before":[null,null],"promise":"[object Promise]","state":["active","unlocked"],"events":["active/unlocked"]}"#
    );

    vm.set_idle_override_and_sync_surface(Some(crate::protocol_types::EmulatedIdleOverride {
        is_user_active: false,
        is_screen_unlocked: false,
    }))
    .expect("idle override should update the top-level realm");
    assert_eq!(
        vm.eval(
            "JSON.stringify({state:[idleDetector.userState,idleDetector.screenState],events:idleEvents})"
        )
        .expect("overridden idle state should be observable"),
        r#"{"state":["idle","locked"],"events":["active/unlocked","idle/locked"]}"#
    );

    vm.set_idle_override_and_sync_surface(Some(crate::protocol_types::EmulatedIdleOverride {
        is_user_active: false,
        is_screen_unlocked: false,
    }))
    .expect("reapplying an idle override should succeed");
    assert_eq!(
        vm.eval("String(idleEvents.length)")
            .expect("unchanged override should not dispatch another event"),
        "2"
    );

    vm.set_idle_override_and_sync_surface(None)
        .expect("clearing idle override should restore the real headless state");
    assert_eq!(
        vm.eval(
            "JSON.stringify({state:[idleDetector.userState,idleDetector.screenState],events:idleEvents})"
        )
        .expect("cleared idle state should be observable"),
        r#"{"state":["active","unlocked"],"events":["active/unlocked","idle/locked","active/unlocked"]}"#
    );
}

#[test]
fn idle_detector_enforces_permission_threshold_and_single_start() {
    let mut vm = new_storage_test_vm("https://idle-detector-validation.test/");
    let denied = vm
        .eval(
            r#"
globalThis.deniedDetector = new IdleDetector();
deniedDetector.start().catch(error => {
  globalThis.deniedResult = `${error.name}:${error.message}`;
});
"denied-pending";
"#,
        )
        .expect("denied IdleDetector start should return a rejected promise");
    assert_eq!(denied, "denied-pending");
    vm.eval("0")
        .expect("permission rejection microtasks should run");
    assert_eq!(
        vm.eval("deniedResult")
            .expect("permission rejection should settle"),
        "NotAllowedError:Idle detection permission denied"
    );

    vm.set_permission_overrides(&[idle_permission_override()]);
    let validation = vm
        .eval(
            r#"
globalThis.validationResults = [];
globalThis.validationDetector = new IdleDetector();
validationDetector.start({threshold: 59999}).catch(error => validationResults.push(error.name + ':' + error.message));
validationDetector.start().then(() => validationDetector.start()).catch(error => validationResults.push(error.name + ':' + error.message));
"validation-pending";
"#,
        )
        .expect("IdleDetector validation calls should return promises");
    assert_eq!(validation, "validation-pending");
    vm.eval("0").expect("validation microtasks should run");
    assert_eq!(
        vm.eval("JSON.stringify(validationResults)")
            .expect("validation promises should settle"),
        r#"["TypeError:Minimum threshold is 1 minute.","InvalidStateError:Idle detector is already started."]"#
    );
}
