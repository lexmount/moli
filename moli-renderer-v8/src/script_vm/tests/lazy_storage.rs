use super::*;

fn constructor_materialization_count(vm: &mut ScriptVm, name: &str) -> usize {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        Ok(crate::context_bootstrap::lazy_constructor_materialization_count(scope, name))
    })
    .expect("lazy constructor diagnostics")
}

fn total_constructor_materializations(vm: &mut ScriptVm) -> usize {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        Ok(crate::context_bootstrap::lazy_storage_constructor_materialization_count(scope))
    })
    .expect("lazy constructor diagnostics")
}

fn navigator_storage_diagnostics(vm: &mut ScriptVm) -> (bool, bool) {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        crate::context_bootstrap::navigator_storage_wrapper_diagnostics(scope)
            .ok_or_else(|| anyhow::anyhow!("Navigator diagnostics are unavailable"))
            .map(|diagnostics| {
                (
                    diagnostics.storage_manager_materialized,
                    diagnostics.storage_bucket_manager_materialized,
                )
            })
    })
    .expect("Navigator storage-wrapper diagnostics")
}

#[test]
fn blank_window_only_materializes_bootstrap_required_interface_objects() {
    const BOOTSTRAP_REQUIRED_INTERFACES: &[(&str, usize)] = &[
        // Keep the associated object available when navigator.userActivation
        // is first read after the Window's realm retires.
        ("UserActivation", 1),
        ("History", 1),
        ("Location", 1),
        ("Navigation", 1),
        ("NavigationHistoryEntry", 1),
        ("NavigationActivation", 1),
        ("EventTarget", 1),
        ("CharacterData", 1),
        ("Node", 1),
        ("Document", 1),
        ("HTMLDocument", 1),
        ("XMLDocument", 1),
        ("DocumentFragment", 1),
        ("DocumentType", 1),
        ("ShadowRoot", 1),
        ("Element", 1),
        ("HTMLElement", 1),
        ("HTMLScriptElement", 1),
        ("Text", 1),
        ("Comment", 1),
        ("ProcessingInstruction", 1),
        ("CDATASection", 1),
    ];

    let mut vm = new_storage_test_vm("https://lazy-blank-window.test/");
    let (materialized, ready_templates) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            Ok((
                crate::context_bootstrap::lazy_materialized_constructor_names(scope),
                crate::context_bootstrap::lazy_ready_constructor_template_names(scope),
            ))
        })
        .expect("lazy interface diagnostics");

    assert_eq!(
        materialized, BOOTSTRAP_REQUIRED_INTERFACES,
        "blank Window bootstrap materialization boundary changed"
    );
    let mut expected_ready_templates = materialized
        .iter()
        .map(|(name, _)| *name)
        .chain(["Window"])
        .collect::<Vec<_>>();
    expected_ready_templates.sort_unstable();
    let mut ready_templates = ready_templates;
    ready_templates.sort_unstable();
    assert_eq!(
        ready_templates, expected_ready_templates,
        "blank Window must retain only Window and templates required by materialized wrappers"
    );
}

#[test]
fn lazy_window_interface_feature_detection_does_not_materialize_constructors() {
    let mut vm = new_storage_test_vm("https://lazy-window-feature-detection.test/");
    let (interface_names, before, ready_before) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            Ok((
                crate::context_bootstrap::lazy_window_interface_names(scope),
                crate::context_bootstrap::lazy_materialized_constructor_names(scope),
                crate::context_bootstrap::lazy_ready_constructor_template_names(scope),
            ))
        })
        .expect("lazy Window interface diagnostics");
    let names_json = serde_json::to_string(&interface_names).expect("interface names JSON");

    let result = vm
        .eval(&format!(
            r#"
            (() => {{
              const names = {names_json};
              const enumerableNames = new Set(Object.keys(globalThis));
              return JSON.stringify({{
                present: names.every(name => name in globalThis),
                own: names.every(name => Object.hasOwn(globalThis, name)),
                enumerable: names.some(name => enumerableNames.has(name))
              }});
            }})()
            "#
        ))
        .expect("lazy Window feature detection should evaluate");
    assert_eq!(result, r#"{"present":true,"own":true,"enumerable":false}"#);

    let (after, ready_after) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            Ok((
                crate::context_bootstrap::lazy_materialized_constructor_names(scope),
                crate::context_bootstrap::lazy_ready_constructor_template_names(scope),
            ))
        })
        .expect("post-feature-detection lazy interface diagnostics");
    assert_eq!(
        after, before,
        "feature detection must not materialize any additional constructor"
    );
    assert_eq!(
        ready_after, ready_before,
        "feature detection must not build any additional interface template"
    );
}

