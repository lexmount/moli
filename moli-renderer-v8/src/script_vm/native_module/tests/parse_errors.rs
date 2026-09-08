use super::super::{module_load_error_value, retain_module_exception};
use super::*;

fn compile_parse_error(vm: &mut ScriptVm, path: &str) -> ModuleLoadError {
    let url = Url::parse(path).unwrap();
    vm.compile_native_module_record(
        ModuleMapKey::java_script(url.clone()),
        &ModuleSource::text("export const = ;".to_owned()),
        &url,
        &ModuleFetchMetadata::default(),
    )
    .expect_err("the module must have a syntax error")
}

fn install_module(vm: &mut ScriptVm, path: &str, source: &str) {
    let url = Url::parse(path).unwrap();
    let key = ModuleMapKey::java_script(url.clone());
    let source = ModuleSource::text(source.to_owned());
    let metadata = ModuleFetchMetadata::default();
    let (record, identity) = vm
        .compile_native_module_record(key.clone(), &source, &url, &metadata)
        .unwrap();
    vm.document_runtime
        .insert_native_module_source(key.clone(), source);
    vm.document_runtime
        .insert_native_compiled_module_record_with_metadata(key, record, identity, metadata);
}

fn graph_error(vm: &mut ScriptVm, specifier: &str) -> ModuleLoadError {
    let mut job = dynamic_import_job_in_vm(
        vm,
        specifier,
        Url::parse("https://module-errors.test/page.html").unwrap(),
        ModuleImportPhase::Evaluation,
    );
    job.advance_dynamic_import_owner_lane(vm)
        .err()
        .expect("module graph should fail")
}

fn reject_with_error(vm: &mut ScriptVm, error: &ModuleLoadError) -> v8::Global<v8::Value> {
    let request = dynamic_import_request_in_vm(
        vm,
        "./bad.mjs",
        Url::parse("https://module-errors.test/page.html").unwrap(),
        ModuleImportPhase::Evaluation,
    );
    let resolver = request.resolver().clone();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(scope, &vm.page_default_context);
            let scope = &mut v8::ContextScope::new(scope, context);
            v8::Local::new(scope, &resolver)
                .get_promise(scope)
                .mark_as_handled();
            Ok(())
        })
        .unwrap();
    vm.reject_native_dynamic_module_import_with_error_selected_task_body(request, error)
        .unwrap();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(scope, &vm.page_default_context);
            let scope = &mut v8::ContextScope::new(scope, context);
            let promise = v8::Local::new(scope, &resolver).get_promise(scope);
            assert_eq!(promise.state(), v8::PromiseState::Rejected);
            Ok(v8::Global::new(scope, promise.result(scope)))
        })
        .unwrap()
}

