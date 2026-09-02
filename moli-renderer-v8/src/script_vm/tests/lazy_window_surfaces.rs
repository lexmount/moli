use super::*;

mod rebind;
mod seeds;

fn current_surface_state(scope: &mut v8::PinScope<'_, '_>) -> (bool, bool, bool) {
    let diagnostics = crate::context_bootstrap::window_lazy_surface_diagnostics(scope);
    (
        diagnostics.navigator_materialized,
        diagnostics.performance_materialized,
        diagnostics.custom_elements_materialized,
    )
}

fn default_surface_state(vm: &mut ScriptVm) -> (bool, bool, bool) {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        Ok(current_surface_state(scope))
    })
    .expect("default Window lazy-surface diagnostics")
}

fn child_surface_state(vm: &mut ScriptVm, execution_context_id: i64) -> (bool, bool, bool) {
    let context_ptr = vm
        .child_frame_realm_context_ptr_for_execution_context_id(execution_context_id)
        .expect("child default context");
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _host_ptr| {
        Ok(current_surface_state(scope))
    })
    .expect("child Window lazy-surface diagnostics")
}

fn isolated_surface_state(vm: &mut ScriptVm, execution_context_id: i64) -> (bool, bool, bool) {
    let context_ptr = vm
        .page_isolated_world_contexts
        .context(execution_context_id)
        .map(|world| &world.context as *const v8::Global<v8::Context>)
        .expect("isolated context");
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _host_ptr| {
        Ok(current_surface_state(scope))
    })
    .expect("isolated Window lazy-surface diagnostics")
}

fn constructor_materialization_count(vm: &mut ScriptVm, name: &str) -> usize {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        Ok(crate::context_bootstrap::lazy_constructor_materialization_count(scope, name))
    })
    .expect("lazy constructor diagnostics")
}

fn default_extended_surface_state(vm: &mut ScriptVm) -> [bool; 7] {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let diagnostics = crate::context_bootstrap::window_lazy_surface_diagnostics(scope);
        Ok([
            diagnostics.navigator_materialized,
            diagnostics.performance_materialized,
            diagnostics.custom_elements_materialized,
            diagnostics.screen_materialized,
            diagnostics.crypto_materialized,
            diagnostics.visual_viewport_materialized,
            diagnostics.speech_synthesis_materialized,
        ])
    })
    .expect("extended Window lazy-surface diagnostics")
}

fn default_navigator_subobjects(vm: &mut ScriptVm) -> Vec<&'static str> {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        Ok(crate::context_bootstrap::materialized_navigator_subobject_keys(scope))
    })
    .expect("Navigator subobject diagnostics")
}

fn default_css_lazy_state(vm: &mut ScriptVm) -> (bool, bool) {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let diagnostics = crate::context_bootstrap::css_lazy_state_diagnostics(scope);
        Ok((
            diagnostics.css_materialized,
            diagnostics.highlights_materialized,
        ))
    })
    .expect("CSS lazy-state diagnostics")
}

fn default_trusted_types_materialized(vm: &mut ScriptVm) -> bool {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        Ok(crate::context_bootstrap::trusted_types_lazy_state_materialized(scope))
    })
    .expect("Trusted Types lazy-state diagnostics")
}

#[test]
fn child_default_and_isolated_bootstrap_keep_expensive_window_surfaces_lazy() {
    let mut vm = new_storage_test_vm("https://lazy-window-bootstrap.test/");
    assert_eq!(default_surface_state(&mut vm), (false, false, false));

    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          frame.id = "lazy-window-bootstrap-frame";
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })()
        "#,
    )
    .expect("child frame should be created");
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "lazy Window bootstrap");
    assert_eq!(
        child_surface_state(&mut vm, child_context_id),
        (false, false, false)
    );

    let frame_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .find(|realm| realm.context_id == child_context_id)
        .and_then(|realm| realm.frame_id)
        .expect("child frame id");
    let isolated_context_id = vm
        .create_isolated_world_for_frame(&frame_id, "lazy-window-bootstrap-isolated", false)
        .expect("child isolated world should be created");
    assert_eq!(
        isolated_surface_state(&mut vm, isolated_context_id),
        (false, false, false)
    );
    assert_eq!(default_surface_state(&mut vm), (false, false, false));

    for name in ["Navigator", "Performance", "CustomElementRegistry"] {
        assert_eq!(
            constructor_materialization_count(&mut vm, name),
            0,
            "{name} must not materialize during default or isolated child bootstrap"
        );
    }
}

