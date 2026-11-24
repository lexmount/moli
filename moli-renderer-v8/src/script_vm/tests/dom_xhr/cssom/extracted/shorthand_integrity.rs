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
    ['border', '1px solid blue'],
    ['font', 'italic 12px serif'],
    ['transition', 'opacity 2s ease-in 1s'],
    ['font-variant', 'small-caps'],
    ['overflow', 'hidden scroll'],
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
    assert!(result["checks"].as_u64().unwrap() > 3000, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
