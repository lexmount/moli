use super::*;

#[test]
fn module_fetch_csp_uses_captured_parser_metadata_and_nonce() {
    let mut vm = new_storage_test_vm("https://module-csp-provenance.test/page.html");
    vm.set_response_content_security_policies(&[
        "script-src 'nonce-allowed' 'strict-dynamic'".to_owned()
    ]);
    let key = crate::module_runtime::ModuleMapKey::java_script(
        Url::parse("blob:https://module-csp-provenance.test/child").unwrap(),
    );

    let dynamic_metadata = crate::module_runtime::ModuleFetchMetadata::default();
    assert!(
        vm.csp_blocked_module_fetch_error_for_owner(&key, &dynamic_metadata)
            .is_none(),
        "strict-dynamic must allow a non-parser-inserted module request"
    );

    let parser_metadata =
        crate::module_runtime::ModuleFetchMetadata::from_top_level_script_fetch_metadata(
            &crate::planning::ScriptFetchMetadata {
                parser_inserted: true,
                ..crate::planning::ScriptFetchMetadata::default()
            },
        );
    assert!(
        vm.csp_blocked_module_fetch_error_for_owner(&key, &parser_metadata)
            .is_some(),
        "the same URL must remain blocked when parser metadata says parser-inserted"
    );

    let trusted_parser_metadata =
        crate::module_runtime::ModuleFetchMetadata::from_top_level_script_fetch_metadata(
            &crate::planning::ScriptFetchMetadata {
                nonce: Some("allowed".to_owned()),
                parser_inserted: true,
                ..crate::planning::ScriptFetchMetadata::default()
            },
        );
    assert!(
        vm.csp_blocked_module_fetch_error_for_owner(&key, &trusted_parser_metadata)
            .is_none(),
        "a matching inherited nonce must authorize a parser-inserted module descendant"
    );
}

#[test]
fn dynamic_inline_script_csp_blocks_untrusted_source_and_accepts_matching_nonce() {
    let mut vm = new_storage_test_vm("https://inline-script-csp.test/page.html");
    vm.set_response_content_security_policies(&["script-src 'nonce-allowed'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const violations = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  document.addEventListener("securitypolicyviolation", event => {
    violations.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      disposition: event.disposition,
    });
  });
  globalThis.__dynamicInlineCspViolations = violations;

  const blocked = document.createElement("script");
  blocked.text = "globalThis.__blockedInlineScriptRan = true";
  root.appendChild(blocked);

  const allowed = document.createElement("script");
  allowed.setAttribute("nonce", "allowed");
  allowed.text = "globalThis.__allowedInlineScriptRan = true";
  root.appendChild(allowed);

  globalThis.__dynamicInlineCspResult = {
    blockedRan: globalThis.__blockedInlineScriptRan === true,
    allowedRan: globalThis.__allowedInlineScriptRan === true,
    violations,
  };
  return "queued";
})()
"#,
        )
        .expect("dynamic inline-script CSP probe should evaluate");

    assert_eq!(result, "queued");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__dynamicInlineCspResult)")
            .expect("queued dynamic inline CSP violation should be observable"),
        r#"{"blockedRan":false,"allowedRan":true,"violations":[{"blockedURI":"inline","effectiveDirective":"script-src-elem","disposition":"enforce"}]}"#
    );
}

#[test]
fn report_only_inline_script_csp_reports_without_blocking_execution() {
    let mut vm = new_storage_test_vm("https://inline-script-report-only.test/page.html");
    vm.set_response_content_security_report_only_policies(&["script-src 'none'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const violations = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  document.addEventListener("securitypolicyviolation", event => {
    violations.push(`${event.disposition}:${event.effectiveDirective}:${event.blockedURI}`);
  });
  globalThis.__reportOnlyInlineCspViolations = violations;
  const script = document.createElement("script");
  script.text = "globalThis.__reportOnlyInlineScriptRan = true";
  root.appendChild(script);
  return globalThis.__reportOnlyInlineScriptRan === true;
})()
"#,
        )
        .expect("report-only inline-script CSP probe should evaluate");

    assert_eq!(result, "true");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("globalThis.__reportOnlyInlineCspViolations.join(',')")
            .expect("queued report-only inline CSP violation should be observable"),
        "report:script-src-elem:inline"
    );
}

