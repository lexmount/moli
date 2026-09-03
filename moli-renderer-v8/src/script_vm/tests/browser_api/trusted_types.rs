use super::*;

#[test]
fn trusted_type_policy_callbacks_follow_webidl_dictionary_and_callback_rules() {
    let mut vm = new_storage_test_vm("https://trusted-type-policy-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error.constructor.name;
    }
  };
  const methodTypes = policy => [
    typeof policy.createHTML,
    typeof policy.createScript,
    typeof policy.createScriptURL
  ];

  const emptyFromNull = trustedTypes.createPolicy("empty-null", null);
  const emptyFromOmission = trustedTypes.createPolicy("empty-omitted");
  const extra = { marker: "extra" };
  let observed;
  class NullCallbacks {
    createHTML(input, ...rest) {
      observed = [this === undefined, input, rest.length, rest[0] === extra, rest[1]];
      return null;
    }
    createScript() {
      return null;
    }
    createScriptURL() {
      return null;
    }
  }
  const nullPolicy = trustedTypes.createPolicy("null-results", new NullCallbacks());
  const nullResults = [
    String(nullPolicy.createHTML({ toString: () => "converted" }, extra, 42)),
    String(nullPolicy.createScript("script")),
    String(nullPolicy.createScriptURL("script-url"))
  ];

  const variadicPolicy = trustedTypes.createPolicy("variadic", {
    createHTML: (a, b, c) => a + b + c,
    createScript: (a, b, c) => a + b + c,
    createScriptURL: (a, b, c) => a + b + c
  });
  const variadicResults = [
    String(variadicPolicy.createHTML("a", "b", "c")),
    String(variadicPolicy.createScript("a", "b")),
    String(variadicPolicy.createScriptURL("a", 123, null))
  ];

  return JSON.stringify({
    emptyMethods: [methodTypes(emptyFromNull), methodTypes(emptyFromOmission)],
    policyBrand: nullPolicy instanceof TrustedTypePolicy,
    nullResults,
    observed,
    variadicResults,
    errors: [
      errorName(() => trustedTypes.createPolicy("primitive-options", 1)),
      errorName(() => trustedTypes.createPolicy("non-callable", { createHTML: null })),
      errorName(() => trustedTypes.createPolicy("throwing-getter", {
        get createHTML() { throw new RangeError("getter"); }
      })),
      errorName(() => emptyFromNull.createHTML("missing callback")),
      errorName(() => nullPolicy.createHTML()),
      errorName(() => emptyFromOmission.createHTML("html")),
      errorName(() => emptyFromOmission.createScript("script")),
      errorName(() => emptyFromOmission.createScriptURL("script-url"))
    ]
  });
})()
"#,
        )
        .expect("TrustedTypePolicy WebIDL callback probe should evaluate");

    assert_eq!(
        result,
        r#"{"emptyMethods":[["function","function","function"],["function","function","function"]],"policyBrand":true,"nullResults":["","",""],"observed":[true,"converted",2,true,42],"variadicResults":["abc","abundefined","a123null"],"errors":["TypeError","TypeError","RangeError","TypeError","TypeError","TypeError","TypeError","TypeError"]}"#
    );
}

#[test]
fn trusted_type_factory_interface_exposes_stable_branded_empty_values() {
    let mut vm = new_storage_test_vm("https://trusted-type-factory-empty-values.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error.constructor.name;
    }
  };
  const assign = (name, value) => {
    "use strict";
    trustedTypes[name] = value;
  };
  const prototype = TrustedTypePolicyFactory.prototype;
  const describeAccessor = name => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      typeof descriptor.get,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable,
      Object.hasOwn(trustedTypes, name)
    ];
  };
  const emptyHTML = trustedTypes.emptyHTML;
  const emptyScript = trustedTypes.emptyScript;
  const errors = [
    errorName(() => new TrustedTypePolicyFactory()),
    errorName(() => assign("emptyHTML", "fake")),
    errorName(() => assign("emptyScript", "fake")),
    errorName(() => Object.getOwnPropertyDescriptor(prototype, "emptyHTML").get.call({}))
  ];
  return JSON.stringify({
    factory: [
      TrustedTypePolicyFactory.name,
      TrustedTypePolicyFactory.length,
      trustedTypes instanceof TrustedTypePolicyFactory,
      Object.getPrototypeOf(trustedTypes) === prototype,
      Object.hasOwn(prototype, "createPolicy"),
      Object.hasOwn(trustedTypes, "createPolicy")
    ],
    accessors: [describeAccessor("emptyHTML"), describeAccessor("emptyScript")],
    html: [
      trustedTypes.isHTML(emptyHTML),
      String(emptyHTML),
      emptyHTML === trustedTypes.emptyHTML
    ],
    script: [
      trustedTypes.isScript(emptyScript),
      String(emptyScript),
      emptyScript === trustedTypes.emptyScript
    ],
    errors
  });
})()
"#,
        )
        .expect("TrustedTypePolicyFactory empty value probe should evaluate");

    assert_eq!(
        result,
        r#"{"factory":["TrustedTypePolicyFactory",0,true,true,true,false],"accessors":[["function",true,true,true,false],["function",true,true,true,false]],"html":[true,"",true],"script":[true,"",true],"errors":["TypeError","TypeError","TypeError","TypeError"]}"#
    );
}

#[test]
fn trusted_type_factory_introspection_reuses_current_sink_classification() {
    let mut vm = new_storage_test_vm("https://trusted-type-attribute-types.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const HTML = "http://www.w3.org/1999/xhtml";
  const SVG = "http://www.w3.org/2000/svg";
  const MATHML = "http://www.w3.org/1998/Math/MathML";
  const XLINK = "http://www.w3.org/1999/xlink";
  const OTHER = "https://example.test/namespace";
  const type = (...args) => trustedTypes.getAttributeType(...args);
  const propertyType = (...args) => trustedTypes.getPropertyType(...args);
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error.constructor.name;
    }
  };
  const attributeDescriptor = Object.getOwnPropertyDescriptor(
    TrustedTypePolicyFactory.prototype,
    "getAttributeType"
  );
  const propertyDescriptor = Object.getOwnPropertyDescriptor(
    TrustedTypePolicyFactory.prototype,
    "getPropertyType"
  );
  const describe = descriptor => [
    typeof descriptor.value,
    descriptor.value.name,
    descriptor.value.length,
    descriptor.enumerable,
    descriptor.configurable
  ];

  return JSON.stringify({
    interface: [
      describe(attributeDescriptor),
      describe(propertyDescriptor),
      Object.hasOwn(trustedTypes, "getAttributeType"),
      Object.hasOwn(trustedTypes, "getPropertyType")
    ],
    htmlDefaults: [
      type("script", "src"),
      type("SCRIPT", "SRC", undefined, undefined),
      type("script", "src", null, null),
      type("script", "src", "", ""),
      type("script", "src", HTML, ""),
      type("ſcript", "src"),
      type("script", "ſrc")
    ],
    iframeAttributes: [
      type("iframe", "srcdoc"),
      type("IFRAME", "SRCDOC", HTML, null),
      type("iframe", "srcdoc", HTML, ""),
      type("iframe", "srcdoc", HTML, OTHER),
      type("div", "srcdoc")
    ],
    urls: [
      type("embed", "src"),
      type("object", "data"),
      type("object", "codebase"),
      type("script", "href"),
      type("script", "href", SVG),
      type("SCRIPT", "HREF", SVG, XLINK),
      type("script", "href", SVG, OTHER),
      type("script", "href", SVG.toUpperCase())
    ],
    properties: [
      propertyType("script", "text"),
      propertyType("SCRIPT", "src"),
      propertyType("script", "sRc"),
      propertyType("div", "innerHTML"),
      propertyType("foo", "outerHTML", OTHER),
      propertyType("script", "src", SVG),
      propertyType("embed", "src"),
      propertyType("object", "data"),
      propertyType("object", "codeBase")
    ],
    handlers: [
      type("div", "onclick"),
      type("g", "ondblclick", SVG),
      type("mrow", "onmousedown", MATHML),
      type("div", "onclick", HTML, OTHER),
      type("foo", "onmouseup", OTHER),
      type("div", "ondoesnotexist")
    ],
    errors: [
      errorName(() => type()),
      errorName(() => type("script")),
      errorName(() => attributeDescriptor.value.call({}, "script", "src")),
      errorName(() => propertyType()),
      errorName(() => propertyType("script")),
      errorName(() => propertyDescriptor.value.call({}, "script", "src"))
    ]
  });
})()
"#,
        )
        .expect("TrustedTypePolicyFactory introspection probe should evaluate");

    assert_eq!(
        result,
        r#"{"interface":[["function","getAttributeType",2,true,true],["function","getPropertyType",2,true,true],false,false],"htmlDefaults":["TrustedScriptURL","TrustedScriptURL","TrustedScriptURL","TrustedScriptURL","TrustedScriptURL",null,null],"iframeAttributes":["TrustedHTML","TrustedHTML","TrustedHTML",null,null],"urls":[null,null,null,null,"TrustedScriptURL","TrustedScriptURL",null,null],"properties":["TrustedScript","TrustedScriptURL",null,"TrustedHTML","TrustedHTML",null,null,null,null],"handlers":["TrustedScript","TrustedScript","TrustedScript",null,null,null],"errors":["TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"]}"#
    );
}

#[test]
fn trusted_type_factory_default_policy_getter_tracks_the_branded_policy() {
    let mut vm = new_storage_test_vm("https://trusted-type-factory-default-policy.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error.constructor.name;
    }
  };
  const initial = trustedTypes.defaultPolicy;
  const policy = trustedTypes.createPolicy("default", { createHTML: value => value });
  const other = trustedTypes.createPolicy("other", { createHTML: value => value });
  const descriptor = Object.getOwnPropertyDescriptor(
    TrustedTypePolicyFactory.prototype,
    "defaultPolicy"
  );
  const errors = [
    errorName(() => {
      "use strict";
      trustedTypes.defaultPolicy = other;
    }),
    errorName(() => descriptor.get.call({})),
    errorName(() => new TrustedTypePolicy())
  ];
  return JSON.stringify({
    initialIsNull: initial === null,
    policy: [
      policy instanceof TrustedTypePolicy,
      policy.name,
      trustedTypes.defaultPolicy === policy,
      trustedTypes.defaultPolicy !== other
    ],
    descriptor: [
      typeof descriptor.get,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable
    ],
    errors
  });
})()
"#,
        )
        .expect("TrustedTypePolicyFactory default policy probe should evaluate");

    assert_eq!(
        result,
        r#"{"initialIsNull":true,"policy":[true,"default",true,true],"descriptor":["function",true,true,true],"errors":["TypeError","TypeError","TypeError"]}"#
    );
}

