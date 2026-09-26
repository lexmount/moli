use super::*;

#[test]
fn css_rule_family_exposes_cssom_rule_branding_and_style_rule_surface() {
    let mut vm = new_storage_test_vm("https://css-rule-family.test/");

    let result = vm
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
  const sheet = new CSSStyleSheet();
  const index = sheet.insertRule('.a, main > .b { color: red; margin: 0 !important; }');
  const rules = sheet.cssRules;
  const rule = rules.item(0);
  const before = [
    index,
    typeof CSSRule,
    typeof CSSRuleList,
    typeof CSSStyleRule,
    Object.prototype.toString.call(rules),
    rules instanceof CSSRuleList,
    Object.hasOwn(rules, 'item'),
    Object.prototype.toString.call(rule),
    rule instanceof CSSStyleRule,
    rule instanceof CSSRule,
    Object.getPrototypeOf(CSSStyleRule.prototype) === CSSGroupingRule.prototype
      && Object.getPrototypeOf(CSSGroupingRule.prototype) === CSSRule.prototype,
    CSSRule.STYLE_RULE,
    CSSRule.prototype.STYLE_RULE,
    rule.type,
    rule.cssText,
    rule.selectorText,
    Object.prototype.toString.call(rule.style),
    rule.style.getPropertyValue('color'),
    rule.style.getPropertyPriority('margin'),
    rule.parentStyleSheet === sheet,
    rule.parentRule === null
  ].join(',');
  rule.selectorText = '.c';
  const afterSelector = [rule.selectorText, rule.cssText].join(',');
  rule.cssText = '.d { display: block; }';
  const afterCssText = [
    rule.selectorText,
    rule.cssText,
    rule.style.getPropertyValue('display'),
    rule.type
  ].join(',');
  rule.style.setProperty('opacity', '0.5', 'important');
  const afterSetProperty = [
    rule.style.cssText,
    rule.cssText
  ].join(',');
  rule.style.display = 'inline';
  const afterNamedProperty = rule.cssText;
  rule.style.removeProperty('opacity');
  const afterRemoveProperty = rule.cssText;
  const selectorSymbol = probe(() => { rule.selectorText = Symbol(); });
  const afterSelectorSymbol = rule.selectorText;
  const selectorThrow = probe(() => { rule.selectorText = { toString() { throw new RangeError('selector'); } }; });
  rule.selectorText = null;
  const afterSelectorNull = [rule.selectorText, rule.cssText].join(',');
  const cssTextSymbol = probe(() => { rule.cssText = Symbol(); });
  const afterCssTextSymbol = rule.cssText;
  const cssTextThrow = probe(() => { rule.cssText = { toString() { throw new RangeError('rule'); } }; });
  rule.cssText = null;
  const afterCssTextNull = rule.cssText;
  const replaceSyncSymbol = probe(() => sheet.replaceSync(Symbol()));
  const replaceSyncThrow = probe(() => sheet.replaceSync({ toString() { throw new RangeError('replace'); } }));
  sheet.replaceSync('main { color: blue; } aside { display: none; }');
  const afterReplaceSync = [
    sheet.cssRules === rules,
    rules.length,
    rules[0].cssText,
    rules[0].parentStyleSheet === sheet,
    rules[1].cssText
  ].join(',');
  return [
    before,
    afterSelector,
    afterCssText,
    afterSetProperty,
    afterNamedProperty,
    afterRemoveProperty,
    [
      selectorSymbol,
      afterSelectorSymbol,
      selectorThrow,
      afterSelectorNull,
      cssTextSymbol,
      afterCssTextSymbol,
      cssTextThrow,
      afterCssTextNull,
      replaceSyncSymbol,
      replaceSyncThrow
    ].join(','),
    afterReplaceSync
  ].join('|');
})()
"#,
        )
        .expect("CSSRule family should expose CSSOM rule surface");

    assert_eq!(
        result,
        "0,function,function,function,[object CSSRuleList],true,false,[object CSSStyleRule],true,true,true,1,1,1,.a, main > .b { color: red; margin: 0px !important; },.a, main > .b,[object CSSStyleProperties],red,important,true,true|.c,.c { color: red; margin: 0px !important; }|.d,.d { display: block; },block,1|display: block; opacity: 0.5 !important;,.d { display: block; opacity: 0.5 !important; }|.d { opacity: 0.5 !important; display: inline; }|.d { display: inline; }|throw:TypeError,.d,throw:RangeError,null,null { display: inline; },throw:TypeError,null { display: inline; },throw:RangeError,null { display: inline; },throw:TypeError,throw:RangeError|true,2,main { color: blue; },true,aside { display: none; }"
    );
}
#[test]
fn css_style_rule_style_stores_border_css_wide_keyword() {
    let mut vm = new_storage_test_vm("https://css-rule-border-wide-keyword.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: unset; border: unset; }');
  const style = sheet.cssRules[0].style;
  return [
    style.getPropertyValue('color'),
    style.getPropertyValue('border'),
    style.getPropertyValue('border-left'),
    style.getPropertyValue('border-color'),
    style.getPropertyValue('border-right-style')
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule style should preserve border CSS-wide keyword");

    assert_eq!(result, "unset|unset|unset|unset|unset");
}
#[test]
fn constructed_css_stylesheet_live_rules_refresh_after_rule_style_mutation() {
    let mut vm = new_storage_test_vm("https://css-constructed-live-rules-refresh.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.one { color: red; }');
  const first = sheet.cssRules[0];
  first.style.setProperty('color', 'blue');
  const index = sheet.insertRule('.two { margin: 0; }', 1);
  return [
    index,
    sheet.cssRules.length,
    sheet.cssRules[0] === first,
    sheet.cssRules[0].cssText,
    sheet.cssRules[1].cssText,
  ].join('|');
})()
"#,
        )
        .expect("constructed live rules should refresh after rule style mutation");

    assert_eq!(
        result,
        "1|2|true|.one { color: blue; }|.two { margin: 0px; }"
    );
}
#[test]
fn css_style_rule_insert_rule_uses_stylo_nested_mutation_context() {
    let mut vm = new_storage_test_vm("https://css-style-rule-nested-mutation-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@namespace svg url("http://www.w3.org/2000/svg"); .host { & .one { color: red; } }');
  const rule = sheet.cssRules[1];
  const index = rule.insertRule('> svg|path { color: blue; }', 1);
  rule.insertRule('margin: 0; padding: 1px;', 0);
  const declaration = rule.cssRules[0];
  const relative = rule.cssRules[2];
  rule.deleteRule(1);
  return [
    index,
    rule.cssRules.length,
    declaration instanceof CSSNestedDeclarations,
    declaration.cssText,
    relative.selectorText,
    relative.cssText,
    rule.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule nested Stylo mutation context should evaluate");

    assert_eq!(
        result,
        "1|2|true|margin: 0px; padding: 1px;|& > svg|path|& > svg|path { color: blue; }|.host {\n  margin: 0px; padding: 1px;\n  & > svg|path { color: blue; }\n}"
    );
}
#[test]
fn css_style_rule_css_text_reset_refreshes_existing_nested_rule_list_from_stylo() {
    let mut vm = new_storage_test_vm("https://css-style-rule-nested-css-text-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@namespace svg url("http://www.w3.org/2000/svg"); .host { & .old { color: red; } }');
  const rule = sheet.cssRules[1];
  const rules = rule.cssRules;
  const old = rules[0];
  rule.cssText = '.host { color: red; & > svg|path { color: blue; } width: 1px; }';
  return [
    rule.cssRules === rules,
    rules.length,
    old.parentRule === null,
    rules[0].selectorText,
    rules[0].cssText,
    rules[1] instanceof CSSNestedDeclarations,
    rules[1].cssText,
    rule.style.getPropertyValue('color'),
    rule.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule cssText reset should refresh existing nested cssRules");

    assert_eq!(
        result,
        "true|2|true|& > svg|path|& > svg|path { color: blue; }|true|width: 1px;|red|.host {\n  color: red;\n  & > svg|path { color: blue; }\n  width: 1px;\n}"
    );
}
#[test]
fn css_keyframes_rule_append_delete_materializes_stylo_mutation_children() {
    let mut vm = new_storage_test_vm("https://css-keyframes-rule-mutation-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes slide { from { opacity: 0; } }');
  const keyframes = sheet.cssRules[0];
  const first = keyframes.cssRules[0];
  keyframes.appendRule('to { opacity: 1; transform: translateX(10px); }');
  const deleted = keyframes.cssRules[1];
  keyframes.appendRule('50% { opacity: 0.5; }');
  const middle = keyframes.cssRules[2];
  keyframes.deleteRule('to');
  return [
    keyframes.cssRules.length,
    keyframes.length,
    keyframes[0] === first,
    keyframes[1] === middle,
    deleted.parentRule === null,
    keyframes.findRule('from') === first,
    keyframes.findRule('to') === null,
    middle.keyText,
    middle.cssText,
    keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframesRule Stylo mutation view should evaluate");

    assert_eq!(
        result,
        "2|2|true|true|true|true|true|50%|50% { opacity: 0.5; }|@keyframes slide {\n0% { opacity: 0; }\n50% { opacity: 0.5; }\n}"
    );
}
#[test]
fn css_keyframes_rule_live_stylesheet_mutation_preserves_stylesheet_mutation_path() {
    let mut vm = new_storage_test_vm("https://css-keyframes-rule-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes slide { from { opacity: 0; } }');
  const keyframes = sheet.cssRules[0];
  const rules = keyframes.cssRules;

  keyframes.appendRule('to { opacity: 1; transform: translateX(10px); }');
  const inserted = sheet.insertRule('.after { margin: 0; }', 1);
  keyframes.deleteRule('to');
  const deleted = sheet.deleteRule(1);

  return [
    inserted,
    deleted === undefined,
    sheet.cssRules.length,
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === rules,
    keyframes.length,
    keyframes.cssRules[0].cssText,
    keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("live keyframes mutation should preserve stylesheet mutation path");

    assert_eq!(
        result,
        "1|true|1|true|true|1|0% { opacity: 0; }|@keyframes slide {\n0% { opacity: 0; }\n}"
    );
}
#[test]
fn css_keyframes_rule_name_mutation_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-keyframes-name-live-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } } .after { color: black; }');
  const keyframes = sheet.cssRules[0];
  const keyframeRules = keyframes.cssRules;
  const first = keyframeRules[0];
  const firstStyle = first.style;

  keyframes.name = 'slide';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);
  keyframes.appendRule('50% { opacity: .5; }');
  keyframes.deleteRule('50%');

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === keyframeRules,
    keyframeRules[0] === first,
    first.style === firstStyle,
    keyframes.name,
    first.keyText,
    first.style.opacity,
    keyframes.findRule('from') === first,
    keyframes.cssText.includes('@keyframes slide'),
    sheet.cssRules[0].cssText === keyframes.cssText,
    sheet.cssRules[1].cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframesRule name mutation should preserve Stylo rule tree");

    assert_eq!(
        result,
        "2|true|true|true|true|slide|0%|0|true|true|true|.after { color: black; }"
    );
}
#[test]
fn css_keyframes_rule_css_text_reset_uses_attached_native_rule() {
    let mut vm = new_storage_test_vm("https://css-keyframes-css-text-stylo-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } } .after { color: black; }');
  const keyframes = sheet.cssRules[0];
  const keyframeRules = keyframes.cssRules;

  keyframes.cssText = '@keyframes "slide show" { from { opacity: .25; } to { transform: translateX(1px); } }';
  const beforeInvalidReset = keyframes.cssText;
  keyframes.cssText = '@keyframes none { from { opacity: 0; } }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === keyframeRules,
    keyframes.length,
    keyframes.name,
    keyframes.cssText === beforeInvalidReset,
    keyframes.cssText.includes('@keyframes slide\\ show'),
    keyframes.findRule('from').style.opacity,
    keyframes.findRule('to').style.transform,
    sheet.cssRules[0].cssText === keyframes.cssText,
    sheet.cssRules[1].cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframesRule cssText reset should use the attached native rule");

    assert_eq!(
        result,
        "2|true|true|2|slide show|true|true|0.25|translateX(1px)|true|.after { color: black; }"
    );
}
#[test]
fn css_keyframes_rule_lazy_css_rules_use_attached_native_rules() {
    let mut vm = new_storage_test_vm("https://css-keyframes-lazy-css-rules-stylo-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } } .after { color: black; }');
  const keyframes = sheet.cssRules[0];

  keyframes.cssText = '@keyframes slide { from { opacity: .25; } to { transform: translateX(1px); } }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  const rules = keyframes.cssRules;
  return [
    keyframes.name,
    keyframes.length,
    rules.length,
    rules[0].cssText,
    rules[1].style.transform,
    sheet.cssRules[0].cssText === keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframesRule lazy cssRules should use attached native rules");

    assert_eq!(
        result,
        "slide|2|2|0% { opacity: 0.25; }|translateX(1px)|true"
    );
}
#[test]
fn css_keyframe_key_text_uses_stylo_selector_helpers() {
    let mut vm = new_storage_test_vm("https://css-keyframe-key-text-stylo.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes slide { from { opacity: 0; } }');
  const keyframes = sheet.cssRules[0];
  const frame = keyframes.cssRules[0];

  frame.keyText = '50%, to';
  const afterValid = [
    frame.keyText,
    frame.cssText,
    keyframes.findRule('50%, 100%') === frame,
    keyframes.findRule('50%, to') === frame,
    keyframes.findRule('100%') === null,
    keyframes.cssText,
  ].join('/');

  frame.keyText = 'body';
  const afterInvalid = [
    frame.keyText,
    keyframes.findRule('body') === null,
    keyframes.findRule('50%, 100%') === frame,
  ].join('/');

  keyframes.deleteRule('50%, to');
  return [afterValid, afterInvalid, keyframes.cssRules.length].join('|');
})()
"#,
        )
        .expect("CSSKeyframeRule keyText Stylo selector helpers should evaluate");

    assert_eq!(
        result,
        "50%, 100%/50%, 100% { opacity: 0; }/true/true/true/@keyframes slide {\n50%, 100% { opacity: 0; }\n}|50%, 100%/true/true|0"
    );
}
#[test]
fn css_keyframe_rule_key_text_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-keyframe-key-text-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } }');
  const keyframes = sheet.cssRules[0];
  const keyframeRules = keyframes.cssRules;
  const keyframe = keyframeRules[1];

  keyframe.keyText = '75%, to';
  keyframes.appendRule('50% { opacity: .5; }');
  keyframes.deleteRule('50%');

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === keyframeRules,
    keyframes.cssRules[1] === keyframe,
    keyframe.keyText,
    keyframe.cssText,
    keyframes.findRule('75%, 100%') === keyframe,
    keyframes.findRule('100%') === null,
    keyframes.cssRules.length,
    keyframes.cssText.includes('75%, 100%'),
    sheet.cssRules[0].cssText === keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("live CSSKeyframeRule keyText mutation path should evaluate");

    assert_eq!(
        result,
        "1|true|true|true|75%, 100%|75%, 100% { opacity: 1; }|true|true|2|true|true"
    );
}
#[test]
fn css_rule_style_properties_forward_assignment_to_css_text() {
    let mut vm = new_storage_test_vm("https://css-rule-style-put-forwards.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    #target { color: red; margin: 1px; }
    @keyframes slide {
      from { margin-left: 100%; width: 300%; }
      to { margin-left: 0%; width: 100%; }
    }
  `);
  const styleRule = sheet.cssRules[0];
  const keyframeRule = sheet.cssRules[1].cssRules[0];
  styleRule.style = 'color: blue; padding: 2px;';
  keyframeRule.style = 'margin-left: 50%; width: 100%;';
  return [
    styleRule.style.cssText,
    styleRule.cssText,
    keyframeRule.style.marginLeft,
    keyframeRule.style.width,
    keyframeRule.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSS rule style [PutForwards] assignment should evaluate");

    assert_eq!(
        result,
        "color: blue; padding: 2px;|#target { color: blue; padding: 2px; }|50%|100%|0% { margin-left: 50%; width: 100%; }"
    );
}
#[test]
fn css_keyframe_rule_style_uses_stylo_declaration_block_for_plain_properties() {
    let mut vm = new_storage_test_vm("https://keyframe-style-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes slide { from { } }');
  const rule = sheet.cssRules[0].cssRules[0];
  const style = rule.style;

  style.setProperty('place-content', 'center start', 'important');
  const pdbWrite = [
    style.length,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.getPropertyValue('place-content'),
    style.getPropertyPriority('place-content'),
    style.cssText,
    rule.cssText
  ].join(',');

  style.cssText = [
    'display: invalid;',
    'display: block;',
    'place-content: center start;',
    'animation-name: spin;'
  ].join(' ');
  const cssTextWrite = [
    style.getPropertyValue('display'),
    style.getPropertyValue('place-content'),
    style.getPropertyValue('animation-name'),
    style.cssText,
    rule.cssText.includes('animation-name')
  ].join(',');

  style.setProperty('animation-timing-function', 'steps(2)');
  const timing = style.getPropertyValue('animation-timing-function');

  style.removeProperty('place-content');
  const removedPlaceContent = [
    timing,
    style.getPropertyValue('place-content'),
    style.getPropertyValue('align-content'),
    style.cssText
  ].join(',');

  return [pdbWrite, cssTextWrite, removedPlaceContent].join('|');
})()
"#,
        )
        .expect("CSSKeyframeRule style should use Stylo declarations for plain properties");

    assert_eq!(
        result,
        "2,align-content,justify-content,center start,important,place-content: center start !important;,0% { place-content: center start !important; }|block,center start,,display: block; place-content: center start;,false|steps(2),,,display: block; animation-timing-function: steps(2);"
    );
}
#[test]
fn css_nested_declarations_update_parent_rule_style() {
    let mut vm = new_storage_test_vm("https://css-nested-declarations.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = '.parent { color: red; .child { background: blue; } font-size: 20px; }';
  (document.head || document.documentElement || document).appendChild(style);
  const parent = document.createElement('div');
  parent.className = 'parent';
  const child = document.createElement('div');
  child.className = 'child';
  parent.appendChild(child);
  (document.body || document.documentElement || document).appendChild(parent);

  const rule = style.sheet.cssRules[0];
  const nestedRule = rule.cssRules[0];
  const nestedDeclarations = rule.cssRules[1];
  const before = [
    typeof CSSNestedDeclarations,
    nestedDeclarations instanceof CSSNestedDeclarations,
    nestedDeclarations instanceof CSSRule,
    nestedRule.cssText,
    nestedDeclarations.style.getPropertyValue('color'),
    nestedDeclarations.style.getPropertyValue('font-size'),
    rule.style.getPropertyValue('color')
  ].join(',');
  nestedDeclarations.style.color = 'green';
  const after = [
    nestedDeclarations.style.getPropertyValue('color'),
    rule.cssText,
    getComputedStyle(parent).color
  ].join(',');
  return `${before}|${after}`;
})()
"#,
        )
        .expect("CSSNestedDeclarations should update parent rule style");

    assert_eq!(
        result,
        "function,true,true,& .child { background: blue; },,20px,red|green,.parent {\n  color: red;\n  & .child { background: blue; }\n  font-size: 20px; color: green;\n},rgb(0, 128, 0)"
    );
}
#[test]
fn css_nested_declarations_cssom_preserves_rule_order_and_group_blocks() {
    let mut vm = new_storage_test_vm("https://css-nested-declarations-cssom.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    .a {
      --a:1;
      --b:1;
      & { --c:1; }
      --d:1;
      --e:1;
      color:hover {}
      @media (width > 100px) {
        --x:1;
        --y:1;
        .b { }
        --z:1;
      }
      --w:1;
    }
  `);
  const outer = sheet.cssRules[0];
  const media = outer.cssRules[3];
  const iterated = [];
  for (const rule of outer.cssRules) {
    iterated.push(rule.cssText);
  }
  return [
    outer.cssRules.length,
    iterated.join('|'),
    media instanceof CSSMediaRule,
    media.cssRules.length,
    media.cssRules[0] instanceof CSSNestedDeclarations,
    media.cssRules[0].cssText,
    media.cssRules[1].cssText,
    media.cssRules[2] instanceof CSSNestedDeclarations,
    media.cssRules[2].cssText,
  ].join('||');
})()
"#,
        )
        .expect("nested declaration CSSOM order should evaluate");

    assert_eq!(
        result,
        "5||& { --c: 1; }|--d: 1; --e: 1;|& color:hover { }|@media (width > 100px) {\n  --x: 1; --y: 1;\n  & .b { }\n  --z: 1;\n}|--w: 1;||true||3||true||--x: 1; --y: 1;||& .b { }||true||--z: 1;"
    );
}
#[test]
fn css_nested_declarations_parent_rule_uses_pdb_serialization() {
    let mut vm = new_storage_test_vm("https://css-nested-declarations-pdb-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.a { & .b { } }');
  const rule = sheet.cssRules[0];
  const index = rule.insertRule(
    'color: rgb(0 128 0 / 50%); width: 0; opacity: 1 !important;',
    1
  );
  const nested = rule.cssRules[index];
  return [
    index,
    nested instanceof CSSNestedDeclarations,
    nested.cssText,
    nested.style.cssText,
    rule.cssText
  ].join('|');
})()
"#,
        )
        .expect("nested declaration runs should serialize parent rule from PDB");

    assert_eq!(
        result,
        "1|true|color: rgba(0, 128, 0, 0.5); width: 0px; opacity: 1 !important;|color: rgba(0, 128, 0, 0.5); width: 0px; opacity: 1 !important;|.a {\n  & .b { }\n  color: rgba(0, 128, 0, 0.5); width: 0px; opacity: 1 !important;\n}"
    );
}
#[test]
fn css_nested_grouping_declarations_local_mutation_syncs_live_parent_rule() {
    let mut vm = new_storage_test_vm("https://css-nested-grouping-declarations-sync.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    .a {
      @media (width > 100px) {
        --x: 1;
        .b {}
        --z: 1;
      }
    }
  `);
  const outer = sheet.cssRules[0];
  const media = outer.cssRules[0];
  const leading = media.cssRules[0];
  leading.style.setProperty('--x', '2');
  return [
    leading.cssText,
    media.cssText,
    outer.cssText
  ].join('||');
})()
"#,
        )
        .expect("nested grouping declaration mutation should sync parent rule text");

    assert_eq!(
        result,
        "--x: 2;||@media (width > 100px) {\n  --x: 2;\n  & .b { }\n  --z: 1;\n}||.a {\n  @media (width > 100px) {\n  --x: 2;\n  & .b { }\n  --z: 1;\n}\n}"
    );
}
#[test]
fn css_style_rule_style_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-style-rule-style-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@supports (display: grid) { .one { color: red; & .child { color: blue; } } }');
  const supports = sheet.cssRules[0];
  const rule = supports.cssRules[0];
  const nestedRules = rule.cssRules;
  const nested = nestedRules[0];
  const style = rule.style;

  rule.style.color = 'green';
  sheet.insertRule('.after { color: blue; }', 1);
  sheet.deleteRule(1);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === supports,
    supports.cssRules[0] === rule,
    rule.cssRules === nestedRules,
    rule.cssRules[0] === nested,
    rule.style === style,
    rule.style.color,
    rule.cssText.includes('color: green'),
    rule.cssText.includes('& .child { color: blue; }'),
    supports.cssText.includes('color: green'),
    sheet.cssRules[0].cssText === supports.cssText,
  ].join('|');
})()
"#,
        )
        .expect("live CSSStyleRule style mutation path should evaluate");

    assert_eq!(
        result,
        "1|true|true|true|true|true|green|true|true|true|true"
    );
}
#[test]
fn css_nested_declarations_style_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-nested-declarations-style-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@supports (display: grid) { .one { & .child { color: blue; } color: red; margin: 0; } }');
  const supports = sheet.cssRules[0];
  const rule = supports.cssRules[0];
  const nestedRules = rule.cssRules;
  const child = nestedRules[0];
  const declarations = nestedRules[1];
  const style = declarations.style;

  declarations.style.color = 'green';
  sheet.insertRule('.after { color: blue; }', 1);
  sheet.deleteRule(1);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === supports,
    supports.cssRules[0] === rule,
    rule.cssRules === nestedRules,
    rule.cssRules[0] === child,
    rule.cssRules[1] === declarations,
    declarations.style === style,
    declarations.style.color,
    declarations.cssText.includes('color: green'),
    rule.cssText.includes('& .child { color: blue; }'),
    rule.cssText.includes('color: green'),
    supports.cssText.includes('color: green'),
  ].join('|');
})()
"#,
        )
        .expect("live CSSNestedDeclarations style mutation path should evaluate");

    assert_eq!(
        result,
        "1|true|true|true|true|true|true|green|true|true|true|true"
    );
}
#[test]
fn css_keyframe_rule_style_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-keyframe-rule-style-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } }');
  const keyframes = sheet.cssRules[0];
  const keyframeRules = keyframes.cssRules;
  const keyframe = keyframeRules[1];
  const style = keyframe.style;

  keyframe.style.opacity = '.5';
  sheet.insertRule('.after { color: blue; }', 1);
  sheet.deleteRule(1);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === keyframeRules,
    keyframes.cssRules[1] === keyframe,
    keyframe.style === style,
    keyframe.style.opacity,
    keyframe.cssText.includes('opacity: 0.5'),
    keyframes.cssText.includes('opacity: 0.5'),
    sheet.cssRules[0].cssText === keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("live CSSKeyframeRule style mutation path should evaluate");

    assert_eq!(result, "1|true|true|true|true|0.5|true|true|true");
}
#[test]
fn css_style_rule_css_text_reset_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-style-rule-css-text-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@supports (display: grid) { .one { color: red; margin: 0; } }');
  const supports = sheet.cssRules[0];
  const rule = supports.cssRules[0];

  rule.cssText = '.one { color: green; padding: 1px; }';
  const style = rule.style;
  sheet.insertRule('.after { color: blue; }', 1);
  sheet.deleteRule(1);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === supports,
    supports.cssRules[0] === rule,
    rule.style === style,
    rule.style.color,
    rule.style.paddingTop,
    rule.cssText.includes('color: green'),
    rule.cssText.includes('padding: 1px'),
    supports.cssText.includes('color: green'),
    sheet.cssRules[0].cssText === supports.cssText,
  ].join('|');
})()
"#,
        )
        .expect("live CSSStyleRule cssText reset path should evaluate");

    assert_eq!(result, "1|true|true|true|green|1px|true|true|true|true");
}
#[test]
fn css_style_rule_css_text_reset_with_style_wrapper_updates_selector() {
    let mut vm = new_storage_test_vm("https://css-style-rule-css-text-selector-wrapper.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.one { color: red; }');
  const rule = sheet.cssRules[0];
  const style = rule.style;

  rule.cssText = '.two { color: green; padding: 1px; }';

  return [
    sheet.cssRules[0] === rule,
    rule.style === style,
    rule.selectorText,
    rule.style.color,
    rule.style.paddingTop,
    rule.cssText.includes('.two'),
    sheet.cssRules[0].cssText.includes('.two'),
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule cssText reset with existing style wrapper should evaluate");

    assert_eq!(result, "true|true|.two|green|1px|true|true");
}
#[test]
fn css_style_rule_css_text_replace_selector_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-style-rule-css-text-replace-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@namespace svg url("http://www.w3.org/2000/svg"); .one { color: red; } .after { color: black; }');
  const rule = sheet.cssRules[1];
  const style = rule.style;

  rule.cssText = 'svg|path { color: blue; & > .icon { opacity: .5; } }';
  const nestedRules = rule.cssRules;
  const nested = nestedRules[0];
  sheet.insertRule('.temp { color: green; }', 3);
  sheet.deleteRule(3);

  return [
    sheet.cssRules.length,
    sheet.cssRules[1] === rule,
    rule.style === style,
    rule.selectorText,
    rule.style.color,
    rule.cssRules === nestedRules,
    nestedRules[0] === nested,
    nested.selectorText,
    nested.style.opacity,
    sheet.cssRules[1].cssText === rule.cssText,
    sheet.cssRules[1].cssText.includes('svg|path'),
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule cssText replace should preserve live tree");

    assert_eq!(
        result,
        "3|true|true|svg|path|blue|true|true|& > .icon|0.5|true|true"
    );
}
#[test]
fn css_style_rule_css_text_invalid_multi_rule_reset_restores_native_rule() {
    let mut vm =
        new_storage_test_vm("https://css-style-rule-css-text-invalid-reset-rollback.test/");

    let initial_length = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: 1000 },
    (_, index) => `& .child-${index} { opacity: .5; }`
  ).join('\n');
  sheet.replaceSync(`.host { color: red; ${children} } .after { color: black; }`);
  const rule = sheet.cssRules[0];
  const style = rule.style;
  const rules = rule.cssRules;
  const child = rules[0];
  const after = sheet.cssRules[1];
  const before = rule.cssText;

  globalThis.__invalidResetSheet = sheet;
  globalThis.__invalidResetRule = rule;
  globalThis.__invalidResetStyle = style;
  globalThis.__invalidResetRules = rules;
  globalThis.__invalidResetChild = child;
  globalThis.__invalidResetAfter = after;
  globalThis.__invalidResetBefore = before;
  return rules.length;
})()
"#,
        )
        .expect("invalid CSSStyleRule rollback fixture should initialize");
    assert_eq!(initial_length, "1000");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__invalidResetSheet;
  const rule = globalThis.__invalidResetRule;
  const style = globalThis.__invalidResetStyle;
  const rules = globalThis.__invalidResetRules;
  const child = globalThis.__invalidResetChild;
  const after = globalThis.__invalidResetAfter;
  const before = globalThis.__invalidResetBefore;

  rule.cssText = '.host { color: green; } .extra { color: purple; }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === rule,
    rule.style === style,
    rule.cssRules === rules,
    rules[0] === child,
    rules.length,
    rule.style.color,
    child.selectorText,
    child.style.opacity,
    rule.cssText === before,
    !rule.cssText.includes('.extra'),
    sheet.cssRules[0].cssText === rule.cssText,
    after.cssText,
  ].join('|');
})()
"#,
        )
        .expect("invalid CSSStyleRule cssText reset should restore attached native state");

    assert_eq!(
        result,
        "2|true|true|true|true|1000|red|& .child-0|0.5|true|true|true|.after { color: black; }"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        0,
        "failed attached cssText replacement must restore only the root wrapper"
    );
}
#[test]
fn css_style_rule_selector_text_mutation_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-style-rule-selector-text-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@namespace svg url("http://www.w3.org/2000/svg"); .one { color: red; & > .icon { opacity: .5; } } .after { color: black; }');
  const rule = sheet.cssRules[1];
  const style = rule.style;
  const nestedRules = rule.cssRules;
  const nested = nestedRules[0];

  rule.selectorText = 'svg|path';
  sheet.insertRule('.temp { color: green; }', 3);
  sheet.deleteRule(3);

  return [
    sheet.cssRules.length,
    sheet.cssRules[1] === rule,
    rule.style === style,
    rule.cssRules === nestedRules,
    nestedRules[0] === nested,
    rule.selectorText,
    rule.style.color,
    nested.selectorText,
    nested.style.opacity,
    sheet.cssRules[1].cssText === rule.cssText,
    sheet.cssRules[1].cssText.includes('svg|path'),
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule selectorText mutation should preserve live tree");

    assert_eq!(
        result,
        "3|true|true|true|true|svg|path|red|& > .icon|0.5|true|true"
    );
}
#[test]
fn css_style_rule_public_reads_use_attached_native_rule() {
    let mut vm = new_storage_test_vm("https://css-style-rule-public-read-stylo-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@namespace svg url("http://www.w3.org/2000/svg"); .host { color: red; } .after { color: black; }');
  const rule = sheet.cssRules[1];

  rule.cssText = 'svg|path { color: blue; & > .icon { opacity: .5; } }';
  const rules = rule.cssRules;
  const nested = rules[0];

  return [
    sheet.cssRules[1] === rule,
    rule.selectorText,
    rules.length,
    nested.selectorText,
    nested.cssText,
    nested.parentRule === rule,
    sheet.cssRules[1].cssText === rule.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule public reads should use the attached native rule");

    assert_eq!(
        result,
        "true|svg|path|1|& > .icon|& > .icon { opacity: 0.5; }|true|true"
    );
}
#[test]
fn css_nested_style_rule_css_text_replace_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-nested-style-rule-css-text-replace-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.host { color: red; & .old { color: blue; } font-size: 12px; }');
  const parent = sheet.cssRules[0];
  const rules = parent.cssRules;
  const child = rules[0];
  const childStyle = child.style;

  child.cssText = '> .new { color: green; padding: 1px; }';
  parent.insertRule('.later { color: purple; }', 2);
  parent.deleteRule(2);

  return [
    sheet.cssRules[0] === parent,
    parent.cssRules === rules,
    rules[0] === child,
    child.style === childStyle,
    child.selectorText,
    child.style.color,
    child.style.paddingTop,
    parent.cssText.includes('& > .new'),
    parent.cssText.includes('font-size: 12px'),
    sheet.cssRules[0].cssText === parent.cssText,
  ].join('|');
})()
"#,
        )
        .expect("nested CSSStyleRule cssText replace should preserve live tree");

    assert_eq!(
        result,
        "true|true|true|true|& > .new|green|1px|true|true|true"
    );
}
#[test]
fn css_nested_style_rule_selector_text_mutation_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-nested-style-rule-selector-text-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.host { color: red; & .old { color: blue; } font-size: 12px; }');
  const parent = sheet.cssRules[0];
  const rules = parent.cssRules;
  const child = rules[0];
  const childStyle = child.style;

  child.selectorText = '> .new';
  parent.insertRule('.later { color: purple; }', 2);
  parent.deleteRule(2);

  return [
    sheet.cssRules[0] === parent,
    parent.cssRules === rules,
    rules[0] === child,
    child.style === childStyle,
    child.selectorText,
    child.style.color,
    parent.cssText.includes('& > .new'),
    parent.cssText.includes('font-size: 12px'),
    sheet.cssRules[0].cssText === parent.cssText,
  ].join('|');
})()
"#,
        )
        .expect("nested CSSStyleRule selectorText mutation should preserve live tree");

    assert_eq!(result, "true|true|true|true|& > .new|blue|true|true|true");
}
#[test]
fn css_keyframe_rule_css_text_reset_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-keyframe-rule-css-text-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } }');
  const keyframes = sheet.cssRules[0];
  const keyframeRules = keyframes.cssRules;
  const keyframe = keyframeRules[1];

  keyframe.cssText = 'to { opacity: .25; transform: translateX(1px); }';
  const style = keyframe.style;
  sheet.insertRule('.after { color: blue; }', 1);
  sheet.deleteRule(1);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === keyframeRules,
    keyframes.cssRules[1] === keyframe,
    keyframe.style === style,
    keyframe.style.opacity,
    keyframe.style.transform,
    keyframe.cssText.includes('opacity: 0.25'),
    keyframes.cssText.includes('translateX(1px)'),
    sheet.cssRules[0].cssText === keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("live CSSKeyframeRule cssText reset path should evaluate");

    assert_eq!(
        result,
        "1|true|true|true|true|0.25|translateX(1px)|true|true|true"
    );
}
#[test]
fn css_keyframe_rule_css_text_reset_with_style_wrapper_updates_key_text() {
    let mut vm = new_storage_test_vm("https://css-keyframe-rule-css-text-key-wrapper.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } }');
  const keyframes = sheet.cssRules[0];
  const keyframe = keyframes.cssRules[1];
  const style = keyframe.style;

  keyframe.cssText = '80% { opacity: .8; transform: scale(1); }';

  return [
    keyframe.keyText,
    keyframe.cssText,
    keyframes.findRule('80%') === keyframe,
    keyframes.findRule('100%') === null,
    keyframe.style === style,
    keyframe.style.opacity,
    keyframe.style.transform,
    keyframes.cssText.includes('80%'),
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframeRule cssText reset with existing style wrapper should evaluate");

    assert_eq!(
        result,
        "80%|80% { opacity: 0.8; transform: scale(1); }|true|true|true|0.8|scale(1)|true"
    );
}
#[test]
fn css_keyframe_rule_css_text_replace_key_text_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-keyframe-rule-css-text-replace-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } }');
  const keyframes = sheet.cssRules[0];
  const keyframeRules = keyframes.cssRules;
  const keyframe = keyframeRules[1];
  const style = keyframe.style;

  keyframe.cssText = '80% { opacity: .8; transform: scale(1); }';
  keyframes.appendRule('50% { opacity: .5; }');
  keyframes.deleteRule('50%');

  return [
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === keyframeRules,
    keyframes.cssRules[1] === keyframe,
    keyframe.style === style,
    keyframe.keyText,
    keyframe.style.opacity,
    keyframe.style.transform,
    keyframes.findRule('80%') === keyframe,
    keyframes.findRule('100%') === null,
    sheet.cssRules[0].cssText === keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframeRule cssText replace should preserve live tree");

    assert_eq!(
        result,
        "true|true|true|true|80%|0.8|scale(1)|true|true|true"
    );
}
#[test]
fn css_page_and_margin_rule_styles_match_chromium_serialization() {
    let mut vm = new_storage_test_vm("https://css-page-rule-style-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page :first { margin-top: 1px; @top-left { content: "x"; color: red; } }');
  const page = sheet.cssRules[0];
  const margin = page.cssRules[0];
  return [
    page instanceof CSSPageRule,
    page.selectorText,
    page.style.cssText,
    page.cssRules.length,
    margin instanceof CSSMarginRule,
    margin.name,
    margin.style.cssText,
    page.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSPageRule and CSSMarginRule style views should evaluate");

    assert_eq!(
        result,
        "true|:first|margin-top: 1px;|1|true|top-left|content: \"x\"; color: red;|@page :first { margin-top: 1px; @top-left { content: \"x\"; color: red; } }"
    );
}
#[test]
fn css_descriptor_rule_style_accessors_validate_receiver_type() {
    let mut vm = new_storage_test_vm("https://css-descriptor-style-receiver.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@font-face { font-family: a; src: local(a); } @page { margin-top: 1px; @top-left { content: "x"; } }');
  const font = sheet.cssRules[0];
  const page = sheet.cssRules[1];
  const margin = page.cssRules[0];
  const probe = callback => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return error && error.name;
    }
  };
  const fontStyle = Object.getOwnPropertyDescriptor(CSSFontFaceRule.prototype, 'style');
  const pageStyle = Object.getOwnPropertyDescriptor(CSSPageRule.prototype, 'style');
  const marginStyle = Object.getOwnPropertyDescriptor(CSSMarginRule.prototype, 'style');
  return [
    probe(() => fontStyle.get.call(font)),
    probe(() => fontStyle.get.call(page)),
    probe(() => pageStyle.get.call(page)),
    probe(() => pageStyle.get.call(font)),
    probe(() => marginStyle.get.call(margin)),
    probe(() => marginStyle.get.call(page)),
    font.style.fontFamily,
    page.style.marginTop,
    margin.style.getPropertyValue('content'),
  ].join('|');
})()
"#,
        )
        .expect("descriptor rule style accessors should validate receiver type");

    assert_eq!(
        result,
        r#"ok|TypeError|ok|TypeError|ok|TypeError|a|1px|"x""#
    );
}
#[test]
fn css_counter_style_rule_survives_stylo_stylesheet_mutations() {
    let mut vm = new_storage_test_vm("https://css-counter-style-stylo-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  const counterIndex = sheet.insertRule('@counter-style thumbs { system: cyclic; symbols: "*"; suffix: " "; }', 0);
  const styleIndex = sheet.insertRule('.after { list-style: thumbs; }', 1);
  sheet.deleteRule(styleIndex);
  const counter = sheet.cssRules[counterIndex];
  return [
    sheet.cssRules.length,
    counter instanceof CSSCounterStyleRule,
    counter.type,
    counter.cssText.includes('\n'),
    counter.cssText
  ].join('|');
})()
"#,
        )
        .expect("counter-style rule should survive Stylo stylesheet mutations");

    assert_eq!(
        result,
        r#"1|true|11|false|@counter-style thumbs { system: cyclic; suffix: " "; symbols: "*"; }"#
    );
}
#[test]
fn css_counter_style_rule_css_text_reset_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-counter-style-css-text-live-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@counter-style thumbs { system: cyclic; symbols: "*"; suffix: " "; } .after { color: black; }');
  const counter = sheet.cssRules[0];

  counter.cssText = '@counter-style dots { system: cyclic; symbols: "."; suffix: " "; }';
  const beforeInvalidReset = counter.cssText;
  counter.cssText = '@counter-style bad { system: cyclic; suffix: " "; }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === counter,
    counter instanceof CSSCounterStyleRule,
    counter.name,
    counter.cssText.includes('@counter-style dots'),
    counter.cssText.includes('symbols: "."'),
    counter.cssText === beforeInvalidReset,
    sheet.cssRules[0].cssText === counter.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSCounterStyleRule cssText reset should preserve Stylo rule tree");

    assert_eq!(
        result,
        r#"2|true|true|dots|true|true|true|true|.after { color: black; }"#
    );
}
#[test]
fn cssom_rule_list_keeps_namespaced_style_rules() {
    let mut vm = new_storage_test_vm("https://cssom-rule-list-namespaces.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('@namespace svg "http://www.w3.org/2000/svg";', 0);
  sheet.insertRule('svg|a { color: white; }', 1);
  sheet.insertRule('@media screen {}', 2);
  sheet.cssRules[2].insertRule('svg|circle { color: blue; }', 0);
  return [
    sheet.cssRules.length,
    sheet.cssRules[0].type,
    sheet.cssRules[1].type,
    sheet.cssRules[1].selectorText,
    sheet.cssRules[1].cssText,
    sheet.cssRules[2].cssRules.length,
    sheet.cssRules[2].cssRules[0].selectorText
  ].join('|');
})()
"#,
        )
        .expect("namespaced CSSOM style rules should evaluate");

    assert_eq!(result, "3|10|1|svg|a|svg|a { color: white; }|1|svg|circle");
}
#[test]
fn css_rule_styles_use_stylo_declaration_block_for_plain_properties() {
    let mut vm = new_storage_test_vm("https://rule-style-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0];
  const style = rule.style;
  style.setProperty('display', 'invalid');
  style.setProperty('place-content', 'center start', 'important');
  const ruleWrite = [
    style.length,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.getPropertyValue('place-content'),
    style.getPropertyPriority('place-content'),
    style.cssText,
    rule.cssText
  ].join(',');

  style.cssText = 'color: invalid; color: red; padding: 1px 2px;';
  const cssTextWrite = [
    style.getPropertyValue('color'),
    style.getPropertyValue('padding'),
    style.cssText,
    rule.cssText
  ].join(',');

  const nestedSheet = new CSSStyleSheet();
  nestedSheet.replaceSync('.parent { color: red; .child { color: blue; } font-size: 12px; }');
  const nestedRule = nestedSheet.cssRules[0];
  const nested = nestedRule.cssRules[1];
  nested.style.setProperty('overflow', 'hidden visible', 'important');
  const nestedWrite = [
    nested.style.getPropertyValue('overflow'),
    nested.style.getPropertyPriority('overflow'),
    nested.style.cssText,
    nested.cssText,
    nestedRule.cssText.includes('overflow: hidden visible !important;')
  ].join(',');

  return [ruleWrite, cssTextWrite, nestedWrite].join('|');
})()
"#,
        )
        .expect("CSS rule styles should use Stylo declaration block");

    assert_eq!(
        result,
        "2,align-content,justify-content,center start,important,place-content: center start !important;,div { place-content: center start !important; }|red,1px 2px,color: red; padding: 1px 2px;,div { color: red; padding: 1px 2px; }|hidden visible,important,font-size: 12px; overflow: hidden visible !important;,font-size: 12px; overflow: hidden visible !important;,true"
    );
}
#[test]
fn css_rule_style_sync_uses_internal_declaration_state_not_css_text_property() {
    let mut vm = new_storage_test_vm("https://rule-style-internal-sync.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    div {}
    @keyframes fade { from {} }
    .parent { .child { color: blue; } font-size: 12px; }
  `);

  const rule = sheet.cssRules[0];
  Object.defineProperty(rule.style, 'cssText', {
    get() { return 'opacity: 0;'; },
    configurable: true
  });
  rule.style.setProperty('color', 'red');

  const keyframe = sheet.cssRules[1].cssRules[0];
  Object.defineProperty(keyframe.style, 'cssText', {
    get() { return 'opacity: 0;'; },
    configurable: true
  });
  keyframe.style.setProperty('opacity', '0.5');

  const parent = sheet.cssRules[2];
  const nested = parent.cssRules[1];
  Object.defineProperty(nested.style, 'cssText', {
    get() { return 'font-size: 1px;'; },
    configurable: true
  });
  nested.style.setProperty('color', 'green');

  return [
    rule.cssText,
    keyframe.cssText,
    nested.cssText,
    parent.cssText.includes('font-size: 12px; color: green;')
  ].join('|');
})()
"#,
        )
        .expect("CSS rule style sync should use internal declaration state");

    assert_eq!(
        result,
        "div { color: red; }|0% { opacity: 0.5; }|font-size: 12px; color: green;|true"
    );
}
#[test]
fn css_rule_style_assignment_uses_internal_declaration_setter() {
    let mut vm = new_storage_test_vm("https://rule-style-internal-assignment.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    div {}
    @keyframes fade { from {} }
    .parent { .child { color: blue; } font-size: 12px; }
    @page :first { margin-top: 1px; @top-left { content: "x"; } }
  `);

  const rule = sheet.cssRules[0];
  Object.defineProperty(rule.style, 'cssText', {
    value: 'opacity: 0;',
    writable: true,
    configurable: true
  });
  rule.style = 'color: rgb(0 128 0 / 50%); width: 0;';

  const keyframe = sheet.cssRules[1].cssRules[0];
  Object.defineProperty(keyframe.style, 'cssText', {
    value: 'opacity: 0;',
    writable: true,
    configurable: true
  });
  keyframe.style = 'background-color: rgb(0 128 0 / 50%); opacity: 1;';

  const parent = sheet.cssRules[2];
  const nested = parent.cssRules[1];
  Object.defineProperty(nested.style, 'cssText', {
    value: 'font-size: 1px;',
    writable: true,
    configurable: true
  });
  nested.style = 'color: rgb(0 128 0 / 50%); width: 0;';

  const page = sheet.cssRules[3];
  const margin = page.cssRules[0];
  Object.defineProperty(page.style, 'cssText', {
    value: 'margin-top: 99px;',
    writable: true,
    configurable: true
  });
  page.style = 'margin-top: 10px;';

  Object.defineProperty(margin.style, 'cssText', {
    value: 'content: "bad";',
    writable: true,
    configurable: true
  });
  margin.style = 'content: "y"; color: red;';

  return [
    rule.style.getPropertyValue('color'),
    rule.cssText,
    keyframe.style.getPropertyValue('background-color'),
    keyframe.cssText,
    nested.style.getPropertyValue('width'),
    nested.cssText,
    parent.cssText.includes('color: rgba(0, 128, 0, 0.5); width: 0px;'),
    page.style.getPropertyValue('margin-top'),
    margin.style.getPropertyValue('content'),
    margin.style.getPropertyValue('color'),
    page.cssText.includes('margin-top: 10px;'),
    page.cssText.includes('content: "y"; color: red;')
  ].join('|');
})()
"#,
        )
        .expect("CSS rule style assignment should use internal declaration setter");

    assert_eq!(
        result,
        "rgba(0, 128, 0, 0.5)|div { color: rgba(0, 128, 0, 0.5); width: 0px; }|rgba(0, 128, 0, 0.5)|0% { background-color: rgba(0, 128, 0, 0.5); opacity: 1; }|0px|color: rgba(0, 128, 0, 0.5); width: 0px;|true|10px|\"y\"|red|true|true"
    );
}
#[test]
fn css_keyframes_rule_name_setter_serializes_reserved_names_as_strings() {
    let mut vm = new_storage_test_vm("https://css-keyframes-name-setter.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('@keyframes spin {}');
  const rule = sheet.cssRules[0];
  const probe = (name) => {
    rule.name = name;
    const cssText = rule.cssText;
    sheet.insertRule(cssText, 1);
    sheet.deleteRule(1);
    return `${rule.name}:${cssText.replace(/\s/g, '')}`;
  };
  return [
    probe('default'),
    probe('revert-rule'),
    probe('initial')
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframesRule name setter serialization should evaluate");

    assert_eq!(
        result,
        "default:@keyframes\"default\"{}|revert-rule:@keyframes\"revert-rule\"{}|initial:@keyframes\"initial\"{}"
    );
}
#[test]
fn rule_css_entries_expand_unresolved_box_shorthand_before_longhand_mutation() {
    let mut vm = new_storage_test_vm("https://rule-style-unresolved-box-shorthand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const margin = sheet.cssRules[0].style;
  margin.setProperty('margin', 'var(--m)');
  margin.setProperty('margin-top', '10px');
  const setPropertyPath = [
    margin.cssText,
    margin.getPropertyValue('margin'),
    margin.getPropertyValue('margin-right'),
    margin.getPropertyValue('margin-top'),
    Array.from({ length: margin.length }, (_, index) => margin.item(index)).join('/')
  ].join(',');

  const sheet2 = new CSSStyleSheet();
  sheet2.insertRule('div {}');
  const padding = sheet2.cssRules[0].style;
  padding.padding = 'var(--p)';
  padding.paddingLeft = 'calc(calc(1px))';
  const namedSetterPath = [
    padding.cssText,
    padding.getPropertyValue('padding'),
    padding.getPropertyValue('padding-right'),
    padding.paddingLeft,
    Array.from({ length: padding.length }, (_, index) => padding.item(index)).join('/')
  ].join(',');

  return [setPropertyPath, namedSetterPath].join('|');
})()
"#,
        )
        .expect(
            "rule style entries should expand unresolved box shorthand before longhand mutation",
        );

    assert_eq!(
        result,
        "margin-right: ; margin-bottom: ; margin-left: ; margin-top: 10px;,,,10px,margin-right/margin-bottom/margin-left/margin-top|padding-top: ; padding-right: ; padding-bottom: ; padding-left: calc(1px);,,,calc(1px),padding-top/padding-right/padding-bottom/padding-left"
    );
}
#[test]
fn css_rule_style_pdb_priority_survives_css_text_reset() {
    let mut vm = new_storage_test_vm("https://rule-style-pdb-priority-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div { color: red; }');
  const rule = sheet.cssRules[0];
  rule.cssText = 'div { display: block; }';
  rule.style.setProperty('opacity', '0.5', 'important');
  return [
    rule.style.getPropertyValue('opacity'),
    rule.style.getPropertyPriority('opacity'),
    rule.style.cssText,
    rule.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule PDB priority should survive cssText reset");

    assert_eq!(
        result,
        "0.5|important|display: block; opacity: 0.5 !important;|div { display: block; opacity: 0.5 !important; }"
    );
}
#[test]
fn css_rule_css_text_reset_uses_pdb_canonical_declarations() {
    let mut vm = new_storage_test_vm("https://rule-style-pdb-canonical-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: red; } @keyframes fade { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  const keyframe = sheet.cssRules[1].cssRules[0];

  rule.cssText = 'div { color: rgb(0 128 0 / 50%); width: 0; }';
  keyframe.cssText = 'from { background-color: rgb(0 128 0 / 50%); opacity: 1; }';

  return [
    rule.cssText,
    rule.style.cssText,
    keyframe.cssText,
    keyframe.style.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSRule cssText reset should canonicalize safe declarations with PDB");

    assert_eq!(
        result,
        "div { color: rgba(0, 128, 0, 0.5); width: 0px; }|color: rgba(0, 128, 0, 0.5); width: 0px;|0% { background-color: rgba(0, 128, 0, 0.5); opacity: 1; }|background-color: rgba(0, 128, 0, 0.5); opacity: 1;"
    );
}