#[test]
fn module_parse_error_preserves_exception_identity_without_merging_equal_messages() {
    let mut vm = new_test_vm("https://module-errors.test/page.html");
    let error = compile_parse_error(&mut vm, "https://module-errors.test/first.mjs");
    let other_error = compile_parse_error(&mut vm, "https://module-errors.test/second.mjs");
    let first = reject_with_error(&mut vm, &error);
    let again = reject_with_error(&mut vm, &error.clone());
    let other = reject_with_error(&mut vm, &other_error);
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let first = v8::Local::new(scope, &first);
        let again = v8::Local::new(scope, &again);
        let other = v8::Local::new(scope, &other);
        assert!(first.is_native_error());
        assert!(
            first.strict_equals(again),
            "cloned load errors must retain the JS exception"
        );
        assert!(
            !first.strict_equals(other),
            "separate modules must retain separate exceptions"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn module_parse_error_identity_survives_document_open_in_the_same_realm() {
    let mut vm = new_test_vm("https://module-errors.test/page.html");
    let error = compile_parse_error(&mut vm, "https://module-errors.test/bad.mjs");
    let first = reject_with_error(&mut vm, &error);
    vm.eval("document.open(); document.write('<p>replacement</p>'); document.close(); 'done'")
        .unwrap();
    let again = reject_with_error(&mut vm, &error);
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let first = v8::Local::new(scope, &first);
        let again = v8::Local::new(scope, &again);
        assert!(
            first.strict_equals(again),
            "document.open must retain the realm's module errors"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn module_parse_error_static_resolution_is_shared_across_roots_but_not_modules() {
    let mut vm = new_test_vm("https://module-errors.test/page.html");
    for (name, source) in [
        ("bad.mjs", "import 'unmapped';"),
        ("other-bad.mjs", "import 'unmapped';"),
        ("first.mjs", "import './bad.mjs';"),
        ("second.mjs", "import './bad.mjs';"),
    ] {
        install_module(
            &mut vm,
            &format!("https://module-errors.test/{name}"),
            source,
        );
    }
    let first = graph_error(&mut vm, "./first.mjs");
    let again = graph_error(&mut vm, "./first.mjs");
    let second = graph_error(&mut vm, "./second.mjs");
    let dependency = graph_error(&mut vm, "./bad.mjs");
    let other = graph_error(&mut vm, "./other-bad.mjs");
    assert!(first.exception_id().is_some());
    assert_eq!(
        first.error_constructor(),
        Some(ScriptErrorConstructorKind::TypeError)
    );
    assert_eq!(first.exception_id(), again.exception_id());
    assert_eq!(first.exception_id(), second.exception_id());
    assert_eq!(first.exception_id(), dependency.exception_id());
    assert_ne!(first.exception_id(), other.exception_id());
    let first = reject_with_error(&mut vm, &first);
    let second = reject_with_error(&mut vm, &second);
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        assert!(v8::Local::new(scope, &first).strict_equals(v8::Local::new(scope, &second)));
        Ok(())
    })
    .unwrap();
}

#[test]
fn module_parse_error_cache_does_not_capture_direct_resolution_fetch_or_link_errors() {
    let mut vm = new_test_vm("https://module-errors.test/page.html");
    let direct = graph_error(&mut vm, "unmapped");
    for error in [
        direct,
        ModuleLoadError::new(ModuleLoadStage::Fetch, "network failure"),
        ModuleLoadError::new(ModuleLoadStage::Instantiate, "missing export")
            .with_error_constructor(ScriptErrorConstructorKind::SyntaxError),
    ] {
        assert!(error.exception_id().is_none());
        let first = reject_with_error(&mut vm, &error);
        let second = reject_with_error(&mut vm, &error);
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            assert!(!v8::Local::new(scope, &first).strict_equals(v8::Local::new(scope, &second)));
            Ok(())
        })
        .unwrap();
    }
}

#[test]
fn module_parse_error_registry_bypasses_public_constructors_and_collection_methods() {
    let mut vm = new_test_vm("https://module-errors.test/page.html");
    vm.eval(
        r#"
      globalThis.__intrinsicSyntaxError = SyntaxError;
      const forbidden = () => { throw new Error('public hook must not run'); };
      Map.prototype.set = Map.prototype.get = Map.prototype.has = forbidden;
      globalThis.Map = globalThis.SyntaxError = globalThis.TypeError = forbidden;
      'installed'
    "#,
    )
    .unwrap();
    let error = compile_parse_error(&mut vm, "https://module-errors.test/bad.mjs");
    let first = reject_with_error(&mut vm, &error);
    let second = reject_with_error(&mut vm, &error);
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let first = v8::Local::new(scope, &first);
        assert!(first.strict_equals(v8::Local::new(scope, &second)));
        let global = scope.get_current_context().global(scope);
        assert_eq!(
            global.set(scope, v8str(scope, "__originalException").into(), first),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval("__originalException instanceof __intrinsicSyntaxError")
            .unwrap(),
        "true"
    );
}

#[test]
fn module_parse_error_registry_is_realm_local_and_does_not_root_retired_realms() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let (weak_context, weak_exception, error) = {
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let message = v8str(scope, "bad module");
        let exception = v8::Exception::syntax_error(scope, message);
        let id = retain_module_exception(scope, exception).unwrap();
        let error =
            ModuleLoadError::new(ModuleLoadStage::Compile, "bad module").with_exception_id(id);
        assert!(
            module_load_error_value(scope, &error)
                .unwrap()
                .strict_equals(exception)
        );
        let weak_context = v8::Weak::new(scope, context);
        let weak_exception = v8::Weak::new(scope, exception);
        let other_context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, other_context);
        assert!(module_load_error_value(scope, &error).is_err());
        let other = v8::Exception::syntax_error(scope, message);
        let other_id = retain_module_exception(scope, other).unwrap();
        assert_ne!(id, other_id);
        assert!(module_load_error_value(scope, &error).is_err());
        (weak_context, weak_exception, error)
    };
    isolate.low_memory_notification();
    assert!(
        weak_context.is_empty(),
        "retained module errors must not root the realm"
    );
    assert!(
        weak_exception.is_empty(),
        "retiring the realm must release its exceptions"
    );
    assert!(
        error.exception_id().is_some(),
        "the portable token may outlive its realm"
    );
}