#[test]
fn trusted_type_policy_creation_reports_name_and_duplicate_csp_violations() {
    let mut vm = new_storage_test_vm("https://trusted-type-policy-csp.test/");
    vm.set_response_content_security_policies(&[
        "trusted-types allowed reportOnly duplicate".to_owned()
    ]);
    vm.set_response_content_security_report_only_policies(&[
        "trusted-types allowed duplicate".to_owned()
    ]);

    let result = vm
        .eval(
            r#"
(() => {
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      originalPolicy: event.originalPolicy,
      disposition: event.disposition,
      sample: event.sample
    });
  });
  globalThis.__trustedTypePolicyCspViolations = violations;
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error.name;
    }
  };

  const allowed = trustedTypes.createPolicy("allowed", {});
  const reportOnly = trustedTypes.createPolicy("reportOnly", {});
  const duplicate = trustedTypes.createPolicy("duplicate", {});
  const duplicateError = errorName(() => trustedTypes.createPolicy("duplicate", {}));
  const blockedError = errorName(() => trustedTypes.createPolicy("blocked", {}));

  return JSON.stringify({
    names: [allowed.name, reportOnly.name, duplicate.name],
    duplicateError,
    blockedError,
    violations
  });
})()
"#,
        )
        .expect("TrustedTypePolicy CSP creation probe should evaluate");

    assert_eq!(
        result,
        r#"{"names":["allowed","reportOnly","duplicate"],"duplicateError":"TypeError","blockedError":"TypeError","violations":[]}"#
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        5
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__trustedTypePolicyCspViolations)")
            .expect("queued TrustedTypePolicy CSP violations should be observable"),
        r#"[{"blockedURI":"trusted-types-policy","effectiveDirective":"trusted-types","originalPolicy":"trusted-types allowed duplicate","disposition":"report","sample":"reportOnly"},{"blockedURI":"trusted-types-policy","effectiveDirective":"trusted-types","originalPolicy":"trusted-types allowed reportOnly duplicate","disposition":"enforce","sample":"duplicate"},{"blockedURI":"trusted-types-policy","effectiveDirective":"trusted-types","originalPolicy":"trusted-types allowed duplicate","disposition":"report","sample":"duplicate"},{"blockedURI":"trusted-types-policy","effectiveDirective":"trusted-types","originalPolicy":"trusted-types allowed reportOnly duplicate","disposition":"enforce","sample":"blocked"},{"blockedURI":"trusted-types-policy","effectiveDirective":"trusted-types","originalPolicy":"trusted-types allowed duplicate","disposition":"report","sample":"blocked"}]"#
    );
}

#[test]
fn element_markup_sinks_enforce_trusted_html_and_standard_sink_names() {
    let mut vm = new_storage_test_vm("https://element-markup-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const rejected = [];

  const inner = document.createElement("div");
  rejected.push(throwsTypeError(() => { inner.innerHTML = "<b>blocked</b>"; }));
  rejected.push(throwsTypeError(() => { inner.innerHTML = null; }));

  const shadowHost = document.createElement("div");
  const shadow = shadowHost.attachShadow({ mode: "open" });
  rejected.push(throwsTypeError(() => { shadow.innerHTML = "<b>blocked</b>"; }));

  const outerContainer = document.createElement("div");
  const outer = document.createElement("span");
  outerContainer.appendChild(outer);
  rejected.push(throwsTypeError(() => { outer.outerHTML = "<b>blocked</b>"; }));

  const unsafe = document.createElement("div");
  rejected.push(throwsTypeError(() => unsafe.setHTMLUnsafe("<b>blocked</b>")));
  rejected.push(throwsTypeError(() => shadow.setHTMLUnsafe("<b>blocked</b>")));

  const adjacent = document.createElement("div");
  rejected.push(throwsTypeError(() => adjacent.insertAdjacentHTML("beforeend", "<b>blocked</b>")));

  const policy = trustedTypes.createPolicy("element-markup", {
    createHTML: value => value
  });
  const trusted = policy.createHTML("<b>trusted</b>");
  const documentRoot = document.documentElement ||
    document.appendChild(document.createElement("html"));
  let documentOuterError = "none";
  try {
    documentRoot.outerHTML = trusted;
  } catch (error) {
    documentOuterError = `${error.name}:${error.code}`;
  }
  inner.innerHTML = trusted;
  shadow.innerHTML = trusted;
  unsafe.setHTMLUnsafe(trusted);
  shadow.setHTMLUnsafe(trusted);
  adjacent.insertAdjacentHTML("beforeend", trusted);

  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createHTML: (value, type, sink) => {
      defaultCalls.push([value, type, sink]);
      return value;
    }
  });
  const defaultInner = document.createElement("div");
  defaultInner.innerHTML = null;
  const defaultOuterContainer = document.createElement("div");
  const defaultOuter = document.createElement("span");
  defaultOuterContainer.appendChild(defaultOuter);
  defaultOuter.outerHTML = null;
  const defaultUnsafe = document.createElement("div");
  defaultUnsafe.setHTMLUnsafe(null);
  const defaultAdjacent = document.createElement("div");
  defaultAdjacent.insertAdjacentHTML("beforeend", null);

  return JSON.stringify({
    rejected,
    documentOuterError,
    accepted: [
      inner.innerHTML,
      shadow.innerHTML,
      unsafe.innerHTML,
      adjacent.innerHTML
    ],
    defaultValues: [
      defaultInner.innerHTML,
      defaultOuterContainer.innerHTML,
      defaultUnsafe.innerHTML,
      defaultAdjacent.innerHTML
    ],
    defaultCalls
  });
})()
"#,
        )
        .expect("Element markup TrustedHTML sink probe should evaluate");

    assert_eq!(
        result,
        r#"{"rejected":[true,true,true,true,true,true,true],"documentOuterError":"NoModificationAllowedError:7","accepted":["<b>trusted</b>","<b>trusted</b>","<b>trusted</b>","<b>trusted</b>"],"defaultValues":["","","null","null"],"defaultCalls":[["","TrustedHTML","Element innerHTML"],["","TrustedHTML","Element outerHTML"],["null","TrustedHTML","Element setHTMLUnsafe"],["null","TrustedHTML","Element insertAdjacentHTML"]]}"#
    );
}

#[tokio::test]
async fn local_child_realm_inherits_parent_meta_trusted_types_policy() {
    let mut vm = new_storage_test_vm("https://trusted-types-local-child-inheritance.test/");

    vm.eval(
        r#"
(() => {
  const policy = trustedTypes.createPolicy("parent-pass-through", {
    createHTML: value => value
  });
  const root = document.documentElement ||
    document.appendChild(document.createElement("html"));
  const head = document.head || root.appendChild(document.createElement("head"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const meta = document.createElement("meta");
  meta.httpEquiv = "Content-Security-Policy";
  meta.content = "require-trusted-types-for 'script'; trusted-types parent-pass-through";
  head.appendChild(meta);

  const frame = document.createElement("iframe");
  frame.id = "trusted-types-local-child";
  frame.srcdoc = policy.createHTML("<!doctype html><div id='target'></div>");
  body.appendChild(frame);
})()
"#,
    )
    .expect("local child Trusted Types inheritance setup should evaluate");

    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "local child should commit before Trusted Types inheritance checks",
    )
    .await;

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById("trusted-types-local-child");
  const child = frame.contentWindow;
  const target = frame.contentDocument.getElementById("target");
  const outcome = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return `${error.name}:${error instanceof child.TypeError}`;
    }
  };

  const childSink = outcome(() => { target.innerHTML = "unsafe"; });
  const childPolicy = outcome(() => {
    child.trustedTypes.createPolicy("blocked", { createHTML: value => value });
  });
  const adopted = document.adoptNode(target);
  const adoptedSink = outcome(() => { adopted.innerHTML = "unsafe"; });

  return JSON.stringify({
    childSink,
    childPolicy,
    adoptedSink,
    ownerIsTop: adopted.ownerDocument === document
  });
})()
"#,
        )
        .expect("local child Trusted Types inheritance checks should evaluate");

    assert_eq!(
        result,
        r#"{"childSink":"TypeError:true","childPolicy":"TypeError:true","adoptedSink":"TypeError:true","ownerIsTop":true}"#
    );
}

#[test]
fn document_parse_html_unsafe_gates_converted_union_source() {
    let mut vm = new_storage_test_vm("https://document-parse-html-unsafe-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error && error.name;
    }
  };
  const custom = trustedTypes.createPolicy("document-parse-html-unsafe-custom", {
    createHTML: value => value
  });
  const blocked = [
    errorName(() => Document.parseHTMLUnsafe("<p>blocked</p>")),
    errorName(() => Document.parseHTMLUnsafe(null))
  ];
  const trusted = Document.parseHTMLUnsafe(
    custom.createHTML("<main>trusted</main>")
  ).body.innerText;

  let sourceConversions = 0;
  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createHTML: (value, type, sink) => {
      defaultCalls.push([value, type, sink]);
      return value === "source" ? "<p>default</p>" : value;
    }
  });
  const source = {
    toString() {
      sourceConversions += 1;
      return "source";
    }
  };
  const defaultValues = [
    Document.parseHTMLUnsafe(source).body.innerText,
    Document.parseHTMLUnsafe(null).body.innerText
  ];
  const callsBeforeMissing = defaultCalls.length;
  const missing = errorName(() => Document.parseHTMLUnsafe());
  const missingSkippedPolicy = defaultCalls.length === callsBeforeMissing;
  const callsBeforeSymbol = defaultCalls.length;
  const symbol = errorName(() => Document.parseHTMLUnsafe(Symbol()));
  const symbolSkippedPolicy = defaultCalls.length === callsBeforeSymbol;

  return JSON.stringify({
    blocked,
    trusted,
    defaultValues,
    sourceConversions,
    missing,
    missingSkippedPolicy,
    symbol,
    symbolSkippedPolicy,
    defaultCalls
  });
})()
"#,
        )
        .expect("Document.parseHTMLUnsafe TrustedHTML union probe should evaluate");

    assert_eq!(
        result,
        r#"{"blocked":["TypeError","TypeError"],"trusted":"trusted","defaultValues":["default","null"],"sourceConversions":1,"missing":"TypeError","missingSkippedPolicy":true,"symbol":"TypeError","symbolSkippedPolicy":true,"defaultCalls":[["source","TrustedHTML","Document parseHTMLUnsafe"],["null","TrustedHTML","Document parseHTMLUnsafe"]]}"#
    );
}

