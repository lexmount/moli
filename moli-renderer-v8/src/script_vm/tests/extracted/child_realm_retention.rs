use super::*;
use moli_page_types::LayoutPolicy;

fn collected_native_context_count(vm: &mut StandaloneScriptVmHarness) -> usize {
    // Two collections also process weak handles cleared by the first pass.
    // No JS WeakRef is dereferenced in the collecting task.
    for _ in 0..2 {
        vm.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    vm.renderer_document_isolate_heap_usage()
        .expect("native context inventory")
        .number_of_native_contexts
}

#[test]
fn retired_child_content_surfaces_do_not_root_unreachable_realms() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let mut vm = new_parsed_test_vm(
            "https://child-content-retention.test/",
            "<!doctype html><body></body>",
        );
        vm.set_layout_policy(policy);
        let baseline = collected_native_context_count(&mut vm);
        for _ in 0..4 {
            vm.eval(
                r#"
                (() => {
                  const frame = document.createElement('iframe');
                  document.body.append(frame);
                  const child = frame.contentDocument;
                  child.body.textContent = 'retire this child';
                  if (frame.contentWindow.document !== child) throw new Error('identity');
                  frame.remove();
                  return 'removed';
                })()
                "#,
            )
            .expect("create, expose and remove child content surfaces");
            vm.prune_stale_child_default_execution_contexts();
        }
        assert_eq!(vm.child_frame_realm_store.len(), 0, "{policy:?}");
        assert_eq!(
            collected_native_context_count(&mut vm),
            baseline,
            "{policy:?}: retiring the frame must release caches that root otherwise unreachable child realms"
        );
    }
}

#[test]
fn retained_child_content_surfaces_preserve_author_state_until_released() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let mut vm = new_parsed_test_vm(
            "https://retained-child-content.test/",
            "<!doctype html><body></body>",
        );
        vm.set_layout_policy(policy);
        let baseline = collected_native_context_count(&mut vm);
        vm.eval(
            r#"
            (() => {
              const frame = document.createElement('iframe');
              document.body.append(frame);
              const child = frame.contentDocument;
              child.body.innerHTML = '<p id="kept">before</p>';
              child.authorMarker = 42;
              globalThis.savedChildDocument = child;
              globalThis.savedChildWindow = frame.contentWindow;
              savedChildWindow.authorMarker = 43;
              globalThis.savedChildNode = child.getElementById('kept');
              frame.remove();
              return 'retained';
            })()
            "#,
        )
        .expect("retain author references across child detachment");
        vm.prune_stale_child_default_execution_contexts();
        assert!(
            collected_native_context_count(&mut vm) > baseline,
            "{policy:?}: an author reference must retain its realm"
        );
        assert_eq!(
            vm.eval(
                r#"
                savedChildNode.textContent = 'after';
                JSON.stringify([
                  savedChildDocument.authorMarker,
                  savedChildWindow.authorMarker,
                  savedChildDocument.getElementById('kept') === savedChildNode,
                  savedChildNode.ownerDocument === savedChildDocument,
                  savedChildDocument.body.textContent
                ])
                "#,
            )
            .expect("retained author state remains observable"),
            r#"[42,43,true,true,"after"]"#,
            "{policy:?}"
        );
        vm.eval("savedChildDocument = savedChildWindow = savedChildNode = undefined; 'released'")
            .expect("release the last author references");
        assert_eq!(
            collected_native_context_count(&mut vm),
            baseline,
            "{policy:?}: caches must not outlive the last author reference"
        );
    }
}