#[test]
fn first_access_materializes_each_window_surface_once_in_its_realm() {
    let mut vm = new_storage_test_vm("https://lazy-window-first-access.test/");
    assert_eq!(default_surface_state(&mut vm), (false, false, false));

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const firstNavigator = navigator;
              const firstPerformance = performance;
              const firstRegistry = customElements;
              return JSON.stringify({
                navigatorSame: firstNavigator === navigator,
                navigatorPrototype:
                  Object.getPrototypeOf(firstNavigator) === Navigator.prototype,
                performanceSame: firstPerformance === performance,
                performancePrototype:
                  Object.getPrototypeOf(firstPerformance) === Performance.prototype,
                registrySame: firstRegistry === customElements,
                registryPrototype:
                  Object.getPrototypeOf(firstRegistry) ===
                    CustomElementRegistry.prototype
              });
            })()
            "#,
        )
        .expect("lazy Window surfaces should materialize"),
        r#"{"navigatorSame":true,"navigatorPrototype":true,"performanceSame":true,"performancePrototype":true,"registrySame":true,"registryPrototype":true}"#
    );
    assert_eq!(default_surface_state(&mut vm), (true, true, true));

    for name in ["Navigator", "Performance", "CustomElementRegistry"] {
        assert_eq!(
            constructor_materialization_count(&mut vm, name),
            1,
            "{name} should materialize once on first access"
        );
    }
    assert_eq!(
        constructor_materialization_count(&mut vm, "StorageManager"),
        0,
        "materializing Navigator must not eagerly build its StorageManager child"
    );

    assert_eq!(
        vm.eval("String(navigator === navigator && performance === performance && customElements === customElements)")
            .expect("repeated lazy Window surface access should evaluate"),
        "true"
    );
    for name in ["Navigator", "Performance", "CustomElementRegistry"] {
        assert_eq!(
            constructor_materialization_count(&mut vm, name),
            1,
            "{name} must preserve SameObject identity after materialization"
        );
    }
}

#[test]
fn same_realm_window_receiver_uses_the_canonical_lazy_surface_owner() {
    let mut vm = new_storage_test_vm("https://lazy-window-receiver-owner.test/");
    assert_eq!(default_surface_state(&mut vm), (false, false, false));

    let facts = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            // V8 may invoke a global-template accessor with a same-realm
            // WindowProxy/holder object as `this`. The receiver identifies the
            // realm, but only that realm's canonical global owns SameObject
            // caches, seeds, and detached callback data.
            let receiver = v8::Object::new(scope);
            let current_context = scope.get_current_context();
            anyhow::ensure!(
                receiver.get_creation_context(scope) == Some(current_context),
                "test receiver must belong to the current Window realm"
            );
            let global = current_context.global(scope);
            anyhow::ensure!(
                !receiver.strict_equals(global.into()),
                "test receiver must not be the canonical Window global"
            );

            let from_receiver = crate::context_bootstrap::ensure_window_lazy_surface_object(
                scope,
                receiver,
                crate::context_bootstrap::WindowLazySurface::Performance,
            )?;
            let from_global = crate::context_bootstrap::ensure_window_lazy_surface_object(
                scope,
                global,
                crate::context_bootstrap::WindowLazySurface::Performance,
            )?;
            let surface = crate::context_bootstrap::WindowLazySurface::Performance;
            Ok((
                from_receiver.strict_equals(from_global.into()),
                crate::util::get_private_value(scope, receiver, surface.slot()).is_some(),
                crate::util::get_private_value(scope, global, surface.slot()).is_some(),
            ))
        })
        .expect("same-realm Window receiver should resolve its canonical lazy-surface owner");

    assert_eq!(
        facts,
        (true, false, true),
        "only the canonical Window global may own the realm's Performance cache"
    );
}

