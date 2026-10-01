use super::*;

#[test]
fn css_rule_native_declarations_preserve_pending_shorthands_on_style_access() {
    let mut vm = new_storage_test_vm("https://css-rule-native-declarations.test/");
    let result = vm.eval(r#"
(() => {
  const declaration = 'border: var(--edge); border-bottom: 0;';
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    .test { ${declaration} }
    @media all { .test { ${declaration} } }
    .host { & .child { color: blue; } ${declaration} }
    @keyframes k { from { ${declaration} } }
  `);
  const rules = [sheet.cssRules[0], sheet.cssRules[1].cssRules[0],
                 sheet.cssRules[2].cssRules[1], sheet.cssRules[3].cssRules[0]];
  for (const rule of rules) {
    const before = rule.cssText;
    const style = rule.style;
    const declarations = style.cssText;
    if (before !== rule.cssText) throw new Error('getter changed rule: ' + before + ' -> ' + rule.cssText);
    if (style !== rule.style) throw new Error('style must be SameObject');
    for (let i = 0; i < style.length; ++i) {
      if (style.item(i) !== style[i]) throw new Error('indexed declaration names differ');
    }
    style.color = 'blue';
    style.removeProperty('color');
    if (before !== rule.cssText || declarations !== style.cssText)
      throw new Error('unrelated property mutation lost pending declarations: ' + rule.cssText);
  }
  return 'ok';
})()
"#).expect("native pending substitution must survive CSSOM reads and writes");
    assert_eq!(result, "ok");
}

#[test]
fn css_rule_native_declarations_preserve_computed_border_after_unrelated_mutation() {
    let mut vm = new_storage_test_vm("https://css-rule-native-computed.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = '.test { --edge: 7px solid red; border: var(--edge); border-bottom: 0; }';
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  html.appendChild(style);
  const element = document.createElement('div');
  element.className = 'test';
  html.appendChild(element);
  const read = () => {
    const c = getComputedStyle(element);
    return [c.borderTopWidth, c.borderTopStyle, c.borderTopColor, c.borderBottomWidth].join(',');
  };
  const before = read();
  style.sheet.cssRules[0].style.color = 'blue';
  return [before, read()].join('|');
})()
"#,
        )
        .expect("CSSOM mutation should preserve the native block used by computed style");
    assert_eq!(
        result,
        "7px,solid,rgb(255, 0, 0),0px|7px,solid,rgb(255, 0, 0),0px"
    );
}

#[test]
fn css_rule_native_declarations_use_stylesheet_url_context_for_property_mutations() {
    let mut vm = new_storage_test_vm("https://css-rule-native-url.test/styles/page.html");
    let result = vm.eval(r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const style = document.createElement('style');
  style.textContent = '.test { --edge: 7px solid red; border: var(--edge); border-bottom: 0; }';
  html.appendChild(style);
  const element = document.createElement('div'); element.className = 'test'; html.appendChild(element);
  const rule = style.sheet.cssRules[0];
  rule.style.setProperty('background-image', 'url(./image.png)');
  const initial = getComputedStyle(element).backgroundImage;
  rule.style.setProperty('--image', 'url(./variable.png)');
  rule.style.backgroundImage = 'var(--image)';
  const variable = getComputedStyle(element).backgroundImage;
  rule.style.cssText = '--edge: 9px solid green; border: var(--edge); border-bottom: 0; background-image: url(./reset.png);';
  rule.style.color = 'blue';
  const final = getComputedStyle(element);
  return [initial, variable, final.backgroundImage, final.borderTopWidth, final.borderTopColor].join('|');
})()
"#).expect("stylesheet URL context must apply to new declarations and custom properties");
    assert_eq!(
        result,
        "url(\"https://css-rule-native-url.test/styles/image.png\")|url(\"https://css-rule-native-url.test/styles/variable.png\")|url(\"https://css-rule-native-url.test/styles/reset.png\")|9px|rgb(0, 128, 0)"
    );
}

#[test]
fn css_rule_native_declarations_keep_cssom_and_computed_values_in_sync_in_quirks_mode() {
    for (doctype, expected) in [
        ("", "BackCompat|5px,5px|12px,12px|3px,3px"),
        ("<!doctype html>", "CSS1Compat|7px,7px|16px,16px|2px,2px"),
    ] {
        let mut vm = new_parsed_test_vm(
            "https://css-rule-native-quirks.test/",
            &format!(
                "{doctype}<html><head><style>.test {{ width: 7px; font-size: 16px; border: 2px solid red; }}</style></head><body><div class=test></div></body></html>"
            ),
        );
        let result = vm.eval(r#"
(() => {
  const style = document.styleSheets[0].cssRules[0].style;
  const element = document.querySelector('.test');
  style.width = '5';
  style.setProperty('font-size', '12');
  style.borderTopWidth = '3';
  const before = style.cssText;
  style.width = 'bogus';
  style.setProperty('border', '2px bogus blue');
  if (style.cssText !== before) throw new Error('invalid declaration changed CSSOM');
  const computed = getComputedStyle(element);
  return [document.compatMode, [style.width, computed.width],
          [style.fontSize, computed.fontSize], [style.borderTopWidth, computed.borderTopWidth]].join('|');
})()
"#).expect("CSSOM and computed style must use the same stylesheet parser context");
        assert_eq!(result, expected);
    }
}

#[test]
fn css_rule_detached_native_declarations_survive_delete_and_replace() {
    let mut vm = new_storage_test_vm("https://css-rule-detached-native.test/");
    let result = vm.eval(r#"
(() => {
  const declaration = 'border: var(--edge); border-bottom: 0;';
  for (const kind of ['style', 'nested-style', 'nested-declarations', 'keyframe']) {
    for (const replace of [false, true]) {
      for (const readStyleBefore of [false, true]) {
        const sheet = new CSSStyleSheet();
        sheet.replaceSync(kind === 'style' ? `.test { ${declaration} }` :
          kind === 'nested-style' ? `@media all { .test { ${declaration} } }` :
          kind === 'nested-declarations' ? `.host { & .child { color: blue; } ${declaration} }` :
          `@keyframes k { from { ${declaration} } }`);
        const root = sheet.cssRules[0];
        const rule = kind === 'style' ? root : root.cssRules[kind === 'nested-declarations' ? 1 : 0];
        const before = rule.cssText;
        const previousStyle = readStyleBefore ? rule.style : null;
        if (replace) sheet.replaceSync('.replacement { color: green; }');
        else sheet.deleteRule(0);
        const style = rule.style;
        if (previousStyle && style !== previousStyle) throw new Error('lost style identity');
        if (before !== rule.cssText) throw new Error(`${kind}/${replace}/${readStyleBefore}: detach or style read changed declarations`);
        style.color = 'blue';
        style.removeProperty('color');
        if (before !== rule.cssText) throw new Error(`${kind}/${replace}/${readStyleBefore}: mutation lost pending declarations`);
        if (replace && sheet.cssRules[0].style.color !== 'green') throw new Error('mutated replacement stylesheet');
      }
    }
  }
  return 'ok';
})()
"#).expect("retained rules must preserve native declarations before and after style materialization");
    assert_eq!(result, "ok");
}

#[test]
fn css_rule_detached_unmaterialized_descendants_keep_native_declarations_lazy() {
    use crate::context_bootstrap::css_stylesheet_runtime::{
        css_rule_declaration_block_creation_count_for_test,
        reset_css_rule_declaration_block_creation_count_for_test,
    };
    let mut vm = new_storage_test_vm("https://css-rule-detached-unread.test/");
    reset_css_rule_declaration_block_creation_count_for_test();
    vm.eval(r#"
globalThis.detachedSheet = new CSSStyleSheet();
detachedSheet.replaceSync('@media all { .host { & .child { border: var(--edge); border-bottom: 0; } } @keyframes k { from { border: var(--edge); border-bottom: 0; } } }');
globalThis.detachedRoot = detachedSheet.cssRules[0];
detachedSheet.replaceSync('.replacement { color: green; }');
"#).expect("detach must retain unread descendant declarations without CSSOM blocks");
    assert_eq!(css_rule_declaration_block_creation_count_for_test(), 0);
    let result = vm.eval(r#"
(() => {
  const rules = [detachedRoot.cssRules[0].cssRules[0], detachedRoot.cssRules[1].cssRules[0]];
  for (const rule of rules) {
    const before = rule.cssText;
    const style = rule.style;
    if (before !== rule.cssText) throw new Error('first access lost unread descendant declarations');
    style.color = 'blue'; style.removeProperty('color');
    if (before !== rule.cssText) throw new Error('descendant mutation lost pending declarations');
  }
  return 'ok';
})()
"#).expect("first access to detached descendants must retain their original native declarations");
    assert_eq!(result, "ok");
    assert_eq!(css_rule_declaration_block_creation_count_for_test(), 2);
}

#[test]
fn css_rule_detached_native_declaration_sources_survive_gc_and_release_with_context() {
    use crate::context_bootstrap::css_stylesheet_runtime::detached_declaration_source_count_for_test;
    let before = detached_declaration_source_count_for_test();
    let mut vm = new_storage_test_vm("https://css-rule-detached-source-lifetime.test/");
    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media all { .test { border: var(--edge); border-bottom: 0; } }');
  globalThis.heldRule = sheet.cssRules[0].cssRules[0];
  globalThis.heldText = heldRule.cssText;
  sheet.deleteRule(0);
  return 'held';
})()
"#,
    )
    .expect("the detached rule should own its declaration source without its original sheet");
    assert!(detached_declaration_source_count_for_test() > before);
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.low_memory_notification();
            Ok(())
        })
        .expect("GC should complete while the detached rule is retained");
    assert_eq!(
        vm.eval("void heldRule.style; String(heldRule.cssText === heldText)")
            .expect("retained declaration source must survive GC"),
        "true"
    );
    drop(vm);
    assert_eq!(detached_declaration_source_count_for_test(), before);
}

#[test]
fn css_rule_detached_native_declaration_sources_release_after_rules_are_collected() {
    use crate::context_bootstrap::css_stylesheet_runtime::detached_declaration_source_count_for_test;
    let before = detached_declaration_source_count_for_test();
    let mut vm = new_storage_test_vm("https://css-rule-detached-source-collection.test/");
    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media all { .test { border: var(--edge); border-bottom: 0; } }');
  globalThis.heldRule = sheet.cssRules[0];
  sheet.deleteRule(0);
  return 'held';
})()
"#,
    )
    .expect("the detached tree should retain its unread native declarations");
    assert!(detached_declaration_source_count_for_test() > before);
    vm.eval("delete globalThis.heldRule; 'released'")
        .expect("remove the last script reference to the detached tree");
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.low_memory_notification();
            Ok(())
        })
        .expect("GC should collect the unreachable detached tree");
    assert_eq!(detached_declaration_source_count_for_test(), before);
    assert_eq!(vm.eval("'still alive'").unwrap(), "still alive");
}

#[test]
fn css_rule_detached_native_declaration_sources_are_invalidated_by_css_text_reset() {
    let mut vm = new_storage_test_vm("https://css-rule-detached-source-reset.test/");
    let result = vm.eval(r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.host { color: red; & .old { border: var(--edge); border-bottom: 0; } } @keyframes k { from { color: red; } }');
  const rule = sheet.cssRules[0], keyframe = sheet.cssRules[1].cssRules[0];
  sheet.replaceSync('.replacement { color: green; }');
  rule.cssText = '.new { color: blue; & .new-child { color: purple; } }';
  keyframe.cssText = '25% { color: blue; }';
  return [rule.style.color, rule.cssRules[0].selectorText, rule.cssRules[0].style.color,
          keyframe.style.color, keyframe.keyText, sheet.cssRules[0].style.color].join('|');
})()
"#).expect("full rule replacement must not restore the old native declaration source or children");
    assert_eq!(result, "blue|& .new-child|purple|blue|25%|green");
}

#[test]
fn css_rule_detached_native_declaration_sources_survive_sibling_insert_and_delete() {
    let mut vm = new_storage_test_vm("https://css-rule-detached-siblings.test/");
    let result = vm.eval(r#"
(() => {
  const declaration = 'border: var(--edge); border-bottom: 0;';
  for (const kind of ['media', 'style', 'keyframes']) {
    for (const insert of [false, true]) {
      const text = kind === 'media' ? `@media all { .first { color: red; } .keep { ${declaration} } }` :
        kind === 'style' ? `.host { & .first { color: red; } & .keep { ${declaration} } }` :
        `@keyframes k { from { opacity: 0; } to { ${declaration} } }`;
      const reference = new CSSStyleSheet(); reference.replaceSync(text);
      const expected = reference.cssRules[0].cssRules[1].cssText;
      const sheet = new CSSStyleSheet(); sheet.replaceSync(text);
      const root = sheet.cssRules[0];
      sheet.deleteRule(0);
      // Do not materialize the retained child before changing its siblings.
      if (kind === 'keyframes') {
        if (insert) root.appendRule('50% { opacity: 0.5; }');
        else root.deleteRule('0%');
      } else {
        if (insert) root.insertRule('.new { color: blue; }', 0);
        else root.deleteRule(0);
      }
      const child = root.cssRules[insert ? (kind === 'keyframes' ? 1 : 2) : 0];
      if (child.cssText !== expected) throw new Error(`${kind}/${insert}: sibling mutation changed native declarations`);
      const style = child.style;
      if (child.cssText !== expected) throw new Error(`${kind}/${insert}: first style read lost native declarations`);
      style.color = 'blue'; style.removeProperty('color');
      if (child.cssText !== expected) throw new Error(`${kind}/${insert}: declaration mutation lost native declarations`);
    }
  }
  return 'ok';
})()
"#).expect("sibling edits must preserve unread native declaration sources");
    assert_eq!(result, "ok");
}

#[test]
fn css_rule_declaration_blocks_are_lazy_for_reads() {
    use crate::context_bootstrap::css_stylesheet_runtime::{
        css_rule_declaration_block_creation_count_for_test,
        reset_css_rule_declaration_block_creation_count_for_test,
    };

    let mut vm = new_storage_test_vm("https://css-rule-lazy-declarations.test/");
    reset_css_rule_declaration_block_creation_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    .first { color: red; }
    .second { color: green; }
    @media screen { .inside { color: blue; } }
    @keyframes pulse { from { opacity: 0; } to { opacity: 1; } }
    @supports (display: grid) {
      .host { & .child { color: blue; } color: red; margin: 0; }
    }
  `);
  const seen = [];
  const visit = rules => {
    for (const rule of rules) {
      seen.push([rule.type, rule.cssText, rule.selectorText]);
      if (rule.cssRules) visit(rule.cssRules);
    }
  };
  visit(sheet.cssRules);
  const first = sheet.cssRules[0];
  const mediaChild = sheet.cssRules[2].cssRules[0];
  const keyframe = sheet.cssRules[3].cssRules[0];
  const declarations = sheet.cssRules[4].cssRules[0].cssRules[1];
  globalThis.__lazyDeclarationRules = { sheet, first, mediaChild, keyframe, declarations };
  return [sheet.cssRules.length, seen.length, first.cssText,
          keyframe.cssText, declarations.cssText].join('|');
})()
"#,
        )
        .expect("reading native CSS rules should not need a second declaration block");
    assert_eq!(
        result,
        "5|11|.first { color: red; }|0% { opacity: 0; }|color: red; margin: 0px;"
    );
    assert_eq!(
        css_rule_declaration_block_creation_count_for_test(),
        0,
        "enumeration, selectors, cssText and nested-rule reads must not create declaration blocks"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const { first, mediaChild, keyframe, declarations } = __lazyDeclarationRules;
  const firstStyle = first.style;
  const keyframeStyle = keyframe.style;
  const initial = [firstStyle === first.style, firstStyle.color, mediaChild.style.color,
                   keyframeStyle.opacity, declarations.style.margin].join(',');
  first.selectorText = '.renamed';
  firstStyle.color = 'purple';
  keyframe.cssText = '25% { opacity: 0.5; }';
  return [initial, first.cssText, keyframeStyle === keyframe.style,
          keyframeStyle.opacity, keyframe.cssText].join('|');
})()
"#,
        )
        .expect("first style access and later native synchronization should retain live identity");
    assert_eq!(
        result,
        "true,red,blue,0,0px|.renamed { color: purple; }|true|0.5|25% { opacity: 0.5; }"
    );
    assert_eq!(
        css_rule_declaration_block_creation_count_for_test(),
        4,
        "only rules whose style is accessed should allocate, without reallocating on synchronization"
    );
}

