use super::*;

#[test]
fn css_variable_specified_values_preserve_cssom_shorthand_boundaries() {
    let mut vm = new_storage_test_vm("https://css-var-cssom-specified.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(target);
  const style = target.style;

  style.cssText = 'margin: var(--prop);';
  const simple = [
    style.cssText,
    style.getPropertyValue('margin'),
    style.getPropertyValue('margin-top')
  ].join(',');

  style.cssText = 'margin: var(--prop); margin-top: 10px';
  const overridden = [
    style.cssText,
    style.getPropertyValue('margin'),
    style.getPropertyValue('margin-left'),
    style.getPropertyValue('margin-top')
  ].join(',');

  style.cssText = 'margin: var(--prop) !important; margin-top: 10px';
  const important = [
    style.getPropertyValue('margin'),
    style.getPropertyValue('margin-top')
  ].join(',');

  style.cssText = 'width: var(--x ()); expando: var(--prop); color: /* drop */ var(--prop)  /* keep */ var(--prop);';
  const validation = [
    style.getPropertyValue('width'),
    style.getPropertyValue('expando'),
    style.getPropertyValue('color'),
    style.cssText
  ].join(',');

  const sheetStyle = document.createElement('style');
  sheetStyle.textContent = 'div { width: var(--open';
  (document.head || document.documentElement || document).appendChild(sheetStyle);
  const leftOpen = sheetStyle.sheet.cssRules[0].style.getPropertyValue('width');

  const borderTarget = document.createElement('div');
  borderTarget.style.cssText = 'border-style: dashed; --border1: 5px solid rgb(0, 0, 0); --border2: 3px dotted red; --width: 1px; border-left: var(--border1); border-width: var(--width);';
  borderTarget.style.borderLeft = 'var(--border2)';
  (document.body || document.documentElement || document).appendChild(borderTarget);
  const computed = getComputedStyle(borderTarget);
  const borderProjection = [
    computed.getPropertyValue('border-left-width'),
    computed.getPropertyValue('border-top-width'),
    computed.getPropertyValue('border-right-width'),
    computed.getPropertyValue('border-bottom-width')
  ].join(',');

  return [simple, overridden, important, validation, leftOpen, borderProjection].join('|');
})()
"#,
        )
        .expect("CSS variable specified CSSOM probe should evaluate");

    assert_eq!(
        result,
        "margin: var(--prop);,var(--prop),|margin-right: ; margin-bottom: ; margin-left: ; margin-top: 10px;,,,10px|var(--prop),|,,var(--prop)  /* keep */ var(--prop),color: var(--prop)  /* keep */ var(--prop);|var(--open|3px,1px,1px,1px"
    );
}
#[test]
fn custom_property_empty_values_and_invalid_cssom_names_match_cssom() {
    let mut vm = new_storage_test_vm("https://css-custom-property-empty-values.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const parent = document.createElement('div');
  parent.style.cssText = '--empty:; --space:  ; --overwrite:value;--overwrite:;';
  const child = document.createElement('div');
  parent.append(child);
  root.append(parent);

  const invalid = document.createElement('div');
  invalid.style.setProperty('--bad ', 'green');
  invalid.style.setProperty('--bad name', 'green');
  invalid.style.setProperty('--ok', 'green');
  root.append(invalid);

  const cssom = document.createElement('div');
  cssom.style.setProperty('--blank', '  ', 'important');
  const blankBeforeRemove = [
    cssom.style.length,
    cssom.style[0],
    cssom.style.getPropertyValue('--blank'),
    cssom.style.getPropertyPriority('--blank'),
    cssom.style.cssText
  ].join(',');
  cssom.style.setProperty('--blank', '');
  const blankAfterRemove = [
    cssom.style.length,
    cssom.style.getPropertyValue('--blank'),
    cssom.style.cssText
  ].join(',');

  const parentComputed = getComputedStyle(parent);
  const childComputed = getComputedStyle(child);
  const invalidComputed = getComputedStyle(invalid);
  const values = [
    parent.style.getPropertyValue('--empty'),
    parent.style.getPropertyValue('--space'),
    parent.style.getPropertyValue('--overwrite'),
    parentComputed.getPropertyValue('--empty'),
    parentComputed.getPropertyValue('--space'),
    parentComputed.getPropertyValue('--overwrite'),
    childComputed.getPropertyValue('--empty'),
    childComputed.getPropertyValue('--space'),
    childComputed.getPropertyValue('--overwrite'),
    invalid.style.getPropertyValue('--bad '),
    invalidComputed.getPropertyValue('--bad '),
    invalidComputed.getPropertyValue('--bad'),
    invalidComputed.getPropertyValue('--bad name'),
    invalidComputed.getPropertyValue('--ok'),
    invalidComputed.getPropertyValue('--ok '),
    blankBeforeRemove,
    blankAfterRemove
  ];
  parent.remove();
  invalid.remove();
  return values.map(value => JSON.stringify(value)).join('|');
})()
"#,
        )
        .expect("custom property empty values should evaluate");

    assert_eq!(
        result,
        r#"" "|" "|" "|" "|" "|" "|" "|" "|" "|""|""|""|""|"green"|""|"1,--blank, ,important,--blank:  !important;"|"0,,""#
    );
}
#[test]
fn custom_property_cycles_ignore_unused_fallback_references() {
    let mut vm = new_storage_test_vm("https://css-custom-property-unused-fallback-cycle.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const element = document.createElement('div');
  element.style.cssText = [
    '--x:var(--a, valid)',
    '--a:var(--y, var(--b, cycle))',
    '--b:var(--y, var(--c, cycle))',
    '--c:var(--y, var(--a, cycle))',
    '--y:valid'
  ].join(';');
  root.append(element);
  const style = getComputedStyle(element);
  const values = ['--a', '--b', '--c', '--x', '--y'].map(name => style.getPropertyValue(name));
  element.remove();
  return values.join('|');
})()
"#,
        )
        .expect("unused fallback cycle should evaluate");

    assert_eq!(result, "valid|valid|valid|valid|valid");
}
#[test]
fn custom_property_cycle_uses_nested_fallback_default() {
    let mut vm = new_storage_test_vm("https://css-custom-property-nested-fallback-cycle.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const element = document.createElement('div');
  element.style.cssText = [
    '--varA: var(--varB)',
    '--varB: var(--varA) var(--varDoesNotExist, var(--varC))',
    '--varC: var(--varB, 13px)'
  ].join(';');
  root.append(element);
  const style = getComputedStyle(element);
  const values = ['--varA', '--varB', '--varC'].map(name => style.getPropertyValue(name));
  element.remove();
  return values.join('|');
})()
"#,
        )
        .expect("nested fallback cycle should evaluate");

    assert_eq!(result, "||13px");
}
#[test]
fn custom_property_var_ident_function_resolves_and_falls_back() {
    let mut vm = new_storage_test_vm("https://css-custom-property-var-ident.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const element = document.createElement('div');
  element.style.cssText = [
    '--myprop3: PASS',
    '--var-with-ident-fn: FAIL1',
    '--var-with-ident-fn: var(ident("--myprop" calc(3 * sign(1em - 1px))), FAIL2)',
    '--nodash: var(ident("nodash"))',
    '--nodash-fallback: var(ident("nodash"), PASS)',
    '--nodash-fallback-inherit: var(ident("nodash"), inherit)'
  ].join(';');
  root.append(element);
  const computed = getComputedStyle(element);
  const values = [
    computed.getPropertyValue('--var-with-ident-fn'),
    computed.getPropertyValue('--nodash'),
    computed.getPropertyValue('--nodash-fallback'),
    computed.getPropertyValue('--nodash-fallback-inherit')
  ];
  element.remove();
  return values.join('|');
})()
"#,
        )
        .expect("custom property var ident function should evaluate");

    assert_eq!(result, "PASS||PASS|");
}
#[test]
fn computed_z_index_resolves_simple_custom_property_reference() {
    let mut vm = new_storage_test_vm("https://style-z-index-custom-property.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const target = document.createElement('div');
  root.append(target);
  target.style.zIndex = '1111111111111111111111111';
  const direct = getComputedStyle(target).zIndex;
  target.style.setProperty('--depth', '1111111111111111111111111');
  target.style.zIndex = 'var(--depth)';
  const large = getComputedStyle(target).zIndex;
  target.style.setProperty('--Depth', '42');
  target.style.zIndex = 'var(--Depth)';
  const caseSensitive = getComputedStyle(target).zIndex;
  target.style.zIndex = 'var(--missing)';
  const missing = getComputedStyle(target).zIndex;
  return [direct, large, caseSensitive, missing].join('|');
})()
"#,
        )
        .expect("computed z-index custom property should evaluate");

    assert_eq!(result, "2147483647|2147483647|42|auto");
}
#[test]
fn css_style_inset_accessor_rejects_quirky_unitless_lengths() {
    let mut vm = new_parsed_test_vm(
        "https://cssom-inset-quirky-length.test/",
        "<html><body><div id=target></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.getElementById('target').style;
  const quirkyValues = [
    '1',
    '1 2px',
    '1px 2',
    '1 2',
    '1 2px 3px',
    '1px 2 3px',
    '1px 2px 3',
    '1 2 3',
    '1 2px 3px 4px',
    '1px 2 3px 4px',
    '1px 2px 3 4px',
    '1px 2px 3px 4',
    '1 2 3 4'
  ];

  style.inset = '5px 6px 7px 8px';
  for (const value of quirkyValues) {
    style.inset = value;
    if (style.inset !== '5px 6px 7px 8px') {
      return `accepted:${value}:${style.inset}`;
    }
  }

  return [
    document.compatMode,
    Object.prototype.hasOwnProperty.call(style, 'inset'),
    style.getPropertyValue('inset'),
    style.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleDeclaration inset accessor should reject quirky lengths");

    assert_eq!(
        result,
        "BackCompat|false|5px 6px 7px 8px|inset: 5px 6px 7px 8px;"
    );
}
#[test]
fn css_style_excluded_properties_reject_quirky_unitless_lengths() {
    let mut vm = new_parsed_test_vm(
        "https://cssom-excluded-quirky-length.test/",
        "<html><body><div id=target></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const properties = [
    'background-blend-mode',
    'background-size',
    'box-shadow',
    'clip-path',
    'column-span',
    'filter',
    'mask',
    'object-position',
    'perspective-origin',
    'text-shadow',
    'transform-origin'
  ];

  if (!CSS.supports('mask', 'none') || CSS.supports('mask', 'banana')) {
    return 'mask-supports';
  }
  target.style.mask = 'none';
  if (target.style.mask !== 'none') {
    return `mask-valid:${target.style.mask}`;
  }
  target.style.mask = '1234';
  if (target.style.mask !== 'none') {
    return `mask-invalid:${target.style.mask}`;
  }

  for (const property of properties) {
    if (!getComputedStyle(target)[property]) {
      return `unsupported:${property}`;
    }
    target.style[property] = '1234';
    const value = target.style[property];
    if (value === '1234' || value === '1234px' || value === '1234px auto') {
      return `accepted:${property}:${value}`;
    }
    if (Object.prototype.hasOwnProperty.call(target.style, property)) {
      return `expando:${property}`;
    }
  }

  return `${document.compatMode}|ok`;
})()
"#,
        )
        .expect("excluded CSS properties should reject quirky unitless lengths");

    assert_eq!(result, "BackCompat|ok");
}
#[test]
fn font_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://font-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const fontLonghands = [
    'font-style',
    'font-variant-ligatures',
    'font-variant-caps',
    'font-variant-numeric',
    'font-variant-east-asian',
    'font-weight',
    'font-stretch',
    'font-size',
    'line-height',
    'font-family',
    'font-kerning'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };
  const lacks = (label, style, name) => {
    const actual = names(style);
    if (actual.includes(name)) failures.push(`${label}:unexpected:${name}:${actual.join(',')}`);
  };

  function exerciseFontStyle(style, label, textOwner) {
    style.setProperty('font-variant-caps', 'small-caps');
    style.setProperty('font', 'italic small-caps 700 16px / 2 Ahem', 'important');
    eq(`${label}-font`, style.getPropertyValue('font'), 'italic small-caps 700 16px / 2 Ahem');
    eq(`${label}-priority`, style.getPropertyPriority('font'), 'important');
    eq(`${label}-style`, style.getPropertyValue('font-style'), 'italic');
    eq(`${label}-caps`, style.getPropertyValue('font-variant-caps'), 'small-caps');
    eq(`${label}-weight`, style.getPropertyValue('font-weight'), '700');
    eq(`${label}-size`, style.getPropertyValue('font-size'), '16px');
    eq(`${label}-line-height`, style.getPropertyValue('line-height'), '2');
    eq(`${label}-family`, style.getPropertyValue('font-family'), 'Ahem');
    hasAll(`${label}-names`, style, fontLonghands);
    ok(`${label}-cssText`, style.cssText.includes('font: italic small-caps 700 16px / 2 Ahem !important;'));
    if (textOwner) {
      ok(`${label}-owner-cssText`, textOwner.cssText.includes('font: italic small-caps 700 16px / 2 Ahem !important;'));
    }

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    const removedFont = style.removeProperty('font');
    eq(`${label}-removed`, removedFont, 'italic small-caps 700 16px / 2 Ahem');
    eq(`${label}-font-after-remove`, style.getPropertyValue('font'), '');
    eq(`${label}-size-after-remove`, style.getPropertyValue('font-size'), '');
    lacks(`${label}-name-after-remove`, style, 'font-size');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('font', 'italic 12px / 1.5 "A B", serif');
    style.setProperty('font-size', '20px', 'important');
    eq(`${label}-font-after-longhand`, style.getPropertyValue('font'), '');
    eq(`${label}-size-after-longhand`, style.getPropertyValue('font-size'), '20px');
    eq(`${label}-size-priority-after-longhand`, style.getPropertyPriority('font-size'), 'important');
  }

  const inline = document.createElement('div').style;
  exerciseFontStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'font: italic small-caps 700 16px/2 Ahem !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-cssText-font', detached.getPropertyValue('font'), 'italic small-caps 700 16px / 2 Ahem');
  eq('detached-cssText-priority', detached.getPropertyPriority('font'), 'important');
  eq('detached-cssText-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-cssText-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  exerciseFontStyle(detached, 'detached');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  exerciseFontStyle(rule.style, 'rule', rule);

  const keyframe = sheet.cssRules[1].cssRules[0];
  exerciseFontStyle(keyframe.style, 'keyframe', keyframe);

  return failures.length ? failures.slice(0, 16).join('|') : 'PASS';
})()
"#,
        )
        .expect("font shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}
#[test]
fn regular_css_style_sheet_replace_is_not_allowed() {
    let mut vm = new_storage_test_vm("https://css-regular-sheet-replace.test/");

    let sync_result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const style = document.createElement('style');
  style.textContent = 'html { background-color: green; }';
  root.append(style);
  const regular = style.sheet;
  const imported = document.createElement('style');
  imported.textContent = '@import url("data:text/css,span%7Bcolor%3Ablue%7D");';
  root.append(imported);
  const childRule = imported.sheet.cssRules[0];
  const child = childRule.styleSheet;
  const before = regular.cssRules[0].cssText;
  const regularSync = probe(() => regular.replaceSync('main { color: red; }'));
  const childSync = probe(() => child.replaceSync('span { color: red; }'));
  globalThis.__regularSheetReplaceProbe = [];
  regular.replace('main { color: red; }').then(
    () => globalThis.__regularSheetReplaceProbe.push('regular:resolved'),
    error => globalThis.__regularSheetReplaceProbe.push(`regular:${error && error.name}:${regular.cssRules[0].cssText}`)
  );
  child.replace('span { color: red; }').then(
    () => globalThis.__regularSheetReplaceProbe.push('child:resolved'),
    error => globalThis.__regularSheetReplaceProbe.push(`child:${error && error.name}:${child.cssRules.length}`)
  );
  style.remove();
  const removedBackground = getComputedStyle(root).backgroundColor;
  regular.replace('html { background-color: red; }').then(
    () => globalThis.__regularSheetReplaceProbe.push('removed:resolved'),
    error => globalThis.__regularSheetReplaceProbe.push(`removed:${error && error.name}:${getComputedStyle(root).backgroundColor}`)
  );
  return [
    regular instanceof CSSStyleSheet,
    before,
    regularSync,
    regular.cssRules[0].cssText,
    child instanceof CSSStyleSheet,
    child.ownerRule === childRule,
    childSync,
    removedBackground
  ].join('|');
})()
"#,
        )
        .expect("regular CSSStyleSheet replaceSync should evaluate");

    let async_result = vm
        .eval("globalThis.__regularSheetReplaceProbe.join('|')")
        .expect("regular CSSStyleSheet replace promises should settle");

    assert_eq!(
        sync_result,
        "true|html { background-color: green; }|throw:NotAllowedError|html { background-color: green; }|true|true|throw:NotAllowedError|rgba(0, 0, 0, 0)"
    );
    assert_eq!(
        async_result,
        "regular:NotAllowedError:html { background-color: green; }|child:NotAllowedError:1|removed:NotAllowedError:rgba(0, 0, 0, 0)"
    );
}
#[test]
fn owner_style_data_import_feeds_computed_style_without_hiding_cssom_import_rule() {
    let mut vm = new_storage_test_vm("https://owner-style-data-import.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const fallback = document.createElement('style');
  fallback.textContent = '@layer { .target { color: red; } }';
  head.appendChild(fallback);

  const target = document.createElement('div');
  target.className = 'target';
  body.appendChild(target);

  const style = document.createElement('style');
  style.textContent = '@import url("data:text/css,.target%7Bcolor:green%7D") supports(display:block);';
  head.appendChild(style);

  const rule = style.sheet.cssRules[0];
  const child = rule.styleSheet;
  return [
    getComputedStyle(target).color,
    style.sheet.cssRules.length,
    rule.constructor.name,
    rule.href,
    rule.supportsText,
    rule.cssText,
    child instanceof CSSStyleSheet,
    child.cssRules.length,
    child.cssRules[0].cssText,
    child.ownerRule === rule,
    child.parentStyleSheet === style.sheet,
    rule.styleSheet === child
  ].join('|');
})()
"#,
        )
        .expect("owner style data import computed-style probe should evaluate");

    assert_eq!(
        result,
        "rgb(0, 128, 0)|1|CSSImportRule|data:text/css,.target%7Bcolor:green%7D|display:block|@import url(\"data:text/css,.target%7Bcolor:green%7D\") supports(display:block);|true|1|.target { color: green; }|true|true|true"
    );
}
#[test]
fn css_scope_nested_declarations_are_exposed_in_cssom() {
    let mut vm = new_storage_test_vm("https://css-scope-nested-declarations-cssom.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function run(prelude) {
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(`
      @scope ${prelude} {
        color: red;
        width: 1px;
        .b {}
        left: 2px;
        right: 3px;
        .c {}
        top: 4px;
        bottom: 5px;
      }
    `);
    const scopeRule = sheet.cssRules[0];
    return [
      scopeRule.cssRules.length,
      scopeRule.cssRules[0] instanceof CSSNestedDeclarations,
      scopeRule.cssRules[0].cssText,
      scopeRule.cssRules[2] instanceof CSSNestedDeclarations,
      scopeRule.cssRules[2].cssText,
      scopeRule.cssRules[4] instanceof CSSNestedDeclarations,
      scopeRule.cssRules[4].cssText,
    ].join('|');
  }
  return [run('(.a)'), run('')].join('||');
})()
"#,
        )
        .expect("@scope nested declarations CSSOM should evaluate");

    assert_eq!(
        result,
        "5|true|color: red; width: 1px;|true|left: 2px; right: 3px;|true|top: 4px; bottom: 5px;||5|true|color: red; width: 1px;|true|left: 2px; right: 3px;|true|top: 4px; bottom: 5px;"
    );
}
#[test]
fn css_scope_relative_nested_style_rules_are_exposed_in_cssom() {
    let mut vm = new_storage_test_vm("https://css-scope-relative-nesting.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function createRuleString(prelude, inner) {
    if (prelude.length === 0) {
      return `${inner} {}`;
    }
    const outermost = prelude[0];
    const rest = createRuleString(prelude.slice(1), inner);
    return `${outermost} { ${rest} }`;
  }
  function createByString(style, prelude, inner) {
    style.textContent = createRuleString(prelude, inner);
  }
  function createByInsertion(style, prelude, inner) {
    let current = style.sheet;
    for (const p of prelude) {
      const idx = current.insertRule(`${p} {}`);
      current = current.cssRules[idx];
    }
    current.insertRule(`${inner} {}`);
  }
  function innermostSelector(depth, rules) {
    let current = rules;
    for (let d = depth + 1; d !== 0; d--) {
      if (current.cssRules.length !== 1) {
        return `len:${current.cssRules.length}`;
      }
      current = current.cssRules[0];
    }
    return current.selectorText;
  }
  function run(prelude, method) {
    try {
      const style = document.createElement('style');
      (document.head || document.documentElement || document).appendChild(style);
      method(style, prelude, '> .foo');
      const selector = innermostSelector(prelude.length, style.sheet);
      style.remove();
      return selector;
    } catch (error) {
      return `error:${error && (error.name || error.message)}`;
    }
  }
  const cases = [
    [['@scope', '.nest'], createByString],
    [['.nest', '@scope'], createByString],
    [['@scope', '.nest', '@media screen'], createByString],
    [['.nest', '@scope', '@media screen'], createByString],
    [['@scope', '.nest'], createByInsertion],
    [['.nest', '@scope'], createByInsertion],
    [['@scope', '.nest', '@media screen'], createByInsertion],
    [['.nest', '@scope', '@media screen'], createByInsertion],
  ];
  return cases.map(([prelude, method]) => run(prelude, method)).join('|');
})()
"#,
        )
        .expect("CSS @scope relative nested selector CSSOM should evaluate");

    assert_eq!(
        result,
        "& > .foo|> .foo|& > .foo|> .foo|& > .foo|> .foo|& > .foo|> .foo"
    );
}
#[test]
fn computed_flex_flow_serializes_direction_and_wrap() {
    let mut vm = new_storage_test_vm("https://computed-flex-flow.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(target);
  const values = [
    'initial',
    'column',
    'wrap',
    'column wrap-reverse',
    'row-reverse wrap'
  ];
  return values.map(value => {
    target.style.flexFlow = value;
    const computed = getComputedStyle(target);
    return [computed.flexFlow, computed.flexDirection, computed.flexWrap].join('|');
  }).join('/');
})()
"#,
        )
        .expect("computed flex-flow serialization should evaluate");

    assert_eq!(
        result,
        "row nowrap|row|nowrap/column nowrap|column|nowrap/row wrap|row|wrap/column wrap-reverse|column|wrap-reverse/row-reverse wrap|row-reverse|wrap"
    );
}
#[test]
fn html_style_disabled_waits_for_associated_stylesheet() {
    let mut vm = new_storage_test_vm("https://style-disabled-cssom.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.disabled = true;
  const beforeAppend = [
    style.disabled,
    style.hasAttribute('disabled'),
    style.sheet === null
  ].join('|');
  head.append(style);
  const sheet = style.sheet;
  const afterAppend = [
    style.disabled,
    sheet.disabled,
    style.hasAttribute('disabled')
  ].join('|');
  style.disabled = true;
  const afterStyleSet = [
    style.disabled,
    sheet.disabled,
    style.hasAttribute('disabled')
  ].join('|');
  style.disabled = false;
  sheet.disabled = true;
  const afterSheetSet = [
    style.disabled,
    sheet.disabled,
    style.hasAttribute('disabled')
  ].join('|');

  const attributed = document.createElement('style');
  attributed.setAttribute('disabled', '');
  head.append(attributed);
  const attributedSheet = attributed.sheet;
  const contentAttributeInitial = [
    attributed.disabled,
    attributedSheet.disabled,
    attributed.hasAttribute('disabled')
  ].join('|');
  attributed.media = 'screen';
  const contentAttributeAfterMedia = [
    attributed.disabled,
    attributedSheet.disabled,
    attributed.hasAttribute('disabled')
  ].join('|');
  return [
    beforeAppend,
    afterAppend,
    afterStyleSet,
    afterSheetSet,
    contentAttributeInitial,
    contentAttributeAfterMedia
  ].join('||');
})()
"#,
        )
        .expect("HTMLStyleElement.disabled should follow the associated stylesheet");

    assert_eq!(
        result,
        "false|false|true||false|false|false||true|true|false||true|true|false||false|false|true||false|false|true"
    );
}
#[test]
fn link_style_sheet_attribute_lives_on_prototype() {
    let mut vm = new_storage_test_vm("https://link-style-sheet-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function prototypeOwns(value, name) {
    for (let proto = Object.getPrototypeOf(value); proto; proto = Object.getPrototypeOf(proto)) {
      if (Object.prototype.hasOwnProperty.call(proto, name)) {
        return true;
      }
    }
    return false;
  }

  const style = document.createElement('style');
  const link = document.createElement('link');
  link.rel = 'stylesheet';

  const placement = [
    Object.prototype.hasOwnProperty.call(style, 'sheet'),
    'sheet' in style,
    prototypeOwns(style, 'sheet'),
    Object.prototype.hasOwnProperty.call(link, 'sheet'),
    'sheet' in link,
    prototypeOwns(link, 'sheet')
  ].join('|');

  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  head.append(style, link);
  style.textContent = 'body { color: red; }';
  const styleSheet = style.sheet;
  const linkSheet = link.sheet;
  return [
    placement,
    styleSheet instanceof CSSStyleSheet,
    styleSheet.ownerNode === style,
    styleSheet.cssRules.length,
    linkSheet === null
  ].join('||');
})()
"#,
        )
        .expect("LinkStyle sheet prototype placement should evaluate");

    assert_eq!(
        result,
        "false|true|true|false|true|true||true||true||1||true"
    );
}
#[test]
fn disconnected_style_owners_do_not_expose_unbound_stylesheets() {
    let mut vm = new_storage_test_vm("https://disconnected-stylesheet-owner.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const SVG_NS = 'http://www.w3.org/2000/svg';
  const style = document.createElement('style');
  style.textContent = 'body { color: red; }';
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  const svg = document.createElementNS(SVG_NS, 'svg');
  const svgStyle = document.createElementNS(SVG_NS, 'style');
  svgStyle.textContent = 'body { color: green; }';
  svg.appendChild(svgStyle);
  const before = [style.sheet, link.sheet, svgStyle.sheet].map(sheet => sheet === null);

  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  head.appendChild(style);
  body.appendChild(svg);
  const styleSheet = style.sheet;
  const svgSheet = svgStyle.sheet;
  style.remove();
  svg.remove();

  return [
    ...before,
    styleSheet instanceof CSSStyleSheet,
    svgSheet instanceof CSSStyleSheet,
    style.sheet === null,
    svgStyle.sheet === null,
    styleSheet.ownerNode === null,
    svgSheet.ownerNode === null,
  ].join('|');
})()
"#,
        )
        .expect("disconnected stylesheet owner behavior should evaluate");

    assert_eq!(result, "true|true|true|true|true|true|true|true|true");
}
#[test]
fn cached_import_bearing_link_creates_a_fresh_native_graph_for_each_owner() {
    let mut vm = new_storage_test_vm("https://link-import-cache.test/");
    let stylesheet_url = url::Url::parse("https://link-import-cache.test/shared.css").unwrap();

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  for (const id of ['first-import-link', 'cached-import-link']) {
    const link = document.createElement('link');
    link.id = id;
    link.rel = 'stylesheet';
    link.href = '/shared.css';
    head.appendChild(link);
  }
})()
"#,
    )
    .expect("import-bearing linked stylesheet cache fixture should evaluate");
    let first = cssom_element_handle_by_id(&vm, "first-import-link");
    let cached = cssom_element_handle_by_id(&vm, "cached-import-link");
    install_linked_stylesheet_for_test(
        &mut vm,
        first,
        stylesheet_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            "@import './child.css'; .root { color: green; }".to_owned(),
            stylesheet_url.clone(),
        )
        .with_sheet_url(stylesheet_url.clone()),
    );
    assert!(
        vm._context_host
            .borrow_mut()
            .install_cached_linked_stylesheet_for_owner(cached, &stylesheet_url),
        "the second owner should rebind from the URL resource cache"
    );
    vm.apply_pending_stylesheet_source_css_projections();

    let result = vm
        .eval(
            r#"
(() => {
  const first = document.querySelector('#first-import-link').sheet;
  const cached = document.querySelector('#cached-import-link').sheet;
  return [
    first !== null,
    cached !== null,
    first !== cached,
    first.cssRules.length,
    cached.cssRules.length,
    first.cssRules[0] instanceof CSSImportRule,
    cached.cssRules[0] instanceof CSSImportRule,
    first.cssRules[0] !== cached.cssRules[0],
    first.cssRules[0].styleSheet === null,
    cached.cssRules[0].styleSheet === null
  ].join('|');
})()
"#,
        )
        .expect("cached import-bearing stylesheet should retain its native rules");

    assert_eq!(result, "true|true|true|2|2|true|true|true|true|true");
}
#[test]
fn element_style_put_forwards_uses_ordinary_get_and_set() {
    let mut vm = new_storage_test_vm("https://element-style-put-forwards.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "style");
  const element = document.createElement("div");
  const originalStyle = element.style;
  let getterCalls = 0;
  Object.defineProperty(element, "style", {
    configurable: true,
    get() { getterCalls++; return originalStyle; },
    set: descriptor.set
  });
  element.style = "color: green";

  let setterCalls = 0;
  const cssText = Object.getOwnPropertyDescriptor(CSSStyleDeclaration.prototype, "cssText");
  Object.defineProperty(originalStyle, "cssText", {
    configurable: true,
    get: cssText.get,
    set(value) { setterCalls++; cssText.set.call(this, value); }
  });
  element.style = "color: blue";

  const fakeStyle = { cssText: "original" };
  Object.defineProperty(element, "style", {
    configurable: true,
    get() { return fakeStyle; },
    set: descriptor.set
  });
  element.style = "color: red";

  const outcome = callback => {
    try { callback(); return "return"; }
    catch (error) { return error && error.name; }
  };
  Object.defineProperty(element, "style", {
    configurable: true,
    get() { throw new SyntaxError(); },
    set: descriptor.set
  });
  const getterError = outcome(() => { element.style = "x"; });
  Object.defineProperty(element, "style", {
    configurable: true,
    get() { return null; },
    set: descriptor.set
  });
  const nonObject = outcome(() => { element.style = "x"; });

  return [
    getterCalls,
    setterCalls,
    originalStyle.color,
    fakeStyle.cssText,
    getterError,
    nonObject
  ].join("|");
})()
"#,
        )
        .expect("Element.style PutForwards probe should evaluate");

    assert_eq!(result, "2|1|blue|color: red|SyntaxError|TypeError");
}
#[test]
fn page_descriptors_do_not_leak_into_element_style_queries() {
    let mut vm = new_storage_test_vm("https://css-page-descriptor-query-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(element);
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page { size: portrait; }');
  return [
    getComputedStyle(element).getPropertyValue('size'),
    element.style.getPropertyValue('size'),
    CSS.supports('size', 'portrait'),
    sheet.cssRules[0].style.getPropertyValue('size')
  ].join('|');
})()
"#,
        )
        .expect("page descriptors should stay scoped to CSSPageDescriptors");

    assert_eq!(result, "||false|portrait");
}
#[test]
fn cssom_attribute_case_flags_reject_namespace_like_trailing_tokens() {
    let mut vm = new_storage_test_vm("https://cssom-attribute-case-flags.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const head = document.head || root.appendChild(document.createElement("head"));
  const style = document.createElement("style");
  head.append(style);
  const invalid = [
    "[foo='bar' |i]",
    "[foo='bar' *|i]",
    "[foo='bar' \\*|i]",
  ];
  const textCounts = invalid.map(selector => {
    style.textContent = `${selector} { color: red; }`;
    return style.sheet.cssRules.length;
  });
  const insertResults = invalid.map(selector => {
    const sheet = new CSSStyleSheet();
    return probe(() => sheet.insertRule(`${selector} { color: red; }`, 0));
  });
  const valid = new CSSStyleSheet();
  valid.insertRule("[|foo='bar' i] { color: green; }", 0);
  valid.insertRule("[*|foo='bar' i] { color: blue; }", 1);
  return [
    textCounts.join(","),
    insertResults.join(","),
    valid.cssRules.length,
    valid.cssRules[0].selectorText,
    valid.cssRules[1].selectorText
  ].join("|");
})()
"#,
        )
        .expect("attribute case flag namespace-like selector validation should evaluate");

    assert_eq!(
        result,
        "0,0,0|SyntaxError,SyntaxError,SyntaxError|2|[foo=\"bar\" i]|[*|foo=\"bar\" i]"
    );
}
#[test]
fn cssom_rejects_terminal_pseudo_element_chains() {
    let mut vm = new_storage_test_vm("https://cssom-terminal-pseudo-chain.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };
  const sheet = new CSSStyleSheet();
  const beforeHighlight = probe(() => {
    sheet.insertRule("::before::highlight(foo) { color: red; }", 0);
  });
  const highlightAfter = probe(() => {
    sheet.insertRule("::highlight(foo)::after { color: red; }", 0);
  });
  sheet.insertRule("::part(label)::highlight(foo) { color: green; }", 0);
  const rule = sheet.cssRules[0];
  rule.selectorText = "::highlight(foo)::after";
  return [
    beforeHighlight,
    highlightAfter,
    sheet.cssRules.length,
    rule.selectorText
  ].join("|");
})()
"#,
        )
        .expect("terminal pseudo-element chain validation should evaluate");

    assert_eq!(
        result,
        "SyntaxError|SyntaxError|1|::part(label)::highlight(foo)"
    );
}
#[test]
fn cssom_namespaced_selector_text_updates_style_matching() {
    let mut vm = new_storage_test_vm("https://cssom-selector-text-namespace-matching.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    @namespace url("http://www.w3.org/1999/xhtml");
    @namespace svg url("http://www.w3.org/2000/svg");
    svg|*.style0 { background-color: rgb(0, 0, 255) !important; }
    svg|*.style1 { background-color: rgb(255, 0, 255); }
  `;
  head.append(style);
  const target = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  target.setAttribute("class", "style1");
  body.append(target);
  const sheet = style.sheet;
  const rule = sheet.cssRules[2];
  const original = rule.selectorText;
  const color = () => getComputedStyle(target).backgroundColor;
  const probe = selector => {
    rule.selectorText = selector;
    const value = [rule.selectorText, color()].join("=>");
    rule.selectorText = original;
    return value;
  };
  return [
    original,
    color(),
    probe(".style1"),
    probe("svg|*.style1  "),
    probe("*|*.style1  "),
    probe(" *.style1  "),
    probe("p")
  ].join("|");
})()
"#,
        )
        .expect("namespaced selectorText style matching should evaluate");

    assert_eq!(
        result,
        "svg|*.style0|rgb(255, 0, 255)|.style1=>rgb(255, 0, 255)|svg|*.style1=>rgb(0, 0, 255)|*|*.style1=>rgb(0, 0, 255)|.style1=>rgb(255, 0, 255)|p=>rgb(255, 0, 255)"
    );
}
#[test]
fn css_layer_rules_expose_cssom_surface() {
    let mut vm = new_storage_test_vm("https://css-layer-rules.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return error && error.name;
    }
  };
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.textContent = `
    @layer foo, bar;
    @import url("data:text/css,") layer(qux);
    @import url("data:text/css,");
    @layer outer { @layer inner {} }
  `;
  head.append(style);
  const statement = style.sheet.cssRules[0];
  const layeredImport = style.sheet.cssRules[1];
  const plainImport = style.sheet.cssRules[2];
  const block = style.sheet.cssRules[3];
  const nested = block.cssRules[0];
  return [
    typeof CSSLayerBlockRule,
    typeof CSSLayerStatementRule,
    block instanceof CSSLayerBlockRule,
    block instanceof CSSGroupingRule,
    !(block instanceof CSSConditionRule),
    statement instanceof CSSLayerStatementRule,
    !(statement instanceof CSSGroupingRule),
    block.name,
    nested.name,
    statement.nameList.join(','),
    Object.isFrozen(statement.nameList),
    layeredImport.layerName,
    plainImport.layerName === null,
    probe(() => CSSLayerBlockRule.prototype.name),
    probe(() => CSSLayerStatementRule.prototype.nameList)
  ].join('|');
})()
"#,
        )
        .expect("CSS layer rule surface should evaluate");

    assert_eq!(
        result,
        "function|function|true|true|true|true|true|outer|inner|foo,bar|true|qux|true|TypeError|TypeError"
    );
}
#[test]
fn css_style_set_property_undefined_value_is_noop() {
    let mut vm = new_storage_test_vm("https://css-set-property-undefined.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  document.appendChild(element);
  const live = element.style;
  live.color = 'white';
  live.setProperty('color', undefined);

  const detached = new DOMParser()
    .parseFromString('<html><body></body></html>', 'text/html')
    .createElement('div')
    .style;
  detached.color = 'white';
  detached.setProperty('color', undefined);

  live.setProperty('background-color', 'red', undefined);
  return [
    live.color,
    detached.color,
    live.backgroundColor,
    live.getPropertyPriority('background-color')
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleDeclaration.setProperty undefined value should evaluate");

    assert_eq!(result, "white|white|red|");
}
#[test]
fn background_image_image_set_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://background-image-image-set-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const value = 'image-set(url("") calc(1x * NaN))';
  const serialized = 'image-set(url("") calc(NaN * 1dppx))';

  function exercise(style, label, ruleText) {
    const beforeNames = names(style);
    style.setProperty('background-image', value, 'important');
    eq(`${label}-value`, style.getPropertyValue('background-image'), serialized);
    eq(`${label}-priority`, style.getPropertyPriority('background-image'), 'important');
    const afterNames = names(style);
    eq(`${label}-length`, String(style.length), String(beforeNames.length + 1));
    ok(`${label}-names`, afterNames.includes('background-image'));
    ok(`${label}-cssText`, style.cssText.includes(`background-image: ${serialized} !important;`));
    if (ruleText) {
      ok(`${label}-rule-cssText`, ruleText().includes(`background-image: ${serialized} !important;`));
    }

    style.setProperty('--token', 'value');
    const removed = style.removeProperty('background-image');
    eq(`${label}-removed`, removed, serialized);
    eq(`${label}-after-remove`, style.getPropertyValue('background-image'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
  }

  exercise(document.createElement('div').style, 'inline');

  const detached = new DOMParser().parseFromString('<html><body></body></html>', 'text/html')
    .createElement('div').style;
  exercise(detached, 'detached');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; }');
  const rule = sheet.cssRules[0];
  exercise(rule.style, 'rule', () => rule.cssText);

  return failures.length ? failures.slice(0, 8).join('|') : 'PASS';
})()
"#,
        )
        .expect("background-image image-set PDB backing should evaluate");

    assert_eq!(result, "PASS");
}
#[test]
fn inline_owner_text_replacement_creates_a_new_sheet_and_detaches_the_old_wrapper() {
    let mut vm = new_storage_test_vm("https://inline-sheet-owner-replacement.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.id = 'target';
  const style = document.createElement('style');
  style.textContent = '#target { color: rgb(1, 2, 3); }';
  head.appendChild(style);

  const oldSheet = style.sheet;
  const oldRule = oldSheet.cssRules[0];
  oldRule.expando = 'retained';
  style.textContent = '#target { color: rgb(4, 5, 6); }';
  const currentSheet = style.sheet;
  oldSheet.insertRule('.detached-only { color: red; }', oldSheet.cssRules.length);

  return [
    currentSheet !== oldSheet,
    oldSheet.ownerNode === null,
    currentSheet.ownerNode === style,
    oldSheet.cssRules[0] === oldRule,
    oldSheet.cssRules[0].expando,
    oldSheet.cssRules.length,
    currentSheet.cssRules.length,
    style.textContent,
    getComputedStyle(target).color,
  ].join('|');
})()
"#,
        )
        .expect("inline owner text replacement should create a new stylesheet");

    assert_eq!(
        result,
        "true|true|true|true|retained|2|1|#target { color: rgb(4, 5, 6); }|rgb(4, 5, 6)"
    );
}
#[test]
fn inline_cssom_mutation_keeps_owner_text_as_input_only() {
    let mut vm = new_storage_test_vm("https://inline-sheet-owner-input.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.id = 'target';
  const style = document.createElement('style');
  style.id = 'owner-input';
  style.textContent = '#target { color: rgb(1, 2, 3); }';
  head.appendChild(style);
  const sheet = style.sheet;
  sheet.cssRules[0].style.color = 'rgb(4, 5, 6)';
  sheet.insertRule('#target { background-color: rgb(7, 8, 9); }');
  return [
    style.textContent,
    sheet.cssRules.length,
    getComputedStyle(target).color,
    getComputedStyle(target).backgroundColor,
  ].join('|');
})()
"#,
        )
        .expect("inline CSSOM mutation should preserve owner input text");

    assert_eq!(
        result,
        "#target { color: rgb(1, 2, 3); }|2|rgb(4, 5, 6)|rgb(7, 8, 9)"
    );
    let owner = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("owner-input")
        .expect("style owner");
    assert_eq!(
        vm._context_host
            .borrow()
            .owner_style_sheet_text(owner)
            .as_deref(),
        Some("#target { color: rgb(1, 2, 3); }")
    );
    assert!(
        vm._context_host
            .borrow()
            .owner_live_stylesheet(owner)
            .is_some_and(|stylesheet| {
                crate::style_engine::StyloStylesheetSource::from_live_stylesheet(&stylesheet)
                    .serialized_css_text()
                    .contains("background-color")
            })
    );
}
#[test]
fn overflow_overlay_uses_pdb_supplemental_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://overflow-overlay-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index)).join(',');
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  function check(style, label) {
    style.setProperty('overflow', 'overlay hidden', 'important');
    eq(`${label}-length`, String(style.length), '2');
    eq(`${label}-names`, names(style), 'overflow-x,overflow-y');
    eq(`${label}-overflow`, style.getPropertyValue('overflow'), 'overlay hidden');
    eq(`${label}-x`, style.getPropertyValue('overflow-x'), 'overlay');
    eq(`${label}-y`, style.getPropertyValue('overflow-y'), 'hidden');
    eq(`${label}-priority`, style.getPropertyPriority('overflow'), 'important');
    eq(`${label}-x-priority`, style.getPropertyPriority('overflow-x'), 'important');
    eq(`${label}-cssText`, style.cssText, 'overflow: overlay hidden !important;');
  }

  check(document.createElement('div').style, 'inline');

  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  check(doc.createElement('div').style, 'detached');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0];
  check(rule.style, 'rule');
  eq('rule-cssText', rule.cssText, 'div { overflow: overlay hidden !important; }');

  return failures.length ? failures.slice(0, 8).join('|') : 'PASS';
})()
"#,
        )
        .expect("overflow overlay PDB supplemental surface should evaluate");

    assert_eq!(result, "PASS");
}
#[test]
fn css_style_set_property_reappends_existing_declaration_order() {
    let mut vm = new_storage_test_vm("https://css-style-set-property-order.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  element.setAttribute('style', [
    'padding-top: 0px;',
    'padding-right: 1px;',
    'padding-bottom: 2px;',
    'padding-left: 3px;',
    'padding-block-start: 4px;',
    'padding-block-end: 5px;',
    'padding-inline-start: 6px;',
    'padding-inline-end: 7px;'
  ].join(' '));
  document.appendChild(element);
  const style = element.style;
  style.setProperty('padding-top', '0px');
  const afterLonghand = Array.from({ length: style.length }, (_, index) => style.item(index)).join(',');
  style.setProperty('padding', '0px 1px 2px 3px');
  const afterShorthand = Array.from({ length: style.length }, (_, index) => style.item(index)).join(',');

  const detached = new DOMParser().parseFromString('<html></html>', 'text/html')
    .createElement('div').style;
  detached.setProperty('color', 'red');
  detached.setProperty('opacity', '0.5');
  detached.setProperty('color', 'blue');
  const detachedOrder = Array.from({ length: detached.length }, (_, index) => detached.item(index)).join(',');

  return [afterLonghand, afterShorthand, detachedOrder].join('|');
})()
"#,
        )
        .expect("CSSStyleDeclaration setProperty order should evaluate");

    assert_eq!(
        result,
        "padding-right,padding-bottom,padding-left,padding-block-start,padding-block-end,padding-inline-start,padding-inline-end,padding-top|padding-block-start,padding-block-end,padding-inline-start,padding-inline-end,padding-top,padding-right,padding-bottom,padding-left|opacity,color"
    );
}
#[test]
fn computed_style_initial_values_match_exposed_css_supports_longhands() {
    let mut vm = new_storage_test_vm("https://css-computed-initial-supports.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(element);
  const { style } = element;
  const computedStyle = getComputedStyle(element);
  const cssProperties = new Set();
  const computedLonghandNames = new Set(
    Array.from({ length: computedStyle.length }, (_, index) => computedStyle.item(index))
  );

  for (let obj = style; obj; obj = Reflect.getPrototypeOf(obj)) {
    for (let name of Object.getOwnPropertyNames(obj)) {
      const property = name.replace(/[A-Z]/g, c => "-" + c.toLowerCase());
      if (CSS.supports(property, "initial")) {
        cssProperties.add(property);
      }
    }
  }

  const cssLonghands = new Set(
    Array.from(cssProperties).filter(property => computedLonghandNames.has(property))
  );

  for (let longhand of cssLonghands) {
    element.style.setProperty(longhand, "initial");
  }

  const bad = [];
  for (let property of cssLonghands) {
    const result = computedStyle.getPropertyValue(property);
    if (!CSS.supports(property, result) && property !== "all") {
      bad.push([property, result]);
    }
  }
  return JSON.stringify(bad);
})()
"#,
        )
        .expect("computed initial CSS.supports surface should evaluate");

    assert_eq!(result, "[]");
}
#[test]
fn css_style_transition_property_accepts_css_ident_tokens() {
    let mut vm = new_storage_test_vm("https://css-style-transition-property-ident.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  style.transitionProperty = 'ALL, INVALID, SYNTAX, SRC';
  const upper = style.transitionProperty;
  style.transitionProperty = 'foo\\ bar, --custom-prop, --\\30 0, \\E9';
  const escaped = style.transitionProperty;
  style.transition = 'foo\\ bar 1s';
  const shorthand = [
    style.transition,
    style.transitionProperty,
    style.transitionDuration
  ].join('/');
  style.transitionProperty = 'none, width';
  const invalidNoneList = style.transitionProperty;
  return [upper, escaped, shorthand, invalidNoneList].join('|');
})()
"#,
        )
        .expect("transition-property CSS ident parsing should evaluate");

    assert_eq!(
        result,
        "all, INVALID, SYNTAX, SRC|foo\\ bar, --custom-prop, --00, é|foo\\ bar 1s/foo\\ bar/1s|foo\\ bar"
    );
}
#[test]
fn cssom_animation_timing_function_accepts_css_easing_math() {
    let mut vm = new_storage_test_vm("https://css-easing-cssom.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  const values = [];

  values.push(CSS.supports('animation-timing-function', 'cubic-bezier(calc(-2), calc(0.7 / 2), calc(1.5), calc(0))'));
  values.push(CSS.supports('animation-timing-function', 'cubic-bezier(-0.1, 0.1, 0.5, 0.9)'));
  values.push(CSS.supports('animation-timing-function', 'steps(calc(1), jump-none)'));
  values.push(CSS.supports('animation-timing-function', 'steps(calc(0/0), jump-none)'));
  values.push(CSS.supports('animation-timing-function', 'linear(0, 1)'));
  values.push(CSS.supports('animation-timing-function', 'linear(0 calc(50px - 50%), 0 calc(50em + 50em))'));

  style.animationTimingFunction = 'cubic-bezier(calc(-2), calc(0.7 / 2), calc(1.5), calc(0))';
  values.push(style.animationTimingFunction);

  style.animationTimingFunction = 'cubic-bezier(0, sibling-index(), 1, sign(2em - 20px))';
  values.push(style.animationTimingFunction);

  style.animationTimingFunction = 'steps(calc(-10), start)';
  values.push(style.animationTimingFunction);

  style.animationTimingFunction = 'steps(calc(1), jump-none)';
  values.push(style.animationTimingFunction);

  style.animationTimingFunction = 'linear(0, 1)';
  values.push(style.animationTimingFunction);

  style.animationTimingFunction = 'linear(0 calc(50% - 50%), 0 calc(50% + 50%))';
  values.push(style.animationTimingFunction);

  style.animationTimingFunction = 'linear(calc(0/0), 1)';
  values.push(style.animationTimingFunction);

  return values.join('|');
})()
"#,
        )
        .expect("animation timing CSSOM math should evaluate");

    assert_eq!(
        result,
        "true|false|true|false|true|false|cubic-bezier(calc(-2), calc(0.35), calc(1.5), calc(0))|cubic-bezier(0, sibling-index(), 1, sign(2em - 20px))|steps(calc(-10), start)|steps(calc(1), jump-none)|linear(0, 1)|linear(0 calc(0%), 0 calc(100%))|linear(0 0%, 1 100%)"
    );
}
#[test]
fn box_shorthand_cssom_writes_use_pdb_boundary() {
    let mut vm = new_storage_test_vm("https://box-shorthand-cssom-pdb-boundary.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, value) => {
    if (!value) failures.push(`${label}:${value}`);
  };
  const check = (style, label, property, valid, invalid, longhand, longhandValue, shorthandValue) => {
    style[property] = invalid;
    eq(`${label}-${property}-invalid-empty-length`, style.length, 0);
    eq(`${label}-${property}-invalid-empty-own`, Object.prototype.hasOwnProperty.call(style, property), false);
    eq(`${label}-${property}-invalid-empty-query`, style.getPropertyValue(property), '');

    style[property] = valid;
    eq(`${label}-${property}-shorthand`, style.getPropertyValue(property), shorthandValue);
    eq(`${label}-${property}-longhand`, style.getPropertyValue(longhand), longhandValue);
    ok(`${label}-${property}-cssText`, style.cssText.includes(`${property}: ${shorthandValue};`));

    style[property] = invalid;
    eq(`${label}-${property}-invalid-preserves-shorthand`, style.getPropertyValue(property), shorthandValue);
    eq(`${label}-${property}-invalid-preserves-longhand`, style.getPropertyValue(longhand), longhandValue);
    eq(`${label}-${property}-invalid-preserves-own`, Object.prototype.hasOwnProperty.call(style, property), false);
  };

  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = doc.createElement('div').style;
  check(detached, 'detached', 'margin', '1px 2px', 'banana', 'margin-left', '2px', '1px 2px');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const ruleStyle = sheet.cssRules[0].style;
  check(ruleStyle, 'rule', 'padding', 'calc(calc(12px)) 2px', 'banana', 'padding-top', 'calc(12px)', 'calc(12px) 2px');

  return failures.length ? failures.join('|') : 'PASS';
})()
"#,
        )
        .expect("box shorthand CSSOM writes should use the PDB boundary");

    assert_eq!(result, "PASS");
}
#[test]
fn logical_box_cssom_writes_use_pdb_boundary() {
    let mut vm = new_storage_test_vm("https://logical-box-cssom-pdb-boundary.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, value) => {
    if (!value) failures.push(`${label}:${value}`);
  };
  const check = (style, label, idl, property, valid, invalid, longhand, longhandValue, queryValue) => {
    ok(`${label}-${property}-idl`, idl in style);
    ok(`${label}-${property}-kebab`, property in style);

    style[idl] = invalid;
    eq(`${label}-${property}-invalid-empty-length`, style.length, 0);
    eq(`${label}-${property}-invalid-empty-own`, Object.prototype.hasOwnProperty.call(style, idl), false);
    eq(`${label}-${property}-invalid-empty-query`, style.getPropertyValue(property), '');

    style[idl] = valid;
    eq(`${label}-${property}-query`, style.getPropertyValue(property), queryValue);
    eq(`${label}-${property}-idl-get`, style[idl], queryValue);
    eq(`${label}-${property}-longhand`, style.getPropertyValue(longhand), longhandValue);
    ok(`${label}-${property}-cssText`, style.cssText.includes(`${property}: ${queryValue};`));

    style[idl] = invalid;
    eq(`${label}-${property}-invalid-preserves-query`, style.getPropertyValue(property), queryValue);
    eq(`${label}-${property}-invalid-preserves-longhand`, style.getPropertyValue(longhand), longhandValue);
    eq(`${label}-${property}-invalid-preserves-own`, Object.prototype.hasOwnProperty.call(style, idl), false);
  };

  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  check(
    doc.createElement('div').style,
    'detached-shorthand',
    'marginBlock',
    'margin-block',
    '1px 2px',
    'banana',
    'margin-block-end',
    '2px',
    '1px 2px'
  );
  check(
    doc.createElement('div').style,
    'detached-longhand',
    'paddingBlockStart',
    'padding-block-start',
    'calc(calc(12px))',
    'banana',
    'padding-block-start',
    'calc(12px)',
    'calc(12px)'
  );

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  check(
    sheet.cssRules[0].style,
    'rule-shorthand',
    'marginInline',
    'margin-inline',
    '3px 4px',
    'banana',
    'margin-inline-start',
    '3px',
    '3px 4px'
  );
  const sheet2 = new CSSStyleSheet();
  sheet2.insertRule('div {}');
  check(
    sheet2.cssRules[0].style,
    'rule-longhand',
    'paddingInlineEnd',
    'padding-inline-end',
    'calc(calc(8px))',
    'banana',
    'padding-inline-end',
    'calc(8px)',
    'calc(8px)'
  );

  return failures.length ? failures.join('|') : 'PASS';
})()
"#,
        )
        .expect("logical box CSSOM writes should use the PDB boundary");

    assert_eq!(result, "PASS");
}
#[test]
fn numeric_standard_cssom_writes_use_pdb_boundary() {
    let mut vm = new_storage_test_vm("https://numeric-standard-cssom-pdb-boundary.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, value) => {
    if (!value) failures.push(`${label}:${value}`);
  };
  const cases = [
    ['backgroundSize', 'background-size', '10px 20px', '1px 2px 3px', '10px 20px'],
    ['blockSize', 'block-size', 'calc(calc(12px))', 'banana', 'calc(12px)'],
    ['letterSpacing', 'letter-spacing', 'clamp(1px,2px,3px)', '1px 2px', 'calc(2px)'],
    ['opacity', 'opacity', '0.5', 'banana', '0.5'],
    ['rotate', 'rotate', '45deg', '1px', '45deg'],
    ['scale', 'scale', '2', 'banana', '2'],
    ['tabSize', 'tab-size', '4', '-1', '4'],
    ['textIndent', 'text-indent', 'calc(calc(12px))', 'banana', 'calc(12px)'],
    ['zIndex', 'z-index', '3', '1.5', '3']
  ];

  const check = (style, label, idl, property, valid, invalid, expected) => {
    ok(`${label}-${property}-idl`, idl in style);
    ok(`${label}-${property}-kebab`, property in style);

    style[idl] = invalid;
    eq(`${label}-${property}-invalid-empty-length`, style.length, 0);
    eq(`${label}-${property}-invalid-empty-query`, style.getPropertyValue(property), '');
    eq(`${label}-${property}-invalid-empty-own`, Object.prototype.hasOwnProperty.call(style, idl), false);

    style[idl] = valid;
    eq(`${label}-${property}-query`, style.getPropertyValue(property), expected);
    eq(`${label}-${property}-idl-get`, style[idl], expected);
    ok(`${label}-${property}-cssText`, style.cssText.includes(`${property}: ${expected};`));

    style[idl] = invalid;
    eq(`${label}-${property}-invalid-preserves-query`, style.getPropertyValue(property), expected);
    eq(`${label}-${property}-invalid-preserves-own`, Object.prototype.hasOwnProperty.call(style, idl), false);
  };

  for (const [idl, property, valid, invalid, expected] of cases) {
    const live = document.createElement('div');
    (document.body || document.documentElement || document).appendChild(live);
    check(live.style, 'live', idl, property, valid, invalid, expected);

    const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
    check(doc.createElement('div').style, 'detached', idl, property, valid, invalid, expected);

    const sheet = new CSSStyleSheet();
    sheet.insertRule('div {}');
    check(sheet.cssRules[0].style, 'rule', idl, property, valid, invalid, expected);
  }

  return failures.length ? failures.slice(0, 30).join('|') : 'PASS';
})()
"#,
        )
        .expect("numeric standard CSSOM writes should use the PDB boundary");

    assert_eq!(result, "PASS");
}
#[test]
fn border_image_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://border-image-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const borderImageLonghands = [
    'border-image-outset',
    'border-image-repeat',
    'border-image-slice',
    'border-image-source',
    'border-image-width'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };
  const lacksAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (actual.includes(name)) failures.push(`${label}:unexpected:${name}:${actual.join(',')}`);
    }
  };

  function exerciseBorderImage(style, label, ruleText) {
    style.setProperty('border-image', 'url("img.png") 30 / 2 / 1 round', 'important');
    eq(`${label}-value`, style.getPropertyValue('border-image'), 'url("img.png") 30 / 2 / 1 round');
    eq(`${label}-priority`, style.getPropertyPriority('border-image'), 'important');
    eq(`${label}-source`, style.getPropertyValue('border-image-source'), 'url("img.png")');
    eq(`${label}-slice`, style.getPropertyValue('border-image-slice'), '30');
    eq(`${label}-width`, style.getPropertyValue('border-image-width'), '2');
    eq(`${label}-outset`, style.getPropertyValue('border-image-outset'), '1');
    eq(`${label}-repeat`, style.getPropertyValue('border-image-repeat'), 'round');
    hasAll(`${label}-names`, style, borderImageLonghands);
    ok(`${label}-length`, style.length >= borderImageLonghands.length);
    ok(`${label}-cssText`, style.cssText.includes('border-image: url("img.png") 30 / 2 / 1 round !important;'));
    if (ruleText) {
      ok(`${label}-rule-cssText`, ruleText().includes('border-image: url("img.png") 30 / 2 / 1 round !important;'));
    }

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    const removed = style.removeProperty('border-image');
    eq(`${label}-removed`, removed, 'url("img.png") 30 / 2 / 1 round');
    eq(`${label}-after-remove`, style.getPropertyValue('border-image'), '');
    eq(`${label}-source-after-remove`, style.getPropertyValue('border-image-source'), '');
    lacksAll(`${label}-names-after-remove`, style, borderImageLonghands);
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('border-image', 'url("img.png") 30 / 2 / 1 round');
    style.setProperty('border', '1px solid red');
    eq(`${label}-border-reset`, style.getPropertyValue('border-image'), 'none');
    eq(`${label}-border-source-reset`, style.getPropertyValue('border-image-source'), 'none');
    eq(`${label}-border-slice-reset`, style.getPropertyValue('border-image-slice'), '100%');
    eq(`${label}-border-width-reset`, style.getPropertyValue('border-image-width'), '1');
    eq(`${label}-border-outset-reset`, style.getPropertyValue('border-image-outset'), '0');
    eq(`${label}-border-repeat-reset`, style.getPropertyValue('border-image-repeat'), 'stretch');
  }

  const inline = document.createElement('div').style;
  exerciseBorderImage(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  exerciseBorderImage(detached, 'detached');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  exerciseBorderImage(rule.style, 'rule', () => rule.cssText);

  const keyframe = sheet.cssRules[1].cssRules[0];
  exerciseBorderImage(keyframe.style, 'keyframe', () => keyframe.cssText);

  return failures.length ? failures.slice(0, 16).join('|') : 'PASS';
})()
"#,
        )
        .expect("border-image shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}
#[test]
fn border_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://border-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const borderLonghands = [
    'border-top-width',
    'border-right-width',
    'border-bottom-width',
    'border-left-width',
    'border-top-style',
    'border-right-style',
    'border-bottom-style',
    'border-left-style',
    'border-top-color',
    'border-right-color',
    'border-bottom-color',
    'border-left-color'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };
  const lacks = (label, style, name) => {
    const actual = names(style);
    if (actual.includes(name)) failures.push(`${label}:unexpected:${name}:${actual.join(',')}`);
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('border-image', 'url("img.png") 30');
    style.setProperty('border', '1px solid red', 'important');
    eq(`${label}-border`, style.getPropertyValue('border'), '1px solid red');
    eq(`${label}-border-priority`, style.getPropertyPriority('border'), 'important');
    eq(`${label}-top-width`, style.getPropertyValue('border-top-width'), '1px');
    eq(`${label}-right-style`, style.getPropertyValue('border-right-style'), 'solid');
    eq(`${label}-bottom-color`, style.getPropertyValue('border-bottom-color'), 'red');
    eq(`${label}-border-image-reset`, style.getPropertyValue('border-image'), 'none');
    hasAll(`${label}-border-names`, style, borderLonghands);
    ok(`${label}-border-image-name`, names(style).includes('border-image-source'));
    ok(`${label}-border-cssText`, style.cssText.includes('border: 1px solid red !important;'));

    style.setProperty('--token', 'value');
    const borderBeforeRemove = style.getPropertyValue('border');
    style.setProperty('-webkit-text-fill-color', 'red');
    const removedBorder = style.removeProperty('border');
    eq(`${label}-border-before-remove`, borderBeforeRemove, '1px solid red');
    eq(`${label}-removed-border`, removedBorder, '1px solid red');
    eq(`${label}-border-after-remove`, style.getPropertyValue('border'), '');
    eq(`${label}-top-width-after-remove`, style.getPropertyValue('border-top-width'), '');
    eq(`${label}-border-image-after-remove`, style.getPropertyValue('border-image'), '');
    lacks(`${label}-border-image-name-after-remove`, style, 'border-image-source');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'border-image: url("img.png") 30; border: 2px dashed blue !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-border', detached.getPropertyValue('border'), '2px dashed blue');
  eq('detached-border-priority', detached.getPropertyPriority('border'), 'important');
  eq('detached-right-style', detached.getPropertyValue('border-right-style'), 'dashed');
  eq('detached-left-color', detached.getPropertyValue('border-left-color'), 'blue');
  eq('detached-border-image-reset', detached.getPropertyValue('border-image'), 'none');
  hasAll('detached-border-names', detached, borderLonghands);
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedDetached = detached.removeProperty('border');
  eq('detached-removed-border', removedDetached, '2px dashed blue');
  eq('detached-border-after-remove', detached.getPropertyValue('border'), '');
  eq('detached-border-image-after-remove', detached.getPropertyValue('border-image'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('border-image', 'url("img.png") 30');
  rule.style.setProperty('border', '3px dotted green', 'important');
  eq('rule-border', rule.style.getPropertyValue('border'), '3px dotted green');
  eq('rule-border-priority', rule.style.getPropertyPriority('border'), 'important');
  eq('rule-border-image-reset', rule.style.getPropertyValue('border-image'), 'none');
  hasAll('rule-border-names', rule.style, borderLonghands);
  ok('rule-cssText-border', rule.cssText.includes('border: 3px dotted green !important;'));
  rule.style.setProperty('--token', 'value');
  const removedRuleBorder = rule.style.removeProperty('border');
  eq('rule-removed-border', removedRuleBorder, '3px dotted green');
  eq('rule-border-after-remove', rule.style.getPropertyValue('border'), '');
  eq('rule-border-image-after-remove', rule.style.getPropertyValue('border-image'), '');
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));
  ok('rule-cssText-border-removed', !rule.cssText.includes('border: 3px dotted green'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('border-image', 'url("img.png") 30');
  keyframe.style.setProperty('border', '4px double purple', 'important');
  eq('keyframe-border', keyframe.style.getPropertyValue('border'), '4px double purple');
  eq('keyframe-border-priority', keyframe.style.getPropertyPriority('border'), 'important');
  eq('keyframe-border-image-reset', keyframe.style.getPropertyValue('border-image'), 'none');
  hasAll('keyframe-border-names', keyframe.style, borderLonghands);
  ok('keyframe-cssText-border', keyframe.cssText.includes('border: 4px double purple !important;'));

  return failures.length ? failures.slice(0, 12).join('|') : 'PASS';
})()
"#,
        )
        .expect("border shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}
#[test]
fn border_component_shorthands_use_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://border-component-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const colorLonghands = [
    'border-top-color',
    'border-right-color',
    'border-bottom-color',
    'border-left-color'
  ];
  const styleLonghands = [
    'border-top-style',
    'border-right-style',
    'border-bottom-style',
    'border-left-style'
  ];
  const widthLonghands = [
    'border-top-width',
    'border-right-width',
    'border-bottom-width',
    'border-left-width'
  ];
  const sideLonghands = side => [
    `border-${side}-width`,
    `border-${side}-style`,
    `border-${side}-color`
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('border-color', 'red blue', 'important');
    eq(`${label}-border-color`, style.getPropertyValue('border-color'), 'red blue');
    eq(`${label}-border-color-priority`, style.getPropertyPriority('border-color'), 'important');
    eq(`${label}-border-top-color`, style.getPropertyValue('border-top-color'), 'red');
    eq(`${label}-border-right-color`, style.getPropertyValue('border-right-color'), 'blue');
    hasAll(`${label}-color-names`, style, colorLonghands);
    ok(`${label}-color-cssText`, style.cssText.includes('border-color: red blue !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('border-width', '1px 2px 3px', 'important');
    eq(`${label}-border-width`, style.getPropertyValue('border-width'), '1px 2px 3px');
    eq(`${label}-border-bottom-width`, style.getPropertyValue('border-bottom-width'), '3px');
    eq(`${label}-border-left-width`, style.getPropertyValue('border-left-width'), '2px');
    eq(`${label}-border-width-priority`, style.getPropertyPriority('border-left-width'), 'important');
    hasAll(`${label}-width-names`, style, widthLonghands);
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    const removedWidth = style.removeProperty('border-width');
    eq(`${label}-removed-width`, removedWidth, '1px 2px 3px');
    eq(`${label}-border-width-after-remove`, style.getPropertyValue('border-width'), '');
    eq(`${label}-border-color-after-remove`, style.getPropertyValue('border-color'), 'red blue');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('border-top', '4px dashed green', 'important');
    eq(`${label}-border-side-top`, style.getPropertyValue('border-top'), '4px dashed green');
    eq(`${label}-border-side-top-width`, style.getPropertyValue('border-top-width'), '4px');
    eq(`${label}-border-side-top-style`, style.getPropertyValue('border-top-style'), 'dashed');
    eq(`${label}-border-side-top-color`, style.getPropertyValue('border-top-color'), 'green');
    eq(`${label}-border-side-top-priority`, style.getPropertyPriority('border-top'), 'important');
    hasAll(`${label}-border-top-names`, style, sideLonghands('top'));
    ok(`${label}-border-top-cssText`, style.cssText.includes('border-top: 4px dashed green !important;'));
    const removedTop = style.removeProperty('border-top');
    eq(`${label}-removed-top`, removedTop, '4px dashed green');
    eq(`${label}-border-top-after-remove`, style.getPropertyValue('border-top'), '');
    eq(`${label}-token-after-top-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-top-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'border-style: solid dotted !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-border-style', detached.getPropertyValue('border-style'), 'solid dotted');
  eq('detached-border-style-priority', detached.getPropertyPriority('border-style'), 'important');
  eq('detached-border-right-style', detached.getPropertyValue('border-right-style'), 'dotted');
  hasAll('detached-style-names', detached, styleLonghands);
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedStyle = detached.removeProperty('border-style');
  eq('detached-removed-style', removedStyle, 'solid dotted');
  eq('detached-border-style-after-remove', detached.getPropertyValue('border-style'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  detached.setProperty('border-right', '2px dashed green', 'important');
  eq('detached-border-right', detached.getPropertyValue('border-right'), '2px dashed green');
  eq('detached-border-right-style', detached.getPropertyValue('border-right-style'), 'dashed');
  hasAll('detached-right-names', detached, sideLonghands('right'));
  ok('detached-right-cssText', detached.cssText.includes('border-right: 2px dashed green !important;'));

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('border-style', 'solid dotted', 'important');
  eq('rule-border-style', rule.style.getPropertyValue('border-style'), 'solid dotted');
  eq('rule-border-style-priority', rule.style.getPropertyPriority('border-style'), 'important');
  hasAll('rule-style-names', rule.style, styleLonghands);
  ok('rule-cssText-style', rule.cssText.includes('border-style: solid dotted !important;'));
  rule.style.setProperty('--token', 'value');
  rule.style.setProperty('border-color', 'green');
  eq('rule-border-color', rule.style.getPropertyValue('border-color'), 'green');
  eq('rule-token', rule.style.getPropertyValue('--token'), 'value');
  const removedRuleStyle = rule.style.removeProperty('border-style');
  eq('rule-removed-style', removedRuleStyle, 'solid dotted');
  eq('rule-border-style-after-remove', rule.style.getPropertyValue('border-style'), '');
  eq('rule-border-color-after-remove', rule.style.getPropertyValue('border-color'), 'green');
  ok('rule-cssText-color', rule.cssText.includes('border-color: green;'));
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));
  ok('rule-cssText-style-removed', !rule.cssText.includes('border-style'));
  rule.style.setProperty('border-bottom', '3px double blue', 'important');
  eq('rule-border-bottom', rule.style.getPropertyValue('border-bottom'), '3px double blue');
  eq('rule-border-bottom-style', rule.style.getPropertyValue('border-bottom-style'), 'double');
  hasAll('rule-bottom-names', rule.style, sideLonghands('bottom'));
  ok('rule-cssText-bottom', rule.cssText.includes('border-bottom: 3px double blue !important;'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('border-width', '4px 5px', 'important');
  eq('keyframe-border-width', keyframe.style.getPropertyValue('border-width'), '4px 5px');
  eq('keyframe-border-left-width', keyframe.style.getPropertyValue('border-left-width'), '5px');
  eq('keyframe-border-width-priority', keyframe.style.getPropertyPriority('border-width'), 'important');
  hasAll('keyframe-width-names', keyframe.style, widthLonghands);
  ok('keyframe-cssText-width', keyframe.cssText.includes('border-width: 4px 5px !important;'));
  keyframe.style.setProperty('border-left', '6px solid purple', 'important');
  eq('keyframe-border-left', keyframe.style.getPropertyValue('border-left'), '6px solid purple');
  eq('keyframe-border-left-width', keyframe.style.getPropertyValue('border-left-width'), '6px');
  hasAll('keyframe-left-names', keyframe.style, sideLonghands('left'));
  ok('keyframe-cssText-left', keyframe.cssText.includes('border-left: 6px solid purple !important;'));

  return failures.length ? failures.slice(0, 8).join('|') : 'PASS';
})()
"#,
        )
        .expect("border component shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}
#[test]
fn border_radius_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://border-radius-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const radiusLonghands = [
    'border-top-left-radius',
    'border-top-right-radius',
    'border-bottom-right-radius',
    'border-bottom-left-radius'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('border-radius', '1px 2px', 'important');
    eq(`${label}-radius`, style.getPropertyValue('border-radius'), '1px 2px');
    eq(`${label}-radius-priority`, style.getPropertyPriority('border-radius'), 'important');
    eq(`${label}-top-left`, style.getPropertyValue('border-top-left-radius'), '1px');
    eq(`${label}-top-right`, style.getPropertyValue('border-top-right-radius'), '2px');
    hasAll(`${label}-radius-names`, style, radiusLonghands);
    ok(`${label}-radius-cssText`, style.cssText.includes('border-radius: 1px 2px !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    const removedRadius = style.removeProperty('border-radius');
    eq(`${label}-removed-radius`, removedRadius, '1px 2px');
    eq(`${label}-radius-after-remove`, style.getPropertyValue('border-radius'), '');
    eq(`${label}-top-left-after-remove`, style.getPropertyValue('border-top-left-radius'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'border-radius: 3px 4px !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-radius', detached.getPropertyValue('border-radius'), '3px 4px');
  eq('detached-radius-priority', detached.getPropertyPriority('border-radius'), 'important');
  eq('detached-top-right', detached.getPropertyValue('border-top-right-radius'), '4px');
  hasAll('detached-radius-names', detached, radiusLonghands);
  ok('detached-radius-cssText', detached.cssText.includes('border-radius: 3px 4px !important;'));
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedDetached = detached.removeProperty('border-radius');
  eq('detached-removed-radius', removedDetached, '3px 4px');
  eq('detached-radius-after-remove', detached.getPropertyValue('border-radius'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('border-radius', '5px 6px', 'important');
  eq('rule-radius', rule.style.getPropertyValue('border-radius'), '5px 6px');
  eq('rule-radius-priority', rule.style.getPropertyPriority('border-radius'), 'important');
  eq('rule-top-left', rule.style.getPropertyValue('border-top-left-radius'), '5px');
  hasAll('rule-radius-names', rule.style, radiusLonghands);
  ok('rule-cssText-radius', rule.cssText.includes('border-radius: 5px 6px !important;'));
  const removedRuleRadius = rule.style.removeProperty('border-radius');
  eq('rule-removed-radius', removedRuleRadius, '5px 6px');
  eq('rule-radius-after-remove', rule.style.getPropertyValue('border-radius'), '');
  ok('rule-cssText-radius-removed', !rule.cssText.includes('border-radius'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('border-radius', '7px 8px', 'important');
  eq('keyframe-radius', keyframe.style.getPropertyValue('border-radius'), '7px 8px');
  eq('keyframe-radius-priority', keyframe.style.getPropertyPriority('border-radius'), 'important');
  eq('keyframe-bottom-left', keyframe.style.getPropertyValue('border-bottom-left-radius'), '8px');
  hasAll('keyframe-radius-names', keyframe.style, radiusLonghands);
  ok('keyframe-cssText-radius', keyframe.cssText.includes('border-radius: 7px 8px !important;'));

  return failures.length ? failures.slice(0, 10).join('|') : 'PASS';
})()
"#,
        )
        .expect("border-radius shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}
#[test]
fn text_decoration_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://text-decoration-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const decorationLonghands = [
    'text-decoration-line',
    'text-decoration-thickness',
    'text-decoration-style',
    'text-decoration-color'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('text-decoration', 'overline from-font dotted green', 'important');
    eq(`${label}-decoration`, style.getPropertyValue('text-decoration'), 'overline from-font dotted green');
    eq(`${label}-decoration-priority`, style.getPropertyPriority('text-decoration'), 'important');
    eq(`${label}-line`, style.getPropertyValue('text-decoration-line'), 'overline');
    eq(`${label}-thickness`, style.getPropertyValue('text-decoration-thickness'), 'from-font');
    eq(`${label}-style`, style.getPropertyValue('text-decoration-style'), 'dotted');
    eq(`${label}-color`, style.getPropertyValue('text-decoration-color'), 'green');
    hasAll(`${label}-decoration-names`, style, decorationLonghands);
    ok(`${label}-decoration-cssText`, style.cssText.includes('text-decoration: overline from-font dotted green !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('text-decoration-skip-ink', 'all', 'important');
    eq(`${label}-skip-ink`, style.getPropertyValue('text-decoration-skip-ink'), 'all');
    eq(`${label}-skip-ink-priority`, style.getPropertyPriority('text-decoration-skip-ink'), 'important');
    ok(`${label}-skip-ink-name`, names(style).includes('text-decoration-skip-ink'));
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('text-decoration-line', 'spelling-error', 'important');
    eq(`${label}-compat-line`, style.getPropertyValue('text-decoration-line'), 'spelling-error');
    eq(`${label}-compat-line-priority`, style.getPropertyPriority('text-decoration-line'), 'important');
    eq(`${label}-compat-decoration`, style.getPropertyValue('text-decoration'), 'spelling-error from-font dotted green');
    eq(`${label}-skip-ink-after-compat-line`, style.getPropertyValue('text-decoration-skip-ink'), '');
    ok(`${label}-compat-line-name`, names(style).includes('text-decoration-line'));
    ok(`${label}-compat-cssText`, style.cssText.includes('text-decoration-line: spelling-error !important;'));
    style.textDecorationStyle = 'solid wavy';
    eq(`${label}-compat-style-after-invalid-idl`, style.getPropertyValue('text-decoration-style'), 'dotted');
    eq(`${label}-compat-decoration-after-invalid-idl`, style.getPropertyValue('text-decoration'), 'spelling-error from-font dotted green');

    const removedDecoration = style.removeProperty('text-decoration');
    eq(`${label}-removed-decoration`, removedDecoration, '');
    eq(`${label}-decoration-after-remove`, style.getPropertyValue('text-decoration'), '');
    eq(`${label}-line-after-remove`, style.getPropertyValue('text-decoration-line'), '');
    eq(`${label}-skip-ink-after-remove`, style.getPropertyValue('text-decoration-skip-ink'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'text-decoration: overline from-font dotted green !important; text-decoration-skip-ink: all; --token: value; -webkit-text-fill-color: red;';
  eq('detached-decoration', detached.getPropertyValue('text-decoration'), 'overline from-font dotted green');
  eq('detached-decoration-priority', detached.getPropertyPriority('text-decoration'), 'important');
  eq('detached-skip-ink', detached.getPropertyValue('text-decoration-skip-ink'), 'all');
  hasAll('detached-decoration-names', detached, decorationLonghands);
  ok('detached-decoration-cssText', detached.cssText.includes('text-decoration: overline from-font dotted green !important;'));
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  detached.setProperty('text-decoration-line', 'grammar-error', 'important');
  eq('detached-compat-line', detached.getPropertyValue('text-decoration-line'), 'grammar-error');
  eq('detached-compat-decoration', detached.getPropertyValue('text-decoration'), 'grammar-error from-font dotted green');
  eq('detached-skip-ink-after-compat-line', detached.getPropertyValue('text-decoration-skip-ink'), '');
  ok('detached-compat-cssText', detached.cssText.includes('text-decoration-line: grammar-error !important;'));
  detached.textDecorationStyle = 'solid wavy';
  eq('detached-compat-style-after-invalid-idl', detached.getPropertyValue('text-decoration-style'), 'dotted');
  eq('detached-compat-decoration-after-invalid-idl', detached.getPropertyValue('text-decoration'), 'grammar-error from-font dotted green');
  const removedDetached = detached.removeProperty('text-decoration');
  eq('detached-removed-decoration', removedDetached, '');
  eq('detached-decoration-after-remove', detached.getPropertyValue('text-decoration'), '');
  eq('detached-compat-line-after-remove', detached.getPropertyValue('text-decoration-line'), '');
  eq('detached-skip-ink-after-remove', detached.getPropertyValue('text-decoration-skip-ink'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('text-decoration', 'underline auto dashed blue', 'important');
  eq('rule-decoration', rule.style.getPropertyValue('text-decoration'), 'underline dashed blue');
  eq('rule-decoration-priority', rule.style.getPropertyPriority('text-decoration'), 'important');
  eq('rule-line', rule.style.getPropertyValue('text-decoration-line'), 'underline');
  eq('rule-style', rule.style.getPropertyValue('text-decoration-style'), 'dashed');
  eq('rule-color', rule.style.getPropertyValue('text-decoration-color'), 'blue');
  hasAll('rule-decoration-names', rule.style, decorationLonghands);
  ok('rule-cssText-decoration', rule.cssText.includes('text-decoration: underline dashed blue !important;'));
  rule.style.setProperty('text-decoration-line', 'spelling-error', 'important');
  eq('rule-compat-line', rule.style.getPropertyValue('text-decoration-line'), 'spelling-error');
  eq('rule-compat-decoration', rule.style.getPropertyValue('text-decoration'), 'spelling-error dashed blue');
  ok('rule-cssText-compat-line', rule.cssText.includes('text-decoration: spelling-error dashed blue !important;'));
  rule.style.setProperty('--token', 'value');
  const removedRuleDecoration = rule.style.removeProperty('text-decoration');
  eq('rule-removed-decoration', removedRuleDecoration, '');
  eq('rule-decoration-after-remove', rule.style.getPropertyValue('text-decoration'), '');
  eq('rule-compat-line-after-remove', rule.style.getPropertyValue('text-decoration-line'), '');
  ok('rule-cssText-decoration-removed', !rule.cssText.includes('text-decoration: underline dashed blue'));
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('text-decoration', 'line-through 2px wavy red', 'important');
  eq('keyframe-decoration', keyframe.style.getPropertyValue('text-decoration'), 'line-through 2px wavy red');
  eq('keyframe-decoration-priority', keyframe.style.getPropertyPriority('text-decoration'), 'important');
  eq('keyframe-line', keyframe.style.getPropertyValue('text-decoration-line'), 'line-through');
  eq('keyframe-thickness', keyframe.style.getPropertyValue('text-decoration-thickness'), '2px');
  eq('keyframe-style', keyframe.style.getPropertyValue('text-decoration-style'), 'wavy');
  eq('keyframe-color', keyframe.style.getPropertyValue('text-decoration-color'), 'red');
  hasAll('keyframe-decoration-names', keyframe.style, decorationLonghands);
  ok('keyframe-cssText-decoration', keyframe.cssText.includes('text-decoration: line-through 2px wavy red !important;'));

  return failures.length ? failures.slice(0, 10).join('|') : 'PASS';
})()
"#,
        )
        .expect("text-decoration shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}
#[test]
fn text_emphasis_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://text-emphasis-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const emphasisLonghands = ['text-emphasis-style', 'text-emphasis-color'];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('text-emphasis', 'dot red', 'important');
    eq(`${label}-emphasis`, style.getPropertyValue('text-emphasis'), 'dot red');
    eq(`${label}-emphasis-priority`, style.getPropertyPriority('text-emphasis'), 'important');
    eq(`${label}-style`, style.getPropertyValue('text-emphasis-style'), 'dot');
    eq(`${label}-color`, style.getPropertyValue('text-emphasis-color'), 'red');
    hasAll(`${label}-emphasis-names`, style, emphasisLonghands);
    ok(`${label}-emphasis-cssText`, style.cssText.includes('text-emphasis: dot red !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('text-emphasis-position', 'over left', 'important');
    eq(`${label}-position`, style.getPropertyValue('text-emphasis-position'), 'over left');
    eq(`${label}-position-priority`, style.getPropertyPriority('text-emphasis-position'), 'important');
    ok(`${label}-position-name`, names(style).includes('text-emphasis-position'));
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    const removedEmphasis = style.removeProperty('text-emphasis');
    eq(`${label}-removed-emphasis`, removedEmphasis, 'dot red');
    eq(`${label}-emphasis-after-remove`, style.getPropertyValue('text-emphasis'), '');
    eq(`${label}-style-after-remove`, style.getPropertyValue('text-emphasis-style'), '');
    eq(`${label}-position-after-remove`, style.getPropertyValue('text-emphasis-position'), 'over left');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'text-emphasis: dot red !important; text-emphasis-position: over left; --token: value; -webkit-text-fill-color: red;';
  eq('detached-emphasis', detached.getPropertyValue('text-emphasis'), 'dot red');
  eq('detached-emphasis-priority', detached.getPropertyPriority('text-emphasis'), 'important');
  eq('detached-position', detached.getPropertyValue('text-emphasis-position'), 'over left');
  hasAll('detached-emphasis-names', detached, emphasisLonghands);
  ok('detached-emphasis-cssText', detached.cssText.includes('text-emphasis: dot red !important;'));
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedDetached = detached.removeProperty('text-emphasis');
  eq('detached-removed-emphasis', removedDetached, 'dot red');
  eq('detached-emphasis-after-remove', detached.getPropertyValue('text-emphasis'), '');
  eq('detached-position-after-remove', detached.getPropertyValue('text-emphasis-position'), 'over left');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('text-emphasis', 'circle blue', 'important');
  eq('rule-emphasis', rule.style.getPropertyValue('text-emphasis'), 'circle blue');
  eq('rule-emphasis-priority', rule.style.getPropertyPriority('text-emphasis'), 'important');
  eq('rule-style', rule.style.getPropertyValue('text-emphasis-style'), 'circle');
  eq('rule-color', rule.style.getPropertyValue('text-emphasis-color'), 'blue');
  hasAll('rule-emphasis-names', rule.style, emphasisLonghands);
  ok('rule-cssText-emphasis', rule.cssText.includes('text-emphasis: circle blue !important;'));
  rule.style.setProperty('--token', 'value');
  const removedRuleEmphasis = rule.style.removeProperty('text-emphasis');
  eq('rule-removed-emphasis', removedRuleEmphasis, 'circle blue');
  eq('rule-emphasis-after-remove', rule.style.getPropertyValue('text-emphasis'), '');
  ok('rule-cssText-emphasis-removed', !rule.cssText.includes('text-emphasis: circle blue'));
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('text-emphasis', 'sesame green', 'important');
  eq('keyframe-emphasis', keyframe.style.getPropertyValue('text-emphasis'), 'sesame green');
  eq('keyframe-emphasis-priority', keyframe.style.getPropertyPriority('text-emphasis'), 'important');
  eq('keyframe-style', keyframe.style.getPropertyValue('text-emphasis-style'), 'sesame');
  eq('keyframe-color', keyframe.style.getPropertyValue('text-emphasis-color'), 'green');
  hasAll('keyframe-emphasis-names', keyframe.style, emphasisLonghands);
  ok('keyframe-cssText-emphasis', keyframe.cssText.includes('text-emphasis: sesame green !important;'));

  return failures.length ? failures.slice(0, 10).join('|') : 'PASS';
})()
"#,
        )
        .expect("text-emphasis shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}
#[test]
fn transition_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://transition-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const transitionLonghands = [
    'transition-property',
    'transition-duration',
    'transition-timing-function',
    'transition-delay',
    'transition-behavior'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('transition', 'display 3s ease-in-out 1s allow-discrete, opacity', 'important');
    eq(`${label}-transition`, style.getPropertyValue('transition'), 'display 3s ease-in-out 1s allow-discrete, opacity');
    eq(`${label}-transition-priority`, style.getPropertyPriority('transition'), 'important');
    eq(`${label}-property`, style.getPropertyValue('transition-property'), 'display, opacity');
    eq(`${label}-duration`, style.getPropertyValue('transition-duration'), '3s, 0s');
    eq(`${label}-timing`, style.getPropertyValue('transition-timing-function'), 'ease-in-out, ease');
    eq(`${label}-delay`, style.getPropertyValue('transition-delay'), '1s, 0s');
    eq(`${label}-behavior`, style.getPropertyValue('transition-behavior'), 'allow-discrete, normal');
    eq(`${label}-duration-priority`, style.getPropertyPriority('transition-duration'), 'important');
    hasAll(`${label}-transition-names`, style, transitionLonghands);
    ok(`${label}-transition-cssText`, style.cssText.includes('transition: display 3s ease-in-out 1s allow-discrete, opacity !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('transition-duration', '4s, 5s', 'important');
    eq(`${label}-transition-after-duration`, style.getPropertyValue('transition'), 'display 4s ease-in-out 1s allow-discrete, opacity 5s');
    ok(`${label}-cssText-after-duration`, style.cssText.includes('transition: display 4s ease-in-out 1s allow-discrete, opacity 5s !important;'));
    ok(`${label}-cssText-side-after-duration`, style.cssText.includes('-webkit-text-fill-color: red;'));
    eq(`${label}-duration-after-duration`, style.getPropertyValue('transition-duration'), '4s, 5s');
    eq(`${label}-duration-priority-after-duration`, style.getPropertyPriority('transition-duration'), 'important');
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    const removedTransition = style.removeProperty('transition');
    eq(`${label}-removed-transition`, removedTransition, 'display 4s ease-in-out 1s allow-discrete, opacity 5s');
    eq(`${label}-transition-after-remove`, style.getPropertyValue('transition'), '');
    eq(`${label}-property-after-remove`, style.getPropertyValue('transition-property'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('transition-duration', 'calc(10s + (sign(2cqw - 10px) * 5s))');
    style.setProperty('transition-timing-function', 'steps(calc(2 * sibling-index()), jump-none)');
    eq(`${label}-dynamic-duration`, style.getPropertyValue('transition-duration'), 'calc(10s + (5s * sign(2cqw - 10px)))');
    eq(`${label}-dynamic-timing`, style.getPropertyValue('transition-timing-function'), 'steps(calc(2 * sibling-index()), jump-none)');
    ok(`${label}-dynamic-duration-name`, names(style).includes('transition-duration'));
    ok(`${label}-dynamic-timing-name`, names(style).includes('transition-timing-function'));
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'transition: display 3s ease-in-out 1s allow-discrete, opacity !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-transition', detached.getPropertyValue('transition'), 'display 3s ease-in-out 1s allow-discrete, opacity');
  eq('detached-transition-priority', detached.getPropertyPriority('transition'), 'important');
  eq('detached-property', detached.getPropertyValue('transition-property'), 'display, opacity');
  eq('detached-duration', detached.getPropertyValue('transition-duration'), '3s, 0s');
  hasAll('detached-transition-names', detached, transitionLonghands);
  ok('detached-transition-cssText', detached.cssText.includes('transition: display 3s ease-in-out 1s allow-discrete, opacity !important;'));
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-kebab', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  eq('detached-webkit-lower', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedDetached = detached.removeProperty('transition-duration');
  eq('detached-removed-duration', removedDetached, '3s, 0s');
  eq('detached-transition-after-duration-remove', detached.getPropertyValue('transition'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const dynamicDetached = detachedDoc.createElement('div').style;
  dynamicDetached.setProperty('transition-duration', 'calc(10s + (sign(2cqw - 10px) * 5s))');
  dynamicDetached.setProperty('transition-timing-function', 'steps(calc(2 * sibling-index()), jump-none)');
  eq('detached-dynamic-duration', dynamicDetached.getPropertyValue('transition-duration'), 'calc(10s + (5s * sign(2cqw - 10px)))');
  eq('detached-dynamic-timing', dynamicDetached.getPropertyValue('transition-timing-function'), 'steps(calc(2 * sibling-index()), jump-none)');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('transition', 'top 1s cubic-bezier(0, -2, 1, 3) -3s', 'important');
  eq('rule-transition', rule.style.getPropertyValue('transition'), 'top 1s cubic-bezier(0, -2, 1, 3) -3s');
  eq('rule-transition-priority', rule.style.getPropertyPriority('transition'), 'important');
  eq('rule-property', rule.style.getPropertyValue('transition-property'), 'top');
  eq('rule-delay', rule.style.getPropertyValue('transition-delay'), '-3s');
  hasAll('rule-transition-names', rule.style, transitionLonghands);
  ok('rule-cssText-transition', rule.cssText.includes('transition: top 1s cubic-bezier(0, -2, 1, 3) -3s !important;'));
  rule.style.setProperty('--token', 'value');
  const removedRuleTransition = rule.style.removeProperty('transition');
  eq('rule-removed-transition', removedRuleTransition, 'top 1s cubic-bezier(0, -2, 1, 3) -3s');
  eq('rule-transition-after-remove', rule.style.getPropertyValue('transition'), '');
  ok('rule-cssText-transition-removed', !rule.cssText.includes('transition: top 1s'));
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));
  rule.style.transitionTimingFunction = 'steps(calc(2 * sibling-index()), jump-none)';
  eq('rule-dynamic-timing', rule.style.getPropertyValue('transition-timing-function'), 'steps(calc(2 * sibling-index()), jump-none)');

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('transition', 'opacity 2s steps(2) 1s', 'important');
  eq('keyframe-transition', keyframe.style.getPropertyValue('transition'), 'opacity 2s steps(2) 1s');
  eq('keyframe-transition-priority', keyframe.style.getPropertyPriority('transition'), 'important');
  eq('keyframe-property', keyframe.style.getPropertyValue('transition-property'), 'opacity');
  eq('keyframe-timing', keyframe.style.getPropertyValue('transition-timing-function'), 'steps(2)');
  hasAll('keyframe-transition-names', keyframe.style, transitionLonghands);
  ok('keyframe-cssText-transition', keyframe.cssText.includes('transition: opacity 2s steps(2) 1s !important;'));
  const dynamicKeyframe = sheet.cssRules[1].cssRules[0];
  dynamicKeyframe.style.removeProperty('transition');
  dynamicKeyframe.style.transitionDuration = 'calc(10s + (sign(2cqw - 10px) * 5s))';
  eq('keyframe-dynamic-duration', dynamicKeyframe.style.getPropertyValue('transition-duration'), 'calc(10s + (5s * sign(2cqw - 10px)))');

  const invalid = document.createElement('div').style;
  invalid.transitionDuration = '1s';
  invalid.transitionDuration = '-2s';
  eq('invalid-duration-preserves-old', invalid.transitionDuration, '1s');

  return failures.length ? failures.slice(0, 10).join('|') : 'PASS';
})()
"#,
        )
        .expect("transition shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}
#[test]
fn animation_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://animation-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const animationLonghands = [
    'animation-duration',
    'animation-timing-function',
    'animation-delay',
    'animation-iteration-count',
    'animation-direction',
    'animation-fill-mode',
    'animation-play-state',
    'animation-name',
    'animation-timeline',
    'animation-range-start',
    'animation-range-end'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('animation', 'fade paused both reverse 3 1s 2s linear', 'important');
    eq(`${label}-animation`, style.getPropertyValue('animation'), '1s linear 2s 3 reverse both paused fade');
    eq(`${label}-animation-priority`, style.getPropertyPriority('animation'), 'important');
    eq(`${label}-duration`, style.getPropertyValue('animation-duration'), '1s');
    eq(`${label}-timing`, style.getPropertyValue('animation-timing-function'), 'linear');
    eq(`${label}-delay`, style.getPropertyValue('animation-delay'), '2s');
    eq(`${label}-iteration`, style.getPropertyValue('animation-iteration-count'), '3');
    eq(`${label}-direction`, style.getPropertyValue('animation-direction'), 'reverse');
    eq(`${label}-fill`, style.getPropertyValue('animation-fill-mode'), 'both');
    eq(`${label}-play`, style.getPropertyValue('animation-play-state'), 'paused');
    eq(`${label}-name`, style.getPropertyValue('animation-name'), 'fade');
    eq(`${label}-timeline`, style.getPropertyValue('animation-timeline'), 'auto');
    eq(`${label}-range-start`, style.getPropertyValue('animation-range-start'), 'normal');
    eq(`${label}-range-end`, style.getPropertyValue('animation-range-end'), 'normal');
    eq(`${label}-duration-priority`, style.getPropertyPriority('animation-duration'), 'important');
    hasAll(`${label}-animation-names`, style, animationLonghands);

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('animation-timing-function', 'linear(0, 1)', 'important');
    eq(`${label}-animation-after-timing`, style.getPropertyValue('animation'), '1s linear(0, 1) 2s 3 reverse both paused fade');
    eq(`${label}-timing-after-timing`, style.getPropertyValue('animation-timing-function'), 'linear(0, 1)');
    eq(`${label}-timing-priority-after-timing`, style.getPropertyPriority('animation-timing-function'), 'important');
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('animation-timing-function', 'ease-in-out', 'important');
    eq(`${label}-animation-after-ordinary-timing`, style.getPropertyValue('animation'), '1s ease-in-out 2s 3 reverse both paused fade');
    eq(`${label}-ordinary-timing`, style.getPropertyValue('animation-timing-function'), 'ease-in-out');
    eq(`${label}-ordinary-timing-priority`, style.getPropertyPriority('animation-timing-function'), 'important');
    ok(`${label}-ordinary-timing-name`, names(style).includes('animation-timing-function'));

    const removedAnimation = style.removeProperty('animation');
    eq(`${label}-removed-animation`, removedAnimation, '1s ease-in-out 2s 3 reverse both paused fade');
    eq(`${label}-animation-after-remove`, style.getPropertyValue('animation'), '');
    eq(`${label}-duration-after-remove`, style.getPropertyValue('animation-duration'), '');
    eq(`${label}-timing-after-remove`, style.getPropertyValue('animation-timing-function'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('animation-range', 'entry 10% exit 20%');
    eq(`${label}-range`, style.getPropertyValue('animation-range'), 'entry 10% exit 20%');
    eq(`${label}-range-start-after-range`, style.getPropertyValue('animation-range-start'), 'entry 10%');
    eq(`${label}-range-end-after-range`, style.getPropertyValue('animation-range-end'), 'exit 20%');
    const removedRange = style.removeProperty('animation-range');
    eq(`${label}-removed-range`, removedRange, 'entry 10% exit 20%');
    eq(`${label}-range-after-remove`, style.getPropertyValue('animation-range'), '');

    style.setProperty('animation-duration', 'calc(10s + (sign(2cqw - 10px) * 5s))');
    style.setProperty('animation-timing-function', 'steps(calc(2 * sibling-index()), jump-none)');
    eq(`${label}-dynamic-duration`, style.getPropertyValue('animation-duration'), 'calc(10s + (5s * sign(2cqw - 10px)))');
    eq(`${label}-dynamic-timing`, style.getPropertyValue('animation-timing-function'), 'steps(calc(2 * sibling-index()), jump-none)');
    ok(`${label}-dynamic-duration-name`, names(style).includes('animation-duration'));
    ok(`${label}-dynamic-timing-name`, names(style).includes('animation-timing-function'));
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'animation: fade paused both reverse 3 1s 2s linear !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-animation', detached.getPropertyValue('animation'), '1s linear 2s 3 reverse both paused fade');
  eq('detached-animation-priority', detached.getPropertyPriority('animation'), 'important');
  eq('detached-duration', detached.getPropertyValue('animation-duration'), '1s');
  eq('detached-timeline', detached.getPropertyValue('animation-timeline'), 'auto');
  hasAll('detached-animation-names', detached, animationLonghands);
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  detached.setProperty('animation-timing-function', 'linear(0, 1)', 'important');
  eq('detached-animation-after-timing', detached.getPropertyValue('animation'), '1s linear(0, 1) 2s 3 reverse both paused fade');
  eq('detached-timing-after-timing', detached.getPropertyValue('animation-timing-function'), 'linear(0, 1)');
  const removedDetached = detached.removeProperty('animation-duration');
  eq('detached-removed-duration', removedDetached, '1s');
  eq('detached-animation-after-duration-remove', detached.getPropertyValue('animation'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const dynamicDetached = detachedDoc.createElement('div').style;
  dynamicDetached.setProperty('animation-duration', 'calc(10s + (sign(2cqw - 10px) * 5s))');
  dynamicDetached.setProperty('animation-timing-function', 'steps(calc(2 * sibling-index()), jump-none)');
  eq('detached-dynamic-duration', dynamicDetached.getPropertyValue('animation-duration'), 'calc(10s + (5s * sign(2cqw - 10px)))');
  eq('detached-dynamic-timing', dynamicDetached.getPropertyValue('animation-timing-function'), 'steps(calc(2 * sibling-index()), jump-none)');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('animation', 'slide running forwards alternate 2 4s -1s ease-in', 'important');
  eq('rule-animation', rule.style.getPropertyValue('animation'), '4s ease-in -1s 2 alternate forwards slide');
  eq('rule-animation-priority', rule.style.getPropertyPriority('animation'), 'important');
  eq('rule-duration', rule.style.getPropertyValue('animation-duration'), '4s');
  eq('rule-delay', rule.style.getPropertyValue('animation-delay'), '-1s');
  eq('rule-timeline', rule.style.getPropertyValue('animation-timeline'), 'auto');
  hasAll('rule-animation-names', rule.style, animationLonghands);
  ok('rule-cssText-animation', rule.cssText.includes('animation: 4s ease-in -1s 2 alternate forwards slide !important;'));
  rule.style.setProperty('--token', 'value');
  rule.style.animationTimingFunction = 'linear(0, 1)';
  eq('rule-animation-after-timing', rule.style.getPropertyValue('animation'), '');
  eq('rule-timing-after-timing', rule.style.getPropertyValue('animation-timing-function'), 'linear(0, 1)');
  const removedRuleAnimation = rule.style.removeProperty('animation');
  eq('rule-removed-animation', removedRuleAnimation, '');
  eq('rule-animation-after-remove', rule.style.getPropertyValue('animation'), '');
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('animation', 'fade 1s linear', 'important');
  keyframe.style.animationDuration = '1s';
  eq('keyframe-animation-ignored', keyframe.style.getPropertyValue('animation'), '');
  eq('keyframe-duration-ignored', keyframe.style.getPropertyValue('animation-duration'), '');
  keyframe.style.animationTimingFunction = 'linear(0, 1)';
  eq('keyframe-timing', keyframe.style.getPropertyValue('animation-timing-function'), 'linear(0, 1)');
  ok('keyframe-timing-name', names(keyframe.style).includes('animation-timing-function'));
  ok('keyframe-cssText-timing', keyframe.cssText.includes('animation-timing-function: linear(0, 1);'));

  const invalid = document.createElement('div').style;
  invalid.animationDuration = '1s';
  invalid.animationDuration = '-2s';
  eq('invalid-duration-preserves-old', invalid.animationDuration, '1s');

  return failures.length ? failures.slice(0, 10).join('|') : 'PASS';
})()
"#,
        )
        .expect("animation shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}
#[test]
fn detached_animation_shorthand_queries_use_pdb_backing() {
    let mut vm = new_storage_test_vm("https://detached-animation-pdb-query.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));

  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const style = doc.createElement('div').style;

  style.cssText = 'animation: fade paused both reverse 3 1s 2s linear !important; --token: value; -webkit-text-fill-color: red;';
  eq('animation', style.getPropertyValue('animation'), '1s linear 2s 3 reverse both paused fade');
  eq('animation-priority', style.getPropertyPriority('animation'), 'important');
  eq('duration', style.getPropertyValue('animation-duration'), '1s');
  eq('range', style.getPropertyValue('animation-range'), 'normal');
  eq('token', style.getPropertyValue('--token'), 'value');
  eq('webkit-transition', style.getPropertyValue('-webkit-text-fill-color'), 'red');

  style.setProperty('animation-range', 'entry 10% exit 20%', 'important');
  eq('range-after-set', style.getPropertyValue('animation-range'), 'entry 10% exit 20%');
  eq('range-priority', style.getPropertyPriority('animation-range'), 'important');
  eq('range-start', style.getPropertyValue('animation-range-start'), 'entry 10%');
  eq('range-end', style.getPropertyValue('animation-range-end'), 'exit 20%');
  ok('range-start-name', names(style).includes('animation-range-start'));
  ok('range-end-name', names(style).includes('animation-range-end'));

  const removedRange = style.removeProperty('animation-range');
  eq('removed-range', removedRange, 'entry 10% exit 20%');
  eq('range-after-remove', style.getPropertyValue('animation-range'), '');
  eq('token-after-remove', style.getPropertyValue('--token'), 'value');
  eq('webkit-after-remove', style.getPropertyValue('-webkit-text-fill-color'), 'red');

  return failures.length ? failures.join('|') : 'PASS';
})()
"#,
        )
        .expect("detached animation PDB query should evaluate");

    assert_eq!(result, "PASS");
}
#[test]
fn outline_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://outline-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const outlineLonghands = ['outline-color', 'outline-style', 'outline-width'];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('outline', '1px solid red', 'important');
    eq(`${label}-outline`, style.getPropertyValue('outline'), 'red solid 1px');
    eq(`${label}-outline-priority`, style.getPropertyPriority('outline'), 'important');
    eq(`${label}-outline-width`, style.getPropertyValue('outline-width'), '1px');
    eq(`${label}-outline-style`, style.getPropertyValue('outline-style'), 'solid');
    eq(`${label}-outline-color`, style.getPropertyValue('outline-color'), 'red');
    eq(`${label}-outline-color-priority`, style.getPropertyPriority('outline-color'), 'important');
    hasAll(`${label}-outline-names`, style, outlineLonghands);
    ok(`${label}-outline-cssText`, style.cssText.includes('outline: red solid 1px !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('outline-width', '2px', 'important');
    eq(`${label}-outline-after-width`, style.getPropertyValue('outline'), 'red solid 2px');
    eq(`${label}-outline-width-after-width`, style.getPropertyValue('outline-width'), '2px');
    eq(`${label}-outline-width-priority`, style.getPropertyPriority('outline-width'), 'important');
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    const removedOutline = style.removeProperty('outline');
    eq(`${label}-removed-outline`, removedOutline, 'red solid 2px');
    eq(`${label}-outline-after-remove`, style.getPropertyValue('outline'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');
  inline.setProperty('outline-color', 'invert');
  eq('inline-invert-value', inline.getPropertyValue('outline-color'), 'invert');
  ok('inline-invert-cssText', inline.cssText.includes('outline-color: invert;'));

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'outline: 3px dotted blue !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-outline', detached.getPropertyValue('outline'), 'blue dotted 3px');
  eq('detached-outline-priority', detached.getPropertyPriority('outline'), 'important');
  eq('detached-outline-style', detached.getPropertyValue('outline-style'), 'dotted');
  hasAll('detached-outline-names', detached, outlineLonghands);
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedStyle = detached.removeProperty('outline-style');
  eq('detached-removed-style', removedStyle, 'dotted');
  eq('detached-outline-after-style-remove', detached.getPropertyValue('outline'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('outline', '1px dashed green', 'important');
  eq('rule-outline', rule.style.getPropertyValue('outline'), 'green dashed 1px');
  eq('rule-outline-priority', rule.style.getPropertyPriority('outline'), 'important');
  hasAll('rule-outline-names', rule.style, outlineLonghands);
  ok('rule-cssText-outline', rule.cssText.includes('outline: green dashed 1px !important;'));
  rule.style.setProperty('--token', 'value');
  const removedRuleOutline = rule.style.removeProperty('outline');
  eq('rule-removed-outline', removedRuleOutline, 'green dashed 1px');
  eq('rule-outline-after-remove', rule.style.getPropertyValue('outline'), '');
  eq('rule-token-after-remove', rule.style.getPropertyValue('--token'), 'value');
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));
  ok('rule-cssText-outline-removed', !rule.cssText.includes('outline'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('outline-width', '4px', 'important');
  eq('keyframe-outline-width', keyframe.style.getPropertyValue('outline-width'), '4px');
  eq('keyframe-outline-width-priority', keyframe.style.getPropertyPriority('outline-width'), 'important');
  ok('keyframe-outline-width-name', names(keyframe.style).includes('outline-width'));
  ok('keyframe-cssText-width', keyframe.cssText.includes('outline-width: 4px !important;'));

  return failures.length ? failures.slice(0, 8).join('|') : 'PASS';
})()
"#,
        )
        .expect("outline shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}
#[test]
fn webkit_text_stroke_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://webkit-text-stroke-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const strokeLonghands = ['-webkit-text-stroke-width', '-webkit-text-stroke-color'];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('-webkit-text-stroke', '1px red', 'important');
    eq(`${label}-stroke`, style.getPropertyValue('-webkit-text-stroke'), '1px red');
    eq(`${label}-stroke-priority`, style.getPropertyPriority('-webkit-text-stroke'), 'important');
    eq(`${label}-width`, style.getPropertyValue('-webkit-text-stroke-width'), '1px');
    eq(`${label}-color`, style.getPropertyValue('-webkit-text-stroke-color'), 'red');
    eq(`${label}-width-priority`, style.getPropertyPriority('-webkit-text-stroke-width'), 'important');
    hasAll(`${label}-stroke-names`, style, strokeLonghands);
    ok(`${label}-stroke-cssText`, style.cssText.includes('-webkit-text-stroke: 1px red !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('-webkit-text-stroke-width', '2px', 'important');
    eq(`${label}-stroke-after-width`, style.getPropertyValue('-webkit-text-stroke'), '2px red');
    eq(`${label}-width-after-width`, style.getPropertyValue('-webkit-text-stroke-width'), '2px');
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    const removedStroke = style.removeProperty('-webkit-text-stroke');
    eq(`${label}-removed-stroke`, removedStroke, '2px red');
    eq(`${label}-stroke-after-remove`, style.getPropertyValue('-webkit-text-stroke'), '');
    eq(`${label}-width-after-remove`, style.getPropertyValue('-webkit-text-stroke-width'), '');
    eq(`${label}-color-after-remove`, style.getPropertyValue('-webkit-text-stroke-color'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = '-webkit-text-stroke: 3px green !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-stroke', detached.getPropertyValue('-webkit-text-stroke'), '3px green');
  eq('detached-stroke-priority', detached.getPropertyPriority('-webkit-text-stroke'), 'important');
  eq('detached-width', detached.webkitTextStrokeWidth, '3px');
  eq('detached-color', detached.webkitTextStrokeColor, 'green');
  hasAll('detached-stroke-names', detached, strokeLonghands);
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedDetachedColor = detached.removeProperty('-webkit-text-stroke-color');
  eq('detached-removed-color', removedDetachedColor, 'green');
  eq('detached-stroke-after-color-remove', detached.getPropertyValue('-webkit-text-stroke'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('-webkit-text-stroke', '4px blue', 'important');
  eq('rule-stroke', rule.style.getPropertyValue('-webkit-text-stroke'), '4px blue');
  eq('rule-stroke-priority', rule.style.getPropertyPriority('-webkit-text-stroke'), 'important');
  hasAll('rule-stroke-names', rule.style, strokeLonghands);
  ok('rule-cssText-stroke', rule.cssText.includes('-webkit-text-stroke: 4px blue !important;'));
  rule.style.setProperty('--token', 'value');
  const removedRuleStroke = rule.style.removeProperty('-webkit-text-stroke');
  eq('rule-removed-stroke', removedRuleStroke, '4px blue');
  eq('rule-stroke-after-remove', rule.style.getPropertyValue('-webkit-text-stroke'), '');
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));
  ok('rule-cssText-stroke-removed', !rule.cssText.includes('-webkit-text-stroke'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('-webkit-text-stroke', '5px purple', 'important');
  eq('keyframe-stroke', keyframe.style.getPropertyValue('-webkit-text-stroke'), '5px purple');
  eq('keyframe-stroke-priority', keyframe.style.getPropertyPriority('-webkit-text-stroke'), 'important');
  hasAll('keyframe-stroke-names', keyframe.style, strokeLonghands);
  ok('keyframe-cssText-stroke', keyframe.cssText.includes('-webkit-text-stroke: 5px purple !important;'));

  return failures.length ? failures.slice(0, 10).join('|') : 'PASS';
})()
"#,
        )
        .expect("-webkit-text-stroke shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn stylesheet_eof_open_var_preserves_cssom_text_views() {
    let mut vm = new_storage_test_vm("https://css-var-stylesheet-eof.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = 'div { width: var(--open';
  (document.head || document.documentElement || document).appendChild(style);
  const rule = style.sheet.cssRules[0];

  const constructed = new CSSStyleSheet();
  constructed.replaceSync('span { height: var(--retained');
  const retained = constructed.cssRules[0];
  constructed.replaceSync('span { height: 1px; }');
  constructed.cssRules[0].style.width = 'var(--written';

  return [
    rule.style.getPropertyValue('width'),
    rule.style.cssText,
    rule.cssText,
    retained.style.getPropertyValue('height'),
    retained.cssText,
    constructed.cssRules[0].style.width
  ].join('|');
})()
"#,
        )
        .expect("stylesheet EOF-open var CSSOM views should evaluate");

    assert_eq!(
        result,
        "var(--open|width: var(--open;|div { width: var(--open; }|var(--retained|span { height: var(--retained; }|var(--written"
    );
}
