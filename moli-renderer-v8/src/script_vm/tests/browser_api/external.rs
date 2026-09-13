use super::*;

#[test]
fn window_external_exposes_native_legacy_contract() {
    for url in ["https://external.test/", "http://external.test/"] {
        let mut vm = new_storage_test_vm(url);
        assert_eq!(
            vm.eval(include_str!(
                "../../../../tests/fixtures/window-external.js"
            ))
            .expect("External WebIDL contract"),
            "window-ok"
        );
    }
}

#[test]
fn window_external_is_lazy_and_uses_intrinsic_interface_identity() {
    let mut vm = new_storage_test_vm("https://external-lazy.test/");
    let materialized = |vm: &mut ScriptVm| {
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            Ok(
                crate::context_bootstrap::window_lazy_surface_diagnostics(scope)
                    .external_materialized,
            )
        })
        .expect("External cache state")
    };
    assert!(!materialized(&mut vm));
    assert_eq!(
        vm.eval(
            "globalThis.externalGetter = Object.getOwnPropertyDescriptor(window, 'external').get; window.external = 1; typeof External"
        ).unwrap(),
        "function"
    );
    assert!(!materialized(&mut vm));
    assert_eq!(
        vm.eval(
            r#"
          const intrinsic = External;
          globalThis.External = function Replacement() { throw new Error('public constructor'); };
          const object = externalGetter.call(window);
          JSON.stringify([object instanceof intrinsic, externalGetter.call(window) === object,
            window.external === 1, object.AddSearchProvider() === undefined])
        "#
        )
        .unwrap(),
        "[true,true,true,true]"
    );
    assert!(materialized(&mut vm));
}

#[test]
fn window_external_borrowed_getter_uses_receiver_realm() {
    let mut vm = new_storage_test_vm("https://external-realm.test/");
    vm.eval(
        r#"
      globalThis.externalFrame = document.createElement('iframe');
      (document.body || document.documentElement || document).append(externalFrame);
    "#,
    )
    .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "External child realm");
    assert_eq!(
        vm.eval(
            r#"
          const child = externalFrame.contentWindow;
          const getter = Object.getOwnPropertyDescriptor(window, 'external').get;
          const object = getter.call(child);
          JSON.stringify([object === child.external, object !== window.external,
            Object.getPrototypeOf(object) === child.External.prototype,
            External.prototype.AddSearchProvider.call(object) === undefined,
            child.External.prototype.IsSearchProviderInstalled.call(window.external) === undefined])
        "#
        )
        .unwrap(),
        "[true,true,true,true,true]"
    );
}

#[test]
fn window_external_stays_denied_after_cross_origin_navigation() {
    let mut vm = new_storage_test_vm("https://external-origin.test/");
    vm.exec(
        r#"
      globalThis.externalFrame = document.createElement('iframe');
      externalFrame.srcdoc = '<body>same origin</body>';
      (document.body || document.documentElement || document).append(externalFrame);
      globalThis.retainedExternalWindow = externalFrame.contentWindow;
      void retainedExternalWindow.external;
      externalFrame.src = 'data:text/html,<body>cross origin</body>';
    "#,
        None,
    )
    .unwrap();
    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval(
            r#"
          (() => {
            try { return typeof retainedExternalWindow.external; }
            catch (error) { return error.name; }
          })()
        "#
        )
        .unwrap(),
        "SecurityError"
    );
}