#[test]
fn dom_parser_gates_converted_union_source_after_webidl_argument_conversion() {
    let mut vm = new_storage_test_vm("https://dom-parser-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error && error.name;
    }
  };
  const parser = new DOMParser();
  const custom = trustedTypes.createPolicy("dom-parser-custom", {
    createHTML: value => value
  });
  const blocked = [
    errorName(() => parser.parseFromString("<p>blocked</p>", "text/html")),
    errorName(() => parser.parseFromString(null, "text/html")),
    errorName(() => parser.parseFromString("<root/>", "application/xml"))
  ];
  const accepted = [
    parser.parseFromString(
      custom.createHTML("<main>trusted</main>"),
      "text/html"
    ).body.innerText,
    parser.parseFromString(
      custom.createHTML("<root/>"),
      "application/xml"
    ).documentElement.tagName
  ];

  let sourceConversions = 0;
  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createHTML: (value, type, sink) => {
      defaultCalls.push([value, type, sink]);
      return value === "source" ? "<p>default</p>" : value;
    }
  });
  const source = {
    toString() {
      sourceConversions += 1;
      return "source";
    }
  };
  const defaultValues = [
    parser.parseFromString(source, "text/html").body.innerText,
    parser.parseFromString(null, "text/html").body.innerText,
    parser.parseFromString("<root/>", "application/xml").documentElement.tagName
  ];
  const callsBeforeInvalidType = defaultCalls.length;
  const invalidType = errorName(() => parser.parseFromString(source, "TEXT/html"));
  const invalidTypeSkippedPolicy = defaultCalls.length === callsBeforeInvalidType;

  return JSON.stringify({
    blocked,
    accepted,
    defaultValues,
    sourceConversions,
    invalidType,
    invalidTypeSkippedPolicy,
    symbolSource: errorName(() => parser.parseFromString(Symbol(), "text/html")),
    defaultCalls
  });
})()
"#,
        )
        .expect("DOMParser TrustedHTML union probe should evaluate");

    assert_eq!(
        result,
        r#"{"blocked":["TypeError","TypeError","TypeError"],"accepted":["trusted","root"],"defaultValues":["default","null","root"],"sourceConversions":2,"invalidType":"TypeError","invalidTypeSkippedPolicy":true,"symbolSource":"TypeError","defaultCalls":[["source","TrustedHTML","DOMParser parseFromString"],["null","TrustedHTML","DOMParser parseFromString"],["<root/>","TrustedHTML","DOMParser parseFromString"]]}"#
    );
}

#[test]
fn document_exec_command_gates_only_insert_html_values() {
    let mut vm = new_storage_test_vm("https://exec-command-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error && error.name;
    }
  };
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const input = document.createElement("input");
  body.append(input);
  input.focus();

  const blocked = [
    errorName(() => document.execCommand("insertHTML", false, "<b>blocked</b>")),
    errorName(() => document.execCommand("InSeRtHtMl", false, null))
  ];
  const unprotected = [
    errorName(() => document.execCommand("insertHTML")),
    errorName(() => document.execCommand("insertHTML", false, undefined)),
    errorName(() => document.execCommand("paste", false, "<b>plain</b>")),
    errorName(() => document.execCommand("insertText", false, "plain"))
  ];

  input.value = "";
  const custom = trustedTypes.createPolicy("exec-command-custom", {
    createHTML: value => value
  });
  const trustedReturned = document.execCommand(
    "insertHTML",
    false,
    custom.createHTML("<b>trusted</b>")
  );
  const trustedValue = input.value;

  let sourceConversions = 0;
  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createHTML: (value, type, sink) => {
      defaultCalls.push([value, type, sink]);
      return "<b>default</b>";
    }
  });
  input.value = "";
  const defaultReturned = document.execCommand("insertHTML", false, {
    toString() {
      sourceConversions += 1;
      return "<i>original</i>";
    }
  });

  return JSON.stringify({
    blocked,
    unprotected,
    trusted: [trustedReturned, trustedValue],
    defaulted: [defaultReturned, input.value, sourceConversions],
    defaultCalls
  });
})()
"#,
        )
        .expect("Document.execCommand TrustedHTML sink probe should evaluate");

    assert_eq!(
        result,
        r#"{"blocked":["TypeError","TypeError"],"unprotected":["none","none","none","none"],"trusted":[true,"trusted"],"defaulted":[true,"default",1],"defaultCalls":[["<i>original</i>","TrustedHTML","Document execCommand"]]}"#
    );
}

#[test]
fn document_write_gates_concatenated_union_values_before_writeln_newline() {
    let mut vm = new_storage_test_vm("https://document-write-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error && error.name;
    }
  };
  const custom = trustedTypes.createPolicy("document-write-custom", {
    createHTML: value => `(${value})`
  });
  const doc = new DOMParser().parseFromString(trustedTypes.emptyHTML, "text/html");
  const replacementDoc = new DOMParser().parseFromString(
    custom.createHTML("seed"),
    "text/html"
  );
  const reset = () => { doc.body.innerHTML = trustedTypes.emptyHTML; };
  const blocked = errorName(() => doc.write("blocked"));

  doc.write(custom.createHTML("1"), custom.createHTML("2"));
  const allTrusted = doc.body.innerHTML;
  replacementDoc.write(custom.createHTML("replacement"));
  replacementDoc.writeln(custom.createHTML("tail"));
  const replacesDetachedContent = replacementDoc.body.innerHTML;
  reset();

  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createHTML: (value, type, sink) => {
      defaultCalls.push([value, type, sink]);
      return `[${value}]`;
    }
  });

  doc.write("1", "2");
  const strings = doc.body.innerHTML;
  reset();
  doc.write(custom.createHTML("1"), "2");
  const mixed = doc.body.innerHTML;
  reset();
  doc.writeln("3", "4");
  const stringLine = doc.body.innerHTML;
  reset();
  doc.writeln(custom.createHTML("3"), custom.createHTML("4"));
  const trustedLine = doc.body.innerHTML;
  reset();
  doc.writeln();
  const emptyLine = doc.body.innerHTML;

  return JSON.stringify({
    blocked,
    allTrusted,
    replacesDetachedContent,
    strings,
    mixed,
    stringLine,
    trustedLine,
    emptyLine,
    defaultCalls
  });
})()
"#,
        )
        .expect("Document.write TrustedHTML union probe should evaluate");

    assert_eq!(
        result,
        r#"{"blocked":"TypeError","allTrusted":"(1)(2)","replacesDetachedContent":"(replacement)(tail)\n","strings":"[12]","mixed":"[(1)2]","stringLine":"[34]\n","trustedLine":"(3)(4)\n","emptyLine":"\n","defaultCalls":[["12","TrustedHTML","Document write"],["(1)2","TrustedHTML","Document write"],["34","TrustedHTML","Document writeln"]]}"#
    );
}

#[test]
fn script_elements_preserve_only_parser_or_trusted_script_source() {
    let mut vm = new_storage_test_vm("https://script-source-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body ||
    (document.documentElement || document.appendChild(document.createElement("html")))
      .appendChild(document.createElement("body"));
  globalThis.__trustedScriptRuns = [];
  const policy = trustedTypes.createPolicy("script-source", {
    createScript: value => value
  });
  const rejected = document.createElement("script");
  let rejectedPlainText = false;
  try {
    rejected.text = "globalThis.__trustedScriptRuns.push('plain-text')";
  } catch (error) {
    rejectedPlainText = error instanceof TypeError;
  }
  let rejectedPlainInnerText = false;
  try {
    document.createElement("script").innerText = "blocked";
  } catch (error) {
    rejectedPlainInnerText = error instanceof TypeError;
  }
  let rejectedPlainTextContent = false;
  try {
    document.createElement("script").textContent = "blocked";
  } catch (error) {
    rejectedPlainTextContent = error instanceof TypeError;
  }

  const trusted = document.createElement("script");
  trusted.text = policy.createScript("globalThis.__trustedScriptRuns.push('trusted')");
  root.appendChild(trusted);

  const trustedInnerText = document.createElement("script");
  trustedInnerText.innerText = policy.createScript(
    "globalThis.__trustedScriptRuns.push('inner-text')"
  );
  root.appendChild(trustedInnerText);

  const trustedTextContent = document.createElement("script");
  trustedTextContent.textContent = policy.createScript(
    "globalThis.__trustedScriptRuns.push('text-content')"
  );
  root.appendChild(trustedTextContent);

  const nodeText = document.createElement("script");
  Object.getOwnPropertyDescriptor(Node.prototype, "textContent").set.call(
    nodeText,
    "globalThis.__trustedScriptRuns.push('node-text')"
  );
  let blockedAppendThrew = false;
  try {
    root.appendChild(nodeText);
  } catch (_error) {
    blockedAppendThrew = true;
  }

  const split = document.createElement("script");
  split.text = policy.createScript("globalThis.__trustedScriptRuns.push('split')");
  split.firstChild.splitText(4);
  split.normalize();
  root.appendChild(split);

  const cloneSource = document.createElement("script");
  cloneSource.text = policy.createScript("globalThis.__trustedScriptRuns.push('clone')");
  root.appendChild(cloneSource.cloneNode(true));

  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createScript: (value, type, sink) => {
      defaultCalls.push([value, type, sink]);
      return value === "default-token"
        ? "globalThis.__trustedScriptRuns.push('default')"
        : null;
    }
  });
  const defaulted = document.createElement("script");
  defaulted.appendChild(document.createTextNode("default-token"));
  root.appendChild(defaulted);

  return JSON.stringify({
    rejectedPlainText,
    rejectedPlainInnerText,
    rejectedPlainTextContent,
    blockedAppendThrew,
    runs: globalThis.__trustedScriptRuns,
    defaultCalls
  });
})()
"#,
        )
        .expect("script-element Trusted Types source probe should evaluate");

    assert_eq!(
        result,
        r#"{"rejectedPlainText":true,"rejectedPlainInnerText":true,"rejectedPlainTextContent":true,"blockedAppendThrew":false,"runs":["trusted","inner-text","text-content","split","default"],"defaultCalls":[["default-token","TrustedScript","HTMLScriptElement text"]]}"#
    );
}