#[test]
fn hyperlink_javascript_url_csp_checks_the_source_document_before_target_selection() {
    for (target, expected_kind) in [("", "top"), ("_blank", "popup")] {
        let mut vm = new_storage_test_vm(&format!(
            "https://hyperlink-javascript-csp-{expected_kind}.test/page.html"
        ));
        vm.set_response_content_security_policies(&["script-src 'none'".to_owned()]);

        vm.eval(&format!(
            r#"
(() => {{
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {{
    violations.push(`${{event.disposition}}:${{event.effectiveDirective}}:${{event.blockedURI}}`);
  }});
  globalThis.__hyperlinkJavascriptCspViolations = violations;
  const root = document.body || document.documentElement ||
      document.appendChild(document.createElement("html"));
  const anchor = document.createElement("a");
  anchor.href = "javascript:globalThis.__blockedHyperlinkJavascriptRan = true";
  anchor.target = "{target}";
  anchor.rel = "opener";
  root.appendChild(anchor);
  anchor.click();
}})()
"#
        ))
        .expect("blocked hyperlink javascript URL activation should evaluate");

        assert!(
            vm.take_pending_location_navigation_with_seed().is_none(),
            "blocked {expected_kind} hyperlink must not queue a top-level navigation"
        );
        assert!(
            vm.take_pending_popup_activations().is_empty(),
            "blocked {expected_kind} hyperlink must not create a popup"
        );
        assert_eq!(
            drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
            1
        );
        assert_eq!(
            vm.eval(
                "JSON.stringify({ ran: globalThis.__blockedHyperlinkJavascriptRan === true, violations: globalThis.__hyperlinkJavascriptCspViolations })"
            )
            .expect("blocked hyperlink javascript URL result should be observable"),
            r#"{"ran":false,"violations":["enforce:script-src-elem:inline"]}"#,
            "case {expected_kind}"
        );
    }
}

#[test]
fn inline_style_csp_blocks_cascade_preserves_text_and_targets_source_elements() {
    let mut vm = new_storage_test_vm("https://inline-style-csp.test/page.html");
    vm.set_response_content_security_policies(&[
        "style-src 'nonce-allowed' 'report-sample'".to_owned()
    ]);

    let result = vm
        .eval(
            r##"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const head = document.head || html.appendChild(document.createElement("head"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const violations = [];

  const blockedElement = document.createElement("style");
  blockedElement.textContent = "#blocked-target { margin-left: 11px; }";
  blockedElement.addEventListener("securitypolicyviolation", event => {
    violations.push({
      kind: "element",
      target: event.target === blockedElement,
      directive: event.effectiveDirective,
      sample: event.sample,
      disposition: event.disposition,
    });
  });
  head.appendChild(blockedElement);

  const blockedTarget = document.createElement("div");
  blockedTarget.id = "blocked-target";
  blockedTarget.setAttribute("style", "margin-right: 13px;");
  blockedTarget.addEventListener("securitypolicyviolation", event => {
    violations.push({
      kind: "attribute",
      target: event.target === blockedTarget,
      directive: event.effectiveDirective,
      sample: event.sample,
      disposition: event.disposition,
    });
  });
  body.appendChild(blockedTarget);

  const nonceStyle = document.createElement("style");
  nonceStyle.setAttribute("nonce", "allowed");
  nonceStyle.textContent = "#nonce-target { margin-left: 17px; }";
  head.appendChild(nonceStyle);
  const nonceTarget = document.createElement("div");
  nonceTarget.id = "nonce-target";
  body.appendChild(nonceTarget);

  const cssomTarget = document.createElement("div");
  body.appendChild(cssomTarget);
  cssomTarget.style.cssText = "margin-left: 19px;";

  globalThis.__inlineStyleCsp = {
    violations,
    blockedSheet: blockedElement.sheet === null,
    blockedText: blockedElement.textContent,
    blockedAttribute: blockedTarget.getAttribute("style"),
    blockedStyleLength: blockedTarget.style.length,
    blockedElementMargin: getComputedStyle(blockedTarget).marginLeft,
    blockedAttributeMargin: getComputedStyle(blockedTarget).marginRight,
    nonceMargin: getComputedStyle(nonceTarget).marginLeft,
    cssomMargin: getComputedStyle(cssomTarget).marginLeft,
  };
  return "queued";
})()
"##,
        )
        .expect("dynamic inline style CSP probe should evaluate");

    assert_eq!(result, "queued");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        2
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__inlineStyleCsp)")
            .expect("queued inline style CSP violations should be observable"),
        r##"{"violations":[{"kind":"element","target":true,"directive":"style-src-elem","sample":"#blocked-target { margin-left: 11px; }","disposition":"enforce"},{"kind":"attribute","target":true,"directive":"style-src-attr","sample":"margin-right: 13px;","disposition":"enforce"}],"blockedSheet":true,"blockedText":"#blocked-target { margin-left: 11px; }","blockedAttribute":"margin-right: 13px;","blockedStyleLength":0,"blockedElementMargin":"0px","blockedAttributeMargin":"0px","nonceMargin":"17px","cssomMargin":"19px"}"##
    );
}