#[test]
fn css_rule_declaration_blocks_stay_lazy_across_detach() {
    use crate::context_bootstrap::css_stylesheet_runtime::{
        css_rule_declaration_block_creation_count_for_test,
        reset_css_rule_declaration_block_creation_count_for_test,
    };

    let mut vm = new_storage_test_vm("https://css-rule-lazy-declarations-detach.test/");
    reset_css_rule_declaration_block_creation_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    .original { color: red; }
    @keyframes pulse { from { opacity: 0; } }
    @supports (display: grid) {
      .host { & .child { color: blue; } color: red; }
    }
  `);
  const rule = sheet.cssRules[0];
  const keyframes = sheet.cssRules[1];
  const keyframe = keyframes.cssRules[0];
  const parent = sheet.cssRules[2].cssRules[0];
  const declarations = parent.cssRules[1];
  rule.selectorText = '.renamed';
  keyframe.keyText = '50%';
  sheet.replaceSync('.replacement { color: purple; }');
  globalThis.__detachedLazyDeclarationRules = { sheet, rule, keyframes, keyframe, parent, declarations };
  return [rule.cssText, keyframe.cssText, declarations.cssText,
          keyframe.parentRule === keyframes, declarations.parentRule === parent,
          sheet.cssRules[0].cssText].join('|');
})()
"#,
        )
        .expect("detaching unread styles should preserve snapshots without declaration blocks");
    assert_eq!(
        result,
        ".renamed { color: red; }|50% { opacity: 0; }|color: red;|true|true|.replacement { color: purple; }"
    );
    assert_eq!(css_rule_declaration_block_creation_count_for_test(), 0);

    let result = vm
        .eval(
            r#"
(() => {
  const { sheet, rule, keyframes, keyframe, parent, declarations } = __detachedLazyDeclarationRules;
  rule.style.color = 'blue';
  keyframe.style.opacity = '0.75';
  declarations.style.color = 'green';
  return [rule.cssText, keyframe.cssText, declarations.cssText,
          keyframes.cssText.includes('opacity: 0.75;'),
          parent.cssText.includes('color: green;'), sheet.cssRules[0].cssText].join('|');
})()
"#,
        )
        .expect("styles first accessed after detach should mutate only their retained old graph");
    assert_eq!(
        result,
        ".renamed { color: blue; }|50% { opacity: 0.75; }|color: green;|true|true|.replacement { color: purple; }"
    );
    assert_eq!(css_rule_declaration_block_creation_count_for_test(), 3);
}

#[test]
fn css_rule_declaration_blocks_are_not_needed_for_native_css_text_updates() {
    use crate::context_bootstrap::css_stylesheet_runtime::{
        css_rule_declaration_block_creation_count_for_test,
        reset_css_rule_declaration_block_creation_count_for_test,
    };

    let mut vm = new_storage_test_vm("https://css-rule-lazy-declarations-update.test/");
    reset_css_rule_declaration_block_creation_count_for_test();
    vm.eval(
        r#"
globalThis.__lazyUpdateSheet = new CSSStyleSheet();
__lazyUpdateSheet.replaceSync('.one { color: red; } @keyframes fade { from { opacity: 0; } }');
globalThis.__lazyUpdateRule = __lazyUpdateSheet.cssRules[0];
globalThis.__lazyUpdateKeyframe = __lazyUpdateSheet.cssRules[1].cssRules[0];
"#,
    )
    .expect("unmaterialized styles should retain native rule bindings");
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();

    let result = vm
        .eval(
            r#"
__lazyUpdateRule.cssText = '.one { color: blue; }';
__lazyUpdateKeyframe.cssText = 'from { opacity: .25; }';
[__lazyUpdateRule.cssText, __lazyUpdateKeyframe.cssText,
 __lazyUpdateSheet.cssRules[0] === __lazyUpdateRule,
 __lazyUpdateSheet.cssRules[1].cssRules[0] === __lazyUpdateKeyframe].join('|');
"#,
        )
        .expect("same-selector cssText writes should update native declarations in place");
    assert_eq!(
        result,
        ".one { color: blue; }|0% { opacity: 0.25; }|true|true"
    );
    assert_eq!(css_rule_declaration_block_creation_count_for_test(), 0);
    let metrics = crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_rule_value_mutations, 2);
    assert_eq!(metrics.native_top_level_mutations, 0);
    assert_eq!(metrics.native_nested_mutations, 0);
    assert_eq!(metrics.native_keyframe_mutations, 0);
}
