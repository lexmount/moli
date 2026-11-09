use super::*;

#[test]
fn typed_attr_fallbacks_resolve_nested_references_and_preserve_cycles() {
    let mut vm = new_storage_html_test_vm("https://typed-attr-fallback.test/page.html");
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
  const attributes = {
    'data-number': '23', 'data-other': '31', 'data-present': '29', 'data-invalid': 'bad',
    'data-weight': '43', 'data-opacity': '0.5', 'data-length': '17px', 'data-color': 'blue',
    'data-gradient': 'linear-gradient(black, white)',
    'data-cycle': 'attr(data-cycle type(*))', 'data-dead-var': 'var(--dead)',
    'data-image': 'url(https://attr-fallback-resource.test/private.png)',
  };
  const make = () => {
    const element = document.createElement('div');
    for (const [name, value] of Object.entries(attributes)) element.setAttribute(name, value);
    element.style.setProperty('--present', '29');
    element.style.setProperty('--dead', 'var(--dead)');
    document.body.appendChild(element);
    return element;
  };
  const value = (element, property, pseudo) =>
    getComputedStyle(element, pseudo).getPropertyValue(property);
  const cases = [
    ['missing typed primary', 'z-index', 'attr(data-missing type(<integer>), attr(data-number type(<integer>), 5))', '23'],
    ['invalid typed primary', 'z-index', 'attr(data-invalid type(<integer>), attr(data-number type(<integer>), 5))', '23'],
    ['present typed primary', 'z-index', 'attr(data-present type(<integer>), attr(data-number type(<integer>), 5))', '29'],
    ['number primary', 'z-index', 'attr(data-missing number, attr(data-number type(<integer>), 5))', '23'],
    ['raw string primary', 'z-index', 'attr(data-missing raw-string, attr(data-number type(<integer>), 5))', '23'],
    ['var fallback', 'z-index', 'var(--missing, attr(data-number type(<integer>), 5))', '23'],
    ['env fallback', 'z-index', 'env(moli-unknown-environment, attr(data-number type(<integer>), 5))', '23'],
    ['var and env fallbacks', 'z-index', 'var(--missing, env(moli-unknown-environment, attr(data-number type(<integer>), 5)))', '23'],
    ['three attribute levels', 'z-index', 'attr(data-missing type(<integer>), attr(data-absent type(<integer>), attr(data-number type(<integer>), 5)))', '23'],
    ['calc fallback', 'z-index', 'calc(attr(data-missing type(<integer>), attr(data-number type(<integer>), 5)) + 2)', '25'],
    ['multiple fallback references', 'z-index', 'calc(var(--missing, attr(data-number type(<integer>))) + env(moli-unknown-environment, attr(data-other type(<integer>))))', '54'],
    ['number type', 'font-weight', 'attr(data-missing type(<number>), attr(data-weight type(<number>), 5))', '43'],
    ['opacity type', 'opacity', 'var(--missing, attr(data-opacity type(<number>), 0))', '0.5'],
    ['length type', 'margin-left', 'attr(data-missing type(<length>), attr(data-length type(<length>), 5px))', '17px'],
    ['shorthand fallback', 'margin', 'var(--missing, attr(data-length type(<length>), 5px))', '17px'],
    ['color type', 'color', 'var(--missing, attr(data-color type(<color>), red))', 'rgb(0, 0, 255)'],
    ['image type', 'background-image', 'var(--missing, attr(data-gradient type(<image>), none))', 'linear-gradient(rgb(0, 0, 0), rgb(255, 255, 255))'],
    ['URL syntax stays invalid', 'z-index', 'attr(data-missing type(<integer>), attr(data-image type(<url>), 5))', '5'],
    ['unused var fallback', 'z-index', 'var(--present, attr(data-cycle type(<integer>), 5))', '29'],
  ];
  for (const [label, property, expression, expected] of cases) {
    const element = make();
    equal(CSS.supports(property, expression), true, label + ': declaration syntax');
    element.style.setProperty(property, expression);
    equal(value(element, property === 'margin' ? 'margin-left' : property), expected, label);
    element.remove();
  }
  for (const [label, expression, expected] of [
    ['custom attr fallback', 'attr(data-missing type(<integer>), attr(data-number type(<integer>), 5))', '23'],
    ['custom var fallback', 'var(--missing, attr(data-number type(<integer>), 5))', '23'],
    ['custom env fallback', 'env(moli-unknown-environment, attr(data-number type(<integer>), 5))', '23'],
    ['custom invalid fallback', 'attr(data-missing type(<integer>), attr(data-invalid type(<integer>), 7))', '7'],
  ]) {
    const element = make();
    element.style.setProperty('--probe', expression);
    equal(value(element, '--probe'), expected, label);
    element.remove();
  }
  for (const [label, expression, expected] of [
    ['used attribute cycle', 'attr(data-missing type(<integer>), attr(data-cycle type(<integer>), 7))', '7'],
    ['unused attribute cycle', 'attr(data-present type(<integer>), attr(data-cycle type(<integer>), 7))', '29'],
    ['used cycle inside var fallback', 'var(--missing, attr(data-cycle type(<integer>), 7))', '7'],
    ['invalid custom variable inside attribute', 'attr(data-missing type(<integer>), attr(data-dead-var type(<integer>), 7))', '7'],
  ]) {
    const element = make();
    element.style.zIndex = expression;
    equal(value(element, 'z-index'), expected, label);
    element.remove();
  }
  for (const primary of ['data-missing', 'data-invalid', 'data-present']) {
    const present = primary === 'data-present';
    const element = make();
    element.setAttribute('data-cycle', 'var(--loop)');
    element.style.setProperty('--loop', 'attr(' + primary +
                              ' type(<integer>), attr(data-cycle type(<integer>), 7))');
    element.style.zIndex = 'var(--loop, 11)';
    equal(value(element, '--loop'), present ? '29' : '', 'custom cycle: ' + primary);
    equal(value(element, 'z-index'), present ? '29' : '11', 'custom cycle consumer: ' + primary);
    element.remove();
  }
  const parent = make();
  parent.style.fontSize = '10px';
  const relative = make();
  parent.appendChild(relative);
  relative.setAttribute('data-relative', '2em');
  relative.style.fontSize = 'var(--missing, attr(data-relative type(<length>), 7px))';
  equal(value(relative, 'font-size'), '20px', 'font-size fallback uses parent font');
  relative.setAttribute('data-relative', '3em');
  relative.style.fontSize = '10px';
  relative.style.lineHeight = 'var(--missing, attr(data-relative type(<length>), 7px))';
  equal(value(relative, 'line-height'), '30px', 'line-height fallback uses computed font');

  const dynamic = make();
  dynamic.style.zIndex = 'attr(data-missing type(<integer>), attr(data-number type(<integer>), 5))';
  equal(value(dynamic, 'z-index'), '23', 'fallback before mutation');
  dynamic.setAttribute('data-number', '31');
  equal(value(dynamic, 'z-index'), '31', 'fallback after mutation');
  dynamic.removeAttribute('data-number');
  equal(value(dynamic, 'z-index'), '5', 'fallback attribute removed');
  dynamic.setAttribute('data-number', '37');
  equal(value(dynamic, 'z-index'), '37', 'fallback attribute restored');

  const sheet = document.createElement('style');
  sheet.textContent = '#typed-fallback-pseudo::before { content: "before"; color: var(--missing, attr(data-color type(<color>), red)); }';
  document.head.appendChild(sheet);
  const generated = make();
  generated.id = 'typed-fallback-pseudo';
  equal(value(generated, 'color', '::before'), 'rgb(0, 0, 255)', 'pseudo fallback initial value');
  generated.setAttribute('data-color', 'green');
  equal(value(generated, 'color', '::before'), 'rgb(0, 128, 0)', 'pseudo fallback attribute mutation');

  const security = make();
  security.style.setProperty('--url-probe', 'var(--missing, attr(data-image type(*)))');
  equal(value(security, '--url-probe'), attributes['data-image'], 'unregistered universal fallback retains URL tokens');
  security.style.backgroundImage = 'var(--missing, attr(data-image type(*)))';
  equal(value(security, 'background-image'), 'none', 'fallback URL remains attr tainted');
  return JSON.stringify({checks, failures});
})()
"#,
        )
        .expect("typed attr fallback probe should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 62, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
