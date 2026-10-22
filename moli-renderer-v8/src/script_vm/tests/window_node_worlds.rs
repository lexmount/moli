use super::*;

pub(super) async fn child_node_world(vm: &mut ScriptVm, name: &str) -> i64 {
    vm.eval(
        "globalThis.frame=document.createElement('iframe');\
         frame.srcdoc='<!doctype html><body><span id=childTarget>child</span></body>';\
         (document.body||document.documentElement||document).appendChild(frame);'ready'",
    )
    .unwrap();
    run_child_navigation_commit_and_host_load_for_test(vm, "child Node world").await;
    let frame_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .and_then(|realm| realm.frame_id)
        .unwrap();
    vm.create_isolated_world_for_frame(&frame_id, name, false)
        .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn isolated_dom_wrappers_preserve_identity_when_adopted_between_windows() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = new_storage_test_vm("https://node-world-adoption.test/");
        vm.set_layout_policy(policy);
        let child = child_node_world(&mut vm, "adopted-node-world").await;
        let facts = vm
            .eval_in_isolated_context(
                child,
                r#"(() => {
                    const node=parent.document.createElement('p');
                    node.marker='before-adopt';
                    const list=node.classList, style=node.style, data=node.dataset;
                    const adopted=document.adoptNode(node);
                    document.body.append(node);
                    const childNode=document.getElementById('childTarget');
                    const text=childNode.firstChild;
                    const getter=Object.getOwnPropertyDescriptor(parent.Node.prototype,'firstChild').get;
                    return JSON.stringify({
                        same:adopted===node, marker:adopted.marker,
                        owner:node.ownerDocument===document,
                        parentPrototype:node instanceof parent.HTMLParagraphElement,
                        childPrototype:node instanceof HTMLParagraphElement,
                        query:document.querySelector('p')===node,
                        list:adopted.classList===list,
                        style:adopted.style===style,
                        data:adopted.dataset===data,
                        borrowed:getter.call(childNode)===text,
                        textPrototype:text instanceof Text
                    });
                })()"#,
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&facts).unwrap(),
            serde_json::json!({
                "same":true,"marker":"before-adopt","owner":true,
                "parentPrototype":true,"childPrototype":false,"query":true,
                "list":true,"style":true,"data":true,"borrowed":true,
                "textPrototype":true
            }),
            "{policy:?}: a native Node and its associated objects have one identity per world"
        );
        assert_eq!(
            vm.eval("String(frame.contentDocument.querySelector('p').marker===undefined)")
                .unwrap(),
            "true",
            "the default world must not inherit isolated Node expandos"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn retiring_one_node_world_member_releases_only_its_native_wrapper_roots() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = new_storage_test_vm("https://node-world-roots.test/");
        vm.set_layout_policy(policy);
        let child = child_node_world(&mut vm, "shared-node-roots").await;
        vm.eval_in_isolated_context(
            child,
            "parent.document.parentTag='survivor';document.body.childTag='retired';'ready'",
        )
        .unwrap();
        let parent = vm
            .page_isolated_world_contexts
            .contexts_with_ids()
            .find_map(|(id, world)| (!world.explicit && world.child_handle.is_none()).then_some(id))
            .unwrap();
        let (context_ptr, parent_token) = {
            let world = vm.page_isolated_world_contexts.context(parent).unwrap();
            (
                &world.context as *const _,
                world.runtime_observable_context_token,
            )
        };
        let child_token = vm
            .page_isolated_world_contexts
            .context(child)
            .unwrap()
            .runtime_observable_context_token;
        let cache = vm
            .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
                Ok(crate::native_bridge::identity::retain_context_wrapper_cache_for_test(scope))
            })
            .unwrap();
        let parent_roots = cache.strong_wrapper_entry_count_for_realm(parent_token);
        assert!(parent_roots > 0);
        assert!(
            cache.strong_wrapper_entry_count_for_realm(child_token) > 0,
            "the parent and child Contexts must observe the same world cache: {policy:?}"
        );
        vm.destroy_isolated_world_context(child);
        assert_eq!(cache.strong_wrapper_entry_count_for_realm(child_token), 0);
        assert_eq!(
            cache.strong_wrapper_entry_count_for_realm(parent_token),
            parent_roots
        );
        assert_eq!(
            vm.eval_in_isolated_context(parent, "document.parentTag")
                .unwrap(),
            "survivor"
        );
        let weak = vm.context_host_weak_for_test();
        let isolate = vm.renderer_document_isolate.clone();
        drop(vm);
        assert_eq!(cache.strong_wrapper_entry_count(), 0);
        for _ in 0..2 {
            isolate.with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
        }
        assert!(
            weak.upgrade().is_none(),
            "a retained native cache must not root the Host"
        );
    }
}

#[test]
fn collected_isolated_worlds_release_their_optional_wrapper_cache_index() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = new_storage_test_vm("https://node-world-cache-gc.test/");
        vm.set_layout_policy(policy);
        assert_eq!(
            vm._context_host
                .borrow()
                .native_bridge()
                .isolated_world_wrapper_cache_state_for_test(),
            None,
            "ordinary default Contexts must not allocate the isolated index"
        );
        for round in 0..4 {
            let world = vm
                .create_isolated_world(&format!("collected-wrapper-world-{round}"), false)
                .unwrap();
            vm.eval_in_isolated_context(world, "document.createElement('div');'ready'")
                .unwrap();
            assert_eq!(
                vm._context_host
                    .borrow()
                    .native_bridge()
                    .isolated_world_wrapper_cache_state_for_test()
                    .map(|state| state.0),
                Some(1)
            );
            vm.destroy_isolated_world_context(world);
            vm.collect_renderer_document_isolate_garbage().unwrap();
            assert_eq!(
                vm._context_host
                    .borrow()
                    .native_bridge()
                    .isolated_world_wrapper_cache_state_for_test(),
                None,
                "collected world metadata and empty capacity must be released: {policy:?}/{round}"
            );
        }
    }
}
