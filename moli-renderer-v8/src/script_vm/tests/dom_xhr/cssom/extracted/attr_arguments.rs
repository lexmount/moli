use super::*;

#[test]
fn attr_arguments_substitute_before_parsing_and_preserve_dependency_cycles() {
    let mut vm = new_storage_html_test_vm("https://attr-arguments.test/page.html");
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
  const make = () => {
    const element = document.createElement('div');
    for (const [name, value] of Object.entries({
      'data-number': '23', 'data-other': '31', 'data-invalid': 'bad',
      'data-extra-numbers': '23 29',
      'data-name': 'data-number', 'data-recipe': 'data-number type(<integer>)',
      'data-color': 'blue', 'data-linked': 'var(--c185-length)',
      'data-image': 'url(https://attr-arguments-resource.test/private.png)',
    })) element.setAttribute(name, value);
    for (const [name, value] of Object.entries({
      '--args': 'data-number type(<integer>)', '--name': 'data-number',
      '--type': 'type(<integer>)', '--unit': 'number',
      '--comma': 'data-number type(<integer>), 99', '--dead': 'var(--dead)',
    })) element.style.setProperty(name, value);
    document.body.appendChild(element);
    return element;
  };
  const value = (element, property, pseudo) =>
    getComputedStyle(element, pseudo).getPropertyValue(property).trim();
  const cases = [
    ['entire first argument', 'attr(var(--args))', '23'],
    ['empty explicit fallback', 'attr(var(--args),)', '23'],
    ['implicit outer closing parenthesis', 'attr(var(--args)', '23'],
    ['implicit inner and outer closing parentheses', 'attr(var(--args', '23'],
    ['explicit fallback', 'attr(var(--args), 7)', '23'],
    ['first argument whitespace', 'attr(  var(--args)  , 7)', '23'],
    ['type position', 'attr(data-number var(--type), 7)', '23'],
    ['name and type positions', 'attr(var(--name) var(--type), 7)', '23'],
    ['unit position', 'attr(data-number var(--unit), 7)', '23'],
    ['invalid typed attribute', 'attr(data-invalid var(--type), 7)', '7'],
    ['invalid number attribute', 'attr(data-invalid var(--unit), 7)', '7'],
    ['number must consume the entire attribute', 'attr(data-extra-numbers var(--unit), 7)', '7'],
    ['missing attribute', 'attr(data-missing var(--type), 7)', '7'],
    ['missing first argument variable', 'attr(var(--missing), 7)', '7'],
    ['variable fallback in first argument', 'attr(var(--missing, data-other type(<integer>)), 7)', '31'],
    ['invalid variable fallback in first argument', 'attr(var(--dead, data-other type(<integer>)), 7)', '31'],
    ['environment fallback in first argument', 'attr(env(moli-c185-missing, var(--args)), 7)', '23'],
    ['attribute in name position', 'attr(attr(data-name type(<custom-ident>)) var(--type), 7)', '23'],
    ['attribute as entire first argument', 'attr(attr(data-recipe type(*)), 7)', '23'],
    ['nested fallback attribute', 'attr(var(--missing), attr(var(--args), 7))', '23'],
    ['comma stays inside first argument', 'attr(var(--comma), 7)', '7'],
    ['comma cannot add a fallback', 'attr(var(--comma))', 'auto'],
    ['no implicit numeric conversion', 'attr(var(--name))', 'auto'],
    ['unknown type', 'attr(var(--name) future-type, 7)', '7'],
    ['invalid computed name', 'attr(var(--unit), 7)', '7'],
    ['invalid literal argument uses fallback', 'attr((), 7)', '7'],
  ];
  for (const [label, expression, expected] of cases) {
    const element = make();
    equal(CSS.supports('z-index', expression), true, label + ': declaration syntax');
    element.style.zIndex = expression;
    equal(value(element, 'z-index'), expected, label);
    element.remove();
  }
  for (const expression of ['attr(var(--args)', 'attr(var(--args']) {
    const element = make();
    element.style.zIndex = expression;
    equal(element.style.getPropertyValue('z-index'), 'attr(var(--args))', 'implicit closers serialize: ' + expression);
    element.remove();
  }
  for (const [label, expression, attribute, expected] of [
    ['type failure after substitution', 'attr(var(--args), 3px)', 'var(--name)', '3px'],
    ['attribute cycle with fallback', 'attr(var(--args), 7)', 'var(--loop)', ''],
    ['attribute cycle without fallback', 'attr(var(--args))', 'var(--loop)', ''],
    ['first argument cycle with fallback', 'attr(var(--loop), 7)', '23', ''],
    ['used fallback cycle', 'attr(var(--missing), var(--loop))', '23', ''],
    ['unused fallback cycle', 'attr(var(--args), var(--loop))', '23', '23'],
    ['unused argument fallback cycle', 'attr(var(--args, var(--loop)), 7)', '23', '23'],
    ['used argument fallback cycle', 'attr(var(--missing, var(--loop)), 7)', '23', ''],
    ['missing raw attribute fallback cycle', 'attr(data-missing raw-string, var(--loop))', '23', ''],
    ['invalid number attribute fallback cycle', 'attr(data-invalid var(--unit), var(--loop))', '23', ''],
    ['invalid type fallback cycle', 'attr(var(--name) future-type, var(--loop))', '23', ''],
    ['valid number ignores fallback cycle', 'attr(var(--name) var(--unit), var(--loop))', '23', '23'],
  ]) {
    const element = make();
    element.setAttribute('data-number', attribute);
    element.style.setProperty('--loop', expression);
    element.style.zIndex = 'var(--loop, 11)';
    equal(value(element, '--loop'), expected, label);
    const consumer = expected === '' ? '11' : expected === '3px' ? 'auto' : expected;
    equal(value(element, 'z-index'), consumer, label + ': consumer');
    element.remove();
  }

  const dynamic = make();
  dynamic.style.zIndex = 'attr(var(--args), 7)';
  equal(value(dynamic, 'z-index'), '23', 'initial dynamic attribute');
  dynamic.setAttribute('data-number', '37');
  equal(value(dynamic, 'z-index'), '37', 'dynamic attribute mutation');
  dynamic.removeAttribute('data-number');
  equal(value(dynamic, 'z-index'), '7', 'dynamic attribute removal');
  dynamic.setAttribute('data-number', '41');
  equal(value(dynamic, 'z-index'), '41', 'dynamic attribute restoration');
  dynamic.style.setProperty('--args', 'data-other type(<integer>)');
  equal(value(dynamic, 'z-index'), '31', 'attribute name changes through custom property');
  dynamic.setAttribute('data-other', '43');
  equal(value(dynamic, 'z-index'), '43', 'new attribute dependency mutation');
  dynamic.style.setProperty('--args', 'data-number raw-string');
  equal(value(dynamic, 'z-index'), 'auto', 'type changes through custom property');

  const nested = make();
  nested.style.zIndex = 'attr(attr(data-recipe type(*)), 7)';
  equal(value(nested, 'z-index'), '23', 'nested argument initial value');
  nested.setAttribute('data-recipe', 'data-other type(<integer>)');
  equal(value(nested, 'z-index'), '31', 'argument-producing attribute changes');
  nested.setAttribute('data-other', '47');
  equal(value(nested, 'z-index'), '47', 'nested argument target changes');

  const deep = make();
  for (let i = 0; i < 8; i++)
    deep.setAttribute('data-chain-' + i, i === 0 ? 'data-number type(<integer>)' : 'data-chain-' + (i - 1) + ' type(*)');
  let expression = 'attr(data-chain-7 type(*))';
  for (let i = 0; i < 8; i++) expression = 'attr(' + expression + ')';
  equal(CSS.supports('z-index', expression), true, 'nested argument syntax');
  deep.style.zIndex = expression;
  equal(value(deep, 'z-index'), '23', 'nested argument dependency chain');

  const sheet = document.createElement('style');
  sheet.textContent = '#attr-arguments-pseudo::before { content: "before"; color: attr(var(--color-args), red); }';
  document.head.appendChild(sheet);
  const pseudo = make();
  pseudo.id = 'attr-arguments-pseudo';
  pseudo.style.setProperty('--color-args', 'data-color type(<color>)');
  equal(value(pseudo, 'color', '::before'), 'rgb(0, 0, 255)', 'pseudo dynamic argument');
  pseudo.setAttribute('data-color', 'green');
  equal(value(pseudo, 'color', '::before'), 'rgb(0, 128, 0)', 'pseudo attribute mutation');

  const inherited = make();
  inherited.style.setProperty('--inherited-value', 'attr(var(--args), 7)');
  const child = document.createElement('div');
  child.style.zIndex = 'var(--inherited-value)';
  inherited.appendChild(child);
  equal(value(child, 'z-index'), '23', 'inherited computed attribute');
  inherited.setAttribute('data-number', '53');
  equal(value(child, 'z-index'), '53', 'inherited computed attribute mutation');

  const security = make();
  security.style.setProperty('--image-args', 'data-image type(<url>)');
  security.style.backgroundImage = 'attr(var(--image-args), none)';
  equal(value(security, 'background-image'), 'none', 'dynamic URL type stays disallowed');
  security.style.setProperty('--image-args', 'data-image type(*)');
  security.style.setProperty('--image-tokens', 'attr(var(--image-args))');
  equal(value(security, '--image-tokens'), security.getAttribute('data-image'), 'universal dynamic attribute keeps raw URL tokens');
  security.style.backgroundImage = 'var(--image-tokens)';
  equal(value(security, 'background-image'), 'none', 'dynamic universal value stays attr tainted');

  CSS.registerProperty({name: '--c185-length', syntax: '<length>', inherits: false, initialValue: '0px'});
  const parent = make();
  parent.style.fontSize = '10px';
  const font = make();
  parent.appendChild(font);
  font.style.setProperty('--c185-length', '2em');
  font.style.setProperty('--args', 'data-linked type(<length>)');
  font.style.fontSize = 'attr(var(--args), 7px)';
  equal(value(font, 'font-size'), '10px', 'registered font dependency cycle');
  equal(value(font, '--c185-length'), '0px', 'registered dependency falls back to initial value');

  return JSON.stringify({checks, failures});
})()
"#,
        )
        .expect("attr argument probe should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 99, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
