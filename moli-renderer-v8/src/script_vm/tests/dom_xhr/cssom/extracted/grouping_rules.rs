use super::*;

#[test]
fn css_grouping_rule_insert_delete_materializes_stylo_mutation_children() {
    let mut vm = new_storage_test_vm("https://css-grouping-rule-mutation-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { .one { color: red; } }');
  const media = sheet.cssRules[0];
  const existing = media.cssRules[0];
  const index = media.insertRule('@supports (display: grid) { .two { display: grid; } }', 1);
  const supports = media.cssRules[1];
  media.deleteRule(0);
  return [
    index,
    media.cssRules.length,
    media.cssRules[0] === supports,
    existing.parentRule === null,
    supports instanceof CSSSupportsRule,
    supports.cssRules[0].cssText,
    media.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSGroupingRule Stylo mutation view should evaluate");

    assert_eq!(
        result,
        "1|1|true|true|true|.two { display: grid; }|@media screen {\n  @supports (display: grid) {\n  .two { display: grid; }\n}\n}"
    );
}
#[test]
fn css_grouping_rule_css_text_reset_refreshes_existing_child_rule_list_from_stylo() {
    let mut vm = new_storage_test_vm("https://css-grouping-rule-css-text-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { .old { color: red; } }');
  const media = sheet.cssRules[0];
  const rules = media.cssRules;
  const old = rules[0];
  media.cssText = '@media screen { .new { color: blue; } @supports (display: grid) { .grid { display: grid; } } }';
  return [
    media.cssRules === rules,
    rules.length,
    old.parentRule === null,
    rules[0].selectorText,
    rules[0].cssText,
    rules[1] instanceof CSSSupportsRule,
    rules[1].cssRules[0].cssText,
    Array.from(rules).map(rule => rule.parentRule === media).join(','),
  ].join('|');
})()
"#,
        )
        .expect("CSSGroupingRule cssText reset should refresh existing child cssRules");

    assert_eq!(
        result,
        "true|2|true|.new|.new { color: blue; }|true|.grid { display: grid; }|true,true"
    );
}
#[test]
fn css_grouping_rule_css_text_reset_uses_native_rule_serialization() {
    let mut vm = new_storage_test_vm("https://css-grouping-rule-css-text-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { .old { color: red; } }');
  const media = sheet.cssRules[0];
  media.cssText = '@media screen { .new { margin: 0; } }';
  return [
    media.cssText,
    media.cssRules[0].cssText,
    sheet.cssRules[0].cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSGroupingRule cssText reset should use Stylo serialization");

    assert_eq!(
        result,
        "@media screen {\n  .new { margin: 0px; }\n}|.new { margin: 0px; }|@media screen {\n  .new { margin: 0px; }\n}"
    );
}
#[test]
fn css_grouping_insert_rule_materializes_supported_at_rules_from_native_seeds() {
    let mut vm = new_storage_test_vm("https://css-grouping-insert-supported-at-rules.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen {}');
  const media = sheet.cssRules[0];
  const supportsIndex = media.insertRule('@supports (display: grid) { .s { display: grid; } }', 0);
  const containerIndex = media.insertRule('@container card (min-width: 10px) { .c { margin: 0; } }', 1);
  const layerIndex = media.insertRule('@layer inner { .l { padding: 1px; } }', 2);
  const scopeIndex = media.insertRule('@scope (.host) { .sc { opacity: 1; } }', 3);
  const [supports, container, layer, scope] = Array.from(media.cssRules);

  return [
    supportsIndex,
    containerIndex,
    layerIndex,
    scopeIndex,
    supports.constructor === CSSSupportsRule,
    supports.cssRules[0].cssText,
    container.constructor === CSSContainerRule,
    container.containerName,
    container.cssRules[0].cssText,
    layer.constructor === CSSLayerBlockRule,
    layer.name,
    layer.cssRules[0].cssText,
    scope.constructor === CSSScopeRule,
    scope.start,
    scope.cssRules[0].cssText,
    sheet.cssRules[0].cssText === media.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSGroupingRule.insertRule should materialize supported at-rules from native seeds");

    assert_eq!(
        result,
        "0|1|2|3|true|.s { display: grid; }|true|card|.c { margin: 0px; }|true|inner|.l { padding: 1px; }|true|.host|.sc { opacity: 1; }|true"
    );
}
#[test]
fn css_grouping_child_css_text_reset_syncs_parent_rule_from_stylo() {
    let mut vm = new_storage_test_vm("https://css-grouping-child-css-text-parent-sync.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { @supports (display: grid) { .old { color: red; } } }');
  const media = sheet.cssRules[0];
  const supports = media.cssRules[0];
  supports.cssText = '@supports (display: flex) { .new { margin: 0; } }';
  return [
    supports.cssText,
    supports.cssRules[0].cssText,
    media.cssText,
    sheet.cssRules[0].cssText,
  ].join('|');
})()
"#,
        )
        .expect("nested CSSGroupingRule cssText reset should sync parent rule");

    assert_eq!(
        result,
        "@supports (display: flex) {\n  .new { margin: 0px; }\n}|.new { margin: 0px; }|@media screen {\n  @supports (display: flex) {\n  .new { margin: 0px; }\n}\n}|@media screen {\n  @supports (display: flex) {\n  .new { margin: 0px; }\n}\n}"
    );
}
#[test]
fn css_media_rule_exposes_mutable_media_list() {
    let mut vm = new_storage_test_vm("https://css-media-list.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('@media screen and (min-width: 480px), print, projection { body { color: red; } }');
  const rule = sheet.cssRules[0];
  const media = rule.media;
  const before = [
    media.length,
    media.mediaText,
    media.toString(),
    media[0],
    media[1],
    media[2],
    media[3] === undefined,
    media.item(0),
    media.item(3) === null
  ].join('|');
  media.deleteMedium('print');
  const afterDelete = [media.length, media.mediaText, media[1], media[2] === undefined, media.item(2) === null].join('|');
  media.appendMedium('speech');
  const afterAppend = [media.length, media.mediaText, media[2], media[3] === undefined, media.item(3) === null].join('|');
  media.mediaText = null;
  const afterNull = [media.length, media.mediaText, media.toString()].join('|');
  rule.media = 'speech';
  const afterRuleSetter = [rule.media === media, rule.media.mediaText, rule.conditionText].join('|');
  return [before, afterDelete, afterAppend, afterNull, afterRuleSetter].join('||');
})()
"#,
        )
        .expect("CSSMediaRule MediaList surface should evaluate");

    assert_eq!(
        result,
        "3|screen and (min-width: 480px), print, projection|screen and (min-width: 480px), print, projection|screen and (min-width: 480px)|print|projection|true|screen and (min-width: 480px)|true||2|screen and (min-width: 480px), projection|projection|true|true||3|screen and (min-width: 480px), projection, speech|speech|true|true||0||||true|speech|speech"
    );
}
#[test]
fn css_conditional_rule_idl_surface_matches_wpt_shape() {
    let mut vm = new_storage_test_vm("https://css-conditional-idl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return error && error.name;
    }
  };
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    @media screen { }
    @media print { }
    @supports (display: block) { }
    @supports (does-not-exist: nope) { }
    @supports (color : red) or ( color:blue ) { }
  `);
  const screen = sheet.cssRules[0];
  const print = sheet.cssRules[1];
  const supported = sheet.cssRules[2];
  const unsupported = sheet.cssRules[3];
  const spaced = sheet.cssRules[4];
  return [
    CSS.supports.length,
    screen.matches,
    print.matches,
    supported.matches,
    unsupported.matches,
    spaced.conditionText,
    probe(() => CSSConditionRule.prototype.conditionText),
    probe(() => CSSMediaRule.prototype.media),
    probe(() => CSSMediaRule.prototype.matches),
    probe(() => CSSSupportsRule.prototype.matches),
    typeof Object.getOwnPropertyDescriptor(CSSMediaRule.prototype, 'matches').get,
    typeof Object.getOwnPropertyDescriptor(CSSSupportsRule.prototype, 'matches').get
  ].join('|');
})()
"#,
        )
        .expect("CSS conditional rule IDL surface should evaluate");

    assert_eq!(
        result,
        "1|true|false|true|false|(color : red) or ( color:blue )|TypeError|TypeError|TypeError|TypeError|function|function"
    );
}
#[test]
fn css_media_rule_serializes_normalized_query_and_block() {
    let mut vm = new_storage_test_vm("https://css-media-rule-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('@media {}', 0);
  const emptyList = sheet.cssRules[0].cssText;
  sheet.insertRule('@media spEech {}', 0);
  const empty = sheet.cssRules[0].cssText;
  sheet.insertRule('@media all and (WiDtH) {}', 0);
  const feature = sheet.cssRules[0].cssText;
  sheet.cssRules[0].insertRule('#foo { z-index: 23; float: left; }', 0);
  const nested = sheet.cssRules[0].cssText;
  sheet.insertRule('@media all and (not-a-real-feature) {}', 0);
  const unknownFeature = sheet.cssRules[0].cssText;
  return [emptyList, empty, feature, nested, unknownFeature].join('||');
})()
"#,
        )
        .expect("CSSMediaRule serialization should evaluate");

    assert_eq!(
        result,
        "@media  {\n}||@media speech {\n}||@media (width) {\n}||@media (width) {\n  #foo { z-index: 23; float: left; }\n}||@media (not-a-real-feature) {\n}"
    );
}
#[test]
fn css_media_rule_media_mutation_uses_native_rule_serialization() {
    let mut vm = new_storage_test_vm("https://css-media-rule-media-mutation-stylo.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@supports (display: grid) { @media screen { .new { margin: 0; } } }');
  const supports = sheet.cssRules[0];
  const media = supports.cssRules[0];
  media.media.mediaText = 'all and (WiDtH)';
  return [
    media.media.mediaText,
    media.cssText,
    supports.cssText,
    sheet.cssRules[0].cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSMediaRule media mutation should use Stylo rule serialization");

    assert_eq!(
        result,
        "(width)|@media (width) {\n  .new { margin: 0px; }\n}|@supports (display: grid) {\n  @media (width) {\n  .new { margin: 0px; }\n}\n}|@supports (display: grid) {\n  @media (width) {\n  .new { margin: 0px; }\n}\n}"
    );
}
#[test]
fn css_container_rule_exposes_condition_surface() {
    let mut vm = new_storage_test_vm("https://css-container-rule.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  head.append(style);
  const sheet = style.sheet;
  sheet.insertRule('@container name (min-width: 100px) {}', 0);
  sheet.insertRule('@container (min-width: 100px) {}', 1);
  const named = sheet.cssRules[0];
  const anonymous = sheet.cssRules[1];
  return [
    named instanceof CSSContainerRule,
    named instanceof CSSConditionRule,
    named.containerName,
    named.containerQuery,
    named.conditionText,
    anonymous.containerName,
    anonymous.containerQuery,
    anonymous.conditionText,
    CSSRule.CONTAINER_RULE
  ].join('|');
})()
"#,
        )
        .expect("CSSContainerRule surface should evaluate");

    assert_eq!(
        result,
        "true|true|name|(min-width: 100px)|name (min-width: 100px)||(min-width: 100px)|(min-width: 100px)|17"
    );
}
#[test]
fn css_named_container_rule_without_query_exposes_empty_query_string() {
    let mut vm = new_storage_test_vm("https://css-container-name-only.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@container sidebar {}');
  const rule = sheet.cssRules[0];
  return [
    rule instanceof CSSContainerRule,
    rule.containerName,
    rule.containerQuery,
    typeof rule.containerQuery,
    rule.conditionText
  ].join('|');
})()
"#,
        )
        .expect("name-only CSSContainerRule getters should evaluate");

    assert_eq!(result, "true|sidebar||string|sidebar");
}
#[test]
fn css_grouping_rule_insert_rule_rejects_invalid_or_disallowed_rules() {
    let mut vm = new_storage_test_vm("https://css-grouping-insert-rule.test/");

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
  const sheet = new CSSStyleSheet();
  sheet.insertRule('@media all { * { color: red; } }', 0);
  const grouping = sheet.cssRules[0];
  const first = grouping.cssRules[0].cssText;
  const syntax = probe(() => grouping.insertRule('???', 0));
  const importResult = probe(() => grouping.insertRule('@import url("foo.css");', 0));
  const namespaceResult = probe(() => grouping.insertRule('@namespace url("http://www.w3.org/1999/xhtml");', 0));
  const inserted = grouping.insertRule('.foo {}');

  return [
    syntax,
    importResult,
    namespaceResult,
    inserted,
    grouping.cssRules.length,
    grouping.cssRules[1].cssText === first
  ].join('|');
})()
"#,
        )
        .expect("CSSGroupingRule insertRule validation should evaluate");

    assert_eq!(
        result,
        "SyntaxError|HierarchyRequestError|HierarchyRequestError|0|2|true"
    );
}
