use super::*;

#[test]
fn legacy_url_alias_is_lazy_and_shares_the_realm_constructor() {
    for scheme in ["http", "https"] {
        for first in [
            "webkitURL",
            "URL",
            "Object.getOwnPropertyDescriptor(globalThis, 'webkitURL').value",
        ] {
            let mut vm = new_storage_test_vm(&format!("{scheme}://url-alias.test/"));
            assert_eq!(constructor_materialization_count(&mut vm, "URL"), 0);
            assert_eq!(
                vm.eval("'webkitURL' in globalThis && Object.hasOwn(globalThis, 'webkitURL')")
                    .unwrap(),
                "true"
            );
            assert_eq!(constructor_materialization_count(&mut vm, "URL"), 0);
            vm.eval(&format!("void ({first})")).unwrap();
            assert_eq!(constructor_materialization_count(&mut vm, "URL"), 1);
            assert_eq!(
                vm.eval(r#"(() => {
                  const alias = webkitURL;
                  const value = new alias('next?q=value', 'https://example.test/base/');
                  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'webkitURL');
                  return alias === URL && alias.name === 'URL' && alias.length === 1 &&
                    value instanceof URL && Object.getPrototypeOf(value) === URL.prototype &&
                    value.href === 'https://example.test/base/next?q=value' &&
                    value.searchParams.get('q') === 'value' &&
                    alias.parse('/next', 'https://example.test/').href === 'https://example.test/next' &&
                    alias.canParse('https://example.test/') && !alias.canParse('relative') &&
                    alias.createObjectURL === URL.createObjectURL &&
                    alias.revokeObjectURL === URL.revokeObjectURL &&
                    descriptor.value === alias && descriptor.writable && descriptor.configurable &&
                    !descriptor.enumerable && !('get' in descriptor) && !('set' in descriptor);
                })()"#).unwrap(),
                "true",
                "{scheme}: first access through {first}"
            );
            assert_eq!(constructor_materialization_count(&mut vm, "URL"), 1);
        }
    }
}

#[test]
fn legacy_url_alias_bindings_can_be_replaced_or_deleted_independently_before_first_read() {
    for binding in ["URL", "webkitURL"] {
        for mode in ["assignment", "deletion", "getter", "readonly"] {
            let mut vm = new_storage_test_vm("https://url-alias-rebinding.test/");
            vm.eval(&format!(
                "globalThis.__binding = {binding:?}; globalThis.__mode = {mode:?};"
            ))
            .unwrap();
            assert_eq!(
                vm.eval(r#"(() => {
                  const other = __binding === 'URL' ? 'webkitURL' : 'URL';
                  const sentinel = {};
                  let getterCalls = 0;
                  const getter = () => { getterCalls++; throw new Error('must not read replaced binding'); };
                  if (__mode === 'assignment') globalThis[__binding] = sentinel;
                  if (__mode === 'deletion' && !Reflect.deleteProperty(globalThis, __binding)) return false;
                  if (__mode === 'getter') Object.defineProperty(globalThis, __binding, {get: getter, configurable: true});
                  if (__mode === 'readonly') Object.defineProperty(globalThis, __binding,
                    {value: sentinel, writable: false, configurable: false});
                  const constructor = globalThis[other];
                  if (typeof constructor !== 'function' || constructor.name !== 'URL') return false;
                  const url = new constructor('https://example.test/?q=value');
                  const parsed = constructor.parse('/parsed', 'https://example.test/');
                  if (url.href !== 'https://example.test/?q=value' || url.searchParams.get('q') !== 'value' ||
                      Object.getPrototypeOf(url) !== constructor.prototype ||
                      parsed.href !== 'https://example.test/parsed' || getterCalls !== 0) return false;
                  const descriptor = Object.getOwnPropertyDescriptor(globalThis, __binding);
                  if (__mode === 'deletion') return descriptor === undefined && !(__binding in globalThis);
                  if (__mode === 'getter') return descriptor.get === getter;
                  return descriptor.value === sentinel &&
                    (__mode !== 'readonly' || (!descriptor.writable && !descriptor.configurable));
                })()"#).unwrap(),
                "true",
                "{mode} of {binding} must not change or be undone by the other binding"
            );
            assert_eq!(constructor_materialization_count(&mut vm, "URL"), 1);
        }
    }
}

#[test]
fn url_parse_ignores_a_public_constructor_replaced_during_argument_conversion() {
    let mut vm = new_storage_test_vm("https://url-parse-intrinsic.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
          const Original = URL;
          const parse = Original.parse;
          let conversions = 0;
          let constructorCalls = 0;
          const input = {toString() {
            conversions++;
            globalThis.URL = function Replacement() { constructorCalls++; return {forged: true}; };
            return '/parsed';
          }};
          const result = parse.call({ignored: true}, input, 'https://example.test/base');
          return conversions === 1 && constructorCalls === 0 &&
            result.href === 'https://example.test/parsed' &&
            Object.getPrototypeOf(result) === Original.prototype && result instanceof Original &&
            globalThis.URL !== Original;
        })()"#
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn legacy_url_alias_materializes_in_its_own_window_realm() {
    let mut vm = new_parsed_test_vm(
        "https://url-alias-realms.test/",
        "<!doctype html><html><body></body></html>",
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
          const frame = document.body.appendChild(document.createElement('iframe'));
          const child = frame.contentWindow;
          const alias = child.webkitURL;
          const url = new alias('https://example.test/');
          const valid = alias === child.URL && alias !== webkitURL && webkitURL === URL &&
            Object.getPrototypeOf(alias) === child.Function.prototype &&
            Object.getPrototypeOf(url) === child.URL.prototype &&
            url instanceof child.URL && !(url instanceof URL);
          child.URL = {};
          const parsed = alias.parse.call(URL, 'https://example.test/parsed');
          frame.remove();
          return valid && Object.getPrototypeOf(parsed) === alias.prototype &&
            parsed instanceof alias && !(parsed instanceof URL) &&
            new alias('/retained', 'https://example.test/').href ===
            'https://example.test/retained';
        })()"#
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn legacy_url_alias_takes_precedence_over_window_named_properties() {
    let mut vm = new_parsed_test_vm(
        "https://url-alias-named.test/",
        "<!doctype html><html><body><div id=webkitURL></div></body></html>",
    );
    assert_eq!(constructor_materialization_count(&mut vm, "URL"), 0);
    assert_eq!(
        vm.eval(
            r#"(() => {
          const named = document.getElementById('webkitURL');
          const alias = window.webkitURL;
          if (typeof alias !== 'function' || alias !== URL || alias === named) return false;
          delete window.webkitURL;
          return window.webkitURL === named && typeof URL === 'function';
        })()"#
        )
        .unwrap(),
        "true"
    );
}
