use super::*;

#[test]
fn cssom_native_shorthand_reads_share_declaration_semantics_across_sources() {
    let mut vm = new_storage_html_test_vm("https://native-shorthand-query.test/");
    let result = vm
        .eval(
            r#"
(() => {
  let checks = 0;
  const failures = [];
  const equal = (actual, expected, label) => {
    checks++;
    if (actual !== expected) failures.push({label, actual, expected});
  };
  const cases = [
    ['container', 'card / inline-size'], ['columns', '10px 3'],
    ['column-rule', '2px solid red'], ['font-synthesis', 'none'],
    ['text-box', 'trim-both cap alphabetic'], ['text-wrap', 'wrap balance'],
    ['place-items', 'center start'], ['place-self', 'center end'],
    ['inset', '1px 2px 3px 4px'], ['inset-block', '1px 2px'], ['inset-inline', '3px 4px'],
    ['grid-row', '2 / 4'], ['grid-area', '1 / 2 / 3 / 4'],
    ['grid-template', '10px / 20px'], ['grid', '10px / 20px'],
    ['scroll-margin', '1px 2px 3px 4px'], ['scroll-margin-block', '1px 2px'],
    ['scroll-margin-inline', '3px 4px'], ['scroll-padding', '1px 2px 3px 4px'],
    ['scroll-padding-block', '1px 2px'], ['scroll-padding-inline', '3px 4px'],
    ['contain-intrinsic-size', '10px 20px'],
    ['border-block', '2px solid red'], ['border-inline', '3px dotted blue'],
    ['border-block-color', 'red blue'], ['border-inline-color', 'blue green'],
    ['border-block-style', 'solid dashed'], ['border-inline-style', 'dotted double'],
    ['border-block-width', '2px 3px'], ['border-inline-width', '4px 5px'],
    ['border-start-start-radius', '1px 2px'],
    ['padding-block', '1px 2px'], ['padding-inline', '3px 4px'],
    ['animation-range', 'entry 10% exit 90%'],
    ['scroll-timeline', '--progress x'], ['view-timeline', '--progress y'],
    ['background', 'red'], ['border', '1px solid blue'], ['font', 'italic 12px serif'],
    ['animation', '2s linear 1s 3 alternate both paused spin'],
    ['transition', 'opacity 2s ease-in 1s'], ['all', 'initial'],
  ];
  for (const [property, value] of cases) {
    const sheet = new CSSStyleSheet(); sheet.replaceSync('div {}');
    const reference = sheet.cssRules[0].style;
    reference.setProperty(property, value, 'important');
    const expected = reference.getPropertyValue(property);
    equal(CSS.supports(property, value), true, property + ': supported fixture');
    equal(expected.length > 0, true, property + ': rule reference is nonempty');
    const longhands = Array.from({length: reference.length}, (_, index) => {
      const name = reference.item(index);
      return [name, reference.getPropertyValue(name)];
    });
    for (const source of ['method', 'idl', 'cssText', 'attribute']) {
      const element = document.createElement('div');
      document.body.appendChild(element);
      const style = element.style;
      const priority = source === 'idl' ? '' : 'important';
      if (source === 'method') style.setProperty(property, value, priority);
      if (source === 'idl') style[property] = value;
      if (source === 'cssText') style.cssText = property + ':' + value + ' !important;';
      if (source === 'attribute') element.setAttribute('style', property + ':' + value + ' !important;');
      const label = property + ':' + source;
      equal(style.getPropertyValue(property), expected, label + ': method read');
      equal(style[property], expected, label + ': IDL read');
      equal(style.getPropertyPriority(property), priority, label + ': priority');
      equal(JSON.stringify(longhands.map(([name]) => [name, style.getPropertyValue(name), style.getPropertyPriority(name)])),
        JSON.stringify(longhands.map(([name, value]) => [name, value, priority])), label + ': longhand projection');
      equal(style.removeProperty(property), expected, label + ': remove returns old value');
      equal(style.getPropertyValue(property), '', label + ': removed value');
      equal(style.getPropertyPriority(property), '', label + ': removed priority');
      style.cssText = property + ':' + value + ' !important; --unrelated:kept;';
      equal(style.getPropertyValue(property), expected, label + ': unrelated declarations');
      equal(style.getPropertyValue('--unrelated'), 'kept', label + ': unrelated custom property');
      equal(element.cloneNode().style.getPropertyValue(property), expected, label + ': cloned attribute');
      element.remove();
    }
    const style = document.createElement('div').style;
    for (const [name, value] of longhands) style.setProperty(name, value, 'important');
    equal(style.getPropertyValue(property), expected, property + ': assembled longhands');
    equal(style.getPropertyPriority(property), 'important', property + ': assembled priority');
    style.setProperty(longhands[0][0], 'initial');
    equal(style.getPropertyValue(property), longhands.length > 1 ? '' : 'initial', property + ': mixed values and priorities');
    equal(style.getPropertyPriority(property), '', property + ': mixed priority');
    style.removeProperty(longhands[0][0]);
    equal(style.getPropertyValue(property), '', property + ': missing longhand');
    equal(style.getPropertyPriority(property), '', property + ': missing longhand priority');
    for (const keyword of ['initial', 'inherit', 'unset', 'revert', 'revert-layer']) {
      style.cssText = '';
      style.setProperty(property, keyword);
      equal(style.getPropertyValue(property), keyword, property + ': CSS-wide ' + keyword);
      equal(style[property], keyword, property + ': CSS-wide IDL ' + keyword);
      equal(style.getPropertyPriority(property), '', property + ': CSS-wide priority ' + keyword);
      equal(style.removeProperty(property), keyword, property + ': CSS-wide removal ' + keyword);
    }
    style.setProperty(property, value, 'important');
    style.setProperty(property, ')');
    equal(style.getPropertyValue(property), expected, property + ': invalid write preserves value');
    equal(style.getPropertyPriority(property), 'important', property + ': invalid write preserves priority');
    style.cssText = '';
    style.setProperty(property, 'var(--pending)');
    equal(style.getPropertyValue(property), 'var(--pending)', property + ': pending substitution');
    equal(style.getPropertyPriority(property), '', property + ': pending substitution priority');
    style.setProperty(longhands[0][0], 'initial');
    equal(style.getPropertyValue(property), longhands.length > 1 ? '' : 'initial', property + ': pending substitution overwritten longhand');
    sheet.deleteRule(0);
    equal(reference.getPropertyValue(property), expected, property + ': detached rule reference');
    equal(reference.getPropertyPriority(property), 'important', property + ': detached rule priority');
  }
  return JSON.stringify({checks, failures});
})()
"#,
        )
        .expect("native shorthand CSSOM read probe should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 3150, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