#[tokio::test]
async fn blocked_inline_style_csp_dispatches_asynchronous_error_for_each_processing() {
    let mut vm = new_storage_test_vm("https://inline-style-csp-error.test/page.html");
    vm.set_response_content_security_policies(&["style-src 'nonce-allowed'".to_owned()]);

    assert_eq!(
        vm.eval(
            r##"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const head = document.head || html.appendChild(document.createElement("head"));
  const style = document.createElement("style");
  const events = [];
  style.textContent = "#target { margin-left: 37px; }";
  style.addEventListener("error", event => {
    events.push({
      type: event.type,
      target: event.target === style,
      bubbles: event.bubbles,
      cancelable: event.cancelable,
    });
  });
  style.addEventListener("load", () => events.push({type: "load"}));
  head.appendChild(style);
  globalThis.__blockedInlineStyle = {style, events};
  return JSON.stringify(events);
})()
"##,
        )
        .expect("blocked inline-style error probe should evaluate"),
        "[]",
        "stylesheet processing events must be asynchronous"
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__blockedInlineStyle.events)")
            .expect("CSP violation dispatch must not consume the style event task"),
        "[]"
    );

    assert!(vm.apply_connected_style_lifecycle_bodies_for_test());
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__blockedInlineStyle.events)")
            .expect("blocked style error should be observable"),
        r#"[{"type":"error","target":true,"bubbles":false,"cancelable":false}]"#
    );

    assert_eq!(
        vm.eval(
            r##"
globalThis.__blockedInlineStyle.style.textContent =
  "#target { margin-left: 41px; }";
globalThis.__blockedInlineStyle.events.length
"##,
        )
        .expect("blocked style contents should reprocess"),
        "1",
        "content mutation must not synchronously dispatch error"
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert!(vm.apply_connected_style_lifecycle_bodies_for_test());
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__blockedInlineStyle.events)")
            .expect("second blocked processing error should be observable"),
        r#"[{"type":"error","target":true,"bubbles":false,"cancelable":false},{"type":"error","target":true,"bubbles":false,"cancelable":false}]"#
    );
}

#[tokio::test]
async fn csp_blocked_declarative_css_module_dispatches_asynchronous_error() {
    let mut vm = new_storage_test_vm("https://declarative-css-module-csp.test/page.html");
    vm.set_response_content_security_policies(&["style-src 'none'".to_owned()]);

    assert_eq!(
        vm.eval(
            r##"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const head = document.head || html.appendChild(document.createElement("head"));
  const style = document.createElement("style");
  const events = [];
  style.type = "module";
  style.setAttribute("specifier", "theme");
  style.textContent = "#target { margin-left: 37px; }";
  style.addEventListener("error", event => {
    events.push({
      type: event.type,
      target: event.target === style,
      bubbles: event.bubbles,
      cancelable: event.cancelable,
    });
  });
  style.addEventListener("load", () => events.push({type: "load"}));
  head.appendChild(style);
  globalThis.__blockedDeclarativeCssModule = {style, events};
  return JSON.stringify(events);
})()
"##,
        )
        .expect("blocked declarative CSS module probe should evaluate"),
        "[]",
        "declarative CSS module processing events must be asynchronous"
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );

    assert!(vm.apply_connected_style_lifecycle_bodies_for_test());
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__blockedDeclarativeCssModule.events)")
            .expect("blocked declarative CSS module error should be observable"),
        r#"[{"type":"error","target":true,"bubbles":false,"cancelable":false}]"#
    );
    assert_eq!(
        vm.eval("globalThis.__blockedDeclarativeCssModule.style.sheet === null")
            .expect("declarative CSS module must not install a classic stylesheet"),
        "true"
    );
}

