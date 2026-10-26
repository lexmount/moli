use super::*;

#[test]
fn detached_non_lazy_window_slots_preserve_their_captured_fallbacks() {
    let mut vm = new_storage_html_test_vm("https://detached-window-slot-boundary.test/");
    vm.eval(
        r#"
        globalThis.frame = document.createElement('iframe');
        document.body.append(frame);
        globalThis.child = frame.contentWindow;
        globalThis.oldHistory = child.history;
        globalThis.oldNavigation = child.navigation;
        "#,
    )
    .unwrap();
    let context_id = materialize_single_child_default_realm_for_test(&mut vm, "cold IndexedDB");
    let context_ptr = vm
        .child_frame_realm_context_ptr_for_execution_context_id(context_id)
        .unwrap();
    let context = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            assert_eq!(
                crate::context_bootstrap::lazy_constructor_materialization_count(
                    scope,
                    "IDBFactory"
                ),
                0
            );
            Ok(v8::Global::new(scope, scope.get_current_context()))
        })
        .unwrap();
    assert_eq!(
        vm.eval(
            r#"
            frame.remove();
            JSON.stringify([
              child.indexedDB === undefined,
              child.indexedDB === undefined,
              child.history === oldHistory,
              child.navigation === oldNavigation
            ])
            "#,
        )
        .unwrap(),
        "[true,true,true,true]"
    );
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(&context, |scope, _| {
        assert_eq!(
            crate::context_bootstrap::lazy_constructor_materialization_count(scope, "IDBFactory"),
            0,
            "an undefined captured IndexedDB fallback must not cold-start a retired runtime"
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(constructor_materialization_count(&mut vm, "IDBFactory"), 0);
}

fn all_surface_cache_state(scope: &mut v8::PinScope<'_, '_>) -> [bool; 8] {
    use crate::context_bootstrap::WindowLazySurface;
    let global = scope.get_current_context().global(scope);
    [
        WindowLazySurface::Navigator,
        WindowLazySurface::Performance,
        WindowLazySurface::Screen,
        WindowLazySurface::CustomElements,
        WindowLazySurface::Crypto,
        WindowLazySurface::VisualViewport,
        WindowLazySurface::SpeechSynthesis,
        WindowLazySurface::External,
    ]
    .map(|surface| crate::util::get_private_value(scope, global, surface.slot()).is_some())
}

fn capture_cold_child_surface_getters(
    vm: &mut ScriptVm,
    context_id: i64,
) -> v8::Global<v8::Context> {
    vm.eval(
        r#"
        globalThis.cold = {
          window: frame.contentWindow,
          names: ['navigator', 'performance', 'screen', 'customElements', 'crypto',
                  'visualViewport', 'speechSynthesis', 'external'],
          interfaces: ['Navigator', 'Performance', 'Screen', 'CustomElementRegistry',
                       'Crypto', 'VisualViewport', 'SpeechSynthesis', 'External']
        };
        cold.getters = cold.names.map(name =>
          Object.getOwnPropertyDescriptor(cold.window, name).get);
        cold.prototypes = cold.interfaces.map(name => cold.window[name].prototype);
        cold.Element = cold.window.HTMLElement;
        cold.Uint8Array = cold.window.Uint8Array;
        "#,
    )
    .unwrap();
    let context_ptr = vm
        .child_frame_realm_context_ptr_for_execution_context_id(context_id)
        .unwrap();
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        assert_eq!(all_surface_cache_state(scope), [false; 8]);
        Ok(v8::Global::new(scope, scope.get_current_context()))
    })
    .unwrap()
}