#[test]
fn first_public_read_builds_one_interface_template_once() {
    let mut vm = new_storage_test_vm("https://lazy-template-first-read.test/");
    let before = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            Ok(
                crate::context_bootstrap::lazy_constructor_template_build_count(
                    scope,
                    "HTMLAreaElement",
                ),
            )
        })
        .expect("pre-read template diagnostics");
    assert_eq!(before, 0);

    assert_eq!(
        vm.eval("typeof HTMLAreaElement + ':' + (HTMLAreaElement === HTMLAreaElement)")
            .expect("HTMLAreaElement public read"),
        "function:true"
    );
    let (build_count, materialization_count) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            Ok((
                crate::context_bootstrap::lazy_constructor_template_build_count(
                    scope,
                    "HTMLAreaElement",
                ),
                crate::context_bootstrap::lazy_constructor_materialization_count(
                    scope,
                    "HTMLAreaElement",
                ),
            ))
        })
        .expect("post-read template diagnostics");
    assert_eq!(build_count, 1);
    assert_eq!(materialization_count, 1);
}

#[test]
fn first_internal_wrapper_builds_template_without_public_constructor_lookup() {
    let mut vm = new_storage_test_vm("https://lazy-template-internal-wrapper.test/");
    assert_eq!(
        vm.eval(
            r#"
            Object.defineProperty(globalThis, "HTMLAreaElement", {
              value: 17,
              writable: true,
              configurable: true
            });
            document.createElement("area");
            String(HTMLAreaElement)
            "#,
        )
        .expect("internal HTMLAreaElement wrapper creation"),
        "17"
    );
    let (build_count, materialization_count) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            Ok((
                crate::context_bootstrap::lazy_constructor_template_build_count(
                    scope,
                    "HTMLAreaElement",
                ),
                crate::context_bootstrap::lazy_constructor_materialization_count(
                    scope,
                    "HTMLAreaElement",
                ),
            ))
        })
        .expect("internal wrapper template diagnostics");
    assert_eq!(build_count, 1);
    assert_eq!(materialization_count, 1);
}

#[test]
fn every_declared_lazy_window_interface_materializes_with_data_property_shape() {
    let mut vm = new_storage_test_vm("https://lazy-all-window-interfaces.test/");
    let interface_names = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            Ok(crate::context_bootstrap::lazy_window_interface_names(scope))
        })
        .expect("lazy Window interface names");
    assert!(
        !interface_names.is_empty(),
        "the exposed-interface registry must contain lazy Window interfaces"
    );
    let names_json = serde_json::to_string(&interface_names).expect("interface names JSON");

    let result = vm
        .eval(&format!(
            r#"
            (() => {{
              const failures = [];
              for (const name of {names_json}) {{
                try {{
                  const first = globalThis[name];
                  const second = globalThis[name];
                  const descriptor =
                    Object.getOwnPropertyDescriptor(globalThis, name);
                  if (typeof first !== "function") {{
                    failures.push(`${{name}}:type:${{typeof first}}`);
                  }} else if (first !== second) {{
                    failures.push(`${{name}}:identity`);
                  }} else if (
                    !descriptor ||
                    descriptor.value !== first ||
                    descriptor.enumerable !== false ||
                    descriptor.writable !== true ||
                    descriptor.configurable !== true
                  ) {{
                    failures.push(`${{name}}:descriptor`);
                  }} else if (
                    (typeof first.prototype !== "object" ||
                      first.prototype === null)
                  ) {{
                    failures.push(`${{name}}:prototype`);
                  }}
                }} catch (error) {{
                  failures.push(
                    `${{name}}:throw:${{error && error.name}}:${{
                      error && error.message
                    }}`
                  );
                }}
              }}
              return JSON.stringify(failures);
            }})()
            "#
        ))
        .expect("all declared lazy Window interfaces should materialize");

    assert_eq!(result, "[]");
    for name in interface_names {
        assert_eq!(
            constructor_materialization_count(&mut vm, name),
            1,
            "{name} should materialize exactly once in one Window realm"
        );
    }
}

