use super::*;
use crate::script_provenance::CompiledStringProvenance;

fn execute_nonce_script(vm: &mut ScriptVm, source: &str, url: &Url, nonce: Option<&str>) {
    vm.exec_in_enclosing_script_turn_with_provenance(
        source,
        &CompiledStringProvenance::at_url(url.clone()),
        0,
        nonce,
        true,
    )
    .map_err(|error| error.into_anyhow())
    .expect("the initiating script should execute");
}

fn consume_string_timer(vm: &mut ScriptVm) {
    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("string timer should execute"),
        crate::host::HostTimeoutRunResult::Consumed
    ));
}

fn assert_import_nonce(vm: &mut ScriptVm, base_url: &Url, nonce: Option<&str>) {
    let request = vm
        .document_runtime
        .take_next_native_dynamic_module_import()
        .expect("the timer should queue a dynamic import")
        .into_dynamic_import_request();
    assert_eq!(request.specifier(), "./dependency.mjs");
    assert_eq!(request.base_url(), base_url);
    assert_eq!(request.fetch_metadata().nonce.as_deref(), nonce);
    assert!(!request.fetch_metadata().parser_inserted);
    assert!(request.fetch_metadata().integrity.is_none());
}

#[test]
fn string_timers_preserve_nonce_for_timeout_interval_and_nested_source() {
    for (source, timer_turns) in [
        (r#"setTimeout("import('./dependency.mjs')", 0);"#, 1),
        (
            r#"globalThis.__nonceInterval = setInterval("clearInterval(__nonceInterval); import('./dependency.mjs')", 0);"#,
            1,
        ),
        (
            r#"setTimeout("setTimeout(\"import('./dependency.mjs')\", 0)", 0);"#,
            2,
        ),
    ] {
        let mut vm = new_storage_test_vm("https://timer-nonce.test/page.html");
        let script_url = Url::parse("https://timer-nonce.test/scripts/initiator.js").unwrap();
        execute_nonce_script(&mut vm, source, &script_url, Some("initiating-nonce"));
        for _ in 0..timer_turns {
            consume_string_timer(&mut vm);
        }
        assert_import_nonce(&mut vm, &script_url, Some("initiating-nonce"));
    }
}

#[test]
fn string_timers_keep_nonce_snapshots_separate_for_identical_source_and_url() {
    let mut vm = new_storage_test_vm("https://timer-nonce.test/page.html");
    let script_url = Url::parse("https://timer-nonce.test/scripts/same.js").unwrap();
    let nonces = [Some("first-nonce"), None, Some("last-nonce")];
    for nonce in nonces {
        execute_nonce_script(
            &mut vm,
            r#"setTimeout("import('./dependency.mjs')", 0);"#,
            &script_url,
            nonce,
        );
    }
    for nonce in nonces {
        consume_string_timer(&mut vm);
        assert_import_nonce(&mut vm, &script_url, nonce);
    }
}

#[test]
fn string_timers_use_the_active_function_nonce_without_borrowing_the_callers_nonce() {
    for initiating_nonce in [Some("function-nonce"), None] {
        let mut vm = new_storage_test_vm("https://timer-nonce.test/page.html");
        let script_url = Url::parse("https://timer-nonce.test/scripts/function.js").unwrap();
        execute_nonce_script(
            &mut vm,
            r#"globalThis.scheduleFromInitiator = () => setTimeout("import('./dependency.mjs')", 0);"#,
            &script_url,
            initiating_nonce,
        );
        execute_nonce_script(
            &mut vm,
            "scheduleFromInitiator();",
            &Url::parse("https://timer-nonce.test/other/caller.js").unwrap(),
            Some("caller-nonce"),
        );
        consume_string_timer(&mut vm);
        assert_import_nonce(&mut vm, &script_url, initiating_nonce);
    }
}

#[test]
fn dynamic_function_imports_preserve_each_initiators_nonce_for_identical_source() {
    let mut vm = new_storage_test_vm("https://function-nonce.test/page.html");
    let script_url = Url::parse("https://function-nonce.test/scripts/same.js").unwrap();
    for nonce in [Some("first-nonce"), None, Some("last-nonce")] {
        execute_nonce_script(
            &mut vm,
            r#"(new Function('x', 'return import(x)'))('./dependency.mjs');"#,
            &script_url,
            nonce,
        );
        assert_import_nonce(&mut vm, &script_url, nonce);
    }
}

#[test]
fn dynamic_function_imports_keep_creation_metadata_when_invoked_by_another_script() {
    let mut vm = new_storage_test_vm("https://function-origin.test/page.html");
    let initiators = [
        (
            "https://function-origin.test/alpha/source.js",
            Some("alpha"),
        ),
        ("https://function-origin.test/beta/source.js", None),
        (
            "https://function-origin.test/gamma/source.js",
            Some("gamma"),
        ),
    ];
    for (url, nonce) in initiators {
        execute_nonce_script(
            &mut vm,
            r#"(globalThis.importers ??= []).push(new Function('x', 'return import(x)'));"#,
            &Url::parse(url).unwrap(),
            nonce,
        );
    }
    execute_nonce_script(
        &mut vm,
        "importers.forEach(importer => importer('./dependency.mjs'));",
        &Url::parse("https://function-origin.test/caller/source.js").unwrap(),
        Some("caller"),
    );
    for (url, nonce) in initiators {
        let request = vm
            .document_runtime
            .take_next_native_dynamic_module_import()
            .expect("each created Function should queue its own import")
            .into_dynamic_import_request();
        assert_eq!(request.base_url(), &Url::parse(url).unwrap());
        assert_eq!(request.fetch_metadata().nonce.as_deref(), nonce);
    }
}

#[test]
fn dynamic_function_imports_keep_origins_for_async_and_generator_constructors() {
    for expression in [
        "new (async function() {}).constructor('x', 'return import(x)')('./dependency.mjs')",
        "new (function*() {}).constructor('x', 'yield import(x)')('./dependency.mjs').next()",
        "new (async function*() {}).constructor('x', 'yield import(x)')('./dependency.mjs').next()",
    ] {
        let mut vm = new_storage_test_vm("https://function-kinds.test/page.html");
        let script_url = Url::parse("https://function-kinds.test/initiator.js").unwrap();
        for nonce in [Some("first"), None, Some("last")] {
            execute_nonce_script(&mut vm, expression, &script_url, nonce);
            let request = vm
                .document_runtime
                .take_next_native_dynamic_module_import()
                .expect("each constructor kind should queue an import")
                .into_dynamic_import_request();
            assert_eq!(request.base_url(), &script_url);
            assert_eq!(request.fetch_metadata().nonce.as_deref(), nonce);
        }
    }
}

#[test]
fn dynamic_function_origin_keys_preserve_source_utf16_and_parameter_validation() {
    let mut vm = new_storage_test_vm("https://function-source.test/page.html");
    let result = vm.eval(r#"(() => {
        const assert = (condition, message) => { if (!condition) throw new Error(message); };
        const cases = [
            [Function, 'function'],
            [(function*() {}).constructor, 'function*'],
            [(async function() {}).constructor, 'async function'],
            [(async function*() {}).constructor, 'async function*'],
        ];
        for (const [Constructor, prefix] of cases) {
            const f = Constructor('x', 'return x;');
            assert(f.toString() === prefix + ' anonymous(x\n) {\nreturn x;\n}', 'source text');
            assert(f.name === 'anonymous' && f.length === 1, 'name and length');
            assert(Object.getPrototypeOf(f) === Constructor.prototype, 'function prototype');
        }
        const surrogate = String.fromCharCode(0xd800);
        const body = 'return "' + surrogate + '";';
        const f = Function(body);
        assert(f().charCodeAt(0) === 0xd800, 'lone surrogate value');
        assert(f.toString() === 'function anonymous(\n) {\n' + body + '\n}', 'lone surrogate source');
        const earlyError = (params, body) => {
            try { Function(params, body); } catch (error) { return error instanceof SyntaxError; }
            return false;
        };
        Function('', 'function anonymous(\n/**/) {\n}');
        assert(earlyError('\n/**/) {\nfunction anonymous(', '}'), 'cache must retain parameter boundary');
        assert(earlyError('/*', '*/ ) {'), 'parameters and body parse separately');
        assert(earlyError('a = 1', '"use strict";'), 'non-simple strict parameters');
        const sourceURL = Function('throw new Error("source URL");\n//# sourceURL=created-function.js');
        try { sourceURL(); } catch (error) {
            assert(error.stack.includes('created-function.js'), 'source URL');
        }
        return 'ok';
    })()"#).expect("Function origin keys must preserve ordinary source semantics");
    assert_eq!(result, "ok");
}

