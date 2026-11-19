use super::*;

#[tokio::test]
async fn initial_about_blank_rebind_keeps_child_window_surfaces_lazy() {
    let mut vm = new_storage_test_vm("https://lazy-window-rebind.test/");
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          frame.id = "lazy-window-rebind-frame";
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })()
        "#,
    )
    .expect("initial about:blank child should be created");
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "lazy Window rebind");
    let frame_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .find(|realm| realm.context_id == child_context_id)
        .and_then(|realm| realm.frame_id)
        .expect("child frame id");
    let isolated_context_id = vm
        .create_isolated_world_for_frame(&frame_id, "lazy-window-rebind-isolated", false)
        .expect("child isolated world should be created");

    assert_eq!(
        child_surface_state(&mut vm, child_context_id),
        (false, false, false)
    );
    assert_eq!(
        isolated_surface_state(&mut vm, isolated_context_id),
        (false, false, false)
    );

    vm.eval(
        r#"
        document.getElementById("lazy-window-rebind-frame").srcdoc =
          "<!doctype html><body><p id='committed-marker'>committed</p></body>";
        "navigating"
        "#,
    )
    .expect("child srcdoc navigation should start");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "lazy Window surface rebind").await;

    assert_eq!(
        child_surface_state(&mut vm, child_context_id),
        (false, false, false),
        "default-world document rebind must update seeds without creating wrappers"
    );
    assert_eq!(
        isolated_surface_state(&mut vm, isolated_context_id),
        (false, false, false),
        "isolated-world document rebind must update seeds without creating wrappers"
    );
    assert_eq!(default_surface_state(&mut vm), (false, false, false));

    assert_eq!(
        vm.eval_in_isolated_context(
            isolated_context_id,
            r#"
            (() => {
              class ReboundElement extends HTMLElement {}
              customElements.define("lazy-rebound-element", ReboundElement);
              const element = document.createElement("lazy-rebound-element");
              return JSON.stringify({
                document: document.getElementById("committed-marker").textContent,
                navigationName:
                  performance.getEntriesByType("navigation")[0].name,
                navigatorRealm:
                  Object.getPrototypeOf(navigator) === Navigator.prototype,
                performanceRealm:
                  Object.getPrototypeOf(performance) === Performance.prototype,
                registryRealm:
                  Object.getPrototypeOf(customElements) ===
                    CustomElementRegistry.prototype,
                customElementUsesReboundDocument:
                  Object.getPrototypeOf(element) === ReboundElement.prototype
              });
            })()
            "#,
        )
        .expect("isolated child surfaces should materialize after document rebind"),
        r#"{"document":"committed","navigationName":"about:srcdoc","navigatorRealm":true,"performanceRealm":true,"registryRealm":true,"customElementUsesReboundDocument":true}"#
    );
    assert_eq!(
        isolated_surface_state(&mut vm, isolated_context_id),
        (true, true, true)
    );
    assert_eq!(
        child_surface_state(&mut vm, child_context_id),
        (false, false, false),
        "isolated access must not populate the default child realm"
    );

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const child =
                document.getElementById("lazy-window-rebind-frame").contentWindow;
              return JSON.stringify({
                navigationName:
                  child.performance.getEntriesByType("navigation")[0].name,
                navigatorRealm:
                  Object.getPrototypeOf(child.navigator) === child.Navigator.prototype,
                performanceRealm:
                  Object.getPrototypeOf(child.performance) ===
                    child.Performance.prototype,
                registryRealm:
                  Object.getPrototypeOf(child.customElements) ===
                    child.CustomElementRegistry.prototype
              });
            })()
            "#,
        )
        .expect("default child surfaces should materialize after document rebind"),
        r#"{"navigationName":"about:srcdoc","navigatorRealm":true,"performanceRealm":true,"registryRealm":true}"#
    );
    assert_eq!(
        child_surface_state(&mut vm, child_context_id),
        (true, true, true)
    );
    assert_eq!(
        default_surface_state(&mut vm),
        (false, false, false),
        "child access must not populate the top Window caches"
    );
}