#[test]
fn many_unused_child_realms_keep_storage_surfaces_unmaterialized() {
    const CHILD_COUNT: usize = 16;

    let mut vm = new_storage_test_vm("https://lazy-unused-child-realms.test/");
    assert_eq!(
        vm.eval(&format!(
            r#"
            (() => {{
              const host = document.body || document.documentElement || document;
              globalThis.__lazyUnusedFrames = [];
              for (let index = 0; index < {CHILD_COUNT}; index += 1) {{
                const frame = document.createElement("iframe");
                host.appendChild(frame);
                const child = frame.contentWindow;
                if (!child || child.Object === Object) {{
                  throw new Error("child realm did not materialize");
                }}
                globalThis.__lazyUnusedFrames.push(frame);
              }}
              return String(globalThis.__lazyUnusedFrames.length);
            }})()
            "#
        ))
        .expect("unused child realms should materialize"),
        CHILD_COUNT.to_string()
    );
    while vm
        .run_child_realm_materialization_body_for_test()
        .expect("child realm materialization should succeed")
    {}
    assert_eq!(
        vm.live_child_default_runtime_realm_inventory().len(),
        CHILD_COUNT
    );

    assert_eq!(
        total_constructor_materializations(&mut vm),
        0,
        "creating and entering child realms must not read any Storage/OPFS interface object"
    );
    let (storage_materialized, storage_buckets_materialized) =
        navigator_storage_diagnostics(&mut vm);
    assert!(!storage_materialized);
    assert!(!storage_buckets_materialized);
    assert!(
        !vm._context_host.borrow().has_opfs_owner_state(),
        "unused child realms must not allocate shared Window OPFS owner state"
    );
}

