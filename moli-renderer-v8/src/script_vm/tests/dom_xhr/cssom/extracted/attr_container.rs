use super::*;

#[test]
fn container_attr_queries_refresh_after_referenced_attribute_mutations() {
    let mut vm = new_storage_html_test_vm("https://container-attr-invalidation.test/page.html");
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
  const sheet = document.createElement('style');
  const rules = [
    ['number', 'style(1px < attr(data-num px, 0px) < 10px)'],
    ['length', 'style(1px < attr(data-length type(<length>), 0px) < 10px)'],
    ['word', 'style(--word: attr(data-word type(<custom-ident>)))'],
    ['variable-fallback', 'style(1px < var(--missing, attr(data-length type(<length>), 0px)) < 10px)'],
    ['environment-fallback', 'style(1px < env(moli-container-missing, attr(data-length type(<length>), 0px)) < 10px)'],
    ['attribute-fallback', 'style(1px < attr((), attr(data-length type(<length>), 0px)) < 10px)'],
    ['dynamic-name', 'style(1px < attr(var(--args), 0px) < 10px)'],
    ['attribute-name', 'style(1px < attr(attr(data-recipe type(*)), 0px) < 10px)'],
    ['compound', 'style(--word: attr(data-word type(<custom-ident>))) and style(--true)'],
    ['disjunction', 'style(--word: attr(data-word type(<custom-ident>))) or style(--word: attr(data-other-word type(<custom-ident>)))'],
    ['negated', 'not style(--word: attr(data-word type(<custom-ident>)))'],
    ['named', 'chosen style(1px < attr(data-length type(<length>), 0px) < 10px)'],
    ['uppercase', 'style(1px < attr(DATA-LENGTH type(<length>), 0px) < 10px)'],
  ];
  sheet.textContent = rules.map(([name, condition]) =>
    '@container ' + condition + ' { .' + name + ' { --hit: true; } }').join('\n') +
    '.generated::before { content: "x"; } @container chosen style(1px < attr(data-length type(<length>), 0px) < 10px) { .generated::before { --hit: true; } }';
  document.head.appendChild(sheet);
  equal(sheet.sheet.cssRules.length, rules.length + 2, 'all query rules parse');
  const make = (name, root = document.body) => {
    const container = document.createElement('main'), child = document.createElement('div');
    container.style.setProperty('--word', 'bar');
    container.style.setProperty('--true', 'true');
    container.style.setProperty('--args', 'data-length type(<length>)');
    container.style.containerName = 'chosen';
    container.setAttribute('data-recipe', 'data-length type(<length>)');
    container.setAttribute('data-other-length', '20px');
    child.className = name; container.appendChild(child); root.appendChild(container);
    return [container, child];
  };
  const value = (element, pseudo) => getComputedStyle(element, pseudo).getPropertyValue('--hit').trim();
  for (const [name] of rules) {
    const [container, child] = make(name);
    const word = ['word', 'compound', 'disjunction', 'negated'].includes(name);
    const attribute = name === 'number' ? 'data-num' : name === 'disjunction' ? 'data-other-word' : word ? 'data-word' : 'data-length';
    const first = name === 'negated' ? 'baz' : word ? 'bar' : name === 'number' ? '5' : '5px';
    const second = name === 'negated' ? 'bar' : word ? 'baz' : name === 'number' ? '20' : '20px';
    if (name === 'disjunction') container.setAttribute('data-word', 'baz');
    container.setAttribute(attribute, first);
    equal(value(child), 'true', name + ': initial');
    container.setAttribute(attribute, second);
    equal(value(child), '', name + ': changed attribute');
    container.setAttribute(attribute, first);
    equal(value(child), 'true', name + ': restored attribute');
    container.removeAttribute(attribute);
    equal(value(child), name === 'negated' ? 'true' : '', name + ': removed attribute');
    container.remove();
  }
  for (const name of ['length', 'dynamic-name', 'attribute-name']) {
    const [container, child] = make(name);
    equal(value(child), '', name + ': initially missing');
    container.setAttribute('data-length', '5px');
    equal(value(child), 'true', name + ': missing attribute added');
    container.remove();
  }
  const [dynamic, dynamicChild] = make('dynamic-name');
  dynamic.setAttribute('data-length', '5px');
  equal(value(dynamicChild), 'true', 'dynamic recipe initial');
  dynamic.style.setProperty('--args', 'data-other-length type(<length>)');
  equal(value(dynamicChild), '', 'custom property changes attribute name');
  dynamic.style.setProperty('--args', 'data-length type(<length>)');
  equal(value(dynamicChild), 'true', 'custom property restores attribute name');
  dynamic.remove();

  const [generated, generatedChild] = make('generated');
  generated.setAttribute('data-length', '5px');
  equal(value(generatedChild, '::before'), 'true', 'pseudo query initial');
  generated.setAttribute('data-length', '20px');
  equal(value(generatedChild, '::before'), '', 'pseudo query mutation');
  generated.remove();

  for (const adopted of [false, true]) {
    const host = document.createElement('section'); document.body.appendChild(host);
    const shadow = host.attachShadow({mode: 'open'});
    const css = '@container style(1px < attr(var(--args), 0px) < 10px) { .shadow-target { --hit: true; } }';
    if (adopted) {
      const adoptedSheet = new CSSStyleSheet(); adoptedSheet.replaceSync(css); shadow.adoptedStyleSheets = [adoptedSheet];
    } else {
      const shadowSheet = document.createElement('style'); shadowSheet.textContent = css; shadow.appendChild(shadowSheet);
    }
    const [container, child] = make('shadow-target', shadow);
    container.setAttribute('data-length', '5px');
    equal(value(child), 'true', 'shadow initial: ' + adopted);
    container.setAttribute('data-length', '20px');
    equal(value(child), '', 'shadow mutation: ' + adopted);
    container.setAttribute('data-length', '5px');
    equal(value(child), 'true', 'shadow restoration: ' + adopted);
    host.remove();
  }
  sheet.remove();
  const ordinary = document.createElement('div'); document.body.appendChild(ordinary);
  ordinary.setAttribute('data-length', '5px');
  ordinary.style.marginLeft = 'attr(DATA-LENGTH type(<length>), 0px)';
  const property = (element, name) => getComputedStyle(element).getPropertyValue(name);
  equal(property(ordinary, 'margin-left'), '5px', 'HTML attribute names fold ASCII case');
  ordinary.setAttribute('data-length', '20px');
  equal(property(ordinary, 'margin-left'), '20px', 'HTML folded attribute dependency invalidates');
  ordinary.setAttributeNS('urn:container-attr', 'ns:data-length', '30px');
  equal(property(ordinary, 'margin-left'), '20px', 'null namespace keeps the unnamespaced value');
  ordinary.removeAttribute('data-length');
  equal(property(ordinary, 'margin-left'), '0px', 'null namespace does not select a namespaced attribute');
  ordinary.style.marginLeft = 'attr(data-length type(<length>), 0px)';
  equal(property(ordinary, 'margin-left'), '0px', 'lowercase null namespace also stays exact');
  const namespaced = document.createElement('style');
  namespaced.textContent = '@namespace ns url(urn:container-attr); #namespace-target { margin-left: attr(ns|data-length type(<length>), 0px); }';
  document.head.appendChild(namespaced); ordinary.style.marginLeft = ''; ordinary.id = 'namespace-target';
  equal(property(ordinary, 'margin-left'), '30px', 'explicit namespace value');
  ordinary.setAttributeNS('urn:container-attr', 'ns:data-length', '40px');
  equal(property(ordinary, 'margin-left'), '40px', 'qualified attribute dependency invalidates');
  ordinary.removeAttributeNS('urn:container-attr', 'data-length');
  equal(property(ordinary, 'margin-left'), '0px', 'explicit namespace removal');
  ordinary.remove(); namespaced.remove();
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); document.body.appendChild(svg);
  svg.setAttribute('DATA-NUM', '0.5'); svg.style.opacity = 'attr(data-num number, 0)';
  equal(property(svg, 'opacity'), '0', 'SVG attribute names remain case sensitive');
  svg.setAttribute('data-num', '0.7');
  equal(property(svg, 'opacity'), '0.7', 'SVG lowercase attribute added');
  svg.setAttribute('DATA-NUM', '0.9');
  equal(property(svg, 'opacity'), '0.7', 'SVG uppercase attribute stays distinct');
  svg.style.opacity = 'attr(DATA-NUM number, 0)';
  equal(property(svg, 'opacity'), '0.9', 'SVG uppercase name selected explicitly');
  svg.setAttribute('DATA-NUM', '0.4');
  equal(property(svg, 'opacity'), '0.4', 'SVG exact dependency invalidates');
  return JSON.stringify({checks, failures});
})()
"#,
        )
        .expect("container attr invalidation probe should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 83, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
