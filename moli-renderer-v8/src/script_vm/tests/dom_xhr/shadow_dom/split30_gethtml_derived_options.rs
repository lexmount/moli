use super::*;
fn check_get_html_options(variant: &str) {
    let mut vm = new_parsed_test_vm(
        "https://get-html-options.test/",
        "<!doctype html><body><iframe></iframe></body>",
    );
    materialize_single_child_default_realm_for_test(&mut vm, "getHTML options callee realm");
    let source = format!(
        "{}\ngetHtmlOptionsProbe({variant:?})",
        include_str!("../get_html_options.js")
    );
    assert_eq!(vm.eval(&source).expect("getHTML options probe"), "ok");
}

#[test]
fn get_html_dictionary_options_use_webidl_defaults_and_conversions() {
    check_get_html_options("dictionary");
}

#[test]
fn get_html_shadow_root_sequences_use_iterators_and_native_brands() {
    check_get_html_options("iterables");
}

#[test]
fn get_html_options_preserve_exceptions_and_stop_getter_evaluation() {
    check_get_html_options("exceptions");
}

#[test]
fn get_html_options_observe_reentrant_tree_mutations() {
    check_get_html_options("mutation");
}

#[test]
fn get_html_accepts_registered_native_proxies_and_rejects_author_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://get-html-native-proxies.test/",
        "<!doctype html><body><iframe></iframe></body>",
    );
    materialize_single_child_default_realm_for_test(&mut vm, "getHTML native proxy realm");
    vm.eval(
        r#"globalThis.getHtmlNativeHost = document.implementation.createHTMLDocument('').createElement('div');
        getHtmlNativeHost.innerHTML = '<b>light</b>';
        globalThis.getHtmlNativeRoot = getHtmlNativeHost.attachShadow({mode:'open'});
        getHtmlNativeRoot.innerHTML = '<i>private</i>';
        true"#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, alias) in [
            ("getHtmlNativeHost", "getHtmlNativeHostAlias"),
            ("getHtmlNativeRoot", "getHtmlNativeRootAlias"),
        ] {
            let key = crate::util::v8str(scope, name);
            let target =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            if name == "getHtmlNativeRoot" {
                assert!(crate::web_api_interfaces::ShadowRoot::is_instance(
                    scope,
                    proxy.into()
                ));
            }
            let key = crate::util::v8str(scope, alias);
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
              const assert = (value, message) => { if (!value) throw Error(message); };
              const other = document.querySelector('iframe').contentWindow;
              let reads = 0, traps = 0;
              const options = {get shadowRoots() { reads++; return []; }};
              const handler = {
                get() { traps++; throw Error('author get'); },
                getPrototypeOf() { traps++; throw Error('author prototype'); }
              };
              for (const realm of [window, other]) {
                const html = realm.Element.prototype.getHTML.call(getHtmlNativeHostAlias, {
                  shadowRoots: new Set([getHtmlNativeRootAlias])
                });
                assert(html === '<template shadowrootmode="open"><i>private</i></template><b>light</b>', 'registered root sequence');
                assert(realm.ShadowRoot.prototype.getHTML.call(getHtmlNativeRootAlias) === '<i>private</i>', 'registered root receiver');
                for (const [method, real] of [
                  [realm.Element.prototype.getHTML, getHtmlNativeHostAlias],
                  [realm.ShadowRoot.prototype.getHTML, getHtmlNativeRootAlias]
                ]) {
                  const revoked = Proxy.revocable(real, {}); revoked.revoke();
                  for (const receiver of [null, undefined, {}, Object.create(real), new Proxy(real, handler), revoked.proxy]) {
                    let error;
                    try { Reflect.apply(method, receiver, [options]); }
                    catch (caught) { error = caught; }
                    assert(error instanceof realm.TypeError, 'receiver TypeError belongs to callee');
                  }
                }
              }
              assert(reads === 0 && traps === 0, 'receiver checks precede getters and author traps');
              return 'ok';
            })()"#,
        )
        .unwrap(),
        "ok"
    );
}
