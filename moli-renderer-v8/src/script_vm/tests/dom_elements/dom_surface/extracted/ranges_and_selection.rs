use super::*;

#[test]
fn stable_both_edges_preserve_numeric_layout_and_scroll_ranges() {
    let mut vm = new_storage_test_vm("https://both-edges-numeric-layout.test/");
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
          document.body.innerHTML = ["block", "flex", "grid"].map(display => `
            <div id="${display}" style="display:${display};width:200px;height:100px;overflow:scroll;scrollbar-gutter:stable both-edges">
              <div style="width:100%;height:100%"></div>
            </div>`).join("") + `
            <div id="overflow" style="width:200px;height:100px;overflow:scroll;scrollbar-gutter:stable both-edges">
              <div style="width:400px;height:200px"></div>
            </div>`;
        })()
        "#,
    )
    .expect("both-edge numeric layout fixture should initialize");
    publish_layout_for_test(&mut vm);

    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify(["block", "flex", "grid", "overflow"].map(id => {
              const scroller = document.getElementById(id);
              const child = scroller.firstElementChild;
              return [
                scroller.clientWidth,
                scroller.clientHeight,
                scroller.scrollWidth,
                scroller.scrollHeight,
                child.offsetWidth,
                child.offsetHeight,
              ];
            }))
            "#,
        )
        .expect("both-edge numeric layout metrics should evaluate"),
        "[[170,85,170,85,170,85],[170,85,170,85,170,85],[170,85,170,85,170,85],[170,85,400,200,400,200]]"
    );
}
#[test]
fn set_range_text_validates_range_before_clamping_offsets() {
    let mut vm = new_storage_test_vm("https://forms-selection-set-range-text-order.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const failures = [];
              const cases = [
                ['abc', 100, 99, 'IndexSizeError', 'abc'],
                ['abc', 4, 3, 'IndexSizeError', 'abc'],
                ['abc', 3, 2, 'IndexSizeError', 'abc'],
                ['', 1, 0, 'IndexSizeError', ''],
                ['abc', -1, -2, 'IndexSizeError', 'abc'],
                ['abc', 99, 100, null, 'abcY'],
                ['abc', 100, 100, null, 'abcY'],
                ['abc', 1, 100, null, 'aY'],
                ['abc', 2 ** 32, 1, null, 'Ybc'],
                ['', 0, 0, null, 'Y']
              ];
              for (const type of ['text', 'search', 'tel', 'url', 'password', 'textarea']) {
                for (const connected of [false, true]) {
                  const field = document.createElement(type === 'textarea' ? 'textarea' : 'input');
                  if (type !== 'textarea') field.type = type;
                  if (connected) {
                    (document.body || document.documentElement || document).appendChild(field);
                  }
                  for (const [value, start, end, expectedError, expectedValue] of cases) {
                    field.value = value;
                    field.setSelectionRange(1, 2, 'backward');
                    const selection = () => JSON.stringify([
                      field.selectionStart, field.selectionEnd, field.selectionDirection
                    ]);
                    const beforeSelection = selection();
                    let errorName = null;
                    let isIndexSizeError = false;
                    try {
                      field.setRangeText('Y', start, end);
                    } catch (error) {
                      errorName = error.name;
                      isIndexSizeError = error instanceof DOMException && error.code === 1;
                    }
                    if (errorName !== expectedError || field.value !== expectedValue ||
                        (expectedError && (!isIndexSizeError || selection() !== beforeSelection))) {
                      failures.push({
                        type, connected, value, start, end, expectedError, expectedValue,
                        errorName, actualValue: field.value, isIndexSizeError,
                        beforeSelection, afterSelection: selection()
                      });
                    }
                  }
                  field.remove();
                }
              }
              return JSON.stringify(failures);
            })()
            "#,
        )
        .expect("setRangeText range validation and clamping should evaluate");

    assert_eq!(result, "[]");
}
#[test]
fn set_range_text_preserve_mode_adjusts_selection_by_replacement_delta() {
    let mut vm = new_storage_test_vm("https://forms-selection-set-range-text.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const input = document.createElement('input');
              input.type = 'text';

              input.value = 'hello world';
              input.setSelectionRange(6, 11);
              input.setRangeText('xy', 0, 5, 'preserve');
              const afterPrefixReplacement = [
                input.value,
                input.selectionStart,
                input.selectionEnd
              ].join(':');

              input.value = 'abcdef';
              input.setSelectionRange(1, 3);
              input.setRangeText('Z', 2, 4, 'preserve');
              const afterOverlapReplacement = [
                input.value,
                input.selectionStart,
                input.selectionEnd
              ].join(':');

              return [afterPrefixReplacement, afterOverlapReplacement].join('|');
            })()
            "#,
        )
        .expect("setRangeText preserve mode should adjust selection positions");

    assert_eq!(result, "xy world:3:8|abZef:1:3");
}
#[test]
fn selection_collapses_to_removed_editing_host_position() {
    let mut vm = new_storage_test_vm("https://selection-editing-host-removal.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const container = document.createElement("div");
  (document.body || document.documentElement || document).appendChild(container);

  function probe(action) {
    container.textContent = "";
    const editingHost = document.createElement("div");
    editingHost.contentEditable = true;
    editingHost.innerHTML = "ABC<br>";
    const wrapper = action === "ancestor-remove" ? document.createElement("div") : null;
    if (wrapper) {
      wrapper.appendChild(editingHost);
      container.appendChild(wrapper);
    } else {
      container.appendChild(editingHost);
    }
    editingHost.focus();
    const selection = getSelection();
    selection.collapse(editingHost, 0);
    if (action === "remove") {
      editingHost.remove();
    } else if (action === "ancestor-remove") {
      wrapper.remove();
    } else if (action === "replace-with-self") {
      editingHost.replaceWith(editingHost);
    } else {
      container.replaceChild(editingHost, editingHost);
    }
    const range = selection.getRangeAt(0);
    return [
      selection.anchorNode === container,
      selection.anchorOffset,
      selection.focusNode === container,
      selection.focusOffset,
      range.startContainer === container,
      range.startOffset,
      range.endContainer === container,
      range.endOffset
    ].join(":");
  }

  return [
    probe("remove"),
    probe("ancestor-remove"),
    probe("replace-with-self"),
    probe("replace-child-self")
  ].join("|");
})()
"##,
        )
        .expect("editing-host removal selection probe should evaluate");

    assert_eq!(
        result,
        "true:0:true:0:true:0:true:0|true:0:true:0:true:0:true:0|true:0:true:0:true:0:true:0|true:0:true:0:true:0:true:0"
    );
}
#[test]
fn selection_collapse_focuses_contenteditable_editing_host() {
    let mut vm = new_storage_test_vm("https://selection-contenteditable-collapse.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const button = document.createElement("button");
  const host1 = document.createElement("div");
  const host2 = document.createElement("div");
  const host3 = document.createElement("div");
  const nonEditable = document.createElement("div");
  const host4 = document.createElement("div");
  const container = document.createElement("div");
  host1.contentEditable = "true";
  host2.contentEditable = "true";
  host3.contentEditable = "true";
  nonEditable.contentEditable = "false";
  host4.contentEditable = "true";
  nonEditable.appendChild(host4);
  host3.appendChild(nonEditable);
  container.append(button, host1, host2, host3);
  (document.body || document.documentElement || document).appendChild(container);

  function clearFocus() {
    button.focus();
  }

  function probe(first, second) {
    clearFocus();
    const selection = getSelection();
    selection.collapse(first, 0);
    const firstActive = document.activeElement;
    selection.collapse(second, 0);
    return [
      firstActive === first,
      document.activeElement === second,
      selection.anchorNode === second,
      selection.anchorOffset,
      selection.focusNode === second,
      selection.focusOffset
    ].join(":");
  }

  return [
    probe(host1, host2),
    probe(host4, host3),
    probe(host3, host4)
  ].join("|");
})()
"##,
        )
        .expect("contenteditable collapse focus probe should evaluate");

    assert_eq!(
        result,
        "true:true:true:0:true:0|true:true:true:0:true:0|true:true:true:0:true:0"
    );
}
#[test]
fn selection_modify_extend_word_updates_focus_and_associated_range() {
    let mut vm = new_storage_test_vm("https://selection-modify-extend-word.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const middleLine = document.createElement("p");
  middleLine.textContent = "middle line";
  (document.body || document.documentElement || document).appendChild(middleLine);
  const text = middleLine.firstChild;
  const selection = getSelection();

  selection.collapse(text, "mid".length);
  selection.modify("extend", "backward", "word");
  const backward = selection.getRangeAt(0);
  const backwardResult = [
    selection.anchorNode === text,
    selection.anchorOffset,
    selection.focusNode === text,
    selection.focusOffset,
    backward.startContainer === text,
    backward.startOffset,
    backward.endContainer === text,
    backward.endOffset
  ].join(":");

  selection.collapse(text, "middle li".length);
  selection.modify("extend", "forward", "word");
  const forward = selection.getRangeAt(0);
  const forwardResult = [
    selection.anchorNode === text,
    selection.anchorOffset,
    selection.focusNode === text,
    selection.focusOffset,
    forward.startContainer === text,
    forward.startOffset,
    forward.endContainer === text,
    forward.endOffset
  ].join(":");

  return `${backwardResult}|${forwardResult}`;
})()
"##,
        )
        .expect("Selection.modify extend word probe should evaluate");

    assert_eq!(
        result,
        "true:3:true:0:true:0:true:3|true:9:true:11:true:9:true:11"
    );
}
#[test]
fn selection_modify_move_line_and_paragraph_collapses_selection_to_adjacent_text() {
    let mut vm = new_storage_test_vm("https://selection-modify-line-paragraph.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement("div");
  host.contentEditable = "true";
  host.innerHTML = "text1<br>text2";
  (document.body || document.documentElement || document).appendChild(host);
  host.focus();

  const first = host.childNodes[0];
  const second = host.childNodes[2];
  const selection = getSelection();

  function label(node) {
    if (node === first) return "first";
    if (node === second) return "second";
    return "other";
  }

  function probe(start, anchorOffset, focusOffset, direction, granularity) {
    selection.setBaseAndExtent(start, anchorOffset, start, focusOffset);
    selection.modify("move", direction, granularity);
    return [
      direction,
      granularity,
      selection.isCollapsed,
      label(selection.focusNode),
      selection.focusOffset,
      label(selection.anchorNode),
      selection.anchorOffset
    ].join(":");
  }

  return [
    probe(second, 0, 5, "backward", "line"),
    probe(second, 5, 0, "backward", "paragraph"),
    probe(first, 0, 5, "forward", "line"),
    probe(first, 5, 0, "forward", "paragraph")
  ].join("|");
})()
"#,
        )
        .expect("Selection.modify line/paragraph move probe should evaluate");

    assert_eq!(
        result,
        "backward:line:true:first:5:first:5|backward:paragraph:true:first:0:first:0|forward:line:true:second:5:second:5|forward:paragraph:true:second:0:second:0"
    );
}
#[test]
fn selection_modify_skips_contenteditable_false_islands() {
    let mut vm = new_storage_test_vm("https://selection-modify-non-editable.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement("div");
  host.contentEditable = "true";
  host.innerHTML =
    " <span contenteditable=false>non-editable</span>editable<span contenteditable=false>non-editable</span> ";
  (document.body || document.documentElement || document).appendChild(host);
  host.focus();

  const preceding = host.firstChild;
  const middle = host.querySelector("span").nextSibling;
  const trailing = host.lastChild;
  const selection = getSelection();

  function label(node) {
    if (node === preceding) return "preceding";
    if (node === middle) return "middle";
    if (node === trailing) return "trailing";
    if (node === host) return "host";
    return "other";
  }

  function probe(node, offset, direction, selectAllFirst) {
    if (selectAllFirst) {
      selection.selectAllChildren(host);
    }
    selection.collapse(node, offset);
    selection.modify("move", direction, "character");
    const range = selection.getRangeAt(0);
    return [
      direction,
      selectAllFirst ? "after-selectAll" : "direct",
      label(range.startContainer),
      range.startOffset,
      label(range.endContainer),
      range.endOffset
    ].join(":");
  }

  const out = [];
  for (const selectAllFirst of [false, true]) {
    for (const direction of ["forward", "right"]) {
      out.push(probe(preceding, preceding.length, direction, selectAllFirst));
      out.push(probe(middle, middle.length, direction, selectAllFirst));
    }
    for (const direction of ["backward", "left"]) {
      out.push(probe(middle, 0, direction, selectAllFirst));
      out.push(probe(trailing, 0, direction, selectAllFirst));
    }
  }
  return out.join("|");
})()
"#,
        )
        .expect("Selection.modify non-editable island probe should evaluate");

    assert_eq!(
        result,
        "forward:direct:middle:0:middle:0|forward:direct:trailing:0:trailing:0|right:direct:middle:0:middle:0|right:direct:trailing:0:trailing:0|backward:direct:preceding:1:preceding:1|backward:direct:middle:8:middle:8|left:direct:preceding:1:preceding:1|left:direct:middle:8:middle:8|forward:after-selectAll:middle:0:middle:0|forward:after-selectAll:trailing:0:trailing:0|right:after-selectAll:middle:0:middle:0|right:after-selectAll:trailing:0:trailing:0|backward:after-selectAll:preceding:1:preceding:1|backward:after-selectAll:middle:8:middle:8|left:after-selectAll:preceding:1:preceding:1|left:after-selectAll:middle:8:middle:8"
    );
}
#[test]
fn exec_command_delete_preserves_inert_selection_endpoint_contents() {
    let mut vm = new_storage_test_vm("https://exec-command-delete-inert-selection.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const host = document.createElement("div");
  host.contentEditable = "true";
  (document.body || document.documentElement || document).appendChild(host);
  const selection = getSelection();

  function run(html, anchorSelector, anchorOffset, focusSelector, focusOffset) {
    host.innerHTML = html;
    host.focus();
    const initial = host.innerHTML;
    const anchor = host.querySelector(anchorSelector).firstChild;
    const focus = host.querySelector(focusSelector).firstChild;
    selection.setBaseAndExtent(anchor, anchorOffset, focus, focusOffset);
    document.execCommand("delete");
    return `${initial}=>${host.innerHTML}`;
  }

  const noDelete = run(
    '<span id="a" inert>abc</span><span id="b">def</span>',
    "#a",
    1,
    "#b",
    2
  );
  const keepFocusInert = run(
    '<span id="a">abc</span><span id="b" inert>def</span>',
    "#a",
    1,
    "#b",
    2
  );
  const keepReverseFocusInert = run(
    '<span id="a" inert>def</span><span id="b">abc</span>',
    "#b",
    2,
    "#a",
    1
  );

  host.innerHTML = 'a<span id="inert" inert>XYZ</span>f';
  const before = host.firstChild;
  const after = host.lastChild;
  selection.setBaseAndExtent(before, 1, after, 0);
  document.execCommand("delete");
  const deleteContained = host.innerHTML;

  return [
    noDelete,
    host.querySelector("#inert") === null && deleteContained === "af",
    /<span id="b" inert="">def<\/span>/.test(keepFocusInert),
    /<span id="a" inert="">def<\/span>/.test(keepReverseFocusInert)
  ].join("|");
})()
"##,
        )
        .expect("execCommand delete inert selection probe should evaluate");

    assert_eq!(
        result,
        r#"<span id="a" inert="">abc</span><span id="b">def</span>=><span id="a" inert="">abc</span><span id="b">def</span>|true|true|true"#
    );
}
#[test]
fn exec_command_insert_text_edits_focused_text_controls_and_contenteditable() {
    let mut vm = new_storage_test_vm("https://exec-command-insert-text.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const elements = [
    document.createElement("input"),
    document.createElement("textarea"),
    document.createElement("div")
  ];
  elements[2].contentEditable = "true";
  body.append(...elements);

  return elements.map(element => {
    const events = [];
    element.addEventListener("textInput", () => events.push("textInput"));
    element.addEventListener("input", () => events.push("input"));
    element.focus();
    const returned = document.execCommand("insertText", false, "a");
    const value = "value" in element ? element.value : element.textContent;
    return [element.localName, returned, value, events.join(",")].join(":");
  }).join("|");
})()
"#,
        )
        .expect("execCommand insertText probe should evaluate");

    assert_eq!(
        result,
        "input:true:a:input|textarea:true:a:input|div:true:a:input"
    );
}
#[test]
fn selection_to_string_uses_rendered_native_range_projection() {
    let mut vm = new_storage_test_vm("https://selection-rendered-string.test/");

    vm
        .eval(
            r##"
(() => {
  const root = document.createElement("div");
  (document.body || document.documentElement || document).appendChild(root);
  const selection = getSelection();

  function selectedStringFor(range) {
    selection.removeAllRanges();
    selection.addRange(range);
    return selection.toString();
  }

  const p = document.createElement("div");
  p.append("\n");
  const hiddenStyle = document.createElement("style");
  hiddenStyle.style.display = "none";
  hiddenStyle.textContent = "hidden";
  const visibleStyle = document.createElement("style");
  visibleStyle.style.display = "block";
  visibleStyle.textContent = "style   text\nline";
  const hiddenScript = document.createElement("script");
  hiddenScript.textContent = "hiddenScript()";
  const visibleScript = document.createElement("script");
  visibleScript.style.display = "block";
  visibleScript.textContent = "function x() {\n  return 1;\n}";
  const pre = document.createElement("pre");
  pre.textContent = "PASS";
  p.append(hiddenStyle, visibleStyle, "\n", hiddenScript, "\n", visibleScript, "\n", pre);
  root.appendChild(p);

  const table = document.createElement("table");
  table.innerHTML = "<tr><td id=left>Foo</td><td id=right>Hello</td></tr>";
  root.appendChild(table);
  const rangeFromElementEnd = document.createRange();
  rangeFromElementEnd.setStart(table.querySelector("#left"), 1);
  rangeFromElementEnd.setEnd(table.querySelector("#right").firstChild, 4);

  const basic = document.createElement("div");
  basic.innerHTML = "\n  a<span style='user-select: none;'>b</span>c\n";
  const nested = document.createElement("div");
  nested.innerHTML = "\n  start <span style='user-select: none;'>unselectable <strong>nested</strong> text</span> end\n";
  const container = document.createElement("div");
  container.style.userSelect = "none";
  container.innerHTML = "<span style='user-select: text;'>selectable</span> unselectable <span style='user-select: text;'>text</span>";
  const contentHidden = document.createElement("div");
  contentHidden.setAttribute("style", "content-visibility: hidden");
  contentHidden.textContent = "hidden content";
  const inlineWhitespace = document.createElement("div");
  inlineWhitespace.append("alpha\n  ", Object.assign(document.createElement("span"), {
    textContent: "\n beta\n"
  }), "\n gamma");
  root.append(basic, nested, container, contentHidden, inlineWhitespace);
globalThis.__readFixture = () => {
  const scriptStyleRange = document.createRange();
  scriptStyleRange.selectNode(p);
  const scriptStyle = selectedStringFor(scriptStyleRange).replace(/\r\n/g, "\n");
  const fromElementEnd = selectedStringFor(rangeFromElementEnd).trim();
  function selectContents(node) {
    const range = document.createRange();
    range.selectNodeContents(node);
    return selectedStringFor(range);
  }

  return [
    scriptStyle,
    fromElementEnd,
    selectContents(basic),
    selectContents(nested),
    selectContents(container),
    selectContents(contentHidden),
    selectContents(inlineWhitespace)
  ].join("|");
};
})()
"##,
        )
        .expect("selection rendered string probe should evaluate");
    vm.publish_layout_for_test()
        .expect("publish prepared fixture");
    let result = vm
        .eval("__readFixture()")
        .expect("selection rendered string probe should evaluate");

    assert_eq!(
        result,
        "\nstyle text line\nfunction x() { return 1; }\n\nPASS|Hell|ac|start  end|selectabletext||alpha beta gamma"
    );
}
#[test]
fn selection_only_applies_inert_attribute_to_html_elements() {
    let mut vm = new_storage_test_vm("https://selection-html-inert-namespace.test/");

    vm.eval(
        r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const root = document.createElement('div');
  body.appendChild(root);
  const selection = getSelection();
  const mathml = 'http://www.w3.org/1998/Math/MathML';

  const selectedText = element => {
    selection.removeAllRanges();
    selection.selectAllChildren(element);
    return selection.toString();
  };
  const mathWithText = text => {
    const math = document.createElementNS(mathml, 'math');
    const mi = document.createElementNS(mathml, 'mi');
    mi.textContent = text;
    math.appendChild(mi);
    return { math, mi };
  };

  const own = mathWithText('math own');
  own.math.setAttribute('inert', '');
  own.mi.setAttribute('inert', '');
  root.appendChild(own.math);

  const nested = mathWithText('math ancestors');
  nested.math.setAttribute('inert', '');
  nested.mi.setAttribute('inert', '');
  root.appendChild(nested.math);

  const htmlChild = document.createElement('span');
  htmlChild.textContent = 'html child';
  htmlChild.inert = true;
  root.appendChild(htmlChild);

  const htmlAncestor = document.createElement('div');
  htmlAncestor.inert = true;
  const inherited = mathWithText('html ancestor');
  htmlAncestor.appendChild(inherited.math);
  root.appendChild(htmlAncestor);

globalThis.__readFixture = () => {
  return [
    selectedText(own.math),
    selectedText(nested.math),
    selectedText(htmlChild),
    selectedText(htmlAncestor)
  ].join('|');
};
})()
"#,
    )
    .expect("Selection inert namespace probe should evaluate");
    vm.publish_layout_for_test()
        .expect("publish prepared fixture");
    let result = vm
        .eval("__readFixture()")
        .expect("Selection inert namespace probe should evaluate");

    assert_eq!(result, "math own|math ancestors||");
}
#[test]
fn month_and_week_inputs_do_not_support_variable_length_selection() {
    let mut vm = new_storage_test_vm("https://forms-selection-temporal.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              function probe(callback) {
                try {
                  return String(callback());
                } catch (error) {
                  return 'throw:' + error.name;
                }
              }
              function summarize(type) {
                const input = document.createElement('input');
                input.type = type;
                input.value = type === 'month' ? '2026-05' : '2026-W20';
                const select = probe(() => input.select());
                const reads = [
                  input.selectionStart === null,
                  input.selectionEnd === null,
                  input.selectionDirection === null
                ].join(',');
                const writes = [
                  probe(() => { input.selectionStart = 0; }),
                  probe(() => { input.selectionEnd = 0; }),
                  probe(() => { input.selectionDirection = 'forward'; }),
                  probe(() => input.setSelectionRange(0, 0)),
                  probe(() => input.setRangeText('', 0, 0))
                ].join(',');
                return `${type}:${select}:${reads}:${writes}`;
              }
              return [summarize('month'), summarize('week')].join('|');
            })()
            "#,
        )
        .expect("temporal input selection APIs should match HTML selection applicability");

    assert_eq!(
        result,
        "month:undefined:true,true,true:throw:InvalidStateError,throw:InvalidStateError,throw:InvalidStateError,throw:InvalidStateError,throw:InvalidStateError|week:undefined:true,true,true:throw:InvalidStateError,throw:InvalidStateError,throw:InvalidStateError,throw:InvalidStateError,throw:InvalidStateError"
    );
}
#[test]
fn range_detach_is_a_no_op() {
    let mut vm = new_storage_test_vm("https://range-detach.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const r = document.createRange();
  if (typeof r.detach !== 'function') return 'no-detach';
  r.detach();
  return [
    r.startContainer === document,
    r.endContainer === document,
    r.startOffset,
    r.endOffset,
    r.collapsed,
  ].join(',');
})()
"#,
        )
        .expect("range detach should evaluate");
    assert_eq!(result, "true,true,0,0,true");
}
#[test]
fn range_point_comparison_uses_tree_root_not_owner_document() {
    let mut vm = new_storage_test_vm("https://range-native-root-comparison.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}:${error && error.code}`;
    }
  };

  const left = document.createElement("div");
  const leftText = document.createTextNode("left");
  left.appendChild(leftText);
  const right = document.createElement("div");
  const rightText = document.createTextNode("right");
  right.appendChild(rightText);

  const leftRange = document.createRange();
  leftRange.setStart(leftText, 0);
  leftRange.setEnd(leftText, 2);
  const rightRange = document.createRange();
  rightRange.setStart(rightText, 0);
  rightRange.setEnd(rightText, 2);

  const sameRootRange = document.createRange();
  sameRootRange.setStart(left, 0);
  sameRootRange.setEnd(left, 1);

  const foreignDoc = document.implementation.createHTMLDocument("");
  const foreignText = foreignDoc.createTextNode("foreign");
  foreignDoc.body.appendChild(foreignText);
  const foreignRange = foreignDoc.createRange();
  foreignRange.setStart(foreignText, 1);
  foreignRange.setEnd(foreignText, 4);

  const doctype = document.implementation.createDocumentType("root", "", "");
  const doctypeDoc = document.implementation.createDocument(null, "root", doctype);
  const doctypeRange = doctypeDoc.createRange();
  doctypeRange.setStart(doctypeDoc, 0);
  doctypeRange.setEnd(doctypeDoc, 1);

  return [
    probe(() => leftRange.compareBoundaryPoints(Range.START_TO_START, rightRange)),
    leftRange.isPointInRange(rightText, 0),
    probe(() => leftRange.comparePoint(rightText, 0)),
    probe(() => sameRootRange.compareBoundaryPoints(Range.START_TO_END, leftRange)),
    sameRootRange.isPointInRange(leftText, 1),
    probe(() => sameRootRange.comparePoint(leftText, 1)),
    foreignRange.isPointInRange(foreignText, 2),
    probe(() => foreignRange.comparePoint(foreignText, 0)),
    probe(() => foreignRange.comparePoint(foreignText, 5)),
    probe(() => doctypeRange.isPointInRange(doctype, 0))
  ].join("|");
})()
"#,
        )
        .expect("Range native root comparison probe should evaluate");

    assert_eq!(
        result,
        "throw:WrongDocumentError:4|false|throw:WrongDocumentError:4|1|true|0|true|-1|1|throw:InvalidNodeTypeError:24"
    );
}
#[test]
fn cdata_split_text_preserves_cdata_node_type_and_updates_ranges() {
    let mut vm = new_storage_test_vm("https://cdata-split-text.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const xml = document.implementation.createDocument(null, "root");
              const cdata = xml.createCDATASection("abcd");
              xml.documentElement.appendChild(cdata);
              const range = xml.createRange();
              range.setStart(cdata, 3);
              range.setEnd(cdata, 4);
              const right = cdata.splitText(2);
              return [
                cdata.data,
                right.data,
                right.nodeType,
                right instanceof CDATASection,
                right instanceof Text,
                xml.documentElement.childNodes.length,
                range.startContainer === right,
                range.startOffset,
                range.endContainer === right,
                range.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("CDATA splitText regression probe should evaluate");

    assert_eq!(result, "ab|cd|4|true|true|2|true|1|true|2");
}
