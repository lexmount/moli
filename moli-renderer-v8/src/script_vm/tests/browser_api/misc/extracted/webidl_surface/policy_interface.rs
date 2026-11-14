use super::*;

#[test]
fn trusted_type_policy_interface_shares_methods_and_rejects_missing_callbacks() {
    let mut vm = new_storage_test_vm("https://trusted-type-policy-declared-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const htmlOnly = trustedTypes.createPolicy("html-only", {
    createHTML: value => `<b>${value}</b>`
  });
  const full = trustedTypes.createPolicy("full", {
    createHTML: value => value,
    createScript: value => value,
    createScriptURL: value => value
  });
  const prototype = TrustedTypePolicy.prototype;
  const describe = name => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      descriptor.enumerable,
      descriptor.writable,
      descriptor.configurable,
      typeof descriptor.value,
      descriptor.value.name,
      descriptor.value.length
    ].join(":");
  };
  const html = htmlOnly.createHTML("ok");
  const script = full.createScript("1 + 1");
  const scriptURL = full.createScriptURL("data:text/javascript,");
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error.constructor.name;
    }
  };
  const nameDescriptor = Object.getOwnPropertyDescriptor(prototype, "name");

  return JSON.stringify({
    htmlOnlyKeys: Object.keys(htmlOnly).join(","),
    fullKeys: Object.keys(full).join(","),
    ownMethods: ["createHTML", "createScript", "createScriptURL"].map(name => Object.hasOwn(htmlOnly, name)),
    prototypeMethods: ["createHTML", "createScript", "createScriptURL"].map(describe),
    name: [
      htmlOnly.name,
      full.name,
      typeof nameDescriptor.get,
      nameDescriptor.set === undefined,
      nameDescriptor.enumerable,
      nameDescriptor.configurable,
      Object.hasOwn(htmlOnly, "name")
    ],
    brands: [
      htmlOnly instanceof TrustedTypePolicy,
      full instanceof TrustedTypePolicy,
      Object.getPrototypeOf(htmlOnly) === prototype,
      htmlOnly.createHTML === full.createHTML
    ],
    missingCallbackErrors: [
      errorName(() => htmlOnly.createScript("1 + 1")),
      errorName(() => htmlOnly.createScriptURL("data:text/javascript,"))
    ],
    trusted: [
      trustedTypes.isHTML(html),
      trustedTypes.isScript(script),
      trustedTypes.isScriptURL(scriptURL)
    ],
    values: [String(html), String(script), String(scriptURL)]
  });
})()
"#,
        )
        .expect("TrustedTypePolicy interface descriptors should evaluate");

    assert_eq!(
        result,
        r#"{"htmlOnlyKeys":"","fullKeys":"","ownMethods":[false,false,false],"prototypeMethods":["createHTML:true:true:true:function:createHTML:1","createScript:true:true:true:function:createScript:1","createScriptURL:true:true:true:function:createScriptURL:1"],"name":["html-only","full","function",true,true,true,false],"brands":[true,true,true,true],"missingCallbackErrors":["TypeError","TypeError"],"trusted":[true,true,true],"values":["<b>ok</b>","1 + 1","data:text/javascript,"]}"#
    );
}

#[test]
fn trusted_type_policy_interface_preserves_realm_brands_and_intrinsics() {
    let mut vm = new_storage_html_test_vm("https://trusted-policy-interface-realms.test/");
    assert_eq!(vm.eval(r#"(() => {
        const child = document.body.appendChild(document.createElement('iframe')).contentWindow;
        const rows = [];
        for (const owner of [window, child]) {
            const Constructor = owner.TrustedTypePolicy;
            const getter = Object.getOwnPropertyDescriptor(Constructor.prototype, 'name').get;
            const create = Constructor.prototype.createHTML;
            const policy = owner.trustedTypes.createPolicy('realm-policy', {createHTML: value => value});
            rows.push(Object.getPrototypeOf(policy) === Constructor.prototype,
                getter.call(policy) === 'realm-policy', create.call(policy, 'ok') instanceof owner.TrustedHTML);
            for (const construct of [() => new Constructor(), () => Constructor()]) {
                try { construct(); rows.push(false); }
                catch(error) { rows.push(error instanceof owner.TypeError); }
            }
            let converted = 0;
            for (const receiver of [{}, Object.create(Constructor.prototype), new Proxy(policy,{})]) {
                try { create.call(receiver, {toString(){ converted++; return 'bad'; }}); rows.push(false); }
                catch(error) { rows.push(error instanceof owner.TypeError); }
                try { getter.call(receiver); rows.push(false); }
                catch(error) { rows.push(error instanceof owner.TypeError); }
            }
            rows.push(converted === 0);
            owner.TrustedTypePolicy = function Spoof() {};
            const later = owner.trustedTypes.createPolicy('later-policy', {createHTML: value => value});
            rows.push(Object.getPrototypeOf(later) === Constructor.prototype,
                getter.call(later) === 'later-policy', create.call(later, 'ok') instanceof owner.TrustedHTML);
        }
        if (!rows.every(Boolean)) throw Error('policy realm checks: ' + JSON.stringify(rows));
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn trusted_type_policy_constructor_materializes_with_its_lazy_realm_state() {
    let mut vm = new_storage_html_test_vm("https://trusted-policy-interface-lazy.test/");
    vm.with_default_context_scope(|scope, _| {
        assert!(!crate::context_bootstrap::trusted_types_lazy_state_materialized(scope));
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("typeof TrustedTypePolicy").unwrap(), "function");
    vm.with_default_context_scope(|scope, _| {
        assert!(crate::context_bootstrap::trusted_types_lazy_state_materialized(scope));
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("Object.getPrototypeOf(trustedTypes.createPolicy('lazy-policy', {})) === TrustedTypePolicy.prototype").unwrap(), "true");
}

#[test]
fn trusted_type_policy_creation_ignores_a_constructor_replaced_before_lazy_materialization() {
    let mut vm = new_storage_html_test_vm("https://trusted-policy-before-lazy.test/");
    assert_eq!(vm.eval(r#"(() => {
        const spoof = function AuthorPolicy() { throw Error('author constructor'); };
        TrustedTypePolicy = spoof;
        const policy = trustedTypes.createPolicy('intrinsic-policy', {createHTML: value => value});
        const prototype = Object.getPrototypeOf(policy);
        return TrustedTypePolicy === spoof && prototype !== spoof.prototype &&
            prototype.constructor.name === 'TrustedTypePolicy' &&
            Object.getOwnPropertyDescriptor(prototype, 'name').get.call(policy) === 'intrinsic-policy' &&
            !Object.hasOwn(policy, 'createHTML') &&
            String(policy.createHTML('ok')) === 'ok';
    })()"#).unwrap(), "true");
}