#[tokio::test]
async fn allowed_declarative_css_module_skips_classic_stylesheet_lifecycle() {
    let mut vm = new_storage_test_vm("https://allowed-declarative-css-module.test/page.html");
    vm.set_response_content_security_policies(&["style-src 'unsafe-inline'".to_owned()]);

    assert_eq!(
        vm.eval(
            r##"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const head = document.head || html.appendChild(document.createElement("head"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const target = document.createElement("div");
  target.id = "target";
  body.appendChild(target);
  const style = document.createElement("style");
  const events = [];
  style.type = "module";
  style.setAttribute("specifier", "theme");
  style.textContent = "#target { margin-left: 37px; }";
  style.addEventListener("error", () => events.push("error"));
  style.addEventListener("load", () => events.push("load"));
  head.appendChild(style);
  globalThis.__allowedDeclarativeCssModule = {style, target, events};
  return "ready";
})()
"##,
        )
        .expect("allowed declarative CSS module probe should evaluate"),
        "ready"
    );

    assert!(
        !vm.apply_connected_style_lifecycle_bodies_for_test(),
        "an allowed declarative CSS module must not enter the classic stylesheet lifecycle"
    );
    assert_eq!(
        vm.eval(
            r##"
JSON.stringify({
  events: globalThis.__allowedDeclarativeCssModule.events,
  sheetIsNull: globalThis.__allowedDeclarativeCssModule.style.sheet === null,
  margin: getComputedStyle(globalThis.__allowedDeclarativeCssModule.target).marginLeft,
})
"##,
        )
        .expect("allowed declarative CSS module state should be observable"),
        r#"{"events":[],"sheetIsNull":true,"margin":"0px"}"#
    );

    let owner = vm
        .current_main_document_task_owner()
        .expect("allowed declarative CSS module document owner");
    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser completion should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should apply");
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "an allowed declarative CSS module must not acquire an unowned style load-delay binding"
    );
}

#[tokio::test]
async fn declarative_css_module_requires_both_style_and_script_csp_to_allow_inline() {
    let mut vm = new_storage_test_vm("https://declarative-css-module-script-csp.test/page.html");
    vm.set_response_content_security_policies(&[
        "script-src 'nonce-allowed'; style-src 'unsafe-inline'".to_owned(),
    ]);

    assert_eq!(
        vm.eval(
            r##"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const head = document.head || html.appendChild(document.createElement("head"));
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push({
      directive: event.effectiveDirective,
      target: event.target && event.target.id,
    });
  });
  const createModule = (id, nonce) => {
    const style = document.createElement("style");
    style.id = id;
    style.type = "module";
    style.setAttribute("specifier", id);
    style.textContent = `#${id} { margin-left: 37px; }`;
    if (nonce) style.setAttribute("nonce", nonce);
    const events = [];
    style.addEventListener("error", () => events.push("error"));
    style.addEventListener("load", () => events.push("load"));
    head.appendChild(style);
    return {style, events};
  };
  globalThis.__declarativeModuleScriptCsp = {
    blocked: createModule("blocked", null),
    allowed: createModule("allowed", "allowed"),
    violations,
  };
  return "queued";
})()
"##,
        )
        .expect("declarative CSS module script CSP probe should evaluate"),
        "queued"
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );

    assert!(vm.apply_connected_style_lifecycle_bodies_for_test());
    assert_eq!(
        vm.eval(
            r##"
JSON.stringify({
  violations: globalThis.__declarativeModuleScriptCsp.violations,
  blockedEvents: globalThis.__declarativeModuleScriptCsp.blocked.events,
  allowedEvents: globalThis.__declarativeModuleScriptCsp.allowed.events,
  blockedSheetIsNull: globalThis.__declarativeModuleScriptCsp.blocked.style.sheet === null,
  allowedSheetIsNull: globalThis.__declarativeModuleScriptCsp.allowed.style.sheet === null,
})
"##,
        )
        .expect("declarative CSS module script CSP result should be observable"),
        r#"{"violations":[{"directive":"script-src-elem","target":"blocked"}],"blockedEvents":["error"],"allowedEvents":[],"blockedSheetIsNull":true,"allowedSheetIsNull":true}"#
    );
}