#[test]
fn nested_window_surfaces_materialize_independently_and_preserve_same_object() {
    let mut vm = new_storage_test_vm("https://lazy-window-nested-surfaces.test/");
    assert_eq!(
        default_extended_surface_state(&mut vm),
        [false, false, false, false, false, false, false]
    );
    for name in [
        "Screen",
        "ScreenOrientation",
        "Crypto",
        "SubtleCrypto",
        "CryptoKey",
        "VisualViewport",
        "SpeechSynthesis",
        "SpeechSynthesisUtterance",
        "SpeechSynthesisVoice",
    ] {
        assert_eq!(
            constructor_materialization_count(&mut vm, name),
            0,
            "{name} must remain unmaterialized during blank Window bootstrap"
        );
    }

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const firstScreen = screen;
              return JSON.stringify({
                same: firstScreen === screen,
                prototype: Object.getPrototypeOf(firstScreen) === Screen.prototype,
                width: firstScreen.width
              });
            })()
            "#,
        )
        .expect("Screen should materialize on first access"),
        r#"{"same":true,"prototype":true,"width":1920}"#
    );
    assert_eq!(
        default_extended_surface_state(&mut vm),
        [false, false, false, true, false, false, false]
    );
    assert_eq!(constructor_materialization_count(&mut vm, "Screen"), 1);
    assert_eq!(
        constructor_materialization_count(&mut vm, "ScreenOrientation"),
        0,
        "Screen.orientation must be independently lazy"
    );

    assert_eq!(
        vm.eval(
            r#"
            String(
              screen.orientation === screen.orientation &&
              Object.getPrototypeOf(screen.orientation) ===
                ScreenOrientation.prototype
            )
            "#,
        )
        .expect("ScreenOrientation should materialize on first access"),
        "true"
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "ScreenOrientation"),
        1
    );

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const firstViewport = visualViewport;
              return String(
                firstViewport === visualViewport &&
                Object.getPrototypeOf(firstViewport) === VisualViewport.prototype
              );
            })()
            "#,
        )
        .expect("VisualViewport should materialize on first access"),
        "true"
    );
    assert_eq!(
        default_extended_surface_state(&mut vm),
        [false, false, false, true, false, true, false]
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "VisualViewport"),
        1
    );

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const firstCrypto = crypto;
              const bytes = firstCrypto.getRandomValues(new Uint8Array(1));
              return String(
                firstCrypto === crypto &&
                Object.getPrototypeOf(firstCrypto) === Crypto.prototype &&
                bytes.byteLength === 1
              );
            })()
            "#,
        )
        .expect("Crypto should materialize on first access"),
        "true"
    );
    assert_eq!(
        default_extended_surface_state(&mut vm),
        [false, false, false, true, true, true, false]
    );
    assert_eq!(constructor_materialization_count(&mut vm, "Crypto"), 1);
    assert_eq!(
        constructor_materialization_count(&mut vm, "SubtleCrypto"),
        0,
        "Crypto.subtle must be independently lazy"
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "CryptoKey"),
        0,
        "unused CryptoKey must remain lazy"
    );

    assert_eq!(
        vm.eval(
            r#"
            String(
              crypto.subtle === crypto.subtle &&
              Object.getPrototypeOf(crypto.subtle) === SubtleCrypto.prototype
            )
            "#,
        )
        .expect("SubtleCrypto should materialize on first access"),
        "true"
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "SubtleCrypto"),
        1
    );
    assert_eq!(constructor_materialization_count(&mut vm, "CryptoKey"), 0);

    assert_eq!(
        vm.eval(
            r#"
            String(
              speechSynthesis === speechSynthesis &&
              Object.getPrototypeOf(speechSynthesis) === SpeechSynthesis.prototype
            )
            "#,
        )
        .expect("SpeechSynthesis should materialize on first access"),
        "true"
    );
    assert_eq!(
        default_extended_surface_state(&mut vm),
        [false, false, false, true, true, true, true]
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "SpeechSynthesis"),
        1
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "SpeechSynthesisUtterance"),
        0,
        "Window.speechSynthesis must not eagerly materialize the utterance constructor"
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "SpeechSynthesisVoice"),
        0,
        "Window.speechSynthesis must not eagerly materialize the voice constructor"
    );
}

