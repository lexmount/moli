(() => {
  const inputs = [
    ['', ''],
    [' \t\r\n\f ', ''],
    ['/**/', ''],
    [' /*comment*/ /**/ ', ''],
    ['named:left', 'named:left'],
    [':RIGHT', ':right'],
    ['named/**/:first', 'named:first'],
    ['named :first', 'before:first'],
    [':first :left', 'before:first'],
    [':notapagepseudo', 'before:first'],
    [',', 'before:first'],
    ['\v', 'before:first'],
    ['name\v', 'before:first'],
    ['name\u00a0', 'name\u00a0'],
    ['\u00a0', '\u00a0'],
    ['\u2003:left', '\u2003:left'],
    ['\\31 page:first', '\\31 page:first'],
    ['name\\ ', 'name\\ '],
    ['\\ name', '\\ name'],
  ];
  let cases = 0;
  const failures = [];
  for (const owner of ['style-element', 'constructed', 'detached']) {
    for (const materialize of [false, true]) {
      for (const [input, expected] of inputs) {
        cases++;
        const check = (condition, message) => {
          if (!condition) failures.push({owner, materialize, input, message});
        };
        const css = '@page before:first { margin-top: 7px; @top-left { content: "x"; } } .after { color: red; }';
        let sheet, element;
        if (owner === 'style-element') {
          element = document.createElement('style');
          element.textContent = css;
          document.head.append(element);
          sheet = element.sheet;
        } else {
          sheet = new CSSStyleSheet();
          sheet.replaceSync(css);
        }
        const page = sheet.cssRules[0];
        const style = page.style;
        const rules = materialize ? page.cssRules : null;
        const margin = rules ? rules[0] : null;
        const marginStyle = margin ? margin.style : null;
        if (owner === 'detached') sheet.deleteRule(0);
        page.selectorText = input;
        check(page.selectorText === expected, 'selector');
        check(page.style === style && style.marginTop === '7px', 'page declaration owner');
        check(page.parentStyleSheet === (owner === 'detached' ? null : sheet), 'sheet owner');
        check(page.cssRules.length === 1 && page.cssRules[0].style.content === '"x"', 'margin declaration');
        check(!rules || (page.cssRules === rules && rules[0] === margin && margin.style === marginStyle), 'margin identities');
        check(sheet.cssRules[owner === 'detached' ? 0 : 1].selectorText === '.after', 'sibling rule');
        if (owner !== 'detached') {
          sheet.insertRule('.transient {}', sheet.cssRules.length);
          sheet.deleteRule(sheet.cssRules.length - 1);
          check(sheet.cssRules[0] === page && page.selectorText === expected, 'native selector after revision');
        }
        if (element) element.remove();
      }
    }
  }
  return JSON.stringify({cases, failures});
})()
