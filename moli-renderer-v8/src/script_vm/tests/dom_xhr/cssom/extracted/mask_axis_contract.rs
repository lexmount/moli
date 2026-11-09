use super::*;

#[test]
fn cssom_mask_position_aliases_share_native_declarations_across_sources() {
    let mut vm = new_storage_html_test_vm("https://mask-position-contract.test/");
    let result = vm.eval(r#"
(() => {
  let checks = 0; const failures = [];
  const equal = (actual, expected, label) => { checks++; if (actual !== expected) failures.push({label, actual, expected}); };
  const x = '-webkit-mask-position-x', y = '-webkit-mask-position-y';
  const sources = ['method', 'cssText', 'attribute', 'detached-document', 'rule'];
  const make = source => {
    const doc = source === 'detached-document' ? document.implementation.createHTMLDocument('') : document;
    const element = doc.createElement('div');
    const sheet = new CSSStyleSheet(); sheet.replaceSync('div {}');
    const style = source === 'rule' ? sheet.cssRules[0].style : element.style;
    return {style, seed(property, value, priority) {
      const text = property + ':' + value + (priority ? ' !important' : '') + ';';
      if (source === 'attribute') element.setAttribute('style', text);
      else if (source === 'cssText' || source === 'rule') style.cssText = text;
      else style.setProperty(property, value, priority);
    }};
  };
  for (const source of sources) {
    for (const property of ['mask', 'mask-position', '-webkit-mask-position']) {
      for (const priority of ['', 'important']) {
        const base = make(source); base.seed(property, 'initial', priority);
        const style = base.style, actualNames = [...style], names = property === 'mask' ? ['mask-image', x, y, 'mask-size', 'mask-repeat', 'mask-origin', 'mask-clip', 'mask-composite', 'mask-mode'] : [x, y], label = [source, property, priority].join(':');
        equal(actualNames.length, property === 'mask' ? 9 : 2, label + ': longhand count');
        equal(actualNames.includes(x), true, label + ': public horizontal name');
        equal(actualNames.includes(y), true, label + ': public vertical name');
        equal(actualNames.includes('mask-position-x'), false, label + ': hidden horizontal name');
        equal(actualNames.includes('mask-position-y'), false, label + ': hidden vertical name');
        equal(style.getPropertyValue(property), 'initial', label + ': initial shorthand');
        equal(style.getPropertyValue('-webkit-mask-position'), style.getPropertyValue('mask-position'), label + ': shorthand alias value');
        equal(style.getPropertyPriority('-webkit-mask-position'), priority, label + ': shorthand alias priority');
        const restored = make(source).style;
        for (const name of actualNames) restored.setProperty(name, 'initial', priority);
        for (const name of names) {
          equal(CSS.supports(name, 'initial'), true, label + ': writable ' + name);
          equal(style.getPropertyValue(name), 'initial', label + ': initial ' + name);
          equal(style.getPropertyPriority(name), priority, label + ': priority ' + name);
          const changed = make(source).style; changed.cssText = style.cssText;
          changed.setProperty(name, 'inherit', priority);
          equal(changed.getPropertyValue(property), '', label + ': mixed keyword ' + name);
          changed.cssText = style.cssText;
          changed.setProperty(name, 'initial', priority ? '' : 'important');
          equal(changed.getPropertyValue(property), '', label + ': mixed priority ' + name);
          changed.cssText = style.cssText;
          equal(changed.removeProperty(name), 'initial', label + ': removed value ' + name);
          equal(changed.getPropertyValue(property), '', label + ': missing longhand ' + name);
        }
        equal(restored.getPropertyValue(property), 'initial', label + ': restore from public names');
        equal(restored.getPropertyPriority(property), priority, label + ': restore priority');
      }
    }
    for (const property of ['mask-position', '-webkit-mask-position']) {
      const item = make(source); item.seed(property, '10px 20px, right bottom', 'important');
      const style = item.style, label = source + ':' + property + ':layers';
      equal(style.getPropertyValue('mask-position'), '10px 20px, right bottom', label + ': standard value');
      equal(style.getPropertyValue('-webkit-mask-position'), '10px 20px, right bottom', label + ': alias value');
      equal(style.getPropertyValue(x), '10px, right', label + ': horizontal list');
      equal(style.getPropertyValue(y), '20px, bottom', label + ': vertical list');
      equal(style.getPropertyPriority(x), 'important', label + ': horizontal priority');
      equal(style.getPropertyPriority(y), 'important', label + ': vertical priority');
    }
    const partial = make(source).style, literal = '"mask-position-x: keep; mask-position-y: keep"';
    partial.setProperty(x, '10px', 'important'); partial.setProperty('--text', literal);
    equal(partial.item(0), x, source + ': partial item');
    equal(partial.cssText.startsWith(x + ': 10px !important;'), true, source + ': partial cssText name');
    equal(partial.getPropertyValue('--text'), literal, source + ': custom tokens retained');
    const copy = make(source).style; copy.cssText = partial.cssText;
    equal(copy.getPropertyValue(x), '10px', source + ': partial roundtrip value');
    equal(copy.getPropertyPriority(x), 'important', source + ': partial roundtrip priority');
    equal(copy.getPropertyValue('--text'), literal, source + ': custom tokens roundtrip');
    copy.setProperty(y, '20px', 'important');
    equal(copy.getPropertyValue('mask-position'), '10px 20px', source + ': completed position');
    copy.setProperty('-webkit-mask-position', '30px 40px');
    equal(copy.getPropertyValue('mask-position'), '30px 40px', source + ': alias replaces axes');
    equal(copy.getPropertyPriority('mask-position'), '', source + ': alias replaces priority');
    equal(copy.removeProperty('-webkit-mask-position'), '30px 40px', source + ': alias removes both axes');
    equal(copy.length, 1, source + ': only custom property remains');
    for (const property of ['mask', 'mask-position', '-webkit-mask-position']) {
      for (const value of ['var(--pending)', 'env(test)']) {
        for (const priority of ['', 'important']) {
          const item = make(source); item.seed(property, value, priority);
          const style = item.style, label = [source, property, value, priority].join(':');
          equal(style.getPropertyValue(property), value, label + ': pending shorthand');
          equal(style.getPropertyValue(x), '', label + ': pending horizontal value');
          equal(style.getPropertyValue(y), '', label + ': pending vertical value');
          equal(style.getPropertyPriority(x), priority, label + ': pending horizontal priority');
          equal(style.getPropertyPriority(y), priority, label + ': pending vertical priority');
          style.setProperty('--side', 'kept');
          equal(style.getPropertyValue(property), value, label + ': unrelated mutation');
          const copy = make(source).style; copy.cssText = style.cssText;
          equal(copy.getPropertyValue(property), value, label + ': pending roundtrip');
          equal(copy.getPropertyPriority(property), priority, label + ': pending roundtrip priority');
          copy.setProperty(x, '12px', priority);
          equal(copy.getPropertyValue(property), '', label + ': override horizontal pending node');
          equal(copy.getPropertyValue(x), '12px', label + ': replaced horizontal value');
        }
      }
    }
    const style = make(source).style;
    for (const name of ['mask-position-x', 'mask-position-y']) {
      equal(CSS.supports(name, 'initial'), false, source + ': unsupported internal name ' + name);
      equal(name in style, false, source + ': no internal accessor ' + name);
      style.setProperty(name, '12px');
      equal(style.getPropertyValue(name), '', source + ': no internal value ' + name);
      equal(style.length, 0, source + ': no internal declaration ' + name);
    }
  }
  for (const keyframes of [false, true]) {
    const body = '-webkit-mask-position-x: 10px' + (keyframes ? '' : ' !important') + '; --text: "mask-position-y: keep";';
    const sheet = new CSSStyleSheet(); sheet.replaceSync(keyframes ? '@keyframes spin { from {' + body + '}}' : 'div {' + body + '}');
    const rule = keyframes ? sheet.cssRules[0].cssRules[0] : sheet.cssRules[0];
    const text = rule.cssText;
    equal(text.includes('-webkit-mask-position-x: 10px'), true, 'unmaterialized rule public name:' + keyframes);
    equal(rule.style.item(0), x, 'rule public first item:' + keyframes);
    equal(rule.style.getPropertyValue(x), '10px', 'rule axis value:' + keyframes);
    equal(rule.style.getPropertyValue('--text'), '"mask-position-y: keep"', 'rule custom tokens:' + keyframes);
  }
  {
    const sheet = new CSSStyleSheet(); sheet.replaceSync('.a { & .b { } }');
    const parent = sheet.cssRules[0];
    const index = parent.insertRule('-webkit-mask-position-x: 10px !important; --text: "mask-position-y: keep";', 1);
    const rule = parent.cssRules[index];
    const text = rule.cssText;
    equal(text.includes('-webkit-mask-position-x: 10px'), true, 'nested declaration public name');
    equal(rule.style.item(0), x, 'nested declaration public item');
    equal(rule.style.getPropertyValue(x), '10px', 'nested declaration axis value');
    equal(rule.style.getPropertyValue('--text'), '"mask-position-y: keep"', 'nested declaration custom tokens');
  }
  return JSON.stringify({checks, failures});
})()
"#).expect("mask position contract should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 1977, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
