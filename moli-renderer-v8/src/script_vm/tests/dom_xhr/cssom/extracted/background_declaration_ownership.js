(() => {
  let checks = 0;
  const failures = [];
  const equal = (actual, expected, label) => {
    checks++;
    if (actual !== expected) failures.push({label, actual, expected});
  };
  const longhands = ['background-image', 'background-position-x', 'background-position-y',
    'background-size', 'background-repeat', 'background-attachment', 'background-origin',
    'background-clip', 'background-color'];
  const gradient = 'linear-gradient(rgb(255, 0, 0), rgb(0, 0, 255))';
  const paint = 'linear-gradient(red, blue) 10px 20px / cover no-repeat fixed padding-box content-box green';
  const values = {
    'background-image': gradient, 'background-position-x': '10px', 'background-position-y': '20px',
    'background-size': 'cover', 'background-repeat': 'no-repeat', 'background-attachment': 'fixed',
    'background-origin': 'padding-box', 'background-clip': 'content-box', 'background-color': 'rgb(0, 128, 0)'
  };
  const specifiedCases = [
    ['background: red !important; background: blue;', 'red', 'important'],
    ['background-color: blue; background: red !important; background-color: green;', 'red', 'important'],
    ['background: red; background-color: blue !important;', 'blue', 'important'],
    ['background: initial !important; background-color: blue;', 'initial', 'important'],
    ['background: var(--paint) !important; background-color: blue;', '', 'important'],
    ['background: red !important; margin: var(--gap); background-color: blue;', 'red', 'important'],
    ['background: var(--paint) !important; border: var(--edge); background-color: blue;', '', 'important']
  ];
  for (const source of ['attribute', 'inline-cssText', 'detached', 'rule-cssText', 'rule-text']) {
    for (const [text, expectedColor, priority] of specifiedCases) {
      const owner = source === 'detached' ? document.implementation.createHTMLDocument('') : document;
      const element = owner.createElement('div');
      const sheet = new CSSStyleSheet();
      sheet.replaceSync(source === 'rule-text' ? 'div {' + text + '}' : 'div {}');
      const style = source.startsWith('rule-') ? sheet.cssRules[0].style : element.style;
      if (source === 'attribute') element.setAttribute('style', text);
      else if (source !== 'rule-text') style.cssText = text;
      for (const stage of ['initial', 'unrelated']) {
        if (stage === 'unrelated') style.setProperty('--unrelated', 'kept');
        const label = source + ':' + stage + ':' + text;
        equal(style.getPropertyValue('background-color'), expectedColor, label + ': color');
        equal(style.getPropertyPriority('background-color'), priority, label + ': color importance');
        equal(style.getPropertyPriority('background-image'), text.includes('background: red;') ? '' : priority, label + ': image importance');
        const names = [...style];
        equal(new Set(names).size, names.length, label + ': unique property names');
        for (const property of longhands) equal(names.includes(property), true, label + ': enumerates ' + property);
      }
      style.setProperty('background-color', 'green');
      equal(style.backgroundColor, 'green', source + ': imperative replacement');
      equal(style.getPropertyPriority('background-color'), '', source + ': imperative priority replacement');
      const beforeInvalidWrite = style.cssText;
      style.background = 'not-a-background-value';
      equal(style.cssText, beforeInvalidWrite, source + ': invalid shorthand is atomic');
    }
  }

  for (const source of ['method', 'attribute', 'inline-cssText', 'live-rule', 'nested-rule']) {
    for (const priority of ['', 'important']) {
      const element = document.createElement('div');
      const id = 'background-native-' + source + '-' + (priority || 'normal');
      element.id = id;
      document.body.append(element);
      const styleElement = document.createElement('style');
      const selector = '#' + id;
      const text = '--paint: ' + paint + '; background: var(--paint)' + (priority ? ' !important' : '') + ';';
      styleElement.textContent = source === 'nested-rule'
        ? selector + ' { & {} ' + text + ' }'
        : selector + ' { ' + (source === 'live-rule' ? text : '') + ' }';
      document.head.append(styleElement);
      const style = source === 'nested-rule' ? styleElement.sheet.cssRules[0].cssRules[1].style
        : source === 'live-rule' ? styleElement.sheet.cssRules[0].style : element.style;
      if (source === 'attribute') element.setAttribute('style', text);
      else if (source === 'inline-cssText') style.cssText = text;
      else if (source === 'method') {
        style.setProperty('--paint', paint);
        style.setProperty('background', 'var(--paint)', priority);
      }
      const label = source + ':' + priority;
      equal(style.background, 'var(--paint)', label + ': original shorthand');
      const retained = getComputedStyle(element);
      for (const property of longhands) equal(retained.getPropertyValue(property), values[property], label + ': initial ' + property);
      style.setProperty('border', 'var(--edge)');
      equal(retained.backgroundImage, gradient, label + ': pending background survives mixed compatibility entries');
      style.backgroundColor = 'blue';
      equal(style.background, '', label + ': partially overwritten shorthand');
      for (const property of longhands) {
        equal(style.getPropertyValue(property), property === 'background-color' ? 'blue' : '', label + ': specified ' + property);
        equal(style.getPropertyPriority(property), property === 'background-color' ? '' : priority, label + ': priority ' + property);
        equal(retained.getPropertyValue(property), property === 'background-color' ? 'rgb(0, 0, 255)' : values[property], label + ': retained computed ' + property);
      }
      style.setProperty('--unrelated', 'kept');
      equal(retained.backgroundImage, gradient, label + ': image survives unrelated mutation');
      style.setProperty('--paint', 'linear-gradient(black, white) 30px 40px / contain repeat local content-box border-box red');
      equal(retained.backgroundImage, 'linear-gradient(rgb(0, 0, 0), rgb(255, 255, 255))', label + ': substituted value stays live');
      equal(retained.backgroundPosition, '30px 40px', label + ': position stays live');
      equal(retained.backgroundColor, 'rgb(0, 0, 255)', label + ': explicit longhand remains independent');
      style.removeProperty('background');
      equal(retained.backgroundImage, 'none', label + ': removed pending nodes leave no stale cascade');
      styleElement.remove();
      element.remove();
    }
  }

  const element = document.createElement('div');
  document.body.append(element);
  element.style.cssText = '--paint: ' + paint + '; background: var(--paint);';
  element.style.backgroundColor = 'blue';
  equal(getComputedStyle(element).backgroundImage, gradient, 'before style attribute replacement');
  element.setAttribute('style', 'background-color: red;');
  equal(getComputedStyle(element).backgroundImage, 'none', 'attribute replacement clears pending nodes');
  element.style.cssText = '--paint: ' + paint + '; background: var(--paint);';
  element.style.backgroundColor = 'blue';
  element.removeAttribute('style');
  equal(getComputedStyle(element).backgroundImage, 'none', 'attribute removal clears pending nodes');

  const urlBase = document.baseURI;
  element.style.setProperty('background', 'var(--missing, url(tile.png) no-repeat red)');
  element.style.backgroundColor = 'blue';
  equal(getComputedStyle(element).backgroundImage, 'url("' + new URL('tile.png', urlBase).href + '")', 'pending URL fallback uses owning document base');
  element.style.background = 'var(--missing, url(tile.png) no-repeat red)';
  element.style.backgroundImage = 'url(explicit.png)';
  equal(getComputedStyle(element).backgroundImage, 'url("' + new URL('explicit.png', urlBase).href + '")', 'explicit URL is reparsed in the owning context');
  equal(getComputedStyle(element).backgroundColor, 'rgb(255, 0, 0)', 'remaining fallback longhand still substitutes');
  element.style.cssText = '--paint: ' + paint + '; background: var(--paint);';
  element.style.backgroundColor = 'blue';
  const detachedDocument = document.implementation.createHTMLDocument('');
  detachedDocument.body.append(detachedDocument.adoptNode(element));
  document.body.append(document.adoptNode(element));
  equal(getComputedStyle(element).backgroundImage, gradient, 'pending nodes survive adoption through an inactive document');
  equal(getComputedStyle(element).backgroundColor, 'rgb(0, 0, 255)', 'adoption retains explicit override');
  element.style.cssText = 'all: initial !important; --paint: ' + paint + '; background: var(--paint);';
  element.style.backgroundColor = 'blue';
  equal(getComputedStyle(element).backgroundImage, 'none', 'important all wins over normal pending substitution');
  equal(getComputedStyle(element).backgroundColor, 'rgba(0, 0, 0, 0)', 'important all wins over normal explicit override');
  const base = document.createElement('base');
  base.href = 'https://background-ownership.invalid/a/';
  document.head.prepend(base);
  element.style.cssText = '';
  element.style.maskImage = 'var(--missing, url(mask.png))';
  const maskURL = 'url("https://background-ownership.invalid/a/mask.png")';
  equal(getComputedStyle(element).maskImage, maskURL, 'pending mask captures its original URL base');
  base.href = 'https://background-ownership.invalid/b/';
  element.style.background = 'blue';
  equal(getComputedStyle(element).maskImage, maskURL, 'background shorthand preserves the unrelated mask URL base');
  equal(getComputedStyle(element).backgroundColor, 'rgb(0, 0, 255)', 'background color still changes after a base change');
  base.remove();
  element.remove();
  return {checks, failures};
})()
