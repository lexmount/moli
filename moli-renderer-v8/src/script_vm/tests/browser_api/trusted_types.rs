use super::*;

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
  script.appendChild(document.createTextNode("untrusted-source"));
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