#[test]
fn detached_child_cold_surface_matrix_keeps_realm_identity_and_operations() {
    let mut vm = new_storage_html_test_vm("https://cold-detached-surface-matrix.test/");
    vm.eval("globalThis.frame = document.createElement('iframe'); document.body.append(frame); void frame.contentWindow;")
        .unwrap();
    let context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "cold surface matrix");
    let context = capture_cold_child_surface_getters(&mut vm, context_id);
    let facts = vm
        .eval(
            r#"
            frame.remove();
            (() => {
              const rows = cold.names.map((name, index) => {
                const value = cold.window[name];
                return [typeof value === 'object', value === cold.window[name],
                        Object.getPrototypeOf(value) === cold.prototypes[index],
                        Object.getPrototypeOf(value) !== globalThis[cold.interfaces[index]].prototype];
              });
              const registry = cold.window.customElements;
              class DetachedElement extends cold.Element {}
              const defineResult = registry.define('cold-detached-element', DetachedElement);
              const bytes = new cold.Uint8Array(8);
              const viewport = cold.window.visualViewport;
              const speech = cold.window.speechSynthesis;
              return JSON.stringify({
                rows,
                registryInactive: defineResult === undefined && registry.get('cold-detached-element') === undefined,
                randomValues: cold.window.crypto.getRandomValues(bytes) === bytes,
                viewport: [viewport.width, viewport.height],
                speech: [speech.pending, speech.speaking, speech.paused, speech.getVoices().length],
                externalMethods: ['AddSearchProvider', 'IsSearchProviderInstalled'].every(
                  name => typeof cold.window.external[name] === 'function')
              });
            })()
            "#,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&facts).unwrap(),
        serde_json::json!({
            "rows": vec![[true; 4]; 8],
            "registryInactive": true,
            "randomValues": true,
            "viewport": [0, 0],
            "speech": [false, false, false, 0],
            "externalMethods": true
        })
    );
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(&context, |scope, _| {
        assert_eq!(all_surface_cache_state(scope), [true; 8]);
        Ok(())
    })
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        assert_eq!(all_surface_cache_state(scope), [false; 8]);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              class ParentElement extends HTMLElement {}
              customElements.define('cold-parent-only-element', ParentElement);
              return customElements.get('cold-detached-element') === undefined &&
                cold.window.customElements.get('cold-parent-only-element') === undefined;
            })()
            "#,
        )
        .unwrap(),
        "true",
        "a detached registry must neither write nor read its parent's definitions"
    );
}

#[tokio::test]
async fn cold_old_window_getters_follow_reused_proxy_origin_and_receiver() {
    for cross_origin in [false, true] {
        let mut vm = new_storage_html_test_vm("https://cold-old-window-getters.test/");
        vm.eval(
            "globalThis.frame = document.createElement('iframe'); \
             frame.srcdoc = '<body>original</body>'; document.body.append(frame); void frame.contentWindow;",
        )
        .unwrap();
        run_child_navigation_commit_and_host_load_for_test(&mut vm, "original getter realm").await;
        let old_context_id = vm.live_child_default_runtime_realm_inventory()[0].context_id;
        capture_cold_child_surface_getters(&mut vm, old_context_id);
        vm.eval(if cross_origin {
            "frame.removeAttribute('srcdoc'); frame.src = 'data:text/html,<body>cross origin</body>'; 'queued'"
        } else {
            "frame.srcdoc = '<body>same origin replacement</body>'; 'queued'"
        })
        .unwrap();
        run_child_navigation_commit_and_host_load_for_test(&mut vm, "replacement getter realm")
            .await;
        let new_context_id = vm.live_child_default_runtime_realm_inventory()[0].context_id;
        assert_ne!(old_context_id, new_context_id);
        assert_eq!(
            vm.eval("cold.window === frame.contentWindow").unwrap(),
            "true"
        );
        let facts = vm
            .eval(
                r#"
                JSON.stringify(cold.getters.map((getter, index) => {
                  const name = cold.names[index];
                  let target;
                  try {
                    const value = getter.call(frame.contentWindow);
                    target = value === frame.contentWindow[name] &&
                      Object.getPrototypeOf(value) === frame.contentWindow[cold.interfaces[index]].prototype &&
                      Object.getPrototypeOf(value) !== cold.prototypes[index];
                  } catch (error) { target = error.name; }
                  let invalid;
                  try { getter.call({}); invalid = 'accepted'; }
                  catch (error) { invalid = error.name; }
                  return [target, invalid];
                }))
                "#,
            )
            .unwrap();
        let expected = if cross_origin {
            serde_json::json!(["SecurityError", "TypeError"])
        } else {
            serde_json::json!([true, "TypeError"])
        };
        assert_eq!(
            serde_json::from_str::<Vec<serde_json::Value>>(&facts).unwrap(),
            vec![expected; 8],
            "cross_origin={cross_origin}: stale getters must authorize the current receiver before materialization"
        );
        let new_context_ptr = vm
            .child_frame_realm_context_ptr_for_execution_context_id(new_context_id)
            .unwrap();
        vm.with_context_scope_by_ptr_and_checkpoint_for_test(new_context_ptr, |scope, _| {
            assert_eq!(all_surface_cache_state(scope), [!cross_origin; 8]);
            Ok(())
        })
        .unwrap();
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            assert_eq!(all_surface_cache_state(scope), [false; 8]);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            vm.eval(
                "JSON.stringify(cold.getters.map((getter, index) => \
                 getter.call(window) === window[cold.names[index]]))",
            )
            .unwrap(),
            "[true,true,true,true,true,true,true,true]",
            "an old getter borrowed onto the accessible parent must use the parent's instances"
        );
    }
}

