use super::*;

#[test]
fn trusted_type_policy_preserves_dom_string_inputs_and_callback_results() {
    let mut vm = new_storage_test_vm("https://trusted-policy-utf16.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  let total = 0;
  const check = (name, passed) => { total++; if (!passed) failures.push(name); };
  for (const kind of ['HTML', 'Script', 'ScriptURL']) {
    let inputCalls = 0, outputCalls = 0, seenInput;
    const policy = trustedTypes.createPolicy('utf16-' + kind, {
      ['create' + kind](input) {
        seenInput = input;
        return {toString() {outputCalls++; return 'out:' + input;}};
      }
    });
    for (const input of ['', '\ud800', '\udc00', '\ud83d\ude00', 'nul\u0000\ud800']) {
      inputCalls = 0; outputCalls = 0;
      const value = policy['create' + kind]({toString() {inputCalls++; return input;}});
      const expected = kind === 'ScriptURL' ? ('out:' + input).toWellFormed() : 'out:' + input;
      check(kind + '/input/' + input, seenInput === input && inputCalls === 1);
      check(kind + '/output/' + input, String(value) === expected && outputCalls === 1);
      check(kind + '/JSON/' + input, value.toJSON() === expected);
    }
    let error;
    try { policy['create' + kind](Symbol('input')); } catch (caught) { error = caught; }
    check(kind + '/Symbol', error instanceof TypeError);
  }
  return JSON.stringify({total, failures});
})()
"#,
        )
        .expect("Trusted Type policy conversion should preserve Web IDL string types");
    assert_eq!(result, r#"{"total":48,"failures":[]}"#);
}

#[test]
fn trusted_script_is_code_like_for_direct_and_indirect_eval_without_enforcement() {
    let mut vm = new_storage_test_vm("https://trusted-script-code-like.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const policy = trustedTypes.createPolicy("code-like", {
    createScript: value => value
  });
  const script = policy.createScript("(() => 42)");
  const direct = eval(script);
  const indirect = (0, eval)(script);
  return JSON.stringify({
    scriptType: typeof script,
    scriptTag: Object.prototype.toString.call(script),
    directType: typeof direct,
    directValue: direct(),
    indirectType: typeof indirect,
    indirectValue: indirect()
  });
})()
"#,
        )
        .expect("TrustedScript should be accepted as code by eval");

    assert_eq!(
        result,
        r#"{"scriptType":"object","scriptTag":"[object TrustedScript]","directType":"function","directValue":42,"indirectType":"function","indirectValue":42}"#
    );
}

#[test]
fn trusted_script_code_like_brand_drives_function_constructor_with_enforcement() {
    let mut vm = new_storage_test_vm("https://trusted-script-function-code-like.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const policy = trustedTypes.createPolicy("function-code-like", {
    createScript: value => value
  });
  const trustedValue = new Function(policy.createScript("return 42"))();
  let stringError = "none";
  try {
    new Function("return 7");
  } catch (error) {
    stringError = `${error.name}:${error instanceof EvalError}`;
  }
  return JSON.stringify({ trustedValue, stringError });
})()
"#,
        )
        .expect("TrustedScript code-like brand should drive Function construction");

    assert_eq!(
        result,
        r#"{"trustedValue":42,"stringError":"EvalError:true"}"#
    );
}