#[test]
fn child_and_parent_realms_materialize_storage_surfaces_independently() {
    let mut vm = new_storage_test_vm("https://lazy-child-parent-realms.test/");
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              globalThis.__lazyStorageFrame = frame;
              return String(frame.contentWindow !== null);
            })()
            "#
        )
        .expect("child frame should be created"),
        "true"
    );
    materialize_single_child_default_realm_for_test(&mut vm, "lazy storage child realm");

    assert_eq!(total_constructor_materializations(&mut vm), 0);
    let (storage_materialized, storage_buckets_materialized) =
        navigator_storage_diagnostics(&mut vm);
    assert!(!storage_materialized);
    assert!(!storage_buckets_materialized);

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const child = __lazyStorageFrame.contentWindow;
              const storage = child.navigator.storage;
              globalThis.__lazyChildStorage = storage;
              return JSON.stringify({
                same: storage === child.navigator.storage,
                instance: storage instanceof child.StorageManager,
                prototype:
                  Object.getPrototypeOf(storage) === child.StorageManager.prototype
              });
            })()
            "#
        )
        .expect("child StorageManager should materialize"),
        r#"{"same":true,"instance":true,"prototype":true}"#
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "StorageManager"),
        1
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "StorageBucketManager"),
        0
    );
    let (storage_materialized, storage_buckets_materialized) =
        navigator_storage_diagnostics(&mut vm);
    assert!(
        !storage_materialized,
        "child navigator access must not fill the parent Navigator backing slot"
    );
    assert!(!storage_buckets_materialized);

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const child = __lazyStorageFrame.contentWindow;
              const directory = child.FileSystemDirectoryHandle;
              return [
                directory === child.FileSystemDirectoryHandle,
                Object.getPrototypeOf(directory) === child.FileSystemHandle,
                Object.getPrototypeOf(directory.prototype) ===
                  child.FileSystemHandle.prototype
              ].join("|");
            })()
            "#
        )
        .expect("child OPFS constructors should materialize"),
        "true|true|true"
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "FileSystemHandle"),
        1
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "FileSystemDirectoryHandle"),
        1
    );

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const child = __lazyStorageFrame.contentWindow;
              const buckets = child.navigator.storageBuckets;
              const bucketConstructor = child.StorageBucket;
              globalThis.__lazyChildStorageBuckets = buckets;
              globalThis.__lazyChildStorageBucketConstructor = bucketConstructor;
              return String(
                buckets === child.navigator.storageBuckets &&
                buckets instanceof child.StorageBucketManager &&
                typeof bucketConstructor === "function" &&
                bucketConstructor === child.StorageBucket
              );
            })()
            "#
        )
        .expect("child StorageBucketManager should materialize"),
        "true"
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "StorageBucketManager"),
        1
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "StorageBucket"),
        1
    );
    let (storage_materialized, storage_buckets_materialized) =
        navigator_storage_diagnostics(&mut vm);
    assert!(!storage_materialized);
    assert!(!storage_buckets_materialized);

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const child = __lazyStorageFrame.contentWindow;
              const base = FileSystemHandle;
              const directory = FileSystemDirectoryHandle;
              const storage = navigator.storage;
              const buckets = navigator.storageBuckets;
              const bucketConstructor = StorageBucket;
              return JSON.stringify({
                storageSame: storage === navigator.storage,
                bucketsSame: buckets === navigator.storageBuckets,
                storageDistinct: storage !== __lazyChildStorage,
                bucketsDistinct: buckets !== __lazyChildStorageBuckets,
                storageConstructorDistinct:
                  StorageManager !== child.StorageManager,
                bucketConstructorDistinct:
                  bucketConstructor !== __lazyChildStorageBucketConstructor,
                directoryConstructorDistinct:
                  directory !== child.FileSystemDirectoryHandle,
                parentInheritance:
                  Object.getPrototypeOf(directory) === base &&
                  Object.getPrototypeOf(directory.prototype) === base.prototype
              });
            })()
            "#
        )
        .expect("parent Storage/OPFS surfaces should materialize"),
        r#"{"storageSame":true,"bucketsSame":true,"storageDistinct":true,"bucketsDistinct":true,"storageConstructorDistinct":true,"bucketConstructorDistinct":true,"directoryConstructorDistinct":true,"parentInheritance":true}"#
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "StorageManager"),
        2
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "StorageBucketManager"),
        2
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "StorageBucket"),
        2
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "FileSystemHandle"),
        2
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "FileSystemDirectoryHandle"),
        2
    );
    let (storage_materialized, storage_buckets_materialized) =
        navigator_storage_diagnostics(&mut vm);
    assert!(storage_materialized);
    assert!(storage_buckets_materialized);
    assert!(
        !vm._context_host.borrow().has_opfs_owner_state(),
        "constructors and Navigator wrappers alone must not create OPFS task/handle state"
    );
}

#[test]
fn lazy_child_inheritance_uses_intrinsic_parent_after_public_override() {
    let mut vm = new_storage_test_vm("https://lazy-intrinsic-parent.test/");
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              function FakeFileSystemHandle() {}
              globalThis.FileSystemHandle = FakeFileSystemHandle;

              const directory = FileSystemDirectoryHandle;
              const intrinsicParent = Object.getPrototypeOf(directory);
              const intrinsicParentPrototype =
                Object.getPrototypeOf(directory.prototype);
              return JSON.stringify({
                publicOverridePreserved:
                  globalThis.FileSystemHandle === FakeFileSystemHandle,
                constructorParentIsIntrinsic:
                  intrinsicParent !== FakeFileSystemHandle &&
                  intrinsicParent.name === "FileSystemHandle",
                prototypeParentIsIntrinsic:
                  intrinsicParentPrototype !== FakeFileSystemHandle.prototype &&
                  intrinsicParentPrototype.constructor === intrinsicParent,
                childStable:
                  directory === FileSystemDirectoryHandle
              });
            })()
            "#
        )
        .expect("child constructor should use the hidden intrinsic parent"),
        r#"{"publicOverridePreserved":true,"constructorParentIsIntrinsic":true,"prototypeParentIsIntrinsic":true,"childStable":true}"#
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "FileSystemHandle"),
        1,
        "the hidden parent must materialize exactly once"
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "FileSystemDirectoryHandle"),
        1
    );
}