#[test]
fn dynamic_function_origin_keys_preserve_trusted_types_source_checks() {
    let mut vm = new_storage_test_vm("https://function-trusted-source.test/page.html");
    vm.set_response_content_security_policies(&[
        "require-trusted-types-for 'script'; script-src 'unsafe-eval'".to_owned(),
    ]);
    let result = vm
        .eval(
            r#"(() => {
                let transform = false;
                let replacement = '(function anonymous(\n) {\nreturn 7;\n}) /*';
                const seen = [];
                trustedTypes.createPolicy('default', {createScript: source => {
                    seen.push(source);
                    return transform ? replacement : source;
                }});
                let invalidError;
                try { Function('return 1; /*'); } catch (error) { invalidError = error.name; }
                if (invalidError !== 'SyntaxError') throw new Error('invalid source was repaired');
                const f = Function('return 1;');
                if (f() !== 1 || f.toString() !== 'function anonymous(\n) {\nreturn 1;\n}') {
                    throw new Error('identity policy changed source');
                }
                const offered = seen[seen.length - 1];
                if (offered !== f.toString() && offered !== '(' + f.toString() + ')') {
                    throw new Error('policy received host-modified source');
                }
                transform = true;
                for (const source of [replacement, '(function anonymous(\n) {\nreturn 7;\n})']) {
                    replacement = source;
                    let transformedError;
                    try { Function('return 1;'); } catch (error) { transformedError = error.name; }
                    if (transformedError !== 'EvalError') throw new Error('transformed source was accepted');
                }
                return 'ok';
            })()"#,
        )
        .expect("cache keys must preserve Trusted Types replacement syntax");
    assert_eq!(result, "ok");
}
