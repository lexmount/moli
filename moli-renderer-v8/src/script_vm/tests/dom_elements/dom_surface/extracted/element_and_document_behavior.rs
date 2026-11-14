use super::*;

#[test]
fn svg_create_rect_supports_capability_detection_and_detached_float_values() {
    let mut vm = new_storage_test_vm("https://svg-rect.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../../../tests/fixtures/svg-create-rect.js"
        ))
        .expect("SVGRect contract should pass"),
        "svg-create-rect:ok"
    );
}
#[test]
fn document_wrapper_does_not_expose_element_only_accessors() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const props = ["dataset", "classList", "id", "className", "name"];
              return JSON.stringify({
                own: Object.fromEntries(props.map((key) => [key, Object.prototype.hasOwnProperty.call(document, key)])),
                enumerableKeys: Object.keys(document).filter((key) => props.includes(key)),
                querySelectorType: typeof document.querySelector,
                getElementsByTagNameType: typeof document.getElementsByTagName
              });
            })()
            "#,
        )
        .expect("document element-only accessor probe should evaluate");

    assert_eq!(
        result,
        r#"{"own":{"dataset":false,"classList":false,"id":false,"className":false,"name":false},"enumerableKeys":[],"querySelectorType":"function","getElementsByTagNameType":"function"}"#
    );
}
#[test]
fn vertical_block_flow_uses_writing_mode_instead_of_inline_direction_for_x_anchor() {
    let mut vm = new_storage_test_vm("https://vertical-block-flow-anchor.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          document.documentElement.style.margin = "0";
          document.body.style.margin = "0";
          document.body.innerHTML = `
            <style>
              .case {
                width: 200px;
                height: 200px;
                margin: 1px;
                padding: 0;
                border: 0;
                overflow: auto;
                scrollbar-gutter: stable;
              }
              .case > div { width: 300px; height: 300px; }
              .vlr { writing-mode: vertical-lr; }
              .vrl { writing-mode: vertical-rl; }
              .thin { scrollbar-width: thin; }
              .none { scrollbar-width: none; }
              .rtl { direction: rtl; }
              .boxed {
                box-sizing: border-box;
                width: 240px;
                border-style: solid;
                border-width: 0 4px 0 3px;
                padding: 0 7px 0 11px;
                scrollbar-width: none;
              }
              .boxed > div {
                margin-left: 13px;
                margin-right: 17px;
                position: relative;
                left: 5px;
              }
            </style>
            <div id="vlr-auto" class="case vlr"><div></div></div>
            <div id="vlr-thin" class="case vlr thin"><div></div></div>
            <div id="vlr-none" class="case vlr none"><div></div></div>
            <div id="vrl-auto" class="case vrl"><div></div></div>
            <div id="vrl-thin" class="case vrl thin"><div></div></div>
            <div id="vrl-none" class="case vrl none"><div></div></div>
            <div id="vlr-rtl" class="case vlr rtl"><div></div></div>
            <div id="vrl-rtl" class="case vrl rtl"><div></div></div>
            <div id="boxed-vlr" class="case boxed vlr rtl"><div></div></div>
            <div id="boxed-vrl" class="case boxed vrl"><div></div></div>`;
        })()
        "#,
    )
    .expect("vertical block-flow fixture should initialize");
    publish_layout_for_test(&mut vm);

    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify([
              "vlr-auto", "vlr-thin", "vlr-none",
              "vrl-auto", "vrl-thin", "vrl-none",
              "vlr-rtl", "vrl-rtl", "boxed-vlr", "boxed-vrl"
            ].map(id => {
              const container = document.getElementById(id);
              const content = container.firstElementChild;
              return [
                container.offsetWidth,
                container.clientWidth,
                container.offsetLeft,
                content.offsetLeft,
                content.offsetWidth,
              ];
            }))
            "#,
        )
        .expect("vertical block-flow metrics should evaluate"),
        "[[200,185,1,1,300],[200,190,1,1,300],[200,200,1,1,300],[200,185,1,-114,300],[200,190,1,-109,300],[200,200,1,-99,300],[200,185,1,1,300],[200,185,1,-114,300],[240,233,1,33,300],[240,233,1,-82,300]]"
    );
}
#[test]
fn domexception_names_share_central_legacy_code_table_without_constructors() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const cases = [
                ["NotFoundError", 8],
                ["InvalidStateError", 11],
                ["SecurityError", 18],
                ["NetworkError", 19],
                ["AbortError", 20],
                ["TimeoutError", 23],
                ["DataError", 0],
                ["OperationError", 0],
                ["ConstraintError", 0],
                ["TransactionInactiveError", 0],
                ["VersionError", 0],
                ["UnknownError", 0],
                ["NotAllowedError", 0],
                ["EncodingError", 0],
                ["ReadOnlyError", 0],
                ["NotReadableError", 0],
                ["WebSocketError", 0]
              ];
              const failures = [];
              for (const [name, code] of cases) {
                const error = new DOMException("message", name);
                if (error.name !== name || error.message !== "message" || error.code !== code) {
                  failures.push(`${name}:shape:${error.name}:${error.message}:${error.code}`);
                }
              }
              for (const name of [
                "AbortError",
                "DataError",
                "OperationError",
                "InvalidStateError",
                "EncodingError",
                "UnknownError"
              ]) {
                if (typeof globalThis[name] !== "undefined") {
                  failures.push(`${name}:constructor`);
                }
              }
              const socketNamed = new DOMException("message", "WebSocketError");
              if (socketNamed instanceof WebSocketError) {
                failures.push("WebSocketError:ordinary-constructor");
              }
              if (DOMException.NOT_FOUND_ERR !== 8 || DOMException.DATA_CLONE_ERR !== 25) {
                failures.push("constants");
              }
              return JSON.stringify(failures);
            })()
            "#,
        )
        .expect("DOMException code table probe should evaluate");

    assert_eq!(result, "[]");
}
#[test]
fn autocorrect_reflects_boolean_and_inherits_from_form_owner() {
    let mut vm = new_parsed_test_vm(
        "https://autocorrect.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const div = document.createElement('div');
  const canonical = [null, 'on', 'ON', 'off', 'OFF', 'invalid', ''].map(value => {
    if (value === null) div.removeAttribute('autocorrect');
    else div.setAttribute('autocorrect', value);
    return div.autocorrect;
  });

  div.autocorrect = 'hello';
  const truthy = [div.autocorrect, div.getAttribute('autocorrect')];
  div.autocorrect = 0;
  const falsy = [div.autocorrect, div.getAttribute('autocorrect')];

  const form = document.createElement('form');
  form.id = 'owner';
  form.setAttribute('autocorrect', 'off');
  document.body.appendChild(form);
  const inheritedNames = ['button', 'fieldset', 'input', 'output', 'select', 'textarea'];
  const inherited = inheritedNames.map(name => {
    const child = document.createElement(name);
    form.appendChild(child);
    const external = document.createElement(name);
    external.setAttribute('form', 'owner');
    external.setAttributeNS('urn:test', 'autocorrect', 'on');
    document.body.appendChild(external);
    const values = [child.autocorrect, external.autocorrect];
    child.setAttribute('autocorrect', '');
    values.push(child.autocorrect);
    child.setAttribute('autocorrect', 'off');
    values.push(child.autocorrect);
    return values.join(':');
  });

  const nonInherited = ['img', 'object'].map(name => {
    const element = document.createElement(name);
    form.appendChild(element);
    return element.autocorrect;
  });
  const forcedOff = ['password', 'email', 'url'].map(type => {
    const input = document.createElement('input');
    input.type = type;
    input.autocorrect = true;
    return input.autocorrect;
  });

  const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'autocorrect');
  const incompatible = operation => {
    try {
      operation(document.createElementNS('urn:test', 'div'));
      return 'none';
    } catch (error) {
      return error.name;
    }
  };

  return JSON.stringify({
    descriptor: [descriptor.enumerable, descriptor.configurable],
    canonical,
    truthy,
    falsy,
    inherited,
    nonInherited,
    forcedOff,
    incompatible: [
      incompatible(receiver => descriptor.get.call(receiver)),
      incompatible(receiver => descriptor.set.call(receiver, true))
    ]
  });
})()
"#,
        )
        .expect("autocorrect semantics should evaluate");

    assert_eq!(
        result,
        r#"{"descriptor":[true,true],"canonical":[true,true,true,false,false,true,true],"truthy":[true,"on"],"falsy":[false,"off"],"inherited":["false:false:true:false","false:false:true:false","false:false:true:false","false:false:true:false","false:false:true:false","false:false:true:false"],"nonInherited":[true,true],"forcedOff":[false,false,false],"incompatible":["TypeError","TypeError"]}"#
    );
}
#[test]
fn writing_suggestions_reflects_and_inherits_nearest_ancestor_state() {
    let mut vm = new_parsed_test_vm(
        "https://writing-suggestions.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.createElement('section');
  const child = document.createElement('div');
  const grandchild = document.createElement('span');
  parent.append(child);
  child.append(grandchild);
  parent.setAttribute('writingsuggestions', 'FaLsE');

  const inherited = [
    parent.writingSuggestions,
    child.writingSuggestions,
    grandchild.writingSuggestions
  ];
  child.setAttribute('writingsuggestions', '');
  const emptyOverride = [child.writingSuggestions, grandchild.writingSuggestions];
  child.setAttribute('writingsuggestions', 'invalid');
  const invalidOverride = [child.writingSuggestions, grandchild.writingSuggestions];
  child.removeAttribute('writingsuggestions');
  const restoredInheritance = grandchild.writingSuggestions;

  grandchild.writingSuggestions = false;
  const booleanSetter = [
    grandchild.writingSuggestions,
    grandchild.getAttribute('writingsuggestions')
  ];
  grandchild.writingSuggestions = { toString() { return 'TrUe'; } };
  const domStringSetter = [
    grandchild.writingSuggestions,
    grandchild.getAttribute('writingsuggestions')
  ];

  const namespaceOnly = document.createElement('div');
  namespaceOnly.setAttributeNS('urn:test', 'writingsuggestions', 'false');

  const detachedParent = document.createElement('div');
  const detachedChild = document.createElement('span');
  detachedParent.setAttribute('writingsuggestions', 'false');
  detachedParent.append(detachedChild);

  const descriptor = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    'writingSuggestions'
  );
  const incompatible = operation => {
    try {
      operation(document.createElementNS('urn:test', 'div'));
      return 'none';
    } catch (error) {
      return error.name;
    }
  };

  return JSON.stringify({
    descriptor: [descriptor.enumerable, descriptor.configurable],
    inherited,
    emptyOverride,
    invalidOverride,
    restoredInheritance,
    booleanSetter,
    domStringSetter,
    namespaceOnly: [
      namespaceOnly.writingSuggestions,
      namespaceOnly.getAttribute('writingsuggestions')
    ],
    detached: detachedChild.writingSuggestions,
    incompatible: [
      incompatible(receiver => descriptor.get.call(receiver)),
      incompatible(receiver => descriptor.set.call(receiver, 'false'))
    ]
  });
})()
"#,
        )
        .expect("writingSuggestions semantics should evaluate");

    assert_eq!(
        result,
        r#"{"descriptor":[true,true],"inherited":["false","false","false"],"emptyOverride":["true","true"],"invalidOverride":["true","true"],"restoredInheritance":"false","booleanSetter":["false","false"],"domStringSetter":["true","TrUe"],"namespaceOnly":["true","false"],"detached":"false","incompatible":["TypeError","TypeError"]}"#
    );
}
#[test]
fn document_query_command_support_and_enabled_follow_design_mode() {
    let mut vm = new_storage_test_vm("https://query-command-design-mode.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const methods = [
    "queryCommandEnabled",
    "queryCommandIndeterm",
    "queryCommandState",
    "queryCommandSupported",
    "queryCommandValue"
  ].map(name => {
    const method = Document.prototype[name];
    return `${name}:${typeof method}:${method.length}`;
  });
  const supported = ["delete", "DeLeTe", "forwardDelete", "selectAll", "copy"]
    .map(command => document.queryCommandSupported(command));
  const initial = [document.designMode, document.queryCommandEnabled("delete")];
  document.designMode = "ON";
  const enabled = [document.designMode, document.queryCommandEnabled("DeLeTe")];
  document.designMode = "invalid";
  const invalid = [document.designMode, document.queryCommandEnabled("delete")];
  document.designMode = "off";
  const disabled = [document.designMode, document.queryCommandEnabled("delete")];
  const neutral = [
    document.queryCommandIndeterm("delete"),
    document.queryCommandState("delete"),
    document.queryCommandValue("delete")
  ];
  const xml = document.implementation.createDocument(null, "root");
  const xmlErrors = [
    () => xml.execCommand("delete"),
    () => xml.queryCommandEnabled("delete"),
    () => xml.queryCommandIndeterm("delete"),
    () => xml.queryCommandState("delete"),
    () => xml.queryCommandSupported("delete"),
    () => xml.queryCommandValue("delete")
  ].map(invoke => {
    try {
      invoke();
      return null;
    } catch (error) {
      return error.name;
    }
  });
  const xhtml = new DOMParser().parseFromString(
    '<html xmlns="http://www.w3.org/1999/xhtml"><body/></html>',
    "application/xhtml+xml"
  );
  const xhtmlResults = [
    xhtml.queryCommandSupported("delete"),
    xhtml.queryCommandEnabled("delete"),
    xhtml.queryCommandIndeterm("delete"),
    xhtml.queryCommandState("delete"),
    xhtml.queryCommandValue("delete")
  ];
  return JSON.stringify({
    methods,
    supported,
    unknown: document.queryCommandSupported("not-a-command"),
    initial,
    enabled,
    invalid,
    disabled,
    neutral,
    xmlErrors,
    xhtmlResults
  });
})()
"#,
        )
        .expect("Document editing-command query probe should evaluate");

    assert_eq!(
        result,
        r#"{"methods":["queryCommandEnabled:function:1","queryCommandIndeterm:function:1","queryCommandState:function:1","queryCommandSupported:function:1","queryCommandValue:function:1"],"supported":[true,true,true,true,true],"unknown":false,"initial":["off",false],"enabled":["on",true],"invalid":["on",true],"disabled":["off",false],"neutral":[false,false,""],"xmlErrors":["InvalidStateError","InvalidStateError","InvalidStateError","InvalidStateError","InvalidStateError","InvalidStateError"],"xhtmlResults":[true,false,false,false,""]}"#
    );
}
#[test]
fn exec_command_select_all_targets_editing_host_and_skips_inert_text() {
    let mut vm = new_storage_test_vm("https://exec-command-select-all-selection.test/");

    vm.eval(
        r##"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const root = document.createElement("section");
  root.id = "selection-root";
  body.appendChild(root);
  root.innerHTML = '<div inert>hidden inert</div><div>visible text</div>';
  return 'installed';
})()
"##,
    )
    .expect("document selectAll fixture should initialize");
    publish_layout_for_test(&mut vm);
    let document_result = vm
        .eval(
            r##"
(() => {
  const body = document.body;
  const selection = getSelection();
  body.focus();
  const documentReturned = document.execCommand("selectAll");
  const documentText = selection.toString().trim();
  const documentRange = selection.getRangeAt(0);
  const documentRangeSpansBody =
    documentRange.startContainer === body &&
    documentRange.startOffset === 0 &&
    documentRange.endContainer === body &&
    documentRange.endOffset === body.childNodes.length;
  selection.removeAllRanges();
  return JSON.stringify({ documentReturned, documentText, documentRangeSpansBody });
})()
"##,
        )
        .expect("document selectAll selection probe should evaluate");
    assert_eq!(
        document_result,
        r#"{"documentReturned":true,"documentText":"visible text","documentRangeSpansBody":true}"#
    );

    vm.eval(
        r##"
(() => {
  const root = document.getElementById("selection-root");
  root.innerHTML =
    '<p>preceding text</p><div id="host" contenteditable>editable text</div><p>following text</p>';
  return 'installed';
})()
"##,
    )
    .expect("editing-host selectAll fixture should initialize");
    publish_layout_for_test(&mut vm);
    let host_result = vm
        .eval(
            r##"
(() => {
  const root = document.getElementById("selection-root");
  const selection = getSelection();
  const host = root.querySelector("#host");
  host.focus();
  const hostReturned = document.execCommand("selectAll");
  const hostText = selection.toString();
  const hostRange = selection.getRangeAt(0);

  return JSON.stringify({
    hostReturned,
    hostHasEditableText: hostText.includes("editable text"),
    hostHasPrecedingText: hostText.includes("preceding text"),
    hostHasFollowingText: hostText.includes("following text"),
    hostRangeSpansHost:
      hostRange.startContainer === host &&
      hostRange.startOffset === 0 &&
      hostRange.endContainer === host &&
      hostRange.endOffset === host.childNodes.length
  });
})()
"##,
        )
        .expect("execCommand selectAll selection probe should evaluate");

    assert_eq!(
        host_result,
        r#"{"hostReturned":true,"hostHasEditableText":true,"hostHasPrecedingText":false,"hostHasFollowingText":false,"hostRangeSpansHost":true}"#
    );
}
#[test]
fn exec_command_select_all_respects_modal_dialog_inertness() {
    let mut vm = new_storage_test_vm("https://exec-command-select-all-modal-dialog.test/");

    let result = eval_with_layout_publications(
        &mut vm,
        r##"
(function* () {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  body.textContent = "";
  body.append(
    document.createTextNode("Here is a text node you can't select while the dialog is open."),
    document.createElement("div"),
    document.createTextNode("Trailing text.")
  );
  const wrapper = body.querySelector("div");
  const dialog = document.createElement("dialog");
  wrapper.appendChild(dialog);
  dialog.textContent = "I'm selectable.";
  const selection = getSelection();

  dialog.showModal();
yield; // Publish this scene before reading its geometry.
  selection.selectAllChildren(body);
  const manualBodyText = selection.toString();
  selection.removeAllRanges();

  const commandReturned = document.execCommand("selectAll");
  const commandText = selection.toString();
  const commandRange = selection.getRangeAt(0);
  const commandRangeSpansDialog =
    commandRange.startContainer === dialog &&
    commandRange.startOffset === 0 &&
    commandRange.endContainer === dialog &&
    commandRange.endOffset === dialog.childNodes.length;

  wrapper.inert = true;
  selection.selectAllChildren(body);
  const inertAncestorText = selection.toString();
  wrapper.inert = false;

  dialog.close();
  selection.selectAllChildren(body);
yield; // Publish this scene before reading its geometry.
  const afterCloseText = selection.toString();

  return JSON.stringify({
    manualBodyText,
    commandReturned,
    commandText,
    commandRangeSpansDialog,
    inertAncestorText,
    afterCloseHasOutside: afterCloseText.includes("text node you can't select"),
    afterCloseHasDialog: afterCloseText.includes("I'm selectable."),
    afterCloseHasTrailing: afterCloseText.includes("Trailing text.")
  });
})()
"##,
    )
    .expect("execCommand selectAll modal dialog inertness probe should evaluate");

    assert_eq!(
        result,
        r#"{"manualBodyText":"I'm selectable.","commandReturned":true,"commandText":"I'm selectable.","commandRangeSpansDialog":true,"inertAncestorText":"I'm selectable.","afterCloseHasOutside":true,"afterCloseHasDialog":false,"afterCloseHasTrailing":true}"#
    );
}
#[test]
fn input_button_offset_width_reflects_label_value() {
    let mut vm = new_storage_test_vm("https://forms-input-button-width.test/");

    vm.eval(r#"
              const empty = document.createElement('input');
              empty.type = 'button';
              const labelled = document.createElement('input');
              labelled.type = 'button';
              labelled.value = 'BUTTON';
              const html = document.documentElement || document.appendChild(document.createElement('html'));
              const body = document.body || html.appendChild(document.createElement('body'));
              body.append(empty, labelled);
"#).expect("prepare button label geometry");
    publish_layout_for_test(&mut vm);
    let result = vm
        .eval(
            r#"(() => {              return [
                empty.value,
                labelled.value,
                empty.offsetWidth,
                labelled.offsetWidth,
                labelled.offsetWidth > empty.offsetWidth
              ].join('|');
            })()"#,
        )
        .expect("button input intrinsic width should reflect its label");

    let fields = result.split('|').collect::<Vec<_>>();
    assert_eq!(fields.len(), 5);
    assert_eq!(fields[0], "");
    assert_eq!(fields[1], "BUTTON");
    let empty_width = fields[2]
        .parse::<u32>()
        .expect("empty button width should be numeric");
    let labelled_width = fields[3]
        .parse::<u32>()
        .expect("labelled button width should be numeric");
    assert!(empty_width > 0);
    assert!(labelled_width > empty_width);
    assert_eq!(fields[4], "true");
}
#[test]
fn html_script_element_supports_static_method_matches_supported_type_tokens() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const descriptor = Object.getOwnPropertyDescriptor(HTMLScriptElement, "supports");
              return JSON.stringify({
              type: typeof HTMLScriptElement.supports,
              name: HTMLScriptElement.supports.name,
              length: HTMLScriptElement.supports.length,
              descriptor: [
                !!descriptor,
                descriptor && descriptor.enumerable,
                descriptor && descriptor.writable,
                descriptor && descriptor.configurable
              ],
              keysContainSupports: Object.keys(HTMLScriptElement).includes("supports"),
              classic: HTMLScriptElement.supports("classic"),
              module: HTMLScriptElement.supports("module"),
              importmap: HTMLScriptElement.supports("importmap"),
              jsMime: HTMLScriptElement.supports("text/javascript"),
              padded: HTMLScriptElement.supports(" module "),
              upper: HTMLScriptElement.supports("Module"),
              unsupported: HTMLScriptElement.supports("unsupported"),
              missingThrows: (() => {
                try {
                  HTMLScriptElement.supports();
                  return false;
                } catch (error) {
                  return error instanceof TypeError;
                }
              })()
            });
            })()
            "#,
        )
        .expect("HTMLScriptElement.supports probe should evaluate");

    assert_eq!(
        result,
        r#"{"type":"function","name":"supports","length":1,"descriptor":[true,true,true,true],"keysContainSupports":true,"classic":true,"module":true,"importmap":true,"jsMime":false,"padded":false,"upper":false,"unsupported":false,"missingThrows":true}"#
    );
}
#[test]
fn draggable_uses_html_element_defaults_for_auto_state() {
    let mut vm = new_storage_test_vm("https://draggable-defaults.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const value = (name, draggable, href = false) => {
    const element = document.createElement(name);
    if (draggable !== null) element.setAttribute('draggable', draggable);
    if (href) element.setAttribute('href', 'target');
    return element.draggable;
  };

  const namespacedDraggable = document.createElement('img');
  namespacedDraggable.setAttributeNS('urn:test', 'draggable', 'false');
  const namespacedHref = document.createElement('a');
  namespacedHref.setAttributeNS('urn:test', 'href', 'target');

  return JSON.stringify({
    div: [value('div', null), value('div', 'true'), value('div', 'auto')],
    anchor: [
      value('a', null),
      value('a', null, true),
      value('a', 'false', true),
      value('a', 'AUTO', true),
      value('a', 'invalid', true)
    ],
    image: [
      value('img', null),
      value('img', 'false'),
      value('img', 'FaLsE'),
      value('img', 'falſe'),
      value('img', 'invalid')
    ],
    namespaced: [namespacedDraggable.draggable, namespacedHref.draggable]
  });
})()
"#,
        )
        .expect("draggable default-state probe should evaluate");

    assert_eq!(
        result,
        r#"{"div":[false,true,false],"anchor":[false,true,false,true,true],"image":[true,false,false,true,true],"namespaced":[true,false]}"#
    );
}
#[test]
fn dataset_only_exposes_supported_property_names() {
    let mut vm = new_storage_test_vm("https://dataset-supported-names.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  element.setAttribute('data--foo', 'upper');
  element.setAttribute('data---bar', 'dash-upper');
  const dataset = element.dataset;

  if (dataset.Foo !== 'upper' || dataset['-Bar'] !== 'dash-upper') {
    throw new Error('supported dataset names should reflect their attributes');
  }
  if (dataset['-foo'] !== undefined || '-foo' in dataset ||
      Object.getOwnPropertyDescriptor(dataset, '-foo') !== undefined) {
    throw new Error('invalid alias should not be exposed');
  }
  if (!Object.keys(dataset).includes('-Bar') ||
      Object.getOwnPropertyDescriptor(dataset, '-Bar')?.value !== 'dash-upper') {
    throw new Error('consecutive dashes should produce a supported property');
  }

  if (!delete dataset['-foo'] || element.getAttribute('data--foo') !== 'upper') {
    throw new Error('deleting an invalid alias should not remove the attribute');
  }
  if (!delete dataset['-Bar'] || element.hasAttribute('data---bar')) {
    throw new Error('deleting a supported property should remove the attribute');
  }
  return 'ok';
})()
"#,
        )
        .expect("dataset supported-property-name probe should evaluate");

    assert_eq!(result, "ok");
}
#[test]
fn content_editable_accessors_validate_keywords_and_receivers() {
    let mut vm = new_storage_test_vm("https://content-editable-setter.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  element.contentEditable = 'TRUE';
  const truthy = [element.contentEditable, element.getAttribute('contenteditable')];

  element.contentEditable = { toString() { return 'PLAINTEXT-ONLY'; } };
  const plaintext = [element.contentEditable, element.getAttribute('contenteditable')];

  element.contentEditable = 'INHERIT';
  const inherited = [element.contentEditable, element.getAttribute('contenteditable')];

  element.setAttribute('contenteditable', 'false');
  let invalid;
  try {
    element.contentEditable = 'falſe';
    invalid = 'none';
  } catch (error) {
    invalid = error.name;
  }

  const setter = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    'contentEditable'
  ).set;
  const getter = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    'contentEditable'
  ).get;
  let incompatible;
  try {
    setter.call(document.createElementNS('urn:test', 'div'), 'true');
    incompatible = 'none';
  } catch (error) {
    incompatible = error.name;
  }

  let incompatibleGetter;
  try {
    getter.call(document.createElementNS('urn:test', 'div'));
    incompatibleGetter = 'none';
  } catch (error) {
    incompatibleGetter = error.name;
  }

  const namespaced = document.createElement('div');
  namespaced.setAttributeNS('urn:test', 'contenteditable', 'true');

  return JSON.stringify({
    truthy,
    plaintext,
    inherited,
    invalid: [invalid, element.getAttribute('contenteditable')],
    incompatible: [incompatibleGetter, incompatible],
    namespaced: [namespaced.contentEditable, namespaced.isContentEditable]
  });
})()
"#,
        )
        .expect("contentEditable setter probe should evaluate");

    assert_eq!(
        result,
        r#"{"truthy":["true","true"],"plaintext":["plaintext-only","plaintext-only"],"inherited":["inherit",null],"invalid":["SyntaxError","false"],"incompatible":["TypeError","TypeError"],"namespaced":["inherit",false]}"#
    );
}
#[test]
fn dataset_numeric_names_use_named_property_semantics() {
    let mut vm = new_storage_test_vm("https://dataset-numeric-name.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  const dataset = element.dataset;
  element.setAttribute('data-9', 'initial');
  const initialDescriptor = Object.getOwnPropertyDescriptor(dataset, 9);

  const prototype = Object.getPrototypeOf(dataset);
  let setterCalls = 0;
  Object.defineProperty(prototype, 10, {
    get() { return 'prototype'; },
    set() { setterCalls++; },
    configurable: true
  });
  const inheritedBeforeSet = dataset[10];
  dataset[10] = 'written';
  delete prototype[10];

  return JSON.stringify({
    initialDescriptor,
    keys: Object.keys(dataset),
    inheritedBeforeSet,
    setterCalls,
    written: dataset[10],
    attribute: element.getAttribute('data-10')
  });
})()
"#,
        )
        .expect("dataset numeric-name binding probe should evaluate");

    assert_eq!(
        result,
        r#"{"initialDescriptor":{"value":"initial","writable":true,"enumerable":true,"configurable":true},"keys":["9","10"],"inheritedBeforeSet":"prototype","setterCalls":0,"written":"written","attribute":"written"}"#
    );
}
#[test]
fn autocapitalize_canonicalizes_and_inherits_from_form_owner() {
    let mut vm = new_parsed_test_vm(
        "https://autocapitalize.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const div = document.createElement('div');
  const canonical = [null, '', 'NoNe', 'OFF', 'characters', 'WORDS', 'on', 'invalid']
    .map(value => {
      if (value === null) div.removeAttribute('autocapitalize');
      else div.setAttribute('autocapitalize', value);
      return div.autocapitalize;
    });

  div.autocapitalize = { toString() { return 'ON'; } };
  const setter = [div.autocapitalize, div.getAttribute('autocapitalize')];

  const form = document.createElement('form');
  form.id = 'owner';
  form.setAttribute('autocapitalize', 'WORDS');
  document.body.appendChild(form);
  const inheritedNames = ['button', 'fieldset', 'input', 'output', 'select', 'textarea'];
  const inherited = inheritedNames.map(name => {
    const child = document.createElement(name);
    form.appendChild(child);
    const external = document.createElement(name);
    external.setAttribute('form', 'owner');
    document.body.appendChild(external);
    child.setAttribute('autocapitalize', '');
    external.setAttributeNS('urn:test', 'autocapitalize', 'off');
    const values = [child.autocapitalize, external.autocapitalize];
    child.setAttribute('autocapitalize', 'off');
    values.push(child.autocapitalize);
    return values.join(':');
  });

  const nonInherited = ['img', 'object'].map(name => {
    const element = document.createElement(name);
    form.appendChild(element);
    return element.autocapitalize;
  });

  const descriptor = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    'autocapitalize'
  );
  const incompatible = operation => {
    try {
      operation(document.createElementNS('urn:test', 'div'));
      return 'none';
    } catch (error) {
      return error.name;
    }
  };

  return JSON.stringify({
    descriptor: [descriptor.enumerable, descriptor.configurable],
    canonical,
    setter,
    inherited,
    nonInherited,
    incompatible: [
      incompatible(receiver => descriptor.get.call(receiver)),
      incompatible(receiver => descriptor.set.call(receiver, 'words'))
    ]
  });
})()
"#,
        )
        .expect("autocapitalize semantics should evaluate");

    assert_eq!(
        result,
        r#"{"descriptor":[true,true],"canonical":["","","none","none","characters","words","sentences","sentences"],"setter":["sentences","ON"],"inherited":["words:words:none","words:words:none","words:words:none","words:words:none","words:words:none","words:words:none"],"nonInherited":["",""],"incompatible":["TypeError","TypeError"]}"#
    );
}
#[test]
fn document_element_name_validation_matches_dom_edge_cases() {
    let mut vm = new_storage_test_vm("https://document-element-name-edge-cases.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const XHTML_NS = "http://www.w3.org/1999/xhtml";
  const xml = document.implementation.createDocument(null, "");
  const xhtml = document.implementation.createDocument(XHTML_NS, "html");
  const htmlUpper = document.createElementNS(XHTML_NS, "FOO");
  const xhtmlPlain = xhtml.createElement("f:oo");
  const xhtmlNsNull = xhtml.createElementNS(null, "foo");
  const xmlControl = xml.createElement("A\u0001");
  const attrControl = probe(() => document.createAttribute("\u0001"));
  function probe(callback) {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  }
  return [
    xhtmlPlain.localName,
    xhtmlPlain.namespaceURI,
    xhtmlNsNull.localName,
    xhtmlNsNull.namespaceURI,
    xhtmlNsNull.nodeName,
    xmlControl.localName.charCodeAt(1),
    attrControl,
    htmlUpper instanceof HTMLUnknownElement,
    probe(() => document.createElement("foo/bar")),
    probe(() => document.createElementNS("urn:test", "/:div")),
    probe(() => document.createAttribute("="))
  ].join("|");
})()
"#,
        )
        .expect("document element name edge case probe should evaluate");

    assert_eq!(
        result,
        "f:oo|http://www.w3.org/1999/xhtml|foo||foo|1|ok|true|InvalidCharacterError|InvalidCharacterError|InvalidCharacterError"
    );
}
#[test]
fn processing_instruction_target_validation_matches_xml_names() {
    let mut vm = new_storage_test_vm("https://processing-instruction-names.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const detached = document.implementation.createHTMLDocument("");
  function probe(doc, target, data = "x") {
    try {
      const node = doc.createProcessingInstruction(target, data);
      return `${node.target}:${node.data}`;
    } catch (error) {
      return error && error.name;
    }
  }
  return [
    probe(document, "xml:fail"),
    probe(document, "A\u00b7A"),
    probe(document, "\u00b7A"),
    probe(document, "\u00d7A"),
    probe(document, "A\u00d7"),
    probe(document, "\\A"),
    probe(document, "\f"),
    probe(document, "A", "?>"),
    probe(detached, "a0"),
    probe(detached, "\u00d7A")
  ].join("|");
})()
"#,
        )
        .expect("processing instruction target validation probe should evaluate");

    assert_eq!(
        result,
        "xml:fail:x|A·A:x|InvalidCharacterError|InvalidCharacterError|InvalidCharacterError|InvalidCharacterError|InvalidCharacterError|InvalidCharacterError|a0:x|InvalidCharacterError"
    );
}
#[test]
fn lookup_namespace_uri_handles_non_element_roots_and_attrs() {
    let mut vm = new_storage_test_vm("https://lookup-namespace-uri.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const fragment = document.createDocumentFragment();
  const doctype = document.implementation.createDocumentType("html", "", "");
  const detachedDocument = new Document();
  const disconnectedAttr = document.createAttribute("foo");
  const connectedAttr = document.createAttribute("bar");
  root.setAttributeNode(connectedAttr);
  const value = item => item === null ? "null" : String(item);
  return [
    fragment.lookupNamespaceURI("xml"),
    fragment.lookupNamespaceURI("xmlns"),
    doctype.lookupNamespaceURI("xml"),
    doctype.lookupNamespaceURI("xmlns"),
    typeof detachedDocument.lookupNamespaceURI,
    detachedDocument.lookupNamespaceURI("xml"),
    disconnectedAttr.lookupNamespaceURI("xml"),
    connectedAttr.lookupNamespaceURI("xml"),
    connectedAttr.lookupNamespaceURI("xmlns")
  ].map(value).join("|");
})()
"#,
        )
        .expect("lookupNamespaceURI namespace edge cases should evaluate");

    assert_eq!(
        result,
        "null|null|null|null|function|null|null|http://www.w3.org/XML/1998/namespace|http://www.w3.org/2000/xmlns/"
    );
}
#[test]
fn attr_state_slot_ignores_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://attr-private-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const internalNames = attr => Object.getOwnPropertyNames(attr)
    .filter(name => name.startsWith("__moliAttr"))
    .sort();
  const element = document.createElement("div");
  const other = document.createElement("section");
  element.setAttribute("data-real", "before");
  const live = element.getAttributeNode("data-real");
  const detached = document.createAttributeNS("urn:moli:test", "lm:flag");
  detached.value = "real";
  const internalNamesBefore = {
    live: internalNames(live),
    detached: internalNames(detached)
  };
  live.__moliAttrState = {
    name: "data-spoof",
    value: "spoofed",
    ownerElement: other,
    ownerDocument: document,
    namespaceURI: null,
    prefix: null,
    localName: "data-spoof"
  };
  detached.__moliAttrState = {
    name: "fake:flag",
    value: "spoofed",
    ownerElement: other,
    ownerDocument: document,
    namespaceURI: "urn:fake",
    prefix: "fake",
    localName: "flag"
  };
  live.value = "after";
  const fake = {
    __moliAttrState: {
      name: "fake",
      value: "fake",
      ownerElement: other,
      ownerDocument: document,
      namespaceURI: "urn:fake",
      prefix: "fake",
      localName: "fake"
    }
  };
  return JSON.stringify({
    internalNamesBefore,
    liveName: live.name,
    liveValue: live.value,
    elementValue: element.getAttribute("data-real"),
    spoofedOwnerValue: other.getAttribute("data-spoof"),
    detachedName: detached.name,
    detachedLocalName: detached.localName,
    detachedPrefix: detached.prefix,
    detachedNamespace: detached.namespaceURI,
    detachedValue: detached.value,
    fakeClone: live.cloneNode.call(fake),
    fakeLookupNamespace: live.lookupNamespaceURI.call(fake, "xml"),
    fakeStateValue: fake.__moliAttrState.value
  });
})()
"#,
        )
        .expect("Attr private slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"internalNamesBefore":{"live":[],"detached":[]},"liveName":"data-real","liveValue":"after","elementValue":"after","spoofedOwnerValue":null,"detachedName":"lm:flag","detachedLocalName":"flag","detachedPrefix":"lm","detachedNamespace":"urn:moli:test","detachedValue":"real","fakeClone":null,"fakeLookupNamespace":null,"fakeStateValue":"fake"}"#
    );
}
#[test]
fn closest_invalid_uses_effective_option_value() {
    let mut vm = new_storage_test_vm("https://closest-invalid-select.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const fieldset = document.createElement("fieldset");
  fieldset.id = "fieldset";
  const select = document.createElement("select");
  select.id = "select";
  select.required = true;
  const option = document.createElement("option");
  option.id = "option";
  option.selected = true;
  option.textContent = "non-empty fallback";
  const input = document.createElement("input");
  input.required = true;
  select.append(option);
  fieldset.append(select, input);
  root.append(fieldset);
  return [
    select.matches(":invalid"),
    fieldset.matches(":invalid"),
    option.closest(":invalid").id
  ].join("|");
})()
"#,
        )
        .expect("closest :invalid probe should evaluate");

    assert_eq!(result, "false|true|fieldset");
}
#[test]
fn element_matches_delegates_cross_realm_elements() {
    let mut vm = new_storage_test_vm("https://matches-cross-realm.test/#target");
    let result = vm
        .eval(
            r##"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = body.appendChild(document.createElement("iframe"));
  const doc = frame.contentDocument;
  const frameRoot = doc.documentElement || doc.appendChild(doc.createElement("html"));
  const frameBody = doc.body || frameRoot.appendChild(doc.createElement("body"));
  frameBody.innerHTML = `
    <div id="universal"><address id="address"><code id="code"></code></address></div>
    <p id="nth"><em id="em1"></em><strong></strong><em id="em2"></em><strong></strong><em id="em3"></em></p>
    <fieldset disabled><input id="disabledInput"></fieldset>
    <div id="target"></div>
  `;
  const code = doc.getElementById("code");
  return [
    code.matches("*"),
    code.matches("#universal > * > *"),
    doc.getElementById("em3").matches("#nth em:nth-of-type(3)"),
    doc.getElementById("disabledInput").matches(":disabled"),
    doc.getElementById("target").matches("#target")
  ].join("|");
})()
"##,
        )
        .expect("cross-realm matches probe should evaluate");

    assert_eq!(result, "true|true|true|true|true");
}
#[test]
fn html_rel_list_supported_tokens_are_ascii_case_insensitive_and_owner_specific() {
    let mut vm = new_storage_test_vm("https://rel-list-supports.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const supports = (tag, token) => document.createElement(tag).relList.supports(token);
              return JSON.stringify({
                linkManifest: supports("link", "manifest"),
                linkUppercaseModulepreload: supports("link", "MODULEPRELOAD"),
                linkNoopener: supports("link", "noopener"),
                anchorNoopener: supports("a", "noopener"),
                anchorUppercaseNoreferrer: supports("a", "NOREFERRER"),
                anchorManifest: supports("a", "manifest"),
                areaOpener: supports("area", "opener"),
                formPreload: supports("form", "preload")
              });
            })()
            "#,
        )
        .expect("HTML relList supported-token matrix should evaluate");

    assert_eq!(
        result,
        r#"{"linkManifest":true,"linkUppercaseModulepreload":true,"linkNoopener":false,"anchorNoopener":true,"anchorUppercaseNoreferrer":true,"anchorManifest":false,"areaOpener":true,"formPreload":false}"#
    );
}
#[test]
fn form_control_autocomplete_uses_the_html_autofill_token_parser() {
    let mut vm = new_storage_test_vm("https://autocomplete.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const value = (tag, autocomplete, type = null) => {
                const element = document.createElement(tag);
                if (type !== null) element.type = type;
                if (autocomplete !== null) element.setAttribute("autocomplete", autocomplete);
                return element.autocomplete;
              };
              const inputDescriptor = Object.getOwnPropertyDescriptor(
                HTMLInputElement.prototype,
                "autocomplete"
              );
              const crossReceiver = callback => {
                try {
                  callback();
                  return "returned";
                } catch (error) {
                  return error && error.name;
                }
              };
              const setterInput = document.createElement("input");
              setterInput.autocomplete = " SECTION-LOGIN  shipping work TEL webauthn ";
              return JSON.stringify({
                missing: value("input", null),
                canonicalField: value("input", " NAME\t"),
                contact: value("textarea", "billing  work  email"),
                credential: value("select", "section-LOGIN shipping work tel webauthn"),
                invalid: value("select", "foo section-foo billing name"),
                hiddenOn: value("input", "on", "hidden"),
                rawSetterAttribute: setterInput.getAttribute("autocomplete"),
                canonicalSetterValue: setterInput.autocomplete,
                getterBrand: crossReceiver(() => inputDescriptor.get.call(document.createElement("textarea"))),
                setterBrand: crossReceiver(() => inputDescriptor.set.call(document.createElement("textarea"), "name"))
              });
            })()
            "#,
        )
        .expect("form-control autocomplete parser probe should evaluate");

    assert_eq!(
        result,
        r#"{"missing":"","canonicalField":"name","contact":"billing work email","credential":"section-login shipping work tel webauthn","invalid":"","hiddenOn":"","rawSetterAttribute":" SECTION-LOGIN  shipping work TEL webauthn ","canonicalSetterValue":"section-login shipping work tel webauthn","getterBrand":"TypeError","setterBrand":"TypeError"}"#
    );
}
#[test]
fn class_tokens_use_ascii_html_whitespace_only() {
    let mut vm = new_storage_test_vm("https://class-token-whitespace.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const nbsp = "\u00A0";
              const probe = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const live = document.createElement("div");
              live.className = `alpha${nbsp}beta`;
              const root = document.documentElement || document.appendChild(document.createElement("html"));
              root.append(live);

              const detached = document.implementation.createHTMLDocument("");
              const detachedElement = detached.createElement("div");
              detachedElement.className = nbsp;
              detached.body.append(detachedElement);

              return JSON.stringify({
                liveBeta: document.getElementsByClassName("beta").length,
                liveCombined: document.getElementsByClassName(`alpha${nbsp}beta`).length,
                liveNbsp: document.getElementsByClassName(nbsp).length,
                selectorBeta: document.querySelectorAll(".beta").length,
                classListLength: live.classList.length,
                classListCombined: live.classList.contains(`alpha${nbsp}beta`),
                classListNbspAdd: probe(() => {
                  live.classList.add(nbsp);
                  return live.classList.contains(nbsp);
                }),
                detachedNbsp: detached.getElementsByClassName(nbsp).length
              });
            })()
            "#,
        )
        .expect("class token whitespace probe should evaluate");

    assert_eq!(
        result,
        r#"{"liveBeta":0,"liveCombined":1,"liveNbsp":0,"selectorBeta":0,"classListLength":1,"classListCombined":true,"classListNbspAdd":"true","detachedNbsp":1}"#
    );
}
#[test]
fn document_named_item_does_not_shadow_legacy_unforgeable_document_alias() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                if (!document.documentElement) {
                    const html = document.createElement("html");
                    document.appendChild(html);
                }
                if (!document.body) {
                    const body = document.createElement("body");
                    document.documentElement.appendChild(body);
                }
                const node = document.createElement("div");
                node.id = "document";
                document.body.appendChild(node);
                const desc = Object.getOwnPropertyDescriptor(globalThis, "document");
                return [
                    document === globalThis.document,
                    document.body === globalThis.document.body,
                    document.getElementById("document") === node,
                    desc && desc.configurable === false,
                    desc && desc.enumerable === true,
                    typeof (desc && desc.get),
                    typeof (desc && desc.set)
                ].join("|");
            })()
            "#,
        )
        .expect("document alias should not be shadowed by named items");

    assert_eq!(result, "true|true|true|true|true|function|undefined");
}
#[test]
fn response_csp_sandbox_disables_top_document_scripting_semantics() {
    let mut vm = new_storage_html_test_vm("https://response-sandbox-noscript.test/");
    vm.set_response_content_security_policies(&["sandbox".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  const inner = document.createElement("div");
  inner.innerHTML = "<noscript><span id=inner-fallback></span></noscript>";
  root.appendChild(inner);

  globalThis.__sandboxDynamicScriptRan = false;
  const script = document.createElement("script");
  script.textContent = "globalThis.__sandboxDynamicScriptRan = true";
  root.appendChild(script);

  globalThis.__sandboxHandlerRan = false;
  const button = document.createElement("button");
  button.setAttribute("onclick", "globalThis.__sandboxHandlerRan = true");
  root.appendChild(button);
  button.click();

  const beforeOpen = [
    document.getElementById("inner-fallback") !== null,
    globalThis.__sandboxDynamicScriptRan,
    globalThis.__sandboxHandlerRan
  ];

  document.open();
  document.write(
    "<!doctype html><noscript><main id=document-write-fallback></main></noscript>"
  );
  document.close();
  return beforeOpen.concat(
    document.getElementById("document-write-fallback") !== null
  ).join("|");
})()
"#,
        )
        .expect("response CSP sandbox scripting probe should evaluate");

    assert_eq!(result, "true|false|false|true");
}
#[tokio::test(flavor = "current_thread")]
async fn one_sided_document_domain_disables_original_tuple_origin_fast_path() {
    const HOST: &str = "www.example.test";

    let server = StaticHttpServer::spawn(2).await;
    let parent_url = server.url_for_host(HOST, "/page.html");
    let child_url = server.url_for_host(HOST, "/child.html");
    let loader = static_http_loader([server.resolve_entry(HOST)]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.eval(&format!(
        "globalThis.__oneSidedDomainChildUrl = {};",
        serde_json::to_string(child_url.as_str()).expect("serialize document.domain child URL")
    ))
    .expect("document.domain child URL should install");

    vm.exec(
        r#"
const frame = document.createElement("iframe");
globalThis.__oneSidedDomainChildLoaded = false;
frame.onload = () => { globalThis.__oneSidedDomainChildLoaded = true; };
frame.src = globalThis.__oneSidedDomainChildUrl;
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__oneSidedDomainFrame = frame;
globalThis.__probeOneSidedDomainFrame = () => {
  try {
    return frame.contentWindow.document.domain;
  } catch (error) {
    return error && error.name;
  }
};
"#,
        None,
    )
    .expect("same-origin document.domain frame setup should run");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__oneSidedDomainChildLoaded)",
        "true",
        "same-origin document.domain child should load",
    )
    .await;

    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("same-origin child realm should materialize");
    assert_eq!(
        vm.eval("__probeOneSidedDomainFrame()")
            .expect("initial same-origin access probe should evaluate"),
        "www.example.test"
    );
    assert_eq!(
        vm.eval("document.domain = document.domain; __probeOneSidedDomainFrame()")
            .expect("one-sided exact-domain access probe should evaluate"),
        "SecurityError"
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            r#"
(() => {
  const before = (() => {
    try {
      return parent.document.domain;
    } catch (error) {
      return error && error.name;
    }
  })();
  document.domain = document.domain;
  return [before, parent.document.domain].join("|");
})()
"#,
        )
        .expect("two-sided exact-domain access probe should evaluate"),
        "SecurityError|www.example.test"
    );
    assert_eq!(
        vm.eval("__probeOneSidedDomainFrame()")
            .expect("restored exact-domain access probe should evaluate"),
        "www.example.test"
    );

    vm.exec(
        r#"
const retiredFrame = document.createElement("iframe");
globalThis.__retiredOneSidedDomainChildLoaded = false;
retiredFrame.onload = () => { globalThis.__retiredOneSidedDomainChildLoaded = true; };
retiredFrame.src = globalThis.__oneSidedDomainChildUrl;
(document.body || document.documentElement || document).appendChild(retiredFrame);
globalThis.__retiredOneSidedDomainWindow = retiredFrame.contentWindow;
"#,
        None,
    )
    .expect("one-sided document.domain retired Window setup should run");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__retiredOneSidedDomainChildLoaded)",
        "true",
        "fresh child without document.domain should load",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const probe = () => {
    try {
      return __retiredOneSidedDomainWindow.document.domain;
    } catch (error) {
      return error && error.name;
    }
  };
  const beforeRemoval = probe();
  document.querySelectorAll("iframe")[1].remove();
  return [beforeRemoval, probe()].join("|");
})()
"#,
        )
        .expect("retired one-sided document.domain Window probe should evaluate"),
        "SecurityError|SecurityError"
    );
    assert_eq!(
        server.finish_targets().await,
        vec!["/child.html", "/child.html"]
    );
}

#[test]
fn location_reload_replaces_initial_empty_iframe_document_and_dispatches_load() {
    let mut vm = new_storage_test_vm("https://initial-empty-reload.test/page.html");

    vm.exec(
        r#"
const frame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__initialReloadFrame = frame;
globalThis.__initialReloadDocument = frame.contentDocument;
globalThis.__initialReloadLoads = 0;
frame.onload = () => ++__initialReloadLoads;
frame.contentWindow.location.reload();
"#,
        None,
    )
    .expect("initial-empty iframe reload should evaluate");
    assert!(
        vm.has_pending_child_navigation_commit_for_test(),
        "reloading an ordinary initial about:blank iframe must queue a navigation"
    );

    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval(
            r#"[
  __initialReloadFrame.contentDocument !== __initialReloadDocument,
  __initialReloadLoads,
  __initialReloadFrame.contentWindow.location.href
].join('|')"#,
        )
        .expect("initial-empty iframe reload result should evaluate"),
        "true|1|about:blank"
    );
}
