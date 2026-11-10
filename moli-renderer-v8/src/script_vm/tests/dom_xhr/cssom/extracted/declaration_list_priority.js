(() => {
  let checks = 0;
  const failures = [];
  const eq = (actual, expected, label) => {
    checks++;
    if (actual !== expected) failures.push({label, actual, expected});
  };
  const cases = [
    ['color: red !important; color: blue;', 'color', 'red', 'important'],
    ['color: red; color: blue !important; color: green;', 'color', 'blue', 'important'],
    ['color: red !important; color: blue !important;', 'color', 'blue', 'important'],
    ['color: red; color: blue;', 'color', 'blue', ''],
    ['color: red !important; color: invalid-color;', 'color', 'red', 'important'],
    ['outline: red solid 2px !important; outline-color: blue;', 'outline-color', 'red', 'important'],
    ['outline-color: red !important; outline: blue solid 2px;', 'outline-color', 'red', 'important'],
    ['--tone: red !important; --tone: blue;', '--tone', 'red', 'important'],
    ['--tone: red; color: var(--tone) !important; color: blue;', 'color', 'var(--tone)', 'important'],
    ['-webkit-user-select: text !important; user-select: none;', 'user-select', 'text', 'important'],
  ];
  for (const source of ['attribute', 'inline-cssText', 'detached-cssText', 'rule-cssText', 'rule-text']) {
    for (const [text, property, expected, priority] of cases) {
      const doc = source === 'detached-cssText' ? document.implementation.createHTMLDocument('') : document;
      const element = doc.createElement('div');
      const sheet = new CSSStyleSheet();
      sheet.replaceSync(source === 'rule-text' ? 'div {' + text + '}' : 'div {}');
      const style = source.startsWith('rule-') ? sheet.cssRules[0].style : element.style;
      if (source === 'attribute') element.setAttribute('style', text);
      else if (source !== 'rule-text') style.cssText = text;
      const label = source + ':' + text;
      eq(style.getPropertyValue(property), expected, label + ': parsed winner');
      eq(style.getPropertyPriority(property), priority, label + ': parsed priority');
      style.setProperty('--unrelated', 'kept');
      eq(style.getPropertyValue(property), expected, label + ': winner after unrelated mutation');
      eq(style.getPropertyPriority(property), priority, label + ': priority after unrelated mutation');
      if (property === 'outline-color') {
        eq(style.getPropertyValue('outline-style'), 'solid', label + ': shorthand expansion');
        eq(style.getPropertyValue('outline-width'), '2px', label + ': shorthand width');
      }
      style.setProperty(property, property === '-webkit-mask-composite' ? 'xor' : property === 'user-select' ? 'none' : 'green');
      eq(style.getPropertyValue(property), property === '-webkit-mask-composite' ? 'xor' : property === 'user-select' ? 'none' : 'green', label + ': imperative replacement');
      eq(style.getPropertyPriority(property), '', label + ': imperative priority replacement');
    }
  }
  const element = document.createElement('div');
  element.id = 'declaration-list-priority';
  document.body.appendChild(element);
  const styleElement = document.createElement('style');
  styleElement.textContent = '#declaration-list-priority { color: red !important; color: blue; outline: red solid 2px !important; outline-color: blue; }';
  document.head.appendChild(styleElement);
  const rule = styleElement.sheet.cssRules[0];
  // Reading style and then mutating an unrelated declaration must preserve the
  // parsed winner in the live stylesheet used by the actual cascade.
  eq(rule.style.getPropertyValue('color'), 'red', 'live rule: parsed winner');
  rule.style.setProperty('--unrelated', 'kept');
  eq(getComputedStyle(element).color, 'rgb(255, 0, 0)', 'live rule: computed color');
  eq(getComputedStyle(element).outlineColor, 'rgb(255, 0, 0)', 'live rule: computed outline color');
  rule.style.cssText = 'color: green !important; color: blue;';
  eq(getComputedStyle(element).color, 'rgb(0, 128, 0)', 'live rule cssText: computed winner');
  rule.style.setProperty('color', 'blue');
  eq(getComputedStyle(element).color, 'rgb(0, 0, 255)', 'live rule setter: computed replacement');
  styleElement.remove();
  element.remove();
  return {checks, failures};
})()