#[test]
fn promoted_child_content_surfaces_follow_author_reachability() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let mut vm = new_parsed_test_vm(
            "https://promoted-child-content.test/",
            "<!doctype html><body></body>",
        );
        vm.set_layout_policy(policy);
        let baseline = collected_native_context_count(&mut vm);
        vm.eval(
            r#"
            (() => {
              globalThis.savedFrame = document.createElement('iframe');
              document.body.append(savedFrame);
              const child = savedFrame.contentDocument;
              child.authorMarker = 42;
              child.body.innerHTML = '<p id="kept">before</p>';
              globalThis.savedChildNode = child.getElementById('kept');
              savedChildNode.authorMarker = 43;
              return 'live';
            })()
            "#,
        )
        .expect("expose live child surfaces");
        assert!(vm.run_child_realm_materialization_body_for_test().unwrap());
        assert_eq!(vm.child_frame_realm_store.len(), 1);
        assert!(collected_native_context_count(&mut vm) > baseline);
        assert_eq!(
            vm.eval(
                "savedFrame.contentDocument === savedChildNode.ownerDocument && \
                 savedFrame.contentWindow.document === savedChildNode.ownerDocument",
            )
            .unwrap(),
            "true",
            "{policy:?}: live registry owns the surfaces across GC"
        );
        vm.eval("savedFrame.remove(); savedFrame = undefined; 'removed'")
            .unwrap();
        vm.prune_stale_child_default_execution_contexts();
        assert_eq!(vm.child_frame_realm_store.len(), 0);
        assert!(collected_native_context_count(&mut vm) > baseline);
        assert_eq!(
            vm.eval(
                r#"
                savedChildNode.textContent = 'after';
                JSON.stringify([
                  savedChildNode.authorMarker,
                  savedChildNode.ownerDocument.authorMarker,
                  savedChildNode.ownerDocument.getElementById('kept') === savedChildNode,
                  savedChildNode.ownerDocument.body.textContent
                ])
                "#,
            )
            .unwrap(),
            r#"[43,42,true,"after"]"#,
            "{policy:?}: retaining only a Node preserves its Document and identity"
        );
        vm.eval("savedChildNode = undefined; 'released'").unwrap();
        assert_eq!(
            collected_native_context_count(&mut vm),
            baseline,
            "{policy:?}"
        );
    }
}

#[test]
fn inert_iframe_remains_windowless_across_gc() {
    let mut vm = new_storage_test_vm("https://inert-content-cache.test/");
    assert_eq!(
        vm.eval(
            r#"
        const detached = document.implementation.createHTMLDocument('');
        globalThis.savedDetachedFrame = detached.createElement('iframe');
        savedDetachedFrame.srcdoc = '<p>inert</p>';
        detached.body.append(savedDetachedFrame);
        JSON.stringify([savedDetachedFrame.contentDocument, savedDetachedFrame.contentWindow])
    "#
        )
        .unwrap(),
        "[null,null]"
    );
    for _ in 0..2 {
        vm.collect_renderer_document_isolate_garbage().unwrap();
    }
    vm.notify_renderer_document_isolate_memory_pressure()
        .unwrap();
    assert_eq!(
        vm.eval(
            r#"
        JSON.stringify([
          savedDetachedFrame.contentDocument,
          savedDetachedFrame.contentWindow,
          savedDetachedFrame.ownerDocument.querySelector('iframe') === savedDetachedFrame
        ])
    "#
        )
        .unwrap(),
        "[null,null,true]"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .detached_content_surface_cache_counts_for_test(),
        [0; 6]
    );
}

fn churn_child_content_surfaces_for_inventory(vm: &mut StandaloneScriptVmHarness, count: usize) {
    for _ in 0..count {
        vm.eval(
            r#"
            (() => {
              const frame = document.createElement('iframe');
              document.body.append(frame);
              const child = frame.contentDocument;
              child.body.textContent = 'retire this child';
              if (frame.contentWindow.document !== child) throw new Error('identity');
              frame.remove();
              return 'removed';
            })()
            "#,
        )
        .expect("create, expose and normally detach child content surfaces");
        vm.prune_stale_child_default_execution_contexts();
    }
}