#[tokio::test]
async fn module_parse_error_compilation_and_resolution_use_the_child_realm() {
    let mut vm = new_test_vm("https://module-errors.test/page.html");
    vm.eval(
        r#"
      const root = document.appendChild(document.createElement('html'));
      const body = root.appendChild(document.createElement('body'));
      globalThis.frame = document.createElement('iframe');
      frame.srcdoc = '<script>parent.__moduleErrorFrameReady = true;<\/script>';
      body.appendChild(frame);
      'ready'
    "#,
    )
    .unwrap();
    commit_child_document_and_run_parser_script_for_dynamic_import_test(
        &mut vm,
        "module error child",
    )
    .await;
    finish_child_document_after_parser_script_for_dynamic_import_test(
        &mut vm,
        "module error child",
    )
    .await;
    let realm_id = vm
        .child_frame_realm_store
        .values()
        .next()
        .unwrap()
        .owner_realm_id;
    let url = Url::parse("https://module-errors.test/bad.mjs").unwrap();
    let syntax_error = vm
        .compile_native_module_record_for_frame_realm(
            realm_id,
            ModuleMapKey::java_script(url.clone()),
            &ModuleSource::text("export const = ;".to_owned()),
            &url,
            &ModuleFetchMetadata::default(),
        )
        .unwrap_err();
    let resolution_error = vm
        .preserve_native_module_load_error(
            Some(realm_id),
            ModuleLoadError::new(ModuleLoadStage::Resolve, "unmapped dependency")
                .with_error_constructor(ScriptErrorConstructorKind::TypeError),
        )
        .unwrap();
    let (syntax, resolution) = vm
        .with_frame_realm_scope_and_checkpoint_for_test(realm_id, |scope, _| {
            let syntax = module_load_error_value(scope, &syntax_error)?;
            let resolution = module_load_error_value(scope, &resolution_error)?;
            Ok((
                v8::Global::new(scope, syntax),
                v8::Global::new(scope, resolution),
            ))
        })
        .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        assert!(module_load_error_value(scope, &syntax_error).is_err());
        assert!(module_load_error_value(scope, &resolution_error).is_err());
        let global = scope.get_current_context().global(scope);
        let syntax = v8::Local::new(scope, &syntax);
        let resolution = v8::Local::new(scope, &resolution);
        assert_eq!(
            global.set(scope, v8str(scope, "__childSyntaxError").into(), syntax),
            Some(true)
        );
        assert_eq!(
            global.set(
                scope,
                v8str(scope, "__childResolutionError").into(),
                resolution
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"
      __childSyntaxError instanceof frame.contentWindow.SyntaxError &&
      !(__childSyntaxError instanceof SyntaxError) &&
      __childResolutionError instanceof frame.contentWindow.TypeError &&
      !(__childResolutionError instanceof TypeError)
    "#
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn module_parse_error_registry_does_not_follow_a_reused_window_proxy() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let (current_context, old_context, old_exception) = {
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let (proxy, old_context, old_exception, error) = {
            let scope = &mut v8::ContextScope::new(scope, context);
            let exception = v8::Exception::syntax_error(scope, v8str(scope, "old module"));
            let id = retain_module_exception(scope, exception).unwrap();
            let error =
                ModuleLoadError::new(ModuleLoadStage::Compile, "old module").with_exception_id(id);
            (
                context.global(scope),
                v8::Weak::new(scope, context),
                v8::Weak::new(scope, exception),
                error,
            )
        };
        context.detach_global();
        let replacement = v8::Context::new(
            scope,
            v8::ContextOptions {
                global_object: Some(proxy.into()),
                ..Default::default()
            },
        );
        let scope = &mut v8::ContextScope::new(scope, replacement);
        assert!(replacement.global(scope).strict_equals(proxy.into()));
        assert!(
            module_load_error_value(scope, &error).is_err(),
            "a new realm must not inherit the previous module cache via WindowProxy"
        );
        (
            v8::Global::new(scope, replacement),
            old_context,
            old_exception,
        )
    };
    isolate.low_memory_notification();
    assert!(
        old_context.is_empty(),
        "the current WindowProxy must not root the old realm"
    );
    assert!(
        old_exception.is_empty(),
        "the current realm must not retain old module errors"
    );
    drop(current_context);
}