#[test]
fn trusted_types_default_policy_can_make_changed_empty_script_sources_executable() {
    let mut vm = new_storage_test_vm("https://empty-script-source-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let document = vm.document_runtime.document_handle();
    let html = vm.document_runtime.dom_host_mut().create_element("html");
    let body = vm.document_runtime.dom_host_mut().create_element("body");
    let html_container = vm.document_runtime.dom_host_mut().create_element("div");
    let svg_container = vm
        .document_runtime
        .dom_host_mut()
        .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg")
        .expect("SVG container should be created");
    assert!(vm.document_runtime.dom_host_mut().set_attribute(
        html_container,
        "id",
        "html-container"
    ));
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .set_attribute(svg_container, "id", "svg-container")
    );
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(document, html)
    );
    assert!(vm.document_runtime.dom_host_mut().append_child(html, body));
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(body, html_container)
    );
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(body, svg_container)
    );

    let html_script = vm.document_runtime.dom_host_mut().create_element("script");
    let svg_script = vm
        .document_runtime
        .dom_host_mut()
        .create_element_ns(Some("http://www.w3.org/2000/svg"), "script")
        .expect("SVG script should be created");
    for (script, id, parent) in [
        (html_script, "html-script", html_container),
        (svg_script, "svg-script", svg_container),
    ] {
        assert!(
            vm.document_runtime
                .dom_host_mut()
                .set_attribute(script, "id", id)
        );
        assert!(
            vm.document_runtime
                .dom_host_mut()
                .set_attribute(script, "type", "unknown")
        );
        let text = vm.document_runtime.dom_host_mut().create_text_node(";");
        assert!(
            vm.document_runtime
                .dom_host_mut()
                .append_child(script, text)
        );
        assert!(
            vm.document_runtime
                .dom_host_mut()
                .finish_parsing_script_children(script)
        );
        assert!(
            vm.document_runtime
                .dom_host_mut()
                .append_child(parent, script)
        );
    }

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__emptyScriptRuns = [];
  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createScript(value, type, sink) {
      defaultCalls.push([value, type, sink]);
      return value.length
        ? ""
        : `globalThis.__emptyScriptRuns.push(${JSON.stringify(sink)})`;
    }
  });

  for (const [scriptId, containerId] of [
    ["html-script", "html-container"],
    ["svg-script", "svg-container"]
  ]) {
    const script = document.getElementById(scriptId);
    script.remove();
    script.removeAttribute("type");
    script.firstChild.remove();
    document.getElementById(containerId).appendChild(script);
  }

  document.getElementById("html-container").appendChild(
    document.createElement("script")
  );

  return JSON.stringify({ defaultCalls, runs: globalThis.__emptyScriptRuns });
})()
"#,
        )
        .expect("changed empty script sources should be prepared and executed");

    assert_eq!(
        result,
        r#"{"defaultCalls":[["","TrustedScript","HTMLScriptElement text"],["","TrustedScript","SVGScriptElement text"]],"runs":["HTMLScriptElement text","SVGScriptElement text"]}"#
    );
}

#[test]
fn trusted_types_default_policy_prepares_runtime_import_maps_before_registration() {
    let mut vm = new_storage_test_vm("https://runtime-import-map-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body ||
    (document.documentElement || document.appendChild(document.createElement("html")))
      .appendChild(document.createElement("body"));
  const calls = [];
  trustedTypes.createPolicy("default", {
    createScript(value, type, sink) {
      calls.push([value, type, sink]);
      const specifier = sink === "SVGScriptElement text" ? "svg-mapped" : "html-mapped";
      return JSON.stringify({ imports: { [specifier]: `/${specifier}.mjs` } });
    }
  });

  const htmlScript = document.createElement("script");
  htmlScript.type = "importmap";
  htmlScript.appendChild(document.createTextNode("html-map"));
  root.appendChild(htmlScript);

  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  root.appendChild(svg);
  const svgScript = document.createElementNS("http://www.w3.org/2000/svg", "script");
  svgScript.setAttribute("type", "importmap");
  svgScript.appendChild(document.createTextNode("svg-map"));
  svg.appendChild(svgScript);

  return JSON.stringify(calls);
})()
"#,
        )
        .expect("runtime import maps should pass through the Trusted Types source gate");

    assert_eq!(
        result,
        r#"[["html-map","TrustedScript","HTMLScriptElement text"],["svg-map","TrustedScript","SVGScriptElement text"]]"#
    );
    let base_url = vm.document_runtime.document_url().clone();
    for (specifier, expected) in [
        (
            "html-mapped",
            "https://runtime-import-map-trusted-types.test/html-mapped.mjs",
        ),
        (
            "svg-mapped",
            "https://runtime-import-map-trusted-types.test/svg-mapped.mjs",
        ),
    ] {
        assert_eq!(
            vm.document_runtime
                .resolve_module_specifier(specifier, &base_url)
                .expect("default-policy import map entry should resolve")
                .as_str(),
            expected
        );
    }
}

#[test]
fn trusted_types_default_policy_type_mutation_precedes_script_classification() {
    let mut vm = new_storage_test_vm("https://script-type-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement ||
    document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const svg = body.appendChild(
    document.createElementNS("http://www.w3.org/2000/svg", "svg")
  );
  const source = `globalThis.__trustedTypeScriptKinds.push("CLASSIC");`;
  globalThis.__trustedTypeScriptKinds = [];
  const defaultCalls = [];
  let script;
  trustedTypes.createPolicy("default", {
    createScript(value, type, sink) {
      defaultCalls.push([type, sink]);
      if (script.hasAttribute("type")) {
        script.removeAttribute("type");
      } else {
        script.setAttribute("type", "text/plain");
      }
      return value;
    }
  });

  for (const [namespace, parent] of [
    ["http://www.w3.org/1999/xhtml", body],
    ["http://www.w3.org/2000/svg", svg]
  ]) {
    script = document.createElementNS(namespace, "script");
    script.appendChild(document.createTextNode(source));
    script.setAttribute("type", "module");
    parent.appendChild(script);

    script = document.createElementNS(namespace, "script");
    script.appendChild(document.createTextNode(source));
    parent.appendChild(script);
  }

  return JSON.stringify({ defaultCalls, runs: globalThis.__trustedTypeScriptKinds });
})()
"#,
        )
        .expect("Trusted Types script type mutation probe should evaluate");

    assert_eq!(
        result,
        r#"{"defaultCalls":[["TrustedScript","HTMLScriptElement text"],["TrustedScript","HTMLScriptElement text"],["TrustedScript","SVGScriptElement text"],["TrustedScript","SVGScriptElement text"]],"runs":["CLASSIC","CLASSIC"]}"#
    );
}

#[test]
fn inline_module_graph_roots_use_trusted_types_compliant_source() {
    let mut vm = new_storage_test_vm("https://module-source-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let document = vm.document_runtime.document_handle();
    let html = vm.document_runtime.dom_host_mut().create_element("html");
    let body = vm.document_runtime.dom_host_mut().create_element("body");
    let html_script = vm.document_runtime.dom_host_mut().create_element("script");
    let svg_script = vm
        .document_runtime
        .dom_host_mut()
        .create_element_ns(Some("http://www.w3.org/2000/svg"), "script")
        .expect("SVG script element should be created");
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(document, html)
    );
    assert!(vm.document_runtime.dom_host_mut().append_child(html, body));
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(body, html_script)
    );
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(body, svg_script)
    );

    let document_url = vm.document_runtime.document_url().clone();
    let prepared = |node_id, position| PreparedScript {
        position,
        node_id,
        kind: ScriptKind::Module,
        mode: ScriptMode::ModuleInOrder,
        source_kind: ScriptSourceKind::Inline,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: ScriptSource::Inline("postMessage('original', '*');".to_owned()),
        url: document_url.clone(),
        base_url: document_url.clone(),
        initiator_url: document_url.clone(),
        host_script_handle: None,
    };
    let html_module = prepared(html_script, 1);
    let svg_module = prepared(svg_script, 2);

    assert_eq!(
        vm.inline_module_script_source_for_graph_start(
            &html_module,
            "postMessage('blocked', '*');"
        ),
        crate::module_runtime::ModuleSource::text(String::new()),
        "a module blocked by Trusted Types should enter the graph as an inert root"
    );

    vm.eval(
        r#"
globalThis.__inlineModuleDefaultCalls = [];
trustedTypes.createPolicy("default", {
  createScript(value, type, sink) {
    globalThis.__inlineModuleDefaultCalls.push([value, type, sink]);
    return value.replace("original", "transformed");
  }
});
"#,
    )
    .expect("inline-module default policy should install");

    let expected =
        crate::module_runtime::ModuleSource::text("postMessage('transformed', '*');".to_owned());
    assert_eq!(
        vm.inline_module_script_source_for_graph_start(
            &html_module,
            "postMessage('original', '*');"
        ),
        expected.clone()
    );
    assert_eq!(
        vm.inline_module_script_source_for_graph_start(
            &svg_module,
            "postMessage('original', '*');"
        ),
        expected
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__inlineModuleDefaultCalls)")
            .expect("inline-module default-policy calls should remain observable"),
        r#"[["postMessage('original', '*');","TrustedScript","HTMLScriptElement text"],["postMessage('original', '*');","TrustedScript","SVGScriptElement text"]]"#
    );
}