fn detached_content_surface_inventory(vm: &StandaloneScriptVmHarness) -> [usize; 6] {
    vm._context_host
        .borrow()
        .detached_content_surface_cache_counts_for_test()
}

#[test]
fn collected_child_content_surface_caches_release_entries_and_capacity() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let mut vm = new_parsed_test_vm(
            "https://weak-content-inventory.test/",
            "<!doctype html><body></body>",
        );
        vm.set_layout_policy(policy);
        let baseline_contexts = collected_native_context_count(&mut vm);
        assert_eq!(detached_content_surface_inventory(&vm), [0; 6]);
        for _ in 0..2 {
            churn_child_content_surfaces_for_inventory(&mut vm, 4);
            assert_eq!(vm.child_frame_realm_store.len(), 0, "{policy:?}");
            assert_eq!(
                collected_native_context_count(&mut vm),
                baseline_contexts,
                "{policy:?}: a dead Weak entry is not a live Realm"
            );
            vm.collect_renderer_document_isolate_garbage()
                .expect("explicit collection also cleans native child surface caches");
            assert_eq!(
                detached_content_surface_inventory(&vm),
                [0; 6],
                "{policy:?}: release dead Weak control blocks and empty backing storage"
            );
        }
    }
}

#[test]
fn child_content_surface_maintenance_preserves_author_identity() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let mut vm = new_parsed_test_vm(
            "https://weak-content-author-identity.test/",
            "<!doctype html><body></body>",
        );
        vm.set_layout_policy(policy);
        let baseline_contexts = collected_native_context_count(&mut vm);
        assert_eq!(
            detached_content_surface_inventory(&vm),
            [0; 6],
            "fresh fixture has no child surface cache"
        );
        vm.eval(
            r#"
            (() => {
              const frame = document.createElement('iframe');
              document.body.append(frame);
              const child = frame.contentDocument;
              child.body.innerHTML = '<p id="kept">before</p>';
              child.authorMarker = 42;
              globalThis.savedChildDocument = child;
              globalThis.savedChildWindow = frame.contentWindow;
              savedChildWindow.authorMarker = 43;
              globalThis.savedChildNode = child.getElementById('kept');
              frame.remove();
              return 'retained';
            })()
            "#,
        )
        .expect("hold author-owned content surfaces across normal child detachment");
        vm.prune_stale_child_default_execution_contexts();
        churn_child_content_surfaces_for_inventory(&mut vm, 4);
        assert!(collected_native_context_count(&mut vm) > baseline_contexts);
        vm.notify_renderer_document_isolate_memory_pressure()
            .expect("existing owner maintenance prunes already collected targets");
        assert_eq!(
            detached_content_surface_inventory(&vm)[..4],
            [1, 0, 1, 0],
            "{policy:?}: keep only the author-held Window and Document identities"
        );
        assert_eq!(
            vm.eval(
                r#"
                savedChildNode.textContent = 'after';
                JSON.stringify([
                  savedChildDocument.authorMarker,
                  savedChildWindow.authorMarker,
                  savedChildDocument.getElementById('kept') === savedChildNode,
                  savedChildNode.ownerDocument === savedChildDocument,
                  savedChildWindow.document === savedChildDocument,
                  savedChildDocument.body.textContent
                ])
                "#,
            )
            .expect("GC of unrelated children preserves author-held identity and state"),
            r#"[42,43,true,true,true,"after"]"#,
            "{policy:?}"
        );
        vm.eval("savedChildDocument = savedChildWindow = savedChildNode = undefined; 'released'")
            .expect("release the remaining author references");
        assert_eq!(collected_native_context_count(&mut vm), baseline_contexts);
        vm.notify_renderer_document_isolate_memory_pressure()
            .expect("reclaim native cache storage after the last author reference");
        assert_eq!(
            detached_content_surface_inventory(&vm),
            [0; 6],
            "{policy:?}: maintenance releases dead entries and backing storage"
        );
    }
}