#[test]
fn custom_element_definition_aborts_when_a_constructor_getter_detaches_its_realm() {
    let mut vm = new_storage_html_test_vm("https://detached-custom-element-definition.test/");
    vm.eval("globalThis.frame = document.createElement('iframe'); document.body.append(frame); void frame.contentWindow;")
        .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "reentrant custom element definition");
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const child = frame.contentWindow;
              const registry = child.customElements;
              let observed = 0;
              class DetachedElement extends child.HTMLElement {
                static get observedAttributes() {
                  observed++;
                  frame.remove();
                  return [];
                }
                attributeChangedCallback() {}
              }
              registry.define('reentrant-detached-element', DetachedElement);
              return JSON.stringify([
                observed, frame.isConnected,
                registry.get('reentrant-detached-element') === undefined,
                customElements.get('reentrant-detached-element') === undefined
              ]);
            })()
            "#,
        )
        .unwrap(),
        "[1,false,true,true]"
    );
}

fn collected_context_count(vm: &mut ScriptVm) -> usize {
    for _ in 0..2 {
        vm.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    vm.renderer_document_isolate_heap_usage()
        .unwrap()
        .number_of_native_contexts
}

#[test]
fn detached_cold_surface_objects_release_their_realm_and_native_host() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
    ] {
        let mut vm = new_storage_html_test_vm("https://cold-detached-surfaces-gc.test/");
        vm.set_layout_policy(policy);
        let baseline = collected_context_count(&mut vm);
        vm.eval("globalThis.frame = document.createElement('iframe'); document.body.append(frame); void frame.contentWindow;")
            .unwrap();
        let context_id =
            materialize_single_child_default_realm_for_test(&mut vm, "collectible cold surfaces");
        let context_ptr = vm
            .child_frame_realm_context_ptr_for_execution_context_id(context_id)
            .unwrap();
        vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            assert_eq!(all_surface_cache_state(scope), [false; 8]);
            Ok(())
        })
        .unwrap();
        vm.eval(
            r#"
            (() => {
              const child = frame.contentWindow;
              frame.remove();
              globalThis.coldValues = ['navigator', 'performance', 'screen', 'customElements',
                'crypto', 'visualViewport', 'speechSynthesis', 'external'].map(name => child[name]);
              frame = undefined;
              return 'only associated objects remain';
            })()
            "#,
        )
        .unwrap();
        vm.prune_stale_child_default_execution_contexts();
        assert_eq!(vm.child_frame_realm_store.len(), 0);
        assert!(
            collected_context_count(&mut vm) > baseline,
            "{policy:?}: author-held objects retain their Realm"
        );
        assert_eq!(
            vm.eval("coldValues.every(value => typeof value === 'object')")
                .unwrap(),
            "true"
        );
        vm.eval("coldValues = undefined; 'released'").unwrap();
        assert_eq!(
            collected_context_count(&mut vm),
            baseline,
            "{policy:?}: lazy caches must not root retired Realms"
        );
        let host = vm.context_host_weak_for_test();
        let isolate = vm.renderer_document_isolate.clone();
        drop(vm);
        for _ in 0..2 {
            isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
        }
        assert!(
            host.upgrade().is_none(),
            "{policy:?}: cold materialization must not retain the native host"
        );
    }
}