#[test]
fn script_src_enforces_trusted_script_url_and_applies_the_default_policy() {
    let mut vm = new_storage_test_vm("https://script-src-trusted-types.test/base/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      sample: event.sample
    });
  });
  globalThis.__scriptSrcViolations = violations;

  const rejected = document.createElement("script");
  let plainStringRejected = false;
  try {
    rejected.src = "plain.js";
  } catch (error) {
    plainStringRejected = error instanceof TypeError;
  }

  const explicitPolicy = trustedTypes.createPolicy("script-url", {
    createScriptURL: value => value
  });
  const explicit = document.createElement("script");
  explicit.src = explicitPolicy.createScriptURL("explicit.js");

  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createScriptURL: (value, type, sink) => {
      defaultCalls.push([value, type, sink]);
      return `default-${value}`;
    }
  });
  const defaulted = document.createElement("script");
  defaulted.src = "input.js";

  return JSON.stringify({
    plainStringRejected,
    rejectedAttribute: rejected.getAttribute("src"),
    explicitAttribute: explicit.getAttribute("src"),
    defaultedAttribute: defaulted.getAttribute("src"),
    defaultCalls,
    violations
  });
})()
"#,
        )
        .expect("HTMLScriptElement.src TrustedScriptURL sink probe should evaluate");

    assert_eq!(
        result,
        r#"{"plainStringRejected":true,"rejectedAttribute":null,"explicitAttribute":"explicit.js","defaultedAttribute":"default-input.js","defaultCalls":[["input.js","TrustedScriptURL","HTMLScriptElement src"]],"violations":[]}"#
    );

    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__scriptSrcViolations)")
            .expect("queued Trusted Types violation should be observable"),
        r#"[{"blockedURI":"trusted-types-sink","effectiveDirective":"require-trusted-types-for","sample":"HTMLScriptElement src|plain.js"}]"#
    );
}

#[test]
fn event_handler_attribute_writes_enforce_trusted_script_at_the_attribute_boundary() {
    let mut vm = new_storage_test_vm("https://event-handler-attribute-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__eventHandlerAttributeRuns = [];
  const root = document.body ||
    (document.documentElement || document.appendChild(document.createElement("html")))
      .appendChild(document.createElement("body"));
  const policy = trustedTypes.createPolicy("event-handler-script", {
    createScript: value => value,
    createHTML: value => value
  });

  const trusted = document.createElement("button");
  trusted.setAttribute(
    "onclick",
    policy.createScript("globalThis.__eventHandlerAttributeRuns.push('trusted')")
  );
  root.appendChild(trusted);
  trusted.click();

  const trustedNs = document.createElement("button");
  trustedNs.setAttributeNS(
    null,
    "onclick",
    policy.createScript("globalThis.__eventHandlerAttributeRuns.push('trusted-ns')")
  );
  root.appendChild(trustedNs);
  trustedNs.click();

  const unsuitable = document.createElement("button");
  let unsuitableRejected = false;
  try {
    unsuitable.setAttribute(
      "onclick",
      policy.createHTML("globalThis.__eventHandlerAttributeRuns.push('unsuitable')")
    );
  } catch (error) {
    unsuitableRejected = error instanceof TypeError;
  }

  const plain = document.createElement("button");
  let plainRejected = false;
  try {
    plain.setAttribute(
      "onclick",
      "globalThis.__eventHandlerAttributeRuns.push('plain')"
    );
  } catch (error) {
    plainRejected = error instanceof TypeError;
  }

  const plainNs = document.createElement("button");
  let plainNsRejected = false;
  try {
    plainNs.setAttributeNS(
      null,
      "onclick",
      "globalThis.__eventHandlerAttributeRuns.push('plain-ns')"
    );
  } catch (error) {
    plainNsRejected = error instanceof TypeError;
  }

  const ordinary = document.createElement("div");
  ordinary.setAttribute("data-onclick", "plain-data");
  ordinary.setAttribute("ondoesnotexist", "plain-unknown");
  ordinary.setAttributeNS("urn:test", "onclick", "plain-namespaced");
  const uppercaseSvg = document.createElementNS("http://www.w3.org/2000/svg", "g");
  uppercaseSvg.setAttributeNS(null, "ONCLICK", "plain-uppercase");
  const elementSpecificRejected = [
    "onwaitingforkey",
    "onbegin",
    "onfullscreenchange",
    "onfullscreenerror"
  ].map(name => {
    try {
      document.createElement("div").setAttribute(name, "plain-special");
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  });

  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createScript: (value, type, sink) => {
      defaultCalls.push([value, type, sink]);
      return "globalThis.__eventHandlerAttributeRuns.push('default')";
    }
  });
  const defaulted = document.createElement("button");
  defaulted.setAttribute("onclick", "default-input");
  root.appendChild(defaulted);
  defaulted.click();

  return JSON.stringify({
    runs: globalThis.__eventHandlerAttributeRuns,
    unsuitableRejected,
    unsuitableAttribute: unsuitable.getAttribute("onclick"),
    plainRejected,
    plainAttribute: plain.getAttribute("onclick"),
    plainNsRejected,
    plainNsAttribute: plainNs.getAttribute("onclick"),
    ordinary: [
      ordinary.getAttribute("data-onclick"),
      ordinary.getAttribute("ondoesnotexist"),
      ordinary.getAttributeNS("urn:test", "onclick"),
      uppercaseSvg.getAttribute("ONCLICK")
    ],
    elementSpecificRejected,
    defaultedAttribute: defaulted.getAttribute("onclick"),
    defaultCalls
  });
})()
"#,
        )
        .expect("event handler attribute Trusted Types sink probe should evaluate");

    assert_eq!(
        result,
        r#"{"runs":["trusted","trusted-ns","default"],"unsuitableRejected":true,"unsuitableAttribute":null,"plainRejected":true,"plainAttribute":null,"plainNsRejected":true,"plainNsAttribute":null,"ordinary":["plain-data","plain-unknown","plain-namespaced","plain-uppercase"],"elementSpecificRejected":[true,true,true,true],"defaultedAttribute":"globalThis.__eventHandlerAttributeRuns.push('default')","defaultCalls":[["default-input","TrustedScript","Element onclick"]]}"#
    );
}

#[test]
fn iframe_srcdoc_writes_enforce_trusted_html_for_property_and_attribute_sinks() {
    let mut vm = new_storage_test_vm("https://iframe-srcdoc-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const explicit = trustedTypes.createPolicy("iframe-srcdoc", {
    createHTML: value => value,
    createScript: value => value
  });
  const trustedProperty = document.createElement("iframe");
  trustedProperty.srcdoc = explicit.createHTML("trusted-property");
  const trustedAttribute = document.createElement("iframe");
  trustedAttribute.setAttribute("srcdoc", explicit.createHTML("trusted-attribute"));
  const trustedAttributeNs = document.createElement("iframe");
  trustedAttributeNs.setAttributeNS(
    null,
    "srcdoc",
    explicit.createHTML("trusted-attribute-ns")
  );

  const rejected = [
    [document.createElement("iframe"), (element, value) => { element.srcdoc = value; }],
    [document.createElement("iframe"), (element, value) => element.setAttribute("srcdoc", value)],
    [document.createElement("iframe"), (element, value) => element.setAttributeNS(null, "srcdoc", value)]
  ].flatMap(([element, setter]) => ["plain", explicit.createScript("wrong")].map(value => {
    try {
      setter(element, value);
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  }));

  const ordinaryNamespace = document.createElement("iframe");
  ordinaryNamespace.setAttributeNS("urn:test", "srcdoc", "namespaced");
  const ordinaryElement = document.createElement("div");
  ordinaryElement.setAttribute("srcdoc", "plain-div");

  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createHTML: (value, type, sink) => {
      defaultCalls.push([value, type, sink]);
      return `safe-${value}`;
    }
  });
  const defaultProperty = document.createElement("iframe");
  defaultProperty.srcdoc = "default-property";
  const defaultAttribute = document.createElement("iframe");
  defaultAttribute.setAttribute("srcdoc", "default-attribute");
  const defaultAttributeNs = document.createElement("iframe");
  defaultAttributeNs.setAttributeNS(null, "srcdoc", "default-attribute-ns");

  return JSON.stringify({
    trusted: [
      trustedProperty.srcdoc,
      trustedAttribute.getAttribute("srcdoc"),
      trustedAttributeNs.getAttribute("srcdoc")
    ],
    rejected,
    ordinary: [
      ordinaryNamespace.getAttributeNS("urn:test", "srcdoc"),
      ordinaryElement.getAttribute("srcdoc")
    ],
    defaulted: [
      defaultProperty.srcdoc,
      defaultAttribute.getAttribute("srcdoc"),
      defaultAttributeNs.getAttribute("srcdoc")
    ],
    defaultCalls
  });
})()
"#,
        )
        .expect("iframe srcdoc TrustedHTML sink probe should evaluate");

    assert_eq!(
        result,
        r#"{"trusted":["trusted-property","trusted-attribute","trusted-attribute-ns"],"rejected":[true,true,true,true,true,true],"ordinary":["namespaced","plain-div"],"defaulted":["safe-default-property","safe-default-attribute","safe-default-attribute-ns"],"defaultCalls":[["default-property","TrustedHTML","HTMLIFrameElement srcdoc"],["default-attribute","TrustedHTML","HTMLIFrameElement srcdoc"],["default-attribute-ns","TrustedHTML","HTMLIFrameElement srcdoc"]]}"#
    );
}

