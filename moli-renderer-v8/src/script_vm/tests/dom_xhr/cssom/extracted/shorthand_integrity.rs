use super::*;

#[test]
fn cssom_shorthand_reads_require_every_longhand_and_matching_importance() {
    let mut vm = new_storage_html_test_vm("https://shorthand-integrity.test/");
    let result = vm
        .eval(
            r#"
(() => {
  let checks = 0, mutations = 0;
  const failures = [], coverage = [];
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
    ['font-variant', 'small-caps'], ['overflow', 'hidden scroll'],
    ['text-decoration', 'underline red']
  ];
  for (const [property, value] of cases) {
    const reference = new CSSStyleSheet(); reference.replaceSync('div {}');
    const ref = reference.cssRules[0].style;
    ref.setProperty(property, 'initial', 'important');
    const longhands = Array.from({length: ref.length}, (_, i) => ref.item(i));
    coverage.push({property, longhands});
    if (longhands.length < 2 || property === 'all') continue;
    for (const source of ['method', 'cssText', 'attribute', 'detached-document', 'rule']) {
      const element = (source === 'detached-document' ? document.implementation.createHTMLDocument('') : document).createElement('div');
      const sheet = new CSSStyleSheet(); sheet.replaceSync('div {}');
      const style = source === 'rule' ? sheet.cssRules[0].style : element.style;
      const reset = (value, priority) => {
        style.cssText = '';
        if (source === 'attribute') element.setAttribute('style', property + ':' + value + (priority ? ' !important' : '') + ';');
        else if (source === 'cssText') style.cssText = property + ':' + value + (priority ? ' !important' : '') + ';';
        else style.setProperty(property, value, priority);
        style.setProperty('--unrelated', 'kept');
      };
      for (const longhand of longhands) {
        const prefix = property + ':' + source + ':' + longhand;
        reset('initial', 'important');
        equal(style.getPropertyValue(property), 'initial', prefix + ': initial baseline');
        style.removeProperty(longhand); mutations++;
        equal(style.getPropertyValue(property), '', prefix + ': missing value');
        equal(style.getPropertyPriority(property), '', prefix + ': missing priority');
        style.setProperty(longhand, 'initial', 'important');
        equal(style.getPropertyValue(property), 'initial', prefix + ': restored value');
        equal(style.getPropertyPriority(property), 'important', prefix + ': restored priority');
        reset('initial', 'important');
        style.setProperty(longhand, 'inherit', 'important'); mutations++;
        equal(style.getPropertyValue(property), '', prefix + ': mixed CSS-wide value');
        equal(style.getPropertyPriority(property), 'important', prefix + ': independent priority');
        reset('initial', 'important');
        style.setProperty(longhand, 'initial'); mutations++;
        equal(style.getPropertyValue(property), '', prefix + ': mixed priority value');
        equal(style.getPropertyPriority(property), '', prefix + ': mixed priority');
        reset(value, 'important');
        style.removeProperty(longhand); mutations++;
        equal(style.getPropertyValue(property), '', prefix + ': ordinary missing value');
        equal(style.getPropertyPriority(property), '', prefix + ': ordinary missing priority');
        equal(style.getPropertyValue('--unrelated'), 'kept', prefix + ': unrelated preserved');
      }
    }
  }
  return JSON.stringify({checks, mutations, coverage, failures});
})()
"#,
        )
        .expect("shorthand integrity checks should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(result["checks"].as_u64().unwrap() > 9000, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