#[test]
fn css_namespace_and_highlights_materialize_in_independent_shared_cohorts() {
    let mut vm = new_storage_test_vm("https://lazy-css-runtime.test/");
    assert_eq!(default_css_lazy_state(&mut vm), (false, false));

    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify({
              css: "CSS" in globalThis,
              highlight: "Highlight" in globalThis,
              registry: "HighlightRegistry" in globalThis
            })
            "#,
        )
        .expect("CSS feature detection should evaluate"),
        r#"{"css":true,"highlight":true,"registry":true}"#
    );
    assert_eq!(
        default_css_lazy_state(&mut vm),
        (false, false),
        "feature detection must not materialize lazy CSS globals"
    );

    assert_eq!(
        vm.eval(
            r#"
            String(
              CSS === CSS &&
              typeof CSS.supports === "function" &&
              "highlights" in CSS
            )
            "#,
        )
        .expect("CSS namespace should materialize independently"),
        "true"
    );
    assert_eq!(
        default_css_lazy_state(&mut vm),
        (true, false),
        "CSS namespace access must not run the Highlight runtime"
    );

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const registry = CSS.highlights;
              const highlight = new Highlight();
              registry.set("lazy", highlight);
              return JSON.stringify({
                registrySame: registry === CSS.highlights,
                registryPrototype:
                  Object.getPrototypeOf(registry) === HighlightRegistry.prototype,
                highlightPrototype:
                  Object.getPrototypeOf(highlight) === Highlight.prototype,
                sharedEntry: registry.get("lazy") === highlight
              });
            })()
            "#,
        )
        .expect("CSS Highlights shared state should materialize"),
        r#"{"registrySame":true,"registryPrototype":true,"highlightPrototype":true,"sharedEntry":true}"#
    );
    assert_eq!(default_css_lazy_state(&mut vm), (true, true));
}

#[test]
fn trusted_types_globals_share_one_lazy_native_realm_state() {
    let mut vm = new_storage_test_vm("https://lazy-trusted-types.test/");
    assert!(!default_trusted_types_materialized(&mut vm));

    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify({
              html: "TrustedHTML" in globalThis,
              script: "TrustedScript" in globalThis,
              scriptURL: "TrustedScriptURL" in globalThis,
              factory: "trustedTypes" in globalThis
            })
            "#,
        )
        .expect("Trusted Types feature detection should evaluate"),
        r#"{"html":true,"script":true,"scriptURL":true,"factory":true}"#
    );
    assert!(
        !default_trusted_types_materialized(&mut vm),
        "feature detection must not build Trusted Types prototypes or factory"
    );

    assert_eq!(
        vm.eval("String(typeof TrustedHTML === 'function' && TrustedHTML === TrustedHTML)")
            .expect("TrustedHTML should materialize the shared realm state"),
        "true"
    );
    assert!(default_trusted_types_materialized(&mut vm));

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const factory = trustedTypes;
              const policy = factory.createPolicy("lazy-state", {
                createHTML: value => `<b>${value}</b>`,
                createScript: value => value,
                createScriptURL: value => value
              });
              const html = policy.createHTML("ok");
              const script = policy.createScript("1 + 1");
              const scriptURL = policy.createScriptURL("data:text/javascript,");
              return JSON.stringify({
                factorySame: factory === trustedTypes,
                htmlPrototype: Object.getPrototypeOf(html) === TrustedHTML.prototype,
                scriptPrototype:
                  Object.getPrototypeOf(script) === TrustedScript.prototype,
                scriptURLPrototype:
                  Object.getPrototypeOf(scriptURL) === TrustedScriptURL.prototype,
                htmlValue: String(html)
              });
            })()
            "#,
        )
        .expect("Trusted Types shared lazy state should create branded values"),
        r#"{"factorySame":true,"htmlPrototype":true,"scriptPrototype":true,"scriptURLPrototype":true,"htmlValue":"<b>ok</b>"}"#
    );
}

#[test]
fn media_source_uses_the_shared_lazy_interface_registry() {
    let mut vm = new_storage_test_vm("https://lazy-media-source.test/");
    assert_eq!(constructor_materialization_count(&mut vm, "MediaSource"), 0);
    assert_eq!(
        vm.eval("String('MediaSource' in globalThis)")
            .expect("MediaSource feature detection should evaluate"),
        "true"
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "MediaSource"),
        0,
        "feature detection must not materialize MediaSource"
    );

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const constructor = MediaSource;
              const descriptor =
                Object.getOwnPropertyDescriptor(globalThis, "MediaSource");
              return String(
                constructor === MediaSource &&
                descriptor.value === constructor &&
                typeof descriptor.get === "undefined" &&
                typeof constructor.isTypeSupported === "function"
              );
            })()
            "#,
        )
        .expect("MediaSource should materialize through shared metadata"),
        "true"
    );
    assert_eq!(constructor_materialization_count(&mut vm, "MediaSource"), 1);
}

