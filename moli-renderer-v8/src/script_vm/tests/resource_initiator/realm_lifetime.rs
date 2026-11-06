use super::*;

fn assert_pending_parent_promise_releases_removed_child(register: bool, known_source: bool) {
    let mut vm = new_storage_html_test_vm("https://initiator-lifetime.test/");
    vm.eval(
        r#"
        globalThis.pending = new Promise(resolve => { globalThis.releasePromise = resolve; });
        globalThis.completed = 0;
        globalThis.parentCallback = () => { completed++; };
        globalThis.install = () => { pending.then(parentCallback); };
        globalThis.frame = document.createElement('iframe');
        document.body.appendChild(frame);
        typeof frame.contentWindow.Function;
        "#,
    )
    .unwrap();
    let child = vm
        .prebootstrapped_child_default_contexts
        .borrow()
        .values()
        .next()
        .expect("the iframe should have a context")
        .context
        .clone();
    let source = Url::parse("https://initiator-lifetime.test/child.js").unwrap();
    vm.exec_in_context_ptr_runtime_turn(
        &child,
        if register {
            "globalThis.childObject = {}; parent.install();"
        } else {
            "globalThis.childObject = {};"
        },
        known_source.then_some(&source),
        0,
        true,
        false,
    )
    .unwrap();
    let (context, objects) = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(&child, |scope, _| {
            let context = scope.get_current_context();
            let global = context.global(scope);
            let object = global
                .get(scope, crate::util::v8str(scope, "Object").into())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .unwrap();
            let prototype = object
                .get(scope, crate::util::v8str(scope, "prototype").into())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .unwrap();
            let document = global
                .get(scope, crate::util::v8str(scope, "document").into())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .unwrap();
            let object = global
                .get(scope, crate::util::v8str(scope, "childObject").into())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .unwrap();
            Ok((
                v8::Weak::new(scope, context),
                [prototype, global, document, object].map(|value| v8::Weak::new(scope, value)),
            ))
        })
        .unwrap();
    vm.eval("frame.remove(); frame = null;").unwrap();
    vm.prune_stale_child_default_execution_contexts();
    drop(child);

    // No Local or Global handle, callback, or CDP object id roots the iframe.
    // The parent Promise and its parent-created reaction deliberately survive.
    for _ in 0..3 {
        vm.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                isolate.low_memory_notification();
                isolate.low_memory_notification();
            });
        assert!(
            context.is_empty(),
            "continuation metadata retained the child Context"
        );
        assert!(
            objects.iter().all(v8::Weak::is_empty),
            "continuation metadata retained child objects"
        );
    }
    assert_eq!(vm.eval("String(completed)").unwrap(), "0");
    vm.eval("releasePromise(); pending = releasePromise = parentCallback = install = null;")
        .unwrap();
    assert_eq!(
        vm.eval("String(completed)").unwrap(),
        if register { "1" } else { "0" }
    );
}

#[test]
fn resource_initiator_pending_parent_promise_does_not_retain_removed_child() {
    assert_pending_parent_promise_releases_removed_child(true, true);
}

#[test]
fn resource_initiator_unknown_continuation_does_not_retain_removed_child() {
    assert_pending_parent_promise_releases_removed_child(true, false);
}

#[test]
fn resource_initiator_removed_child_without_parent_reaction_is_collectable() {
    assert_pending_parent_promise_releases_removed_child(false, true);
}

#[test]
fn resource_initiator_unknown_registration_does_not_retain_its_creation_context() {
    let mut vm = new_storage_test_vm("https://initiator-lifetime.test/");
    let (registrations, context) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let captured = crate::resource_initiator::CallbackInitiator::capture(scope);
            Ok(([captured], v8::Weak::new(scope, context)))
        })
        .unwrap();
    for _ in 0..3 {
        vm.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(
        context.is_empty(),
        "an unknown registration retained its creation Realm"
    );
    drop(registrations);
}

#[test]
fn resource_initiator_continuation_preserves_base_url_and_encoded_control_characters() {
    let mut vm = new_storage_test_vm("https://initiator-lifetime.test/");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let source = Url::parse("https://initiator-lifetime.test/a%00.js?x=%0A#%00").unwrap();
        let base = Url::parse("https://base.test/%00/%0A/").unwrap();
        let origin = moli_url::WebOrigin::from_url(&source);
        let data = crate::resource_initiator::classic_data(scope, Some(&source));
        scope.set_continuation_preserved_embedder_data(data);
        let data = crate::util::script_base_url_continuation_data(scope, &base).unwrap();
        scope.set_continuation_preserved_embedder_data(data);
        assert_eq!(
            crate::util::script_base_url_from_continuation_data(scope),
            Some(base.clone())
        );
        assert_eq!(
            crate::resource_initiator::current_url(scope, &origin),
            source.as_str()
        );
        assert_eq!(
            crate::resource_initiator::current_url(scope, &moli_url::WebOrigin::from_url(&base)),
            ""
        );

        let data = crate::resource_initiator::classic_data(scope, None);
        scope.set_continuation_preserved_embedder_data(data);
        assert!(crate::resource_initiator::anchor(scope).is_some());
        assert_eq!(crate::resource_initiator::current_url(scope, &origin), "");
        assert_eq!(
            crate::util::script_base_url_from_continuation_data(scope),
            Some(base)
        );
        scope.set_continuation_preserved_embedder_data(v8::undefined(scope).into());
        Ok(())
    })
    .unwrap();
}
