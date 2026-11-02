use super::*;

fn children() -> LiveCollectionDescriptor {
    LiveCollectionDescriptor {
        collection_kind: CollectionKind::NodeList,
        query_kind: LiveCollectionQueryKind::ChildNodes,
        root: DomHandle::new(0),
        query: None,
        include_root: false,
        tag_name_html_document: None,
        resolution_cache: Default::default(),
    }
}

#[test]
fn isolated_world_partitions_host_local_node_and_collection_ids() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut left = BridgeIdentityStore::default();
    let mut right = BridgeIdentityStore::default();
    let left_id = left.reflector_id(&BridgeHandle::Node(DomHandle::new(0)));
    let right_id = right.reflector_id(&BridgeHandle::Node(DomHandle::new(0)));
    assert_eq!(
        left_id, right_id,
        "reflector IDs remain dense and host-local"
    );

    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let world = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, world);
    let left_node = v8::Object::new(scope);
    let right_node = v8::Object::new(scope);
    let left_children = v8::Object::new(scope);
    let right_children = v8::Object::new(scope);
    left.cache_wrapper(scope, left_id, left_node);
    right.cache_wrapper(scope, right_id, right_node);
    left.cache_live_collection_wrapper(scope, children(), left_children);
    right.cache_live_collection_wrapper(scope, children(), right_children);
    assert_eq!(left.cached_wrapper(scope, left_id), Some(left_node));
    assert_eq!(right.cached_wrapper(scope, right_id), Some(right_node));
    assert_eq!(
        left.cached_live_collection_wrapper(scope, &children()),
        Some(left_children)
    );
    assert_eq!(
        right.cached_live_collection_wrapper(scope, &children()),
        Some(right_children)
    );
    assert!(!Rc::ptr_eq(
        &left.current_wrapper_cache(scope),
        &right.current_wrapper_cache(scope)
    ));
    assert_eq!(
        world
            .get_slot::<IsolatedWorldWrapperCaches>()
            .unwrap()
            .hosts
            .borrow()
            .len(),
        2
    );
}

#[test]
fn default_and_isolated_worlds_preserve_separate_canonical_wrappers() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut identities = BridgeIdentityStore::default();
    let id = identities.reflector_id(&BridgeHandle::Node(DomHandle::new(0)));
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let default = v8::Context::new(scope, Default::default());
    let first_world = v8::Context::new(scope, Default::default());
    let second_world = v8::Context::new(scope, Default::default());
    identities.install_default_world_wrapper_cache(default);
    let wrappers = [default, first_world, second_world].map(|world| {
        let scope = &mut v8::ContextScope::new(scope, world);
        assert!(identities.cached_wrapper(scope, id).is_none());
        let wrapper = v8::Object::new(scope);
        identities.cache_wrapper(scope, id, wrapper);
        wrapper
    });
    for (world, wrapper) in [default, first_world, second_world]
        .into_iter()
        .zip(wrappers)
    {
        let scope = &mut v8::ContextScope::new(scope, world);
        assert_eq!(identities.cached_wrapper(scope, id), Some(wrapper));
    }
    assert!(!contexts_share_wrapper_world(default, first_world));
    assert!(!contexts_share_wrapper_world(first_world, second_world));
    assert!(contexts_share_wrapper_world(first_world, first_world));
}

#[test]
fn closing_host_weakens_foreign_world_partitions_and_future_wrappers() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut producer = BridgeIdentityStore::default();
    let mut consumer = BridgeIdentityStore::default();
    let id = producer.reflector_id(&BridgeHandle::Node(DomHandle::new(0)));
    let late_id = producer.reflector_id(&BridgeHandle::Node(DomHandle::new(1)));
    let consumer_id = consumer.reflector_id(&BridgeHandle::Node(DomHandle::new(0)));
    let (worlds, retained, discarded, discarded_collection, caches, default_cache) = {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let default = v8::Context::new(scope, Default::default());
        producer.install_default_world_wrapper_cache(default);
        {
            let scope = &mut v8::ContextScope::new(scope, default);
            let node = v8::Object::new(scope);
            producer.cache_wrapper(scope, id, node);
        }
        let worlds = [
            v8::Context::new(scope, Default::default()),
            v8::Context::new(scope, Default::default()),
        ];
        let (retained, discarded_collection, first_cache) = {
            let scope = &mut v8::ContextScope::new(scope, worlds[0]);
            let node = v8::Object::new(scope);
            let collection = v8::Object::new(scope);
            producer.cache_wrapper(scope, id, node);
            producer.cache_live_collection_wrapper(scope, children(), collection);
            let consumer_node = v8::Object::new(scope);
            consumer.cache_wrapper(scope, consumer_id, consumer_node);
            (
                v8::Global::new(scope, node),
                v8::Weak::new(scope, collection),
                producer.current_wrapper_cache(scope),
            )
        };
        let (discarded, second_cache) = {
            let scope = &mut v8::ContextScope::new(scope, worlds[1]);
            let node = v8::Object::new(scope);
            producer.cache_wrapper(scope, id, node);
            (
                v8::Weak::new(scope, node),
                producer.current_wrapper_cache(scope),
            )
        };
        {
            let scope = &mut v8::ContextScope::new(scope, worlds[0]);
            producer.retire_host_wrapper_caches(scope);
            assert_eq!(
                producer.cached_wrapper(scope, id),
                Some(v8::Local::new(scope, &retained))
            );
            assert_eq!(
                consumer
                    .current_wrapper_cache(scope)
                    .borrow()
                    .wrappers
                    .len(),
                1,
                "closing a foreign host preserves this world's active host partition"
            );
        }
        (
            worlds.map(|world| v8::Global::new(scope, world)),
            retained,
            discarded,
            discarded_collection,
            [first_cache, second_cache],
            producer.default_world_wrapper_cache.clone(),
        )
    };
    for cache in caches.iter().chain([&default_cache]) {
        assert!(cache.borrow().retired);
        assert_eq!(cache.borrow().wrappers.len(), 0);
        assert!(cache.borrow().live_collection_wrappers.is_empty());
    }
    isolate.low_memory_notification();
    assert!(
        discarded.is_empty(),
        "a live foreign world cannot root discarded producer nodes"
    );
    assert!(
        discarded_collection.is_empty(),
        "collection caches cannot root the closed producer"
    );
    let late = {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        // Materialization after closure also remains weak in a new world.
        let new_world = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, new_world);
        let node = v8::Object::new(scope);
        producer.cache_wrapper(scope, late_id, node);
        let cache = producer.current_wrapper_cache(scope);
        assert!(cache.borrow().retired);
        assert_eq!(cache.borrow().wrappers.len(), 0);
        v8::Weak::new(scope, node)
    };
    isolate.low_memory_notification();
    assert!(late.is_empty());
    drop(retained);
    isolate.low_memory_notification();
    assert!(
        caches[0].borrow().retired_wrappers[&id].is_empty(),
        "JS retention, rather than the world or host cache, controls lifetime"
    );
    drop(worlds);
}