#[test]
fn navigator_scalar_access_does_not_materialize_unused_same_object_children() {
    let mut vm = new_storage_test_vm("https://lazy-navigator-children.test/");
    assert!(default_navigator_subobjects(&mut vm).is_empty());

    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify({
              userAgent: typeof navigator.userAgent,
              language: navigator.language,
              hardwareConcurrency: typeof navigator.hardwareConcurrency
            })
            "#,
        )
        .expect("Navigator scalar properties should evaluate"),
        r#"{"userAgent":"string","language":"en-US","hardwareConcurrency":"number"}"#
    );
    assert!(
        default_navigator_subobjects(&mut vm).is_empty(),
        "scalar Navigator access must not build any nested SameObject surface"
    );
    for name in [
        "Permissions",
        "NavigatorUAData",
        "MediaDevices",
        "StorageManager",
        "StorageBucketManager",
        "PluginArray",
        "MimeTypeArray",
        "Geolocation",
        "GeolocationPositionError",
        "MediaCapabilities",
    ] {
        assert_eq!(
            constructor_materialization_count(&mut vm, name),
            0,
            "{name} must remain lazy after scalar Navigator access"
        );
    }

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const pairs = [
                ["languages", navigator.languages, navigator.languages],
                ["mimeTypes", navigator.mimeTypes, navigator.mimeTypes],
                ["plugins", navigator.plugins, navigator.plugins],
                ["connection", navigator.connection, navigator.connection],
                ["userAgentData", navigator.userAgentData, navigator.userAgentData],
                ["permissions", navigator.permissions, navigator.permissions],
                ["storage", navigator.storage, navigator.storage],
                [
                  "webkitTemporaryStorage",
                  navigator.webkitTemporaryStorage,
                  navigator.webkitTemporaryStorage
                ],
                [
                  "webkitPersistentStorage",
                  navigator.webkitPersistentStorage,
                  navigator.webkitPersistentStorage
                ],
                ["mediaDevices", navigator.mediaDevices, navigator.mediaDevices],
                ["serviceWorker", navigator.serviceWorker, navigator.serviceWorker],
                ["clipboard", navigator.clipboard, navigator.clipboard],
                ["userActivation", navigator.userActivation, navigator.userActivation],
                ["storageBuckets", navigator.storageBuckets, navigator.storageBuckets],
                ["geolocation", navigator.geolocation, navigator.geolocation],
                [
                  "mediaCapabilities",
                  navigator.mediaCapabilities,
                  navigator.mediaCapabilities
                ]
              ];
              return JSON.stringify({
                same: pairs.every(([, first, second]) => first === second),
                languagesFrozen: Object.isFrozen(navigator.languages),
                permissionsRealm:
                  Object.getPrototypeOf(navigator.permissions) ===
                    Permissions.prototype,
                mediaDevicesRealm:
                  Object.getPrototypeOf(navigator.mediaDevices) ===
                    MediaDevices.prototype,
                storageRealm:
                  Object.getPrototypeOf(navigator.storage) ===
                    StorageManager.prototype,
                geolocationRealm:
                  Object.getPrototypeOf(navigator.geolocation) ===
                    Geolocation.prototype,
                mediaCapabilitiesRealm:
                  Object.getPrototypeOf(navigator.mediaCapabilities) ===
                    MediaCapabilities.prototype
              });
            })()
            "#,
        )
        .expect("Navigator lazy subobjects should materialize"),
        r#"{"same":true,"languagesFrozen":false,"permissionsRealm":true,"mediaDevicesRealm":true,"storageRealm":true,"geolocationRealm":true,"mediaCapabilitiesRealm":true}"#
    );
    assert_eq!(
        default_navigator_subobjects(&mut vm),
        vec![
            "languages",
            "mimeTypes",
            "plugins",
            "connection",
            "userAgentData",
            "permissions",
            "storage",
            "webkitTemporaryStorage",
            "webkitPersistentStorage",
            "mediaDevices",
            "serviceWorker",
            "clipboard",
            "userActivation",
            "storageBuckets",
            "geolocation",
            "mediaCapabilities",
        ]
    );
    assert_eq!(constructor_materialization_count(&mut vm, "Geolocation"), 1);
    assert_eq!(
        constructor_materialization_count(&mut vm, "MediaCapabilities"),
        1
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "GeolocationPositionError"),
        0,
        "materializing navigator.geolocation must not eagerly build its error interface"
    );
}