#[test]
fn trusted_type_policy_callbacks_follow_webidl_conversion_and_call_contract() {
    let mut vm = new_storage_test_vm("https://trusted-types-callback-contract.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const facts = {
    getters: [],
    callbackCalls: 0,
    ownCallReads: 0
  };
  const extra = { marker: "extra" };
  const target = function() {
    "use strict";
    facts.callbackCalls += 1;
    facts.thisIsUndefined = this === undefined;
    facts.arguments = [
      arguments[0],
      arguments[1],
      arguments[2] === extra,
      arguments.length
    ];
    return null;
  };
  Object.defineProperty(target, "call", {
    get() {
      facts.ownCallReads += 1;
      throw new Error("callback.call must not be read");
    }
  });
  const callback = new Proxy(target, {
    apply(target, receiver, args) {
      facts.proxyApply = (facts.proxyApply || 0) + 1;
      return Reflect.apply(target, receiver, args);
    }
  });
  const policy = trustedTypes.createPolicy("typed-callback", {
    get createHTML() {
      facts.getters.push("createHTML");
      return callback;
    },
    get createScript() {
      facts.getters.push("createScript");
      return undefined;
    },
    get createScriptURL() {
      facts.getters.push("createScriptURL");
      return () => "\ud800";
    }
  });
  const html = policy.createHTML({
    toString() {
      facts.inputConvertedBeforeCallback = facts.callbackCalls === 0;
      return "input";
    }
  }, 7, extra);
  const scriptURL = policy.createScriptURL("url");

  const missingErrors = [
    trustedTypes.createPolicy("empty-omitted"),
    trustedTypes.createPolicy("empty-null", null),
    policy
  ].map((candidate, index) => {
    try {
      candidate[index === 2 ? "createScript" : "createHTML"]("x");
      return "none";
    } catch (error) {
      return error && error.name;
    }
  });

  const conversionErrors = {};
  for (const [name, options] of [
    ["primitive", 1],
    ["null-member", { createHTML: null }],
    ["noncallable", { createHTML: {} }]
  ]) {
    try {
      trustedTypes.createPolicy(name, options);
      conversionErrors[name] = "none";
    } catch (error) {
      conversionErrors[name] = error && error.name;
    }
  }
  const getterMarker = {};
  try {
    trustedTypes.createPolicy("getter-throw", {
      get createHTML() {
        throw getterMarker;
      }
    });
    conversionErrors.getter = "none";
  } catch (error) {
    conversionErrors.getter = error === getterMarker;
  }
  try {
    trustedTypes.createPolicy();
    conversionErrors.missingName = "none";
  } catch (error) {
    conversionErrors.missingName = error && error.name;
  }

  const revoked = Proxy.revocable(() => "revoked", {});
  revoked.revoke();
  let revokedCreate = "accepted";
  let revokedCall = "none";
  try {
    const revokedPolicy = trustedTypes.createPolicy("revoked", {
      createHTML: revoked.proxy
    });
    try {
      revokedPolicy.createHTML("x");
    } catch (error) {
      revokedCall = error && error.name;
    }
  } catch (error) {
    revokedCreate = error && error.name;
  }

  const symbolPolicy = trustedTypes.createPolicy("symbol-result", {
    createHTML: () => Symbol("result")
  });
  let symbolResultError = "none";
  try {
    symbolPolicy.createHTML("x");
  } catch (error) {
    symbolResultError = error && error.name;
  }

  return JSON.stringify({
    facts,
    methods: [
      typeof policy.createHTML,
      typeof policy.createScript,
      typeof policy.createScriptURL
    ],
    html: String(html),
    scriptURLCodePoint: String(scriptURL).codePointAt(0),
    missingErrors,
    conversionErrors,
    revokedCreate,
    revokedCall,
    symbolResultError
  });
})()
"#,
        )
        .expect("Trusted Types callback-function contract should evaluate");

    assert_eq!(
        result,
        r#"{"facts":{"getters":["createHTML","createScript","createScriptURL"],"callbackCalls":1,"ownCallReads":0,"inputConvertedBeforeCallback":true,"proxyApply":1,"thisIsUndefined":true,"arguments":["input",7,true,3]},"methods":["function","function","function"],"html":"","scriptURLCodePoint":65533,"missingErrors":["TypeError","TypeError","TypeError"],"conversionErrors":{"primitive":"TypeError","null-member":"TypeError","noncallable":"TypeError","getter":true,"missingName":"TypeError"},"revokedCreate":"accepted","revokedCall":"TypeError","symbolResultError":"TypeError"}"#
    );
}

#[test]
fn trusted_type_policy_options_convert_before_csp_policy_checks() {
    let mut vm = new_storage_test_vm("https://trusted-types-callback-csp-order.test/");
    vm.set_response_content_security_policies(&["trusted-types allowed".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const reads = [];
  let blockedError = "none";
  try {
    trustedTypes.createPolicy("blocked", {
      get createHTML() {
        reads.push("createHTML");
        return value => value;
      },
      get createScript() {
        reads.push("createScript");
        return undefined;
      },
      get createScriptURL() {
        reads.push("createScriptURL");
        return undefined;
      }
    });
  } catch (error) {
    blockedError = error && error.name;
  }
  return JSON.stringify({ reads, blockedError });
})()
"#,
        )
        .expect("Trusted Types options/CSP ordering should evaluate");

    assert_eq!(
        result,
        r#"{"reads":["createHTML","createScript","createScriptURL"],"blockedError":"TypeError"}"#
    );
}

#[tokio::test]
async fn trusted_type_policy_callbacks_use_and_retire_with_the_exact_callback_realm() {
    let mut vm = new_storage_test_vm("https://trusted-types-callback-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__trustedTypesCallbackFrame = frame;
})()
"#,
    )
    .expect("Trusted Types callback child Realm setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "Trusted Types callback child Realm setup",
    )
    .await;
    let _ = materialize_single_child_default_realm_for_test(
        &mut vm,
        "Trusted Types callback child Realm setup",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const frame = __trustedTypesCallbackFrame;
  const child = frame.contentWindow;
  child.__trustedTypesCallbackMarker = "child";
  child.__trustedTypesCallbackCalls = 0;
  const callback = child.Function(`
    return function(input) {
      "use strict";
      globalThis.__trustedTypesCallbackCalls += 1;
      const receiver = this === undefined;
      return {
        toString() {
          return [
            globalThis.__trustedTypesCallbackMarker,
            receiver,
            input
          ].join(":");
        }
      };
    };
  `)();
  const policy = trustedTypes.createPolicy("cross-realm", {
    createHTML: callback
  });
  const before = String(policy.createHTML("before"));

  const marker = new child.Error("callback-realm-error");
  const throwing = trustedTypes.createPolicy("cross-realm-throw", {
    createHTML: child.Function("marker", `
      return function() {
        "use strict";
        throw marker;
      };
    `)(marker)
  });
  let thrownIdentity = false;
  let thrownRealm = false;
  try {
    throwing.createHTML("throw");
  } catch (error) {
    thrownIdentity = error === marker;
    thrownRealm = error instanceof child.Error;
  }

  frame.remove();
  let retiredError = "none";
  try {
    policy.createHTML("after");
  } catch (error) {
    retiredError = error && error.name;
  }
  return JSON.stringify({
    before,
    thrownIdentity,
    thrownRealm,
    retiredError,
    callbackCalls: child.__trustedTypesCallbackCalls,
    detached: frame.contentWindow === null
  });
})()
"#,
        )
        .expect("Trusted Types callback Realm/retirement probe should evaluate");

    assert_eq!(
        result,
        r#"{"before":"child:true:before","thrownIdentity":true,"thrownRealm":true,"retiredError":"Error","callbackCalls":1,"detached":true}"#
    );
}