#[test]
fn url_attribute_writes_enforce_only_the_current_non_iframe_script_url_sinks() {
    let mut vm = new_storage_test_vm("https://url-attribute-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const xlink = "http://www.w3.org/1999/xlink";
  const svg = "http://www.w3.org/2000/svg";
  const policy = trustedTypes.createPolicy("attribute-url", {
    createScript: value => value,
    createScriptURL: value => value
  });
  const trusted = value => policy.createScriptURL(value);

  const htmlScript = document.createElement("script");
  htmlScript.setAttribute("src", trusted("script.js"));
  const htmlScriptNs = document.createElement("script");
  htmlScriptNs.setAttributeNS(null, "src", trusted("script-ns.js"));
  const svgScript = document.createElementNS(svg, "script");
  svgScript.setAttribute("href", trusted("svg.js"));
  const svgScriptXlink = document.createElementNS(svg, "script");
  svgScriptXlink.setAttributeNS(xlink, "xlink:href", trusted("svg-xlink.js"));
  const embed = document.createElement("embed");
  embed.setAttribute("src", trusted("embed.js"));
  const object = document.createElement("object");
  object.setAttribute("data", trusted("object-data.js"));
  object.setAttribute("codebase", trusted("object-codebase.js"));

  const rejected = [];
  for (const [element, setter] of [
    [document.createElement("script"), element => element.setAttribute("src", "plain")],
    [document.createElement("script"), element => element.setAttributeNS(null, "src", null)],
    [document.createElementNS(svg, "script"), element => element.setAttribute("href", "plain")],
    [document.createElementNS(svg, "script"), element => element.setAttributeNS(xlink, "xlink:href", policy.createScript("wrong"))]
  ]) {
    try {
      setter(element);
      rejected.push(false);
    } catch (error) {
      rejected.push(error instanceof TypeError);
    }
  }

  const ordinary = document.createElement("script");
  ordinary.setAttributeNS("urn:test", "src", "namespaced");
  const uppercaseSvg = document.createElementNS(svg, "script");
  uppercaseSvg.setAttributeNS(null, "HREF", "uppercase");
  const div = document.createElement("div");
  div.setAttribute("src", "plain-div");
  const legacyEmbed = document.createElement("embed");
  legacyEmbed.setAttribute("src", "plain-embed");
  const legacyObject = document.createElement("object");
  legacyObject.setAttribute("data", "plain-object-data");
  legacyObject.setAttribute("codebase", "plain-object-codebase");

  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createScriptURL: (value, type, sink) => {
      defaultCalls.push([value, type, sink]);
      return `safe-${value}`;
    }
  });
  const defaultHtml = document.createElement("script");
  defaultHtml.setAttribute("src", "default-html");
  const defaultSvg = document.createElementNS(svg, "script");
  defaultSvg.setAttributeNS(xlink, "xlink:href", "default-svg");
  const defaultObject = document.createElement("object");
  defaultObject.setAttribute("codebase", "default-object");

  return JSON.stringify({
    trustedValues: [
      htmlScript.getAttribute("src"),
      htmlScriptNs.getAttribute("src"),
      svgScript.getAttribute("href"),
      svgScriptXlink.getAttributeNS(xlink, "href"),
      embed.getAttribute("src"),
      object.getAttribute("data"),
      object.getAttribute("codebase")
    ],
    rejected,
    ordinary: [
      ordinary.getAttributeNS("urn:test", "src"),
      uppercaseSvg.getAttribute("HREF"),
      div.getAttribute("src"),
      legacyEmbed.getAttribute("src"),
      legacyObject.getAttribute("data"),
      legacyObject.getAttribute("codebase")
    ],
    defaultValues: [
      defaultHtml.getAttribute("src"),
      defaultSvg.getAttributeNS(xlink, "href"),
      defaultObject.getAttribute("codebase")
    ],
    defaultCalls
  });
})()
"#,
        )
        .expect("URL attribute Trusted Types sink table probe should evaluate");

    assert_eq!(
        result,
        r#"{"trustedValues":["script.js","script-ns.js","svg.js","svg-xlink.js","embed.js","object-data.js","object-codebase.js"],"rejected":[true,true,true,true],"ordinary":["namespaced","uppercase","plain-div","plain-embed","plain-object-data","plain-object-codebase"],"defaultValues":["safe-default-html","safe-default-svg","default-object"],"defaultCalls":[["default-html","TrustedScriptURL","HTMLScriptElement src"],["default-svg","TrustedScriptURL","SVGScriptElement href"]]}"#
    );
}

#[test]
fn svg_script_href_base_val_uses_the_owner_reflected_trusted_script_url_sink() {
    let mut vm = new_storage_test_vm("https://svg-script-href-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      sample: event.sample
    });
  });
  globalThis.__svgScriptHrefViolations = violations;

  const svgScript = document.createElementNS("http://www.w3.org/2000/svg", "script");
  const href = svgScript.href;
  let illegalConstructor = false;
  try {
    new SVGAnimatedString();
  } catch (error) {
    illegalConstructor = error instanceof TypeError;
  }
  let plainRejected = false;
  try {
    href.baseVal = "plain.js";
  } catch (error) {
    plainRejected = error instanceof TypeError;
  }
  const rejectedAttribute = svgScript.getAttribute("href");

  const explicit = trustedTypes.createPolicy("svg-script-href", {
    createScriptURL: value => value
  });
  href.baseVal = explicit.createScriptURL("trusted.js");
  const trustedValues = [
    svgScript.getAttribute("href"),
    href.baseVal,
    href.animVal
  ];
  svgScript.setAttribute("href", explicit.createScriptURL("attribute.js"));
  const externallySynced = [href.baseVal, href.animVal];
  const className = svgScript.className;
  className.baseVal = explicit.createScriptURL("trusted-class");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  const useHref = use.href;
  useHref.baseVal = explicit.createScriptURL("trusted-use");

  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createScriptURL: (value, type, sink) => {
      defaultCalls.push([value, type, sink]);
      return `safe-${value}`;
    }
  });
  href.baseVal = "default-input";
  className.baseVal = "plain-class";
  useHref.baseVal = "plain-use";

  return JSON.stringify({
    shape: [
      href instanceof SVGAnimatedString,
      Object.prototype.toString.call(href),
      svgScript.href === href,
      illegalConstructor
    ],
    plainRejected,
    rejectedAttribute,
    trustedValues,
    externallySynced,
    ordinaryAnimatedStrings: [
      className instanceof SVGAnimatedString,
      svgScript.className === className,
      className.baseVal,
      className.animVal,
      svgScript.getAttribute("class"),
      useHref instanceof SVGAnimatedString,
      use.href === useHref,
      useHref.baseVal,
      useHref.animVal,
      use.getAttribute("href")
    ],
    defaulted: [svgScript.getAttribute("href"), href.baseVal, href.animVal],
    defaultCalls,
    violations
  });
})()
"#,
        )
        .expect("SVGScriptElement href Trusted Types sink probe should evaluate");

    assert_eq!(
        result,
        r#"{"shape":[true,"[object SVGAnimatedString]",true,true],"plainRejected":true,"rejectedAttribute":null,"trustedValues":["trusted.js","trusted.js","trusted.js"],"externallySynced":["attribute.js","attribute.js"],"ordinaryAnimatedStrings":[true,true,"plain-class","plain-class","plain-class",true,true,"plain-use","plain-use","plain-use"],"defaulted":["safe-default-input","safe-default-input","safe-default-input"],"defaultCalls":[["default-input","TrustedScriptURL","SVGScriptElement href"]],"violations":[]}"#
    );

    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__svgScriptHrefViolations)")
            .expect("queued SVGScriptElement href violation should be observable"),
        r#"[{"blockedURI":"trusted-types-sink","effectiveDirective":"require-trusted-types-for","sample":"SVGScriptElement href|plain.js"}]"#
    );
}

#[test]
fn attached_attribute_mutations_recheck_trusted_types_after_domstring_conversion() {
    let mut vm = new_storage_test_vm("https://attached-attribute-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const explicit = trustedTypes.createPolicy("attached-attribute", {
    createScript: value => value
  });

  const alreadyAttached = document.createElement("button");
  alreadyAttached.setAttribute("onclick", explicit.createScript("same-input"));
  const alreadyAttachedAttr = alreadyAttached.getAttributeNode("onclick");
  let sameAttributeRejected = false;
  try {
    alreadyAttached.setAttributeNode(alreadyAttachedAttr);
  } catch (error) {
    sameAttributeRejected = error instanceof TypeError;
  }

  const calls = [];
  let detachAttr;
  let moveAttr;
  let moveTarget;
  let moving = false;
  trustedTypes.createPolicy("default", {
    createScript: (value, type, sink) => {
      calls.push([value, type, sink]);
      if (value === "detach-input") {
        detachAttr.ownerElement.removeAttributeNode(detachAttr);
      }
      if (value === "move-input" && !moving) {
        moving = true;
        const owner = moveAttr.ownerElement;
        if (owner) owner.removeAttributeNode(moveAttr);
        moveTarget.setAttributeNode(moveAttr);
        moving = false;
      }
      return `safe-${value}`;
    }
  });

  const nodeSetters = [
    ["setAttributeNode", (element, attr) => element.setAttributeNode(attr)],
    ["setAttributeNodeNS", (element, attr) => element.setAttributeNodeNS(attr)],
    ["setNamedItem", (element, attr) => element.attributes.setNamedItem(attr)],
    ["setNamedItemNS", (element, attr) => element.attributes.setNamedItemNS(attr)]
  ];
  const nodeValues = nodeSetters.map(([name, setter], index) => {
    const element = document.createElement("button");
    const attr = document.createAttribute("onclick");
    attr.value = index === 0
      ? explicit.createScript(`${name}-input`)
      : `${name}-input`;
    const previous = setter(element, attr);
    return [previous === null, attr.ownerElement === element, attr.value];
  });

  const valueSetters = [
    ["value", (attr, value) => { attr.value = value; }],
    ["nodeValue", (attr, value) => { attr.nodeValue = value; }],
    ["textContent", (attr, value) => { attr.textContent = value; }]
  ];
  const valueResults = valueSetters.map(([name, setter], index) => {
    const element = document.createElement("button");
    element.setAttribute("onclick", explicit.createScript("initial"));
    const attr = element.getAttributeNode("onclick");
    setter(attr, index === 0 ? explicit.createScript(`${name}-input`) : `${name}-input`);
    return [attr.ownerElement === element, attr.value];
  });

  const detachedOwner = document.createElement("button");
  detachedOwner.setAttribute("onclick", explicit.createScript("initial"));
  detachAttr = detachedOwner.getAttributeNode("onclick");
  detachAttr.value = "detach-input";

  const originalMoveOwner = document.createElement("button");
  moveTarget = document.createElement("button");
  moveAttr = document.createAttribute("onclick");
  moveAttr.value = "move-input";
  let moveRejected = false;
  try {
    originalMoveOwner.setAttributeNode(moveAttr);
  } catch (error) {
    moveRejected = error.name === "InUseAttributeError";
  }

  return JSON.stringify({
    sameAttributeRejected,
    nodeValues,
    valueResults,
    detached: [detachAttr.ownerElement === null, detachAttr.value],
    moved: [moveRejected, moveAttr.ownerElement === moveTarget, moveAttr.value],
    calls
  });
})()
"#,
        )
        .expect("attached attribute Trusted Types mutation probe should evaluate");

    assert_eq!(
        result,
        r#"{"sameAttributeRejected":true,"nodeValues":[[true,true,"safe-setAttributeNode-input"],[true,true,"safe-setAttributeNodeNS-input"],[true,true,"safe-setNamedItem-input"],[true,true,"safe-setNamedItemNS-input"]],"valueResults":[[true,"safe-value-input"],[true,"safe-nodeValue-input"],[true,"safe-textContent-input"]],"detached":[true,"safe-detach-input"],"moved":[true,true,"safe-move-input"],"calls":[["setAttributeNode-input","TrustedScript","Element onclick"],["setAttributeNodeNS-input","TrustedScript","Element onclick"],["setNamedItem-input","TrustedScript","Element onclick"],["setNamedItemNS-input","TrustedScript","Element onclick"],["value-input","TrustedScript","Element onclick"],["nodeValue-input","TrustedScript","Element onclick"],["textContent-input","TrustedScript","Element onclick"],["detach-input","TrustedScript","Element onclick"],["move-input","TrustedScript","Element onclick"],["move-input","TrustedScript","Element onclick"]]}"#
    );
}