#[test]
fn detached_child_screen_keeps_identity_and_zeros_geometry() {
    let mut vm = new_storage_test_vm("https://detached-child-screen.test/");
    vm.eval(
        r#"
        (() => {
          const root = document.documentElement ||
            document.appendChild(document.createElement("html"));
          const body = document.body || root.appendChild(document.createElement("body"));
          const frame = document.createElement("iframe");
          frame.id = "detached-screen-frame";
          body.appendChild(frame);
          globalThis.__heldChildWindow = frame.contentWindow;
          globalThis.__heldChildScreen = __heldChildWindow.screen;
        })()
        "#,
    )
    .expect("child Screen should materialize before frame removal");
    materialize_single_child_default_realm_for_test(&mut vm, "detached child Screen");

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              document.getElementById("detached-screen-frame").remove();
              const value = __heldChildWindow.screen;
              return JSON.stringify({
                same: value === __heldChildScreen,
                width: value.width,
                height: value.height,
                availWidth: value.availWidth,
                availHeight: value.availHeight,
                availLeft: value.availLeft,
                availTop: value.availTop,
                colorDepth: value.colorDepth,
                pixelDepth: value.pixelDepth
              });
            })()
            "#,
        )
        .expect("retained child Screen should remain readable after frame removal"),
        r#"{"same":true,"width":0,"height":0,"availWidth":0,"availHeight":0,"availLeft":0,"availTop":0,"colorDepth":24,"pixelDepth":24}"#
    );
}

#[test]
fn borrowed_navigator_getters_materialize_subobjects_in_the_receiver_realm() {
    let mut vm = new_storage_test_vm("https://lazy-navigator-borrowed-getter.test/");
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          frame.id = "lazy-navigator-subobject-frame";
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })()
        "#,
    )
    .expect("child frame should be created");
    materialize_single_child_default_realm_for_test(&mut vm, "borrowed Navigator getter");

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const child =
                document.getElementById("lazy-navigator-subobject-frame").contentWindow;
              const languagesGetter =
                Object.getOwnPropertyDescriptor(Navigator.prototype, "languages").get;
              const permissionsGetter =
                Object.getOwnPropertyDescriptor(Navigator.prototype, "permissions").get;
              const languages = languagesGetter.call(child.navigator);
              const permissions = permissionsGetter.call(child.navigator);
              return JSON.stringify({
                languagesSame: languages === child.navigator.languages,
                languagesRealm:
                  Object.getPrototypeOf(languages) === child.Array.prototype,
                permissionsSame: permissions === child.navigator.permissions,
                permissionsRealm:
                  Object.getPrototypeOf(permissions) === child.Permissions.prototype
              });
            })()
            "#,
        )
        .expect("borrowed Navigator getters should use the receiver realm"),
        r#"{"languagesSame":true,"languagesRealm":true,"permissionsSame":true,"permissionsRealm":true}"#
    );
    assert!(
        default_navigator_subobjects(&mut vm).is_empty(),
        "borrowed child getters must not materialize the parent Navigator instance"
    );
}