#[test]
fn lazy_dom_exception_uses_original_error_prototype_after_public_override() {
    let mut vm = new_storage_test_vm("https://lazy-dom-exception-intrinsic.test/");
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const OriginalError = Error;
              function FakeError() {}
              globalThis.Error = FakeError;

              const constructor = DOMException;
              const exception = new constructor("message", "DataError");
              return JSON.stringify({
                publicOverridePreserved: globalThis.Error === FakeError,
                prototypeUsesOriginalError:
                  Object.getPrototypeOf(constructor.prototype) ===
                    OriginalError.prototype,
                prototypeRejectsFakeError:
                  Object.getPrototypeOf(constructor.prototype) !==
                    FakeError.prototype,
                instancePrototype:
                  Object.getPrototypeOf(exception) === constructor.prototype,
                stable: constructor === DOMException
              });
            })()
            "#,
        )
        .expect("DOMException should use the captured Error intrinsic"),
        r#"{"publicOverridePreserved":true,"prototypeUsesOriginalError":true,"prototypeRejectsFakeError":true,"instancePrototype":true,"stable":true}"#
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "DOMException"),
        1
    );
}

#[test]
fn illegal_storage_receivers_do_not_materialize_wrappers_or_opfs_state() {
    let mut vm = new_storage_test_vm("https://lazy-illegal-storage-receivers.test/");
    vm.exec(
        r#"
        globalThis.__lazyIllegalReceiverResult = "pending";
        Promise.all([
          (async () => {
            try {
              await StorageManager.prototype.getDirectory.call({});
              return "getDirectory:fulfilled";
            } catch (error) {
              return "getDirectory:" + error.name;
            }
          })(),
          (async () => {
            try {
              await StorageBucketManager.prototype.keys.call({});
              return "keys:fulfilled";
            } catch (error) {
              return "keys:" + error.name;
            }
          })()
        ]).then(results => {
          globalThis.__lazyIllegalReceiverResult = results.join("|");
        });
        "#,
        None,
    )
    .expect("illegal receiver probes should schedule");
    assert_eq!(
        vm.eval("String(__lazyIllegalReceiverResult)")
            .expect("illegal receiver probes should settle"),
        "getDirectory:TypeError|keys:TypeError"
    );

    assert_eq!(
        constructor_materialization_count(&mut vm, "StorageManager"),
        1
    );
    assert_eq!(
        constructor_materialization_count(&mut vm, "StorageBucketManager"),
        1
    );
    let (storage_materialized, storage_buckets_materialized) =
        navigator_storage_diagnostics(&mut vm);
    assert!(!storage_materialized);
    assert!(!storage_buckets_materialized);
    assert!(
        !vm._context_host.borrow().has_opfs_owner_state(),
        "brand-check failures must happen before allocating OPFS owner state"
    );
}

#[test]
fn insecure_context_exposure_filter_does_not_invoke_lazy_constructors() {
    let mut vm = new_storage_test_vm("http://lazy-insecure-storage.test/");
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const child = frame.contentWindow;
              const names = [
                "StorageManager",
                "StorageEstimate",
                "StorageBucketManager",
                "StorageBucket",
                "FileSystemHandle",
                "FileSystemFileHandle",
                "FileSystemDirectoryHandle",
                "FileSystemWritableFileStream"
              ];
              return String(
                isSecureContext === false &&
                names.every(name => !(name in globalThis)) &&
                names.every(name => !(name in child)) &&
                !("storage" in navigator) &&
                !("storageBuckets" in navigator) &&
                !("storage" in child.navigator) &&
                !("storageBuckets" in child.navigator)
              );
            })()
            "#
        )
        .expect("insecure storage exposure should evaluate"),
        "true"
    );

    assert_eq!(
        total_constructor_materializations(&mut vm),
        0,
        "removing secure-context-only interfaces must not invoke their lazy getters"
    );
    let (storage_materialized, storage_buckets_materialized) =
        navigator_storage_diagnostics(&mut vm);
    assert!(!storage_materialized);
    assert!(!storage_buckets_materialized);
    assert!(!vm._context_host.borrow().has_opfs_owner_state());
}