#[test]
fn empty_default_policy_reports_each_rejected_element_sink() {
    let mut vm = new_storage_test_vm("https://empty-default-policy.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const samples = [];
  document.addEventListener("securitypolicyviolation", event => {
    event.stopPropagation();
    samples.push(event.sample);
  });
  globalThis.__emptyDefaultPolicySamples = samples;
  trustedTypes.createPolicy("default", {});

  for (const [name, property, value] of [
    ["script", "src", "abc"],
    ["div", "innerHTML", "abc"],
    ["script", "text", "done"]
  ]) {
    try {
      document.createElement(name)[property] = value;
    } catch (error) {}
  }
  return JSON.stringify(samples);
})()
"#,
        )
        .expect("empty Trusted Types default policy probe should evaluate");

    assert_eq!(result, "[]");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        3
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__emptyDefaultPolicySamples)")
            .expect("queued empty default policy violations should be observable"),
        r#"["HTMLScriptElement src|abc","Element innerHTML|abc","HTMLScriptElement text|done"]"#
    );
}

#[test]
fn empty_default_policy_report_only_allows_and_reports_each_element_sink() {
    let mut vm = new_storage_test_vm("https://empty-default-policy-report-only.test/");
    vm.set_response_content_security_report_only_policies(&[
        "require-trusted-types-for 'script'".to_owned()
    ]);

    let result = vm
        .eval(
            r#"
(() => {
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push({ sample: event.sample, disposition: event.disposition });
  });
  globalThis.__emptyReportOnlyDefaultPolicyViolations = violations;
  trustedTypes.createPolicy("default", {});

  const scriptUrl = document.createElement("script");
  scriptUrl.src = "abc";
  const markup = document.createElement("div");
  markup.innerHTML = "abc";
  const scriptText = document.createElement("script");
  scriptText.text = "done";

  return JSON.stringify({
    scriptUrl: scriptUrl.getAttribute("src"),
    markup: markup.innerHTML,
    scriptText: scriptText.text,
    violations
  });
})()
"#,
        )
        .expect("report-only empty Trusted Types default policy probe should evaluate");

    assert_eq!(
        result,
        r#"{"scriptUrl":"abc","markup":"abc","scriptText":"done","violations":[]}"#
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        3
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__emptyReportOnlyDefaultPolicyViolations)")
            .expect("queued report-only Trusted Types violations should be observable"),
        r#"[{"sample":"HTMLScriptElement src|abc","disposition":"report"},{"sample":"Element innerHTML|abc","disposition":"report"},{"sample":"HTMLScriptElement text|done","disposition":"report"}]"#
    );
}

#[test]
fn report_only_trusted_types_eval_runs_reports_and_applies_the_default_policy() {
    let mut vm = new_storage_test_vm("https://trusted-types-eval-report-only.test/");
    vm.set_response_content_security_policies(&["script-src 'unsafe-eval'".to_owned()]);
    vm.set_response_content_security_report_only_policies(&[
        "require-trusted-types-for 'script'".to_owned()
    ]);

    let result = vm
        .eval(
            r#"
(() => {
  const violations = [];
  const defaultPolicyCalls = [];
  document.addEventListener("securitypolicyviolation", event => {
    if (event.blockedURI === "trusted-types-sink") {
      violations.push({ sample: event.sample, disposition: event.disposition });
    }
  });
  globalThis.__reportOnlyEvalViolations = violations;

  eval("globalThis.__reportOnlyEval = 1");
  const explicit = trustedTypes.createPolicy("explicit-report-only-eval", {
    createScript: value => value
  });
  eval(explicit.createScript("globalThis.__reportOnlyEval = 2"));
  trustedTypes.createPolicy("default", {
    createScript: (...args) => {
      defaultPolicyCalls.push(args);
      return args[0];
    }
  });
  eval("globalThis.__reportOnlyEval = 3");

  return JSON.stringify({ value: globalThis.__reportOnlyEval, defaultPolicyCalls, violations });
})()
"#,
        )
        .expect("report-only Trusted Types eval probe should evaluate");

    assert_eq!(
        result,
        r#"{"value":3,"defaultPolicyCalls":[["globalThis.__reportOnlyEval = 3","TrustedScript","eval"]],"violations":[]}"#
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__reportOnlyEvalViolations)")
            .expect("queued report-only eval violation should be observable"),
        r#"[{"sample":"eval|globalThis.__reportOnlyEval = 1","disposition":"report"}]"#
    );
}

#[test]
fn javascript_url_navigation_default_policy_is_silent_and_disposition_aware() {
    let policy = "require-trusted-types-for 'script'".to_owned();

    let mut rewritten = new_storage_test_vm("https://javascript-url-rewritten.test/");
    rewritten.set_response_content_security_policies(std::slice::from_ref(&policy));
    rewritten
        .eval(
            r#"
globalThis.__javascriptUrlOriginal = 0;
globalThis.__javascriptUrlModified = 0;
globalThis.__javascriptUrlDefaultCalls = [];
trustedTypes.createPolicy("default", {
  createScript: (...args) => {
    globalThis.__javascriptUrlDefaultCalls.push(args);
    return args[0].replace("Original", "Modified");
  }
});
"ready"
"#,
        )
        .expect("install javascript URL rewriting default policy");
    assert_eq!(
        rewritten
            .eval_javascript_url_runtime_turn(
                "globalThis.__javascriptUrlOriginal++",
                &Url::parse("https://javascript-url-rewritten.test/").expect("test URL"),
            )
            .expect("rewritten javascript URL should execute"),
        None
    );
    assert_eq!(
        rewritten
            .eval(
                "JSON.stringify({ original: __javascriptUrlOriginal, modified: __javascriptUrlModified, calls: __javascriptUrlDefaultCalls })"
            )
            .expect("read rewritten javascript URL result"),
        r#"{"original":0,"modified":1,"calls":[["globalThis.__javascriptUrlOriginal++","TrustedScript","Location href"]]}"#
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut rewritten),
        0
    );

    for (name, callback, report_only, expected_runs, expected_disposition) in [
        (
            "throw-enforce",
            "throw new RangeError('default policy failure')",
            false,
            0,
            "enforce",
        ),
        (
            "invalid-enforce",
            "return '//make:invalid/'",
            false,
            0,
            "enforce",
        ),
        (
            "throw-report",
            "throw new RangeError('default policy failure')",
            true,
            1,
            "report",
        ),
    ] {
        let base_url =
            Url::parse(&format!("https://javascript-url-{name}.test/")).expect("test URL");
        let mut vm = new_storage_test_vm(base_url.as_str());
        if report_only {
            vm.set_response_content_security_report_only_policies(std::slice::from_ref(&policy));
        } else {
            vm.set_response_content_security_policies(std::slice::from_ref(&policy));
        }
        vm.eval(&format!(
            r#"
globalThis.__javascriptUrlRuns = 0;
globalThis.__javascriptUrlViolations = [];
document.addEventListener("securitypolicyviolation", event => {{
  globalThis.__javascriptUrlViolations.push({{
    sample: event.sample,
    disposition: event.disposition,
  }});
}});
trustedTypes.createPolicy("default", {{
  createScript: () => {{ {callback} }}
}});
"ready"
"#
        ))
        .expect("install failing javascript URL default policy");

        assert_eq!(
            vm.eval_javascript_url_runtime_turn("globalThis.__javascriptUrlRuns++", &base_url)
                .expect("default policy failure must not escape the navigation turn"),
            None
        );
        assert_eq!(
            drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
            1
        );
        assert_eq!(
            vm.eval(
                "JSON.stringify({ runs: __javascriptUrlRuns, violations: __javascriptUrlViolations })"
            )
            .expect("read javascript URL failure result"),
            format!(
                r#"{{"runs":{expected_runs},"violations":[{{"sample":"Location href|globalThis.__javascriptUrlRuns++","disposition":"{expected_disposition}"}}]}}"#
            ),
            "case {name}"
        );
    }
}