#[test]
fn borrowed_parent_getters_materialize_child_realm_wrappers() {
    let mut vm = new_storage_test_vm("https://lazy-window-borrowed-getter.test/");
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          frame.id = "lazy-window-borrowed-frame";
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })()
        "#,
    )
    .expect("child frame should be created");
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "borrowed Window getter");

    assert_eq!(default_surface_state(&mut vm), (false, false, false));
    assert_eq!(
        child_surface_state(&mut vm, child_context_id),
        (false, false, false)
    );
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const child =
                document.getElementById("lazy-window-borrowed-frame").contentWindow;
              const navigatorGetter =
                Object.getOwnPropertyDescriptor(globalThis, "navigator").get;
              const performanceGetter =
                Object.getOwnPropertyDescriptor(globalThis, "performance").get;
              const customElementsGetter =
                Object.getOwnPropertyDescriptor(globalThis, "customElements").get;
              const childNavigator = navigatorGetter.call(child);
              const childPerformance = performanceGetter.call(child);
              const childRegistry = customElementsGetter.call(child);
              return JSON.stringify({
                navigatorSame: childNavigator === child.navigator,
                navigatorRealm:
                  Object.getPrototypeOf(childNavigator) === child.Navigator.prototype,
                performanceSame: childPerformance === child.performance,
                performanceRealm:
                  Object.getPrototypeOf(childPerformance) === child.Performance.prototype,
                registrySame: childRegistry === child.customElements,
                registryRealm:
                  Object.getPrototypeOf(childRegistry) ===
                    child.CustomElementRegistry.prototype
              });
            })()
            "#,
        )
        .expect("borrowed parent getters should use the receiver realm"),
        r#"{"navigatorSame":true,"navigatorRealm":true,"performanceSame":true,"performanceRealm":true,"registrySame":true,"registryRealm":true}"#
    );
    assert_eq!(default_surface_state(&mut vm), (false, false, false));
    assert_eq!(
        child_surface_state(&mut vm, child_context_id),
        (true, true, true)
    );
}

#[test]
fn performance_clock_access_keeps_nested_state_lazy_until_its_own_api_is_used() {
    let mut vm = new_storage_test_vm("https://lazy-performance-children.test/");

    assert_eq!(
        vm.eval("String(Number.isFinite(performance.now()))")
            .expect("Performance clock should evaluate"),
        "true"
    );
    assert_eq!(
        default_surface_state(&mut vm),
        (false, true, false),
        "performance.now() should materialize only the top-level Performance wrapper"
    );
    for name in [
        "PerformanceTiming",
        "PerformanceNavigation",
        "EventCounts",
        "PerformanceNavigationTiming",
        "PerformanceResourceTiming",
    ] {
        assert_eq!(
            constructor_materialization_count(&mut vm, name),
            0,
            "{name} must remain lazy after performance.now()"
        );
    }
    assert_eq!(
        vm._context_host
            .borrow()
            .resource_timing_buffer_count_for_test(),
        0,
        "performance.now() must not allocate a host resource-timing buffer"
    );

    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .expect("DOMContentLoaded should update pending Performance state");
    vm.dispatch_window_load_event()
        .expect("load should update pending Performance state");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        crate::context_bootstrap::increment_performance_event_count(scope, "click");
        Ok(())
    })
    .expect("event count should update pending Performance state");
    for name in [
        "PerformanceTiming",
        "PerformanceNavigation",
        "EventCounts",
        "PerformanceNavigationTiming",
    ] {
        assert_eq!(
            constructor_materialization_count(&mut vm, name),
            0,
            "{name} must not materialize while lifecycle state is recorded"
        );
    }

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const timing = performance.timing;
              return String(
                timing === performance.timing &&
                timing.loadEventEnd >= timing.loadEventStart &&
                timing.loadEventStart >= timing.domContentLoadedEventEnd
              );
            })()
            "#,
        )
        .expect("PerformanceTiming should replay lifecycle state"),
        "true"
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "PerformanceTiming"),
        1
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "PerformanceNavigation"),
        0
    );
    assert_eq!(
        vm.eval("String(performance.navigation === performance.navigation)")
            .expect("PerformanceNavigation should materialize"),
        "true"
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "PerformanceNavigation"),
        1
    );
    assert_eq!(
        vm.eval("String(performance.eventCounts.get('click'))")
            .expect("EventCounts should replay pending counts"),
        "1"
    );
    assert_eq!(constructor_materialization_count(&mut vm, "EventCounts"), 1);

    assert_eq!(
        vm.eval("String(performance.getEntriesByType('resource').length)")
            .expect("resource-only query should evaluate"),
        "0"
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "PerformanceNavigationTiming"),
        0,
        "a resource-only query must not create the navigation entry"
    );
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const entry = performance.getEntriesByType("navigation")[0];
              return String(
                entry.loadEventEnd >= entry.loadEventStart &&
                entry.loadEventStart >= entry.domContentLoadedEventEnd
              );
            })()
            "#,
        )
        .expect("navigation query should materialize and replay the entry"),
        "true"
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "PerformanceNavigationTiming"),
        1
    );
}