#[tokio::test]
async fn report_only_script_csp_reports_declarative_css_module_without_blocking() {
    let mut vm = new_storage_test_vm("https://declarative-css-module-report-only.test/page.html");
    vm.set_response_content_security_policies(&["style-src 'unsafe-inline'".to_owned()]);
    vm.set_response_content_security_report_only_policies(&["script-src 'none'".to_owned()]);

    assert_eq!(
        vm.eval(
            r##"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const head = document.head || html.appendChild(document.createElement("head"));
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push({
      directive: event.effectiveDirective,
      disposition: event.disposition,
      target: event.target && event.target.id,
    });
  });
  const style = document.createElement("style");
  const events = [];
  style.id = "reported-module";
  style.type = "module";
  style.setAttribute("specifier", "reported-module");
  style.textContent = "#target { margin-left: 37px; }";
  style.addEventListener("error", () => events.push("error"));
  style.addEventListener("load", () => events.push("load"));
  head.appendChild(style);
  globalThis.__reportedDeclarativeCssModule = {style, events, violations};
  return "queued";
})()
"##,
        )
        .expect("report-only declarative CSS module probe should evaluate"),
        "queued"
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );

    assert!(
        !vm.apply_connected_style_lifecycle_bodies_for_test(),
        "report-only script CSP must not fail declarative module processing"
    );
    assert_eq!(
        vm.eval(
            r##"
JSON.stringify({
  violations: globalThis.__reportedDeclarativeCssModule.violations,
  events: globalThis.__reportedDeclarativeCssModule.events,
  sheetIsNull: globalThis.__reportedDeclarativeCssModule.style.sheet === null,
})
"##,
        )
        .expect("report-only declarative CSS module result should be observable"),
        r#"{"violations":[{"directive":"script-src-elem","disposition":"report","target":"reported-module"}],"events":[],"sheetIsNull":true}"#
    );
}

#[tokio::test]
async fn external_stylesheet_csp_blocks_before_fetch_and_dispatches_async_error() {
    let mut vm = new_storage_test_vm("https://external-style-csp.test/page.html");
    vm.set_response_content_security_policies(&["style-src 'none'".to_owned()]);

    assert_eq!(
        vm.eval(
            r##"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const head = document.head || html.appendChild(document.createElement("head"));
  const link = document.createElement("link");
  const events = [];
  link.id = "blocked-external-style";
  link.rel = "stylesheet";
  link.href = "https://styles.invalid/blocked.css";
  link.addEventListener("error", event => events.push({
    type: event.type,
    target: event.target === link,
    bubbles: event.bubbles,
    cancelable: event.cancelable,
  }));
  link.addEventListener("load", () => events.push({type: "load"}));
  head.appendChild(link);
  globalThis.__blockedExternalStyle = {link, events, violations: []};
  document.addEventListener("securitypolicyviolation", event => {
    globalThis.__blockedExternalStyle.violations.push({
      target: event.target === link,
      directive: event.effectiveDirective,
      blockedURI: event.blockedURI,
    });
  });
  return JSON.stringify(events);
})()
"##,
        )
        .expect("blocked external stylesheet probe should evaluate"),
        "[]",
        "CSP failure must not synchronously dispatch the link error"
    );

    vm.document_runtime
        .note_discovered_live_blocking_stylesheets();
    let owner = vm
        .document_runtime
        .get_element_by_id("blocked-external-style")
        .expect("blocked link owner");
    let request_url = Url::parse("https://styles.invalid/blocked.css").expect("request URL");
    let signature = crate::stylesheet_blocking::DocumentBlockingStylesheetSignature::Link {
        url: request_url,
        options: crate::stylesheet_blocking::StylesheetFetchOptions::default(),
    };
    assert!(
        !vm.document_runtime
            .has_blocking_stylesheet_fetch_for_test(owner, &signature),
        "an enforced CSP rejection must not enter the shared stylesheet fetch state"
    );
    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
  count: document.styleSheets.length,
  sheetPresent: globalThis.__blockedExternalStyle.link.sheet !== null,
  ruleCount: globalThis.__blockedExternalStyle.link.sheet.cssRules.length,
  href: globalThis.__blockedExternalStyle.link.sheet.href,
})"#,
        )
        .expect("blocked external stylesheet surface should evaluate"),
        r#"{"count":1,"sheetPresent":true,"ruleCount":0,"href":"https://styles.invalid/blocked.css"}"#,
        "a CSP-blocked external sheet remains an empty CSSStyleSheet in the document list"
    );

    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__blockedExternalStyle.violations)")
            .expect("stylesheet CSP violation should be observable"),
        r#"[{"target":true,"directive":"style-src-elem","blockedURI":"https://styles.invalid/blocked.css"}]"#
    );
    assert!(vm.apply_connected_style_lifecycle_bodies_for_test());
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__blockedExternalStyle.events)")
            .expect("blocked external stylesheet error should be observable"),
        r#"[{"type":"error","target":true,"bubbles":false,"cancelable":false}]"#
    );
}

