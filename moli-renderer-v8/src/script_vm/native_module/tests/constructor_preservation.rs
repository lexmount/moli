use super::*;
use crate::types::ScriptErrorConstructorKind;

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
fn cached_module_graph_errors_preserve_their_native_constructors() {
    for (kind, constructor) in [
        (ScriptErrorConstructorKind::Error, "Error"),
        (ScriptErrorConstructorKind::SyntaxError, "SyntaxError"),
        (ScriptErrorConstructorKind::TypeError, "TypeError"),
        (
            ScriptErrorConstructorKind::WebAssemblyCompileError,
            "WebAssembly.CompileError",
        ),
        (
            ScriptErrorConstructorKind::WebAssemblyLinkError,
            "WebAssembly.LinkError",
        ),
    ] {
        let mut vm = new_test_vm("https://module-errors.test/page.html");
        let url = Url::parse("https://module-errors.test/cached.mjs").unwrap();
        vm.document_runtime.mark_native_module_failed(
            ModuleMapKey::java_script(url.clone()),
            ModuleLoadError::new(ModuleLoadStage::Instantiate, "cached module failure")
                .with_error_constructor(kind),
        );
        let error = graph_error(&mut vm, url.as_str());
        let rejection = reject_with_error(&mut vm, &error);
        vm.with_default_context_scope(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let rejection = v8::Local::new(scope, &rejection);
            assert_eq!(
                global.set(scope, v8str(scope, "__rejected").into(), rejection),
                Some(true)
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(
            vm.eval(&format!("String(__rejected.constructor === {constructor})"))
                .unwrap(),
            "true",
            "a cached module graph failure must retain {constructor}"
        );
    }
}

#[test]
fn wasm_compile_error_survives_module_graph_and_rejection() {
    let mut vm = new_test_vm("https://module-errors.test/page.html");
    let url = Url::parse("https://module-errors.test/invalid.wasm").unwrap();
    let mut job = dynamic_import_job_in_vm(
        &mut vm,
        url.as_str(),
        Url::parse("https://module-errors.test/page.html").unwrap(),
        ModuleImportPhase::Evaluation,
    );
    let NativeModuleGraphJobAdvance::NeedFetches(mut requests) =
        job.advance_dynamic_import_owner_lane(&mut vm).unwrap()
    else {
        panic!("the Wasm module must start a fetch");
    };
    assert_eq!(requests.len(), 1);
    let error = job
        .finish_dynamic_import_fetch_for_request(
            &mut vm,
            &requests.pop().unwrap(),
            Ok(ModuleGraphFetchedSource::new(
                url,
                false,
                ModuleSource::binary(vec![0]),
            )),
        )
        .err()
        .expect("V8 must reject the invalid Wasm bytes");
    assert_eq!(error.stage(), ModuleLoadStage::Compile);
    assert_eq!(
        error.error_constructor(),
        Some(ScriptErrorConstructorKind::WebAssemblyCompileError)
    );
    vm.eval(
        r#"
globalThis.__originalCompileError = WebAssembly.CompileError;
globalThis.WebAssembly = { get CompileError() { throw new Error('author getter'); } };
"#,
    )
    .unwrap();
    let rejection = reject_with_error(&mut vm, &error);
    vm.with_default_context_scope(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let rejection = v8::Local::new(scope, &rejection);
        assert_eq!(
            global.set(scope, v8str(scope, "__rejected").into(), rejection),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval("String(__rejected.constructor === __originalCompileError)")
            .unwrap(),
        "true",
        "import rejection must retain CompileError without reading page constructors"
    );
}