#[test]
fn child_isolated_world_materializes_realm_local_wrappers_only_on_access() {
    let mut vm = new_storage_test_vm("https://lazy-window-isolated.test/");
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          frame.id = "lazy-window-isolated-frame";
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })()
        "#,
    )
    .expect("child frame should be created");
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "isolated lazy Window");
    let frame_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .find(|realm| realm.context_id == child_context_id)
        .and_then(|realm| realm.frame_id)
        .expect("child frame id");
    let isolated_context_id = vm
        .create_isolated_world_for_frame(&frame_id, "lazy-window-isolated-world", false)
        .expect("child isolated world should be created");

    assert_eq!(
        vm.eval_in_isolated_context(
            isolated_context_id,
            r#"
            (() => {
              const isolatedNavigator = navigator;
              const isolatedPerformance = performance;
              const isolatedRegistry = customElements;
              return JSON.stringify({
                navigator:
                  Object.getPrototypeOf(isolatedNavigator) === Navigator.prototype,
                performance:
                  Object.getPrototypeOf(isolatedPerformance) === Performance.prototype,
                customElements:
                  Object.getPrototypeOf(isolatedRegistry) ===
                    CustomElementRegistry.prototype
              });
            })()
            "#,
        )
        .expect("isolated lazy Window surfaces should materialize"),
        r#"{"navigator":true,"performance":true,"customElements":true}"#
    );
    assert_eq!(
        isolated_surface_state(&mut vm, isolated_context_id),
        (true, true, true)
    );
    assert_eq!(
        child_surface_state(&mut vm, child_context_id),
        (false, false, false),
        "isolated-world access must not populate the child default-world caches"
    );
    assert_eq!(default_surface_state(&mut vm), (false, false, false));
}

#[test]
fn pre_materialization_seed_and_performance_events_survive_lazy_construction() {
    const USER_AGENT: &str = "Moli-Lazy-Surface-Test/1.0";
    const RESOURCE_URL: &str = "https://lazy-window-pending.test/app.js";

    let mut vm = new_storage_test_vm("https://lazy-window-pending.test/");
    let mut fetch_config = moli_fetch::FetchConfig::default();
    fetch_config.set_user_agent(USER_AGENT);
    let loader = ResourceRequestClient::new(&fetch_config).expect("loader");
    vm.replace_document_resource_runtime(&loader);

    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .expect("DOMContentLoaded should be recorded before Performance access");
    vm.dispatch_window_load_event()
        .expect("load should be recorded before Performance access");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        crate::context_bootstrap::increment_performance_event_count(scope, "click");
        crate::context_bootstrap::record_resource_performance_entry(
            scope,
            crate::context_bootstrap::ResourcePerformanceEntry::without_network_result(
                RESOURCE_URL,
                "script",
                None,
            ),
        );
        Ok(())
    })
    .expect("pending Performance data should record");

    assert_eq!(
        default_surface_state(&mut vm),
        (false, false, false),
        "seed updates and pending telemetry must not materialize Window surfaces"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .resource_timing_buffer_count_for_test(),
        0,
        "host-side Performance buffer must remain absent before first access"
    );

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const navigation = performance.getEntriesByType("navigation")[0];
              const resource = performance.getEntriesByType("resource")[0];
              return JSON.stringify({
                userAgent: navigator.userAgent,
                lifecycleOrder:
                  navigation.domContentLoadedEventStart > 0 &&
                  navigation.domContentLoadedEventEnd >=
                    navigation.domContentLoadedEventStart &&
                  navigation.loadEventStart >=
                    navigation.domContentLoadedEventEnd &&
                  navigation.loadEventEnd >= navigation.loadEventStart,
                resource: [resource.name, resource.initiatorType],
                clickCount: performance.eventCounts.get("click")
              });
            })()
            "#,
        )
        .expect("lazy surfaces should replay their pending state"),
        format!(
            r#"{{"userAgent":"{USER_AGENT}","lifecycleOrder":true,"resource":["{RESOURCE_URL}","script"],"clickCount":1}}"#
        )
    );
    assert_eq!(
        default_surface_state(&mut vm),
        (true, true, false),
        "reading Navigator and Performance must not pull in CustomElementRegistry"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .resource_timing_buffer_count_for_test(),
        1,
        "Performance first access should create exactly one host-side resource buffer"
    );
}