#[tokio::test]
async fn hidden_link_nonce_authorizes_stylesheet_csp_before_resource_admission() {
    let mut vm = new_storage_test_vm("https://external-style-nonce.test/page.html");
    vm.set_response_content_security_policies(&[
        "style-src-elem 'nonce-allowed'; style-src 'none'".to_owned(),
    ]);

    assert_eq!(
        vm.eval(
            r##"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const head = document.head || html.appendChild(document.createElement("head"));
  const link = document.createElement("link");
  link.id = "nonce-authorized-external-style";
  link.rel = "stylesheet";
  link.href = "data:text/css,body%7Bcolor%3Agreen%7D";
  link.nonce = "allowed";
  head.appendChild(link);
  return JSON.stringify({
    nonce: link.nonce,
    contentAttribute: link.getAttribute("nonce"),
  });
})()
"##,
        )
        .expect("nonce-authorized stylesheet probe should evaluate"),
        r#"{"nonce":"allowed","contentAttribute":null}"#,
        "insertion must preserve the cryptographic nonce while hiding the content attribute"
    );

    let owner = vm
        .document_runtime
        .get_element_by_id("nonce-authorized-external-style")
        .expect("nonce-authorized link owner");
    vm.document_runtime.queue_connected_style_loads(owner);
    vm.document_runtime.prime_document_lifecycle_processing();
    let request_url =
        Url::parse("data:text/css,body%7Bcolor%3Agreen%7D").expect("stylesheet data URL");
    let expected_options = crate::stylesheet_blocking::StylesheetFetchOptions::from_link_attributes(
        None,
        None,
        None,
        Some("allowed"),
        None,
        None,
    );

    assert!(
        !vm.document_runtime.stylesheet_owner_is_csp_blocked(owner),
        "the live cryptographic nonce must authorize the link request"
    );
    let load = vm
        .document_runtime
        .active_stylesheet_link_client_for_test(owner)
        .expect("the authorized dynamic link must enter the typed load-client state");
    assert_eq!(
        load.request_url(),
        &request_url,
        "the typed load client must preserve the admitted request URL"
    );
    assert_eq!(
        load.fetch().options(),
        &expected_options,
        "the typed load client must preserve the hidden nonce used for CSP admission"
    );
}

