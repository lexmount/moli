use super::*;


#[test]
fn trusted_type_to_json_returns_internal_strings() {
    let mut vm = new_storage_test_vm("https://trusted-type-to-json.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const policy = trustedTypes.createPolicy("to-json", {
    createHTML: value => value,
    createScript: value => value,
    createScriptURL: value => value
  });
  const html = policy.createHTML("html-value");
  const script = policy.createScript("script-value");
  const scriptURL = policy.createScriptURL("https://example.test/script.js");
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return error.constructor.name;
    }
  };
  const fakeValues = [
    Object.create(TrustedHTML.prototype),
    Object.create(TrustedScript.prototype),
    Object.create(TrustedScriptURL.prototype)
  ];
  return JSON.stringify({
    direct: [html.toJSON(), script.toJSON(), scriptURL.toJSON()],
    serialized: JSON.stringify({html, script, scriptURL}),
    fakeResults: [
      probe(() => TrustedHTML.prototype.toJSON.call(fakeValues[0])),
      probe(() => TrustedScript.prototype.toJSON.call(fakeValues[1])),
      probe(() => TrustedScriptURL.prototype.toJSON.call(fakeValues[2]))
    ]
  });
})()
"#,
        )
        .expect("Trusted Type toJSON probe should evaluate");

    assert_eq!(
        result,
        r#"{"direct":["html-value","script-value","https://example.test/script.js"],"serialized":"{\"html\":\"html-value\",\"script\":\"script-value\",\"scriptURL\":\"https://example.test/script.js\"}","fakeResults":["TypeError","TypeError","TypeError"]}"#
    );
}
#[test]
fn trusted_type_prototype_declared_methods_preserve_descriptors() {
    let mut vm = new_storage_test_vm("https://trusted-type-prototype-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const policy = trustedTypes.createPolicy("proto-methods", {
    createHTML: value => `<em>${value}</em>`,
    createScript: value => value,
    createScriptURL: value => value
  });
  const html = policy.createHTML("ok");
  const script = policy.createScript("1 + 1");
  const scriptURL = policy.createScriptURL("data:text/javascript,");
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith("__moliTrustedType"))
    .sort();
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return error.constructor.name;
    }
  };
  const describe = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      descriptor.enumerable,
      descriptor.writable,
      descriptor.configurable,
      descriptor.value.name,
      descriptor.value.length
    ].join(":");
  };
  const initialOwnSlots = {
    html: internalNames(html),
    script: internalNames(script),
    scriptURL: internalNames(scriptURL)
  };
  for (const prototype of [TrustedHTML.prototype, TrustedScript.prototype, TrustedScriptURL.prototype]) {
    prototype.__moliTrustedTypeKind = "html";
    prototype.__moliTrustedTypeValue = "prototype-spoof";
  }
  Object.assign(html, {
    __moliTrustedTypeKind: "script",
    __moliTrustedTypeValue: "own-html-spoof"
  });
  Object.assign(script, {
    __moliTrustedTypeKind: "html",
    __moliTrustedTypeValue: "own-script-spoof"
  });
  Object.assign(scriptURL, {
    __moliTrustedTypeKind: "html",
    __moliTrustedTypeValue: "own-script-url-spoof"
  });
  const fakeHTML = Object.assign(Object.create(TrustedHTML.prototype), {
    __moliTrustedTypeKind: "html",
    __moliTrustedTypeValue: "fake-html"
  });
  const fakeScript = Object.assign(Object.create(TrustedScript.prototype), {
    __moliTrustedTypeKind: "script",
    __moliTrustedTypeValue: "fake-script"
  });
  const fakeScriptURL = Object.assign(Object.create(TrustedScriptURL.prototype), {
    __moliTrustedTypeKind: "script-url",
    __moliTrustedTypeValue: "fake-script-url"
  });
  return JSON.stringify({
    prototypes: [
      Object.getPrototypeOf(html) === TrustedHTML.prototype,
      Object.getPrototypeOf(script) === TrustedScript.prototype,
      Object.getPrototypeOf(scriptURL) === TrustedScriptURL.prototype
    ],
    initialOwnSlots,
    spoofedOwnSlots: {
      html: internalNames(html),
      script: internalNames(script),
      scriptURL: internalNames(scriptURL)
    },
    htmlMethods: ["toString", "valueOf"].map(name => describe(TrustedHTML.prototype, name)),
    scriptMethods: ["toString", "valueOf"].map(name => describe(TrustedScript.prototype, name)),
    scriptURLMethods: ["toString", "valueOf"].map(name => describe(TrustedScriptURL.prototype, name)),
    values: [
      String(html),
      html.valueOf(),
      String(script),
      script.valueOf(),
      String(scriptURL),
      scriptURL.valueOf()
    ],
    fakeResults: [
      probe(() => TrustedHTML.prototype.toString.call(fakeHTML)),
      probe(() => TrustedHTML.prototype.valueOf.call(fakeHTML)),
      probe(() => TrustedScript.prototype.toString.call(fakeScript)),
      probe(() => TrustedScript.prototype.valueOf.call(fakeScript)),
      probe(() => TrustedScriptURL.prototype.toString.call(fakeScriptURL)),
      probe(() => TrustedScriptURL.prototype.valueOf.call(fakeScriptURL))
    ].join("|"),
    trustedChecks: [
      trustedTypes.isHTML(fakeHTML),
      trustedTypes.isScript(fakeScript),
      trustedTypes.isScriptURL(fakeScriptURL),
      trustedTypes.isHTML(html),
      trustedTypes.isScript(script),
      trustedTypes.isScriptURL(scriptURL)
    ]
  });
})()
"#,
        )
        .expect("TrustedType prototype method descriptors should evaluate");

    assert_eq!(
        result,
        r#"{"prototypes":[true,true,true],"initialOwnSlots":{"html":[],"script":[],"scriptURL":[]},"spoofedOwnSlots":{"html":["__moliTrustedTypeKind","__moliTrustedTypeValue"],"script":["__moliTrustedTypeKind","__moliTrustedTypeValue"],"scriptURL":["__moliTrustedTypeKind","__moliTrustedTypeValue"]},"htmlMethods":["false:true:true:toString:0","false:true:true:valueOf:0"],"scriptMethods":["false:true:true:toString:0","false:true:true:valueOf:0"],"scriptURLMethods":["false:true:true:toString:0","false:true:true:valueOf:0"],"values":["<em>ok</em>","<em>ok</em>","1 + 1","1 + 1","data:text/javascript,","data:text/javascript,"],"fakeResults":"TypeError|TypeError|TypeError|TypeError|TypeError|TypeError","trustedChecks":[false,false,false,true,true,true]}"#
    );
}
#[test]
fn trusted_type_policy_creation_obeys_csp_policy_name_list() {
    let mut vm = new_storage_test_vm("https://trusted-type-policy-csp.test/");
    vm.set_response_content_security_policies(&[
        "trusted-types SomeName default".to_owned(),
        "trusted-types * 'allow-duplicates'".to_owned(),
    ]);

    let result = vm
        .eval(
            r#"
(() => {
  const probe = name => {
    try {
      return trustedTypes.createPolicy(name, { createHTML: value => value }).name;
    } catch (error) {
      return `${error.name}:${error instanceof TypeError}`;
    }
  };
  return JSON.stringify({
    allowed: probe("SomeName"),
    defaultAllowed: probe("default"),
    blocked: probe("OtherName")
  });
})()
"#,
        )
        .expect("Trusted Types CSP policy-name probe should evaluate");

    assert_eq!(
        result,
        r#"{"allowed":"SomeName","defaultAllowed":"default","blocked":"TypeError:true"}"#
    );
}
#[test]
fn trusted_type_policy_creation_empty_and_none_csp_block_all_names() {
    for policy in ["trusted-types", "trusted-types 'nONe'"] {
        let mut vm = new_storage_test_vm("https://trusted-type-policy-csp-block.test/");
        vm.set_response_content_security_policies(&[policy.to_owned()]);

        let result = vm
            .eval(
                r#"
(() => {
  try {
    trustedTypes.createPolicy("SomeName", { createHTML: value => value });
    return "created";
  } catch (error) {
    return `${error.name}:${error instanceof TypeError}`;
  }
})()
"#,
            )
            .expect("Trusted Types blocking CSP policy-name probe should evaluate");

        assert_eq!(result, "TypeError:true", "{policy}");
    }
}
#[test]
fn trusted_types_eval_keyword_bypasses_default_policy_and_eval_csp() {
    let mut vm = new_storage_test_vm("https://trusted-types-eval-keyword.test/");
    vm.set_response_content_security_policies(&[
        "script-src 'trusted-types-eval'; require-trusted-types-for 'script'".to_owned(),
    ]);

    let result = vm
        .eval(
            r#"
(() => {
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push(`${event.effectiveDirective}:${event.blockedURI}`);
  });
  let defaultPolicyCalls = 0;
  trustedTypes.createPolicy("default", {
    createScript: value => {
      defaultPolicyCalls += 1;
      throw new Error(`default policy should not receive ${value}`);
    }
  });
  const trustedPolicy = trustedTypes.createPolicy("trusted-eval", {
    createScript: value => value
  });
  const direct = eval("40 + 2");
  const trustedDirect = eval(trustedPolicy.createScript("21 * 2"));
  const constructed = new Function("return 6 * 7")();
  return JSON.stringify({ direct, trustedDirect, constructed, defaultPolicyCalls, violations });
})()
"#,
        )
        .expect("trusted-types-eval keyword probe should evaluate");

    assert_eq!(
        result,
        r#"{"direct":42,"trustedDirect":42,"constructed":42,"defaultPolicyCalls":0,"violations":[]}"#
    );

    let mut without_enforcement =
        new_storage_test_vm("https://trusted-types-eval-without-enforcement.test/");
    without_enforcement
        .set_response_content_security_policies(&["script-src 'trusted-types-eval'".to_owned()]);
    assert_eq!(
        without_enforcement
            .eval(
                r#"(() => {
  try {
    eval("1");
    return "allowed";
  } catch (error) {
    return error.name;
  }
})()"#,
            )
            .expect("non-enforcing trusted-types-eval probe should evaluate"),
        "EvalError"
    );
}
#[test]
fn trusted_script_eval_is_unwrapped_without_trusted_types_enforcement() {
    let mut vm = new_storage_test_vm("https://trusted-script-eval.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const policy = trustedTypes.createPolicy("eval", { createScript: value => value });
  const trusted = policy.createScript("1 + 1");
  const ordinary = { value: "ordinary" };
  return JSON.stringify({
    direct: eval(trusted),
    indirect: eval?.(trusted),
    ordinaryPassesThrough: eval(ordinary) === ordinary
  });
})()
"#,
        )
        .expect("TrustedScript eval probe should evaluate");

    assert_eq!(
        result,
        r#"{"direct":2,"indirect":2,"ordinaryPassesThrough":true}"#
    );
}
#[test]
fn trusted_types_default_policy_receives_type_and_eval_sink_arguments() {
    let mut vm = new_storage_test_vm("https://trusted-types-default-callback-args.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const calls = [];
  trustedTypes.createPolicy("default", {
    createScript: function() {
      "use strict";
      calls.push([this === undefined, ...arguments]);
      return arguments[0];
    }
  });
  const direct = eval("2+2");
  const constructed = new Function("return 2+2")();
  return JSON.stringify({ direct, constructed, calls });
})()
"#,
        )
        .expect("Trusted Types default callback argument probe should evaluate");

    assert_eq!(
        result,
        r#"{"direct":4,"constructed":4,"calls":[[true,"2+2","TrustedScript","eval"],[true,"function anonymous(\n) {\nreturn 2+2\n}","TrustedScript","Function"]]}"#
    );
}
#[test]
fn trusted_types_eval_function_like_source_uses_the_eval_sink() {
    let mut vm = new_storage_test_vm("https://trusted-types-eval-sink.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const calls = [];
  trustedTypes.createPolicy("default", {
    createScript: (value, _type, sink) => {
      calls.push(sink);
      return sink === "Function" ? value : null;
    }
  });
  let evalResult;
  try {
    eval("function anonymous() { return 42; }");
    evalResult = "allowed";
  } catch (error) {
    evalResult = `${error.name}:${error instanceof EvalError}`;
  }
  const constructed = new Function("return 42")();
  return JSON.stringify({ evalResult, constructed, calls });
})()
"#,
        )
        .expect("Function-like eval source Trusted Types sink probe should evaluate");

    assert_eq!(
        result,
        r#"{"evalResult":"EvalError:true","constructed":42,"calls":["eval","Function"]}"#
    );
}
#[test]
fn trusted_types_eval_keyword_does_not_override_another_csp_policy() {
    let restrictive = "require-trusted-types-for 'script'; script-src 'self'";
    let trusted_types_eval = "default-src 'trusted-types-eval'";
    for policies in [
        [restrictive.to_owned(), trusted_types_eval.to_owned()],
        [trusted_types_eval.to_owned(), restrictive.to_owned()],
    ] {
        let mut vm = new_storage_test_vm("https://trusted-types-eval-multi-policy.test/");
        vm.set_response_content_security_policies(&policies);

        let result = vm
            .eval(
                r#"
(() => {
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push(`${event.effectiveDirective}:${event.blockedURI}`);
  });
  let defaultPolicyCalls = 0;
  trustedTypes.createPolicy("default", {
    createScript: value => {
      defaultPolicyCalls += 1;
      return value;
    }
  });
  const results = [];
  try {
    eval("40 + 2");
    results.push("eval:allowed");
  } catch (error) {
    results.push(`eval:${error.name}:${error instanceof EvalError}`);
  }
  try {
    new Function("return 42");
    results.push("Function:allowed");
  } catch (error) {
    results.push(`Function:${error.name}:${error instanceof EvalError}`);
  }
  globalThis.__trustedTypesEvalMultiPolicy = { results, defaultPolicyCalls, violations };
  return "queued";
})()
"#,
            )
            .expect("multi-policy trusted-types-eval probe should evaluate");

        assert_eq!(result, "queued");
        assert_eq!(
            drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
            2
        );
        assert_eq!(
            vm.eval("JSON.stringify(globalThis.__trustedTypesEvalMultiPolicy)")
                .expect("queued eval CSP violations should be observable"),
            r#"{"results":["eval:EvalError:true","Function:EvalError:true"],"defaultPolicyCalls":0,"violations":["script-src:eval","script-src:eval"]}"#
        );
    }
}
#[test]
fn range_create_contextual_fragment_enforces_trusted_html_sink() {
    let mut vm = new_storage_test_vm("https://range-contextual-fragment-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const events = [];
  self.addEventListener("securitypolicyviolation", event => {
    events.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      disposition: event.disposition,
      sample: event.sample,
      instance: event instanceof SecurityPolicyViolationEvent
    });
  });

  const host = document.createElement("div");
  const range = document.createRange();
  range.selectNodeContents(host);
  const policy = trustedTypes.createPolicy("range-html", {
    createHTML: value => `<b>${value}</b>`
  });
  const trustedText = range.createContextualFragment(policy.createHTML("ok")).textContent;

  let stringThrow = "none";
  try {
    range.createContextualFragment("<i>blocked</i>");
  } catch (error) {
    stringThrow = `${error.name}:${error instanceof TypeError}`;
  }

  let defaultSink = "";
  trustedTypes.createPolicy("default", {
    createHTML: (value, _, sink) => {
      defaultSink = sink;
      return `<em>${value}</em>`;
    }
  });
  const defaultText = range.createContextualFragment("allowed").textContent;

  globalThis.__rangeContextualFragmentTrustedTypes = {
    trustedText,
    stringThrow,
    defaultSink,
    defaultText,
    events
  };
  return "queued";
})()
"#,
        )
        .expect("Range.createContextualFragment TrustedHTML sink probe should evaluate");

    assert_eq!(result, "queued");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__rangeContextualFragmentTrustedTypes)")
            .expect("queued Range Trusted Types violation should be observable"),
        r#"{"trustedText":"ok","stringThrow":"TypeError:true","defaultSink":"Range createContextualFragment","defaultText":"allowed","events":[{"blockedURI":"trusted-types-sink","effectiveDirective":"require-trusted-types-for","disposition":"enforce","sample":"Range createContextualFragment|<i>blocked</i>","instance":true}]}"#
    );
}
#[test]
fn trusted_types_eval_reports_sanitized_source_url() {
    let mut vm = new_storage_test_vm("https://trusted-types-source.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const sourceFiles = [];
  document.addEventListener("securitypolicyviolation", event => {
    sourceFiles.push(event.sourceFile);
  });
  const policy = trustedTypes.createPolicy("source-test", { createScript: value => value });
  const inputs = [
    "https://user:password@dummy.test/script.js?query#fragment",
    "webpack://node_modules/sample/script.js",
    "data:text/javascript,void(0)",
    "relative-script.js"
  ];
  const errorNames = inputs.map(sourceURL => {
    const trustedScript = policy.createScript(`eval('');\n//# sourceURL=${sourceURL}`);
    try {
      eval(trustedScript);
      return "no-throw";
    } catch (error) {
      return error.name;
    }
  });
  globalThis.__trustedTypesSourceLocation = { errorNames, sourceFiles };
  return "queued";
})()
"#,
        )
        .expect("Trusted Types eval source location probe should evaluate");

    assert_eq!(result, "queued");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        4
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__trustedTypesSourceLocation)")
            .expect("queued Trusted Types source locations should be observable"),
        r#"{"errorNames":["EvalError","EvalError","EvalError","EvalError"],"sourceFiles":["https://dummy.test/script.js","webpack","data",""]}"#
    );
}
#[test]
fn runtime_evaluate_trusted_types_policy_matches_inspector_scope() {
    let mut vm = new_storage_test_vm("https://runtime-evaluate-trusted-types.test/");
    vm.set_response_content_security_policies(&[
        "require-trusted-types-for 'script'; script-src 'nonce-test'".to_owned(),
    ]);

    let default_nested = vm
        .evaluate_expression_payload_with_await(
            r#"(() => {
  try {
    return eval("40 + 2");
  } catch (error) {
    return error.name;
  }
})()"#,
            true,
            false,
        )
        .expect("default Runtime.evaluate should temporarily allow string code generation");
    assert_eq!(default_nested["type"], "number");
    assert_eq!(default_nested["value"], 42);

    let enforced = vm
        .begin_runtime_evaluate(
            None,
            r#"(() => {
  try {
    eval("1");
    return "allowed";
  } catch (error) {
    return error.name;
  }
})()"#,
            true,
            false,
            None,
            RuntimeEvaluateCodeGenerationPolicy::EnforceContextPolicy,
            RuntimeEvaluateResultMode::RemoteObject,
        )
        .and_then(|outcome| vm.require_completed_runtime_evaluate(outcome))
        .expect("explicit false Runtime.evaluate Trusted Types probe should complete");
    assert_eq!(enforced["type"], "string");
    assert_eq!(enforced["value"], "EvalError");

    let restored = vm
        .eval(
            r#"(() => {
  try {
    eval("1");
    return "allowed";
  } catch (error) {
    return error.name;
  }
})()"#,
        )
        .expect("page Trusted Types restoration probe should run");
    assert_eq!(restored, "EvalError");
}
#[test]
fn runtime_evaluate_does_not_invoke_a_transforming_trusted_types_default_policy() {
    let mut vm = new_storage_test_vm("https://runtime-evaluate-default-policy.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);
    vm.eval(
        r#"
globalThis.__runtimeEvaluateDefaultPolicyCalls = 0;
trustedTypes.createPolicy("default", {
  createScript: value => {
    globalThis.__runtimeEvaluateDefaultPolicyCalls += 1;
    return `${value}; void 0`;
  }
})
"#,
    )
    .expect("default Trusted Types policy should install");

    let direct = vm
        .evaluate_expression_payload_with_await("40 + 2", true, false)
        .expect("Runtime.evaluate should not pass through the page default policy");
    assert_eq!(direct["type"], "number");
    assert_eq!(direct["value"], 42);
    assert_eq!(
        vm.eval("globalThis.__runtimeEvaluateDefaultPolicyCalls")
            .expect("default policy invocation count should be readable"),
        "0"
    );
}

#[test]
fn trusted_script_code_like_brand_drives_function_constructors() {
    let mut vm = new_storage_test_vm("https://trusted-script-function-brand.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const policy = trustedTypes.createPolicy("function-brand", { createScript: value => value });
  const source = ["a", "b", "c = 5", "return (a + b) * c;"];
  const constructors = [
    Function,
    (async function() {}).constructor,
    (function*() {}).constructor,
    (async function*() {}).constructor
  ];
  const mixedBlocked = constructors.map(Constructor => {
    let blocked = 0;
    for (let mask = 0; mask < 15; mask++) {
      const args = source.map((value, index) =>
        mask & (2 ** index) ? policy.createScript(value) : value);
      try {
        new Constructor(...args);
      } catch (error) {
        blocked += error instanceof EvalError;
      }
    }
    return blocked;
  });
  const trusted = source.map(value => policy.createScript(value));
  const functions = constructors.map(Constructor => new Constructor(...trusted));
  let forgedBlocked = 0;
  for (let index = 0; index < source.length; index++) {
    const forged = trusted.slice();
    forged[index] = Object.assign(policy.createScript(source[index]), {
      toString: () => ` ${source[index]} `
    });
    try {
      new Function(...forged);
    } catch (error) {
      forgedBlocked += error instanceof EvalError;
    }
  }
  return JSON.stringify({
    mixedBlocked,
    trustedConstructorTypes: functions.map(value => typeof value),
    functionValues: [functions[0](1, 2, 3), functions[0](1, 2)],
    forgedBlocked
  });
})()
"#,
        )
        .expect("TrustedScript function constructor brand probe should evaluate");

    assert_eq!(
        result,
        r#"{"mixedBlocked":[15,15,15,15],"trustedConstructorTypes":["function","function","function","function"],"functionValues":[9,15],"forgedBlocked":4}"#
    );
}