#[test]
fn report_only_default_policy_transforms_or_preserves_by_callback_outcome() {
    let mut vm = new_storage_test_vm("https://default-policy-report-only.test/");
    vm.set_response_content_security_report_only_policies(&[
        "require-trusted-types-for 'script'".to_owned()
    ]);

    let result = vm
        .eval(
            r#"
(() => {
  const violations = [];
  const calls = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push(event.sample);
  });
  globalThis.__reportOnlyDefaultPolicyViolations = violations;

  const policy = (value, type, sink) => {
    calls.push([value, type, sink]);
    if (value === "throw") throw new RangeError("default policy throw");
    if (value === "null") return null;
    if (value === "undefined") return undefined;
    if (value === "typeerror") return document.missingCallback();
    return `sanitized: ${value}`;
  };
  trustedTypes.createPolicy("default", {
    createScriptURL: policy,
    createHTML: policy,
    createScript: policy
  });

  const cases = [
    ["script", "src"],
    ["div", "innerHTML"],
    ["script", "text"]
  ];
  const values = {};
  const errors = [];
  for (const [name, property] of cases) {
    for (const input of ["abc", "null", "undefined", "throw", "typeerror"]) {
      const element = document.createElement(name);
      try {
        element[property] = input;
        values[`${name}.${property}.${input}`] = property === "src"
          ? element.getAttribute(property)
          : element[property];
      } catch (error) {
        errors.push(`${name}.${property}.${input}:${error.name}`);
      }
    }
  }
  return JSON.stringify({ values, errors, calls, violations });
})()
"#,
        )
        .expect("report-only Trusted Types default policy outcome probe should evaluate");

    assert_eq!(
        result,
        r#"{"values":{"script.src.abc":"sanitized: abc","script.src.null":"null","script.src.undefined":"undefined","div.innerHTML.abc":"sanitized: abc","div.innerHTML.null":"null","div.innerHTML.undefined":"undefined","script.text.abc":"sanitized: abc","script.text.null":"null","script.text.undefined":"undefined"},"errors":["script.src.throw:RangeError","script.src.typeerror:TypeError","div.innerHTML.throw:RangeError","div.innerHTML.typeerror:TypeError","script.text.throw:RangeError","script.text.typeerror:TypeError"],"calls":[["abc","TrustedScriptURL","HTMLScriptElement src"],["null","TrustedScriptURL","HTMLScriptElement src"],["undefined","TrustedScriptURL","HTMLScriptElement src"],["throw","TrustedScriptURL","HTMLScriptElement src"],["typeerror","TrustedScriptURL","HTMLScriptElement src"],["abc","TrustedHTML","Element innerHTML"],["null","TrustedHTML","Element innerHTML"],["undefined","TrustedHTML","Element innerHTML"],["throw","TrustedHTML","Element innerHTML"],["typeerror","TrustedHTML","Element innerHTML"],["abc","TrustedScript","HTMLScriptElement text"],["null","TrustedScript","HTMLScriptElement text"],["undefined","TrustedScript","HTMLScriptElement text"],["throw","TrustedScript","HTMLScriptElement text"],["typeerror","TrustedScript","HTMLScriptElement text"]],"violations":[]}"#
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        6
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__reportOnlyDefaultPolicyViolations)")
            .expect("queued rejected default policy violations should be observable"),
        r#"["HTMLScriptElement src|null","HTMLScriptElement src|undefined","Element innerHTML|null","Element innerHTML|undefined","HTMLScriptElement text|null","HTMLScriptElement text|undefined"]"#
    );
}

#[test]
fn rejected_default_policy_reports_both_dispositions_and_enforces_once() {
    let mut vm = new_storage_test_vm("https://default-policy-both-dispositions.test/");
    let policy = "require-trusted-types-for 'script'".to_owned();
    vm.set_response_content_security_policies(std::slice::from_ref(&policy));
    vm.set_response_content_security_report_only_policies(&[policy]);

    let result = vm
        .eval(
            r#"
(() => {
  const dispositions = [];
  document.addEventListener("securitypolicyviolation", event => {
    dispositions.push(event.disposition);
  });
  globalThis.__bothDispositionViolations = dispositions;
  let calls = 0;
  trustedTypes.createPolicy("default", {
    createHTML: () => {
      calls++;
      return null;
    }
  });
  let threw = false;
  try {
    document.createElement("div").innerHTML = "plain";
  } catch (error) {
    threw = error instanceof TypeError;
  }
  return JSON.stringify({ calls, threw, dispositions });
})()
"#,
        )
        .expect("combined enforce and report-only Trusted Types probe should evaluate");

    assert_eq!(result, r#"{"calls":1,"threw":true,"dispositions":[]}"#);
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        2
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__bothDispositionViolations.slice().sort())")
            .expect("both Trusted Types dispositions should be observable"),
        r#"["enforce","report"]"#
    );
}

#[test]
fn eval_csp_report_samples_use_trusted_types_compliant_source() {
    let mut vm = new_storage_test_vm("https://eval-csp-report-sample.test/");
    vm.set_response_content_security_policies(&[
        "require-trusted-types-for 'script'".to_owned(),
        "script-src 'nonce-test' 'report-sample'".to_owned(),
    ]);

    let result = vm
        .eval(
            r#"
(() => {
  const samples = [];
  const defaultCalls = [];
  document.addEventListener("securitypolicyviolation", event => {
    if (event.effectiveDirective === "script-src") {
      samples.push(event.sample);
    }
  });
  globalThis.__evalCspReportSamples = samples;

  const explicit = trustedTypes.createPolicy("explicit-eval", {
    createScript: value => value
  });
  trustedTypes.createPolicy("default", {
    createScript: value => {
      defaultCalls.push(value);
      return value;
    }
  });
  const errors = [
    () => eval(explicit.createScript("trusted-source")),
    () => eval("default-source")
  ].map(run => {
    try {
      run();
      return "none";
    } catch (error) {
      return `${error.name}:${error instanceof EvalError}`;
    }
  });
  return JSON.stringify({ errors, defaultCalls, samples });
})()
"#,
        )
        .expect("eval CSP report-sample probe should evaluate");

    assert_eq!(
        result,
        r#"{"errors":["EvalError:true","EvalError:true"],"defaultCalls":["default-source"],"samples":[]}"#
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        2
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__evalCspReportSamples)")
            .expect("queued eval CSP samples should be observable"),
        r#"["trusted-source","default-source"]"#
    );
}

#[test]
fn function_constructor_violations_sample_only_parameters_and_body() {
    let mut vm = new_storage_test_vm("https://function-constructor-violation.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const samples = [];
  document.addEventListener("securitypolicyviolation", event => {
    if (event.blockedURI === "trusted-types-sink") {
      samples.push(event.sample);
    }
  });
  globalThis.__functionConstructorViolationSamples = samples;

  const constructors = [
    Function,
    async function() {}.constructor,
    function*() {}.constructor,
    async function*() {}.constructor
  ];
  const errors = constructors.map(Constructor => {
    try {
      new Constructor(`return${";".repeat(100)}`);
      return "none";
    } catch (error) {
      return `${error.name}:${error instanceof EvalError}`;
    }
  });
  return JSON.stringify({ errors, samples });
})()
"#,
        )
        .expect("Function constructor violation probe should evaluate");

    assert_eq!(
        result,
        r#"{"errors":["EvalError:true","EvalError:true","EvalError:true","EvalError:true"],"samples":[]}"#
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        4
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__functionConstructorViolationSamples)")
            .expect("queued Function constructor violations should be observable"),
        r#"["Function|(\n) {\nreturn;;;;;;;;;;;;;;;;;;;;;;;;;;;;","Function|(\n) {\nreturn;;;;;;;;;;;;;;;;;;;;;;;;;;;;","Function|(\n) {\nreturn;;;;;;;;;;;;;;;;;;;;;;;;;;;;","Function|(\n) {\nreturn;;;;;;;;;;;;;;;;;;;;;;;;;;;;"]"#
    );
}

#[test]
fn script_execution_violation_outside_javascript_stack_avoids_v8_frame_probe() {
    let mut vm = new_storage_test_vm("https://script-execution-violation.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    vm.eval(
        r#"
(() => {
  globalThis.__scriptExecutionViolation = null;
  addEventListener("securitypolicyviolation", event => {
    if (event.blockedURI === "trusted-types-sink") {
      globalThis.__scriptExecutionViolation = {
        blockedURI: event.blockedURI,
        sample: event.sample
      };
    }
  });
  const script = document.createElement("script");
  script.id = "untrusted-script-source";
  script.type = "application/json";
  const root = document.body ||
    (document.documentElement || document.appendChild(document.createElement("html")))
      .appendChild(document.createElement("body"));
  root.appendChild(script);
  return "ready";
})()
"#,
    )
    .expect("script execution violation setup should evaluate");

    let script = vm
        .document_runtime
        .get_element_by_id("untrusted-script-source")
        .expect("inert script should exist");
    assert_eq!(
        vm.inline_script_element_source_for_execution(
            script,
            "untrusted-source",
            crate::content_security_policy::ContentSecurityPolicyScriptElementRequest {
                nonce: None,
                integrity: None,
                parser_inserted: false,
            },
        ),
        None
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__scriptExecutionViolation)")
            .expect("script execution violation should be observable"),
        r#"{"blockedURI":"trusted-types-sink","sample":"HTMLScriptElement text|untrusted-source"}"#
    );
}

#[test]
fn service_worker_register_gates_script_url_before_url_resolution() {
    let mut vm = new_storage_test_vm("https://service-worker-register-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error && error.name;
    }
  };
  const policy = trustedTypes.createPolicy("service-worker-register", {
    createHTML: value => value,
    createScriptURL: value => value
  });
  const blockedString = errorName(() => navigator.serviceWorker.register("worker.js"));
  const blockedWrongType = errorName(() =>
    navigator.serviceWorker.register(policy.createHTML("worker.js"))
  );
  const missing = errorName(() => navigator.serviceWorker.register());

  const trustedPromise = navigator.serviceWorker.register(
    policy.createScriptURL("http://[")
  );
  trustedPromise.catch(() => {});

  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createScriptURL: (value, type, sink) => {
      defaultCalls.push([value, type, sink]);
      return "http://[";
    }
  });
  const defaultPromise = navigator.serviceWorker.register("worker.potato");
  defaultPromise.catch(() => {});

  return JSON.stringify({
    blockedString,
    blockedWrongType,
    missing,
    trustedPromise: trustedPromise instanceof Promise,
    defaultPromise: defaultPromise instanceof Promise,
    defaultCalls
  });
})()
"#,
        )
        .expect("ServiceWorkerContainer.register TrustedScriptURL probe should evaluate");

    assert_eq!(
        result,
        r#"{"blockedString":"TypeError","blockedWrongType":"TypeError","missing":"TypeError","trustedPromise":true,"defaultPromise":true,"defaultCalls":[["worker.potato","TrustedScriptURL","ServiceWorkerContainer register"]]}"#
    );
}