#[tokio::test]
async fn style_preload_csp_blocks_before_typed_resource_and_dispatches_async_error() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("style preload CSP server");
    let addr = listener
        .local_addr()
        .expect("style preload CSP server address");
    let mut vm = new_storage_test_vm("https://style-preload-csp.test/page.html");
    vm.set_response_content_security_policies(&["style-src 'none'".to_owned()]);

    assert_eq!(
        vm.eval(&format!(
            r##"
(() => {{
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const head = document.head || html.appendChild(document.createElement("head"));
  const link = document.createElement("link");
  const events = [];
  link.rel = "preload";
  link.as = "style";
  link.href = "http://{addr}/blocked.css";
  link.addEventListener("load", () => events.push("load"));
  link.addEventListener("error", () => events.push("error"));
  head.appendChild(link);
  globalThis.__blockedStylePreload = {{link, events, violations: []}};
  document.addEventListener("securitypolicyviolation", event => {{
    globalThis.__blockedStylePreload.violations.push({{
      target: event.target === link,
      directive: event.effectiveDirective,
      blockedURI: event.blockedURI,
    }});
  }});
  return events.join(",");
}})()
"##
        ))
        .expect("blocked style preload probe should evaluate"),
        "",
        "CSP failure must not synchronously dispatch the preload error"
    );

    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert!(vm.apply_connected_style_lifecycle_bodies_for_test());
    assert_eq!(
        vm.eval("globalThis.__blockedStylePreload.events.join(',')")
            .expect("style preload error should be observable"),
        "error"
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__blockedStylePreload.violations)")
            .expect("style preload CSP violation should be observable"),
        format!(
            r#"[{{"target":true,"directive":"style-src-elem","blockedURI":"http://{addr}/blocked.css"}}]"#
        )
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "CSP-blocked style preload must not create a physical request"
    );
}

#[test]
fn linked_stylesheet_import_csp_violation_targets_the_document() {
    let mut vm = new_storage_test_vm("https://style-import-csp.test/page.html");
    vm.set_response_content_security_policies(&["style-src 'self'".to_owned()]);

    assert_eq!(
        vm.eval(
            r#"
globalThis.__styleImportViolations = [];
document.addEventListener("securitypolicyviolation", event => {
  globalThis.__styleImportViolations.push({
    target: event.target === document,
    directive: event.effectiveDirective,
    blockedURI: event.blockedURI,
  });
});
JSON.stringify(globalThis.__styleImportViolations)
"#,
        )
        .expect("stylesheet import CSP listener should install"),
        "[]"
    );

    vm.queue_linked_stylesheet_import_csp_violations([Url::parse(
        "https://cross-origin.test/imported.css",
    )
    .expect("import URL")]);
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__styleImportViolations)")
            .expect("CSP violation must remain asynchronous"),
        "[]"
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__styleImportViolations)")
            .expect("stylesheet import CSP violation should be observable"),
        r#"[{"target":true,"directive":"style-src-elem","blockedURI":"https://cross-origin.test/imported.css"}]"#
    );
}

#[tokio::test]
async fn report_only_inline_style_csp_reports_without_blocking_style_application() {
    let mut vm = new_storage_test_vm("https://inline-style-report-only.test/page.html");
    vm.set_response_content_security_report_only_policies(&[
        "style-src 'none' 'report-sample'".to_owned()
    ]);

    let result = vm
        .eval(
            r##"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const head = document.head || html.appendChild(document.createElement("head"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push(`${event.disposition}:${event.effectiveDirective}:${event.sample}`);
  });
  globalThis.__inlineStyleReportOnlyErrors = 0;
  const style = document.createElement("style");
  style.addEventListener("error", () => globalThis.__inlineStyleReportOnlyErrors++);
  style.textContent = "#report-target { margin-left: 23px; }";
  head.appendChild(style);
  const target = document.createElement("div");
  target.id = "report-target";
  target.setAttribute("style", "margin-right: 29px;");
  body.appendChild(target);
  globalThis.__inlineStyleReportOnly = {
    violations,
    sheetPresent: style.sheet !== null,
    styleLength: target.style.length,
    left: getComputedStyle(target).marginLeft,
    right: getComputedStyle(target).marginRight,
  };
  return "queued";
})()
"##,
        )
        .expect("report-only inline style CSP probe should evaluate");

    assert_eq!(result, "queued");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        2
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__inlineStyleReportOnly)")
            .expect("queued report-only inline style CSP violations should be observable"),
        r##"{"violations":["report:style-src-elem:#report-target { margin-left: 23px; }","report:style-src-attr:margin-right: 29px;"],"sheetPresent":true,"styleLength":1,"left":"23px","right":"29px"}"##
    );
    assert!(vm.apply_connected_style_lifecycle_bodies_for_test());
    assert_eq!(
        vm.eval("globalThis.__inlineStyleReportOnlyErrors")
            .expect("report-only style event result should be observable"),
        "0",
        "report-only CSP must not turn successful processing into an error"
    );
}

