use std::{cell::RefCell, pin::pin};

use super::*;

type ImportedNonces = RefCell<Vec<Option<String>>>;

fn capture_import_nonce<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Data>,
    _resource: v8::Local<'s, v8::Value>,
    _specifier: v8::Local<'s, v8::String>,
    _attributes: v8::Local<'s, v8::FixedArray>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let nonce = script_nonce_from_host_defined_options(scope, options);
    scope
        .get_slot::<ImportedNonces>()
        .unwrap()
        .borrow_mut()
        .push(nonce);
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let undefined = v8::undefined(scope).into();
    resolver.resolve(scope, undefined);
    Some(promise)
}

fn isolate() -> v8::OwnedIsolate {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_slot(ImportedNonces::default());
    isolate.set_modify_code_generation_from_strings_callback(
        crate::script_vm::string_code_generation_check_callback,
    );
    isolate.set_host_import_module_dynamically_callback(capture_import_nonce);
    isolate
}

fn run(scope: &mut v8::PinScope<'_, '_>, source: &str, nonce: Option<&str>) -> String {
    let url = Url::parse("https://example.test/shared-script.js").unwrap();
    let origin =
        create_script_origin_with_base_url_and_nonce(scope, url.as_str(), 0, Some(&url), nonce);
    let source = v8_string(scope, source).unwrap();
    let script = v8::Script::compile(scope, source, Some(&origin)).unwrap();
    let result = crate::script_execution::execute_compiled_script(scope, script)
        .expect("probe script should execute");
    let result = result.to_rust_string_lossy(scope);
    scope.perform_microtask_checkpoint();
    result
}

#[test]
fn dynamic_function_cache_preserves_nonce_across_realms() {
    let mut isolate = isolate();
    for nonce in [Some("abc"), None, Some("def"), Some("ghi")] {
        let handle = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut handle.init();
        let context = v8::Context::new(scope, Default::default());
        context.set_allow_generation_from_strings(false);
        let scope = &mut v8::ContextScope::new(scope, context);
        run(
            scope,
            r#"(() => {
            const constructors = [Function,
                Object.getPrototypeOf(async function() {}).constructor,
                Object.getPrototypeOf(function*() {}).constructor,
                Object.getPrototypeOf(async function*() {}).constructor];
            for (const Constructor of constructors) {
                const fn = new Constructor('x', 'return import(x)');
                const result = fn('./dependency.js');
                if (result.next) result.next();
            }
            return 'done';
        })()"#,
            nonce,
        );
    }
    let nonces = isolate.get_slot::<ImportedNonces>().unwrap().borrow();
    let expected: Vec<_> = [Some("abc"), None, Some("def"), Some("ghi")]
        .into_iter()
        .flat_map(|nonce| std::iter::repeat_n(nonce.map(str::to_owned), 4))
        .collect();
    assert_eq!(*nonces, expected);
}

#[test]
fn dynamic_function_cache_keeps_creator_nonce_after_later_compilations() {
    let mut isolate = isolate();
    let handle = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut handle.init();
    let context = v8::Context::new(scope, Default::default());
    context.set_allow_generation_from_strings(false);
    let scope = &mut v8::ContextScope::new(scope, context);
    for nonce in ["abc", "def", "ghi"] {
        run(
            scope,
            r#"
            globalThis.savedFunctions ||= [];
            savedFunctions.push(new Function('x', 'return import(x)'));
        "#,
            Some(nonce),
        );
    }
    run(
        scope,
        r#"
        for (const fn of savedFunctions) fn('./dependency.js');
    "#,
        Some("ghi"),
    );
    assert_eq!(
        *scope.get_slot::<ImportedNonces>().unwrap().borrow(),
        ["abc", "def", "ghi"].map(|nonce| Some(nonce.to_owned()))
    );
}

#[test]
fn eval_cache_preserves_nonce_for_direct_and_indirect_eval() {
    let mut isolate = isolate();
    for nonce in [Some("abc"), None, Some("def"), Some("ghi")] {
        let handle = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut handle.init();
        let context = v8::Context::new(scope, Default::default());
        context.set_allow_generation_from_strings(false);
        let scope = &mut v8::ContextScope::new(scope, context);
        run(
            scope,
            r#"(() => {
                eval("import('./dependency.js')");
                (0, eval)("import('./dependency.js')");
                return 'done';
            })()"#,
            nonce,
        );
    }
    let expected: Vec<_> = [Some("abc"), None, Some("def"), Some("ghi")]
        .into_iter()
        .flat_map(|nonce| std::iter::repeat_n(nonce.map(str::to_owned), 2))
        .collect();
    assert_eq!(
        *isolate.get_slot::<ImportedNonces>().unwrap().borrow(),
        expected
    );
}

#[test]
fn nonced_codegen_preserves_function_source_eval_and_utf16() {
    let mut isolate = isolate();
    let source = r#"(() => {
        const constructors = [Function,
            Object.getPrototypeOf(async function() {}).constructor,
            Object.getPrototypeOf(function*() {}).constructor,
            Object.getPrototypeOf(async function*() {}).constructor];
        const sources = constructors.map(Constructor =>
            new Constructor('value', 'return value').toString());
        let invalidEval = false, invalidFunction = false;
        try { eval('/*'); } catch (error) { invalidEval = error instanceof SyntaxError; }
        try { new Function('/*'); } catch (error) { invalidFunction = error instanceof SyntaxError; }
        const value = {};
        return JSON.stringify({sources, invalidEval, invalidFunction,
            completion: eval('17 // trailing comment'),
            lexicalScope: (() => { const lexical = 23; return eval('lexical'); })(),
            indirectCompletion: (0, eval)('29 // trailing comment'),
            strict: eval("'use strict'; (function() {return this})()") === undefined,
            nonString: eval(value) === value,
            globalThisValue: new Function('return this')() === globalThis,
            surrogate: new Function('return "' + String.fromCharCode(0xd800) + '"')().charCodeAt(0),
            sourceUrl: eval("new Error().stack\n//# sourceURL=custom-eval.js").includes('custom-eval.js')});
    })()"#;
    let mut expected = None;
    for nonce in [None, Some("abc"), Some("def")] {
        let handle = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut handle.init();
        let context = v8::Context::new(scope, Default::default());
        // Compare modified code against V8's unmodified compilation path.
        context.set_allow_generation_from_strings(nonce.is_none());
        let scope = &mut v8::ContextScope::new(scope, context);
        let result: serde_json::Value = serde_json::from_str(&run(scope, source, nonce)).unwrap();
        assert_eq!(result["surrogate"], 0xd800);
        assert_eq!(result["completion"], 17);
        assert_eq!(result["lexicalScope"], 23);
        assert_eq!(result["indirectCompletion"], 29);
        for key in [
            "invalidEval",
            "invalidFunction",
            "strict",
            "nonString",
            "globalThisValue",
            "sourceUrl",
        ] {
            assert_eq!(result[key], true, "{key}: {result}");
        }
        if let Some(expected) = &expected {
            assert_eq!(&result, expected);
        } else {
            expected = Some(result);
        }
    }
}