#[test]
fn cssom_same_text_write_replaces_a_blocked_style_attribute_declaration() {
    let mut vm = new_storage_test_vm("https://inline-style-cssom.test/page.html");
    vm.set_response_content_security_policies(&["style-src 'none'".to_owned()]);

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const target = document.createElement("div");
  target.setAttribute("style", "margin-left: 31px;");
  body.appendChild(target);
  const beforeLength = target.style.length;
  target.style.cssText = "margin-left: 31px;";
  globalThis.__sameTextCssom = {
    beforeLength,
    afterLength: target.style.length,
    margin: getComputedStyle(target).marginLeft,
    attribute: target.getAttribute("style"),
  };
  return "queued";
})()
"#,
        )
        .expect("same-text CSSOM write should evaluate"),
        "queued"
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__sameTextCssom)")
            .expect("same-text CSSOM result should remain observable"),
        r#"{"beforeLength":0,"afterLength":1,"margin":"31px","attribute":"margin-left: 31px;"}"#
    );
}

#[tokio::test]
async fn prepared_parser_inline_script_csp_blocks_before_v8_execution_and_reports_start_position() {
    let mut vm = new_storage_test_vm("https://parser-inline-csp.test/page.html");
    vm.set_response_content_security_policies(&["script-src 'none'".to_owned()]);
    vm.eval(
        r#"
        globalThis.__parserInlineScriptRan = false;
        globalThis.__parserInlineViolations = [];
        document.addEventListener("securitypolicyviolation", event => {
          globalThis.__parserInlineViolations.push({
            blockedURI: event.blockedURI,
            effectiveDirective: event.effectiveDirective,
            lineNumber: event.lineNumber,
            columnNumber: event.columnNumber,
          });
        });
        "installed";
        "#,
    )
    .expect("parser inline-script CSP observer should install");

    let document = vm.document_runtime.document_handle();
    let html = vm.document_runtime.dom_host_mut().create_element("html");
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(document, html)
    );
    let script_node = vm
        .document_runtime
        .dom_host_mut()
        .create_parser_element_without_attributes(
            "script".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
    let source = "globalThis.__parserInlineScriptRan = true;";
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .set_script_text_internal_slot(script_node, source)
    );
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(html, script_node)
    );
    let host_script_handle = vm
        .document_runtime
        .bind_parser_owned_script_handle_for_node(script_node);
    vm.document_runtime
        .note_parser_script_start_position(script_node, 17, 23);

    let document_url = Url::parse("https://parser-inline-csp.test/page.html").unwrap();
    let script = PreparedScript {
        position: 0,
        node_id: script_node,
        kind: ScriptKind::Classic,
        mode: ScriptMode::Normal,
        source_kind: ScriptSourceKind::Inline,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: ScriptSource::Inline(source.to_owned()),
        url: document_url.clone(),
        base_url: document_url.clone(),
        initiator_url: document_url,
        host_script_handle: Some(host_script_handle),
    };
    vm.execute_loaded_prepared_script_source(&script, source, None)
        .await
        .expect("blocked inline script should complete without entering V8");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );

    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
              ran: globalThis.__parserInlineScriptRan,
              violations: globalThis.__parserInlineViolations,
            })"#
        )
        .expect("parser inline-script CSP result should remain observable"),
        r#"{"ran":false,"violations":[{"blockedURI":"inline","effectiveDirective":"script-src-elem","lineNumber":17,"columnNumber":23}]}"#
    );

    vm.eval("globalThis.__parserInlineViolations.length = 0")
        .expect("parser CSP violations should reset");
    vm.document_runtime
        .note_parser_script_start_position(script_node, 0, 0);
    vm.execute_loaded_prepared_script_source(&script, source, None)
        .await
        .expect("blocked inline script with unknown position should complete");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__parserInlineViolations)")
            .expect("unknown parser source position should remain observable"),
        r#"[{"blockedURI":"inline","effectiveDirective":"script-src-elem","lineNumber":0,"columnNumber":0}]"#
    );
}
