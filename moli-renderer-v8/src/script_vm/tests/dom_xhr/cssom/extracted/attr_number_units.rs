use super::*;

#[test]
fn attr_numeric_units_consume_one_number_token_and_preserve_value_semantics() {
    let mut vm = new_storage_html_test_vm("https://attr-number-units.test/page.html");
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
  const value = (element, property, pseudo) =>
    getComputedStyle(element, pseudo).getPropertyValue(property).trim();
  const corpus = [
    ['plain', '23', '23'], ['leading space', ' 23', '23'],
    ['trailing space', '23 ', '23'], ['CSS whitespace', '\t23\r\n\f', '23'],
    ['leading comment', '/**/23', '23'], ['trailing comment', '23/**/', '23'],
    ['both comments', '/**/23/**/', '23'], ['unclosed trailing comment', '23/*', '23'],
    ['fraction', '.5', '.5'], ['positive sign', '+23', '+23'],
    ['negative number', '-23', '-23'], ['negative zero', '-0', '-0'],
    ['exponent', '2.5e1', '2.5e1'], ['exponent whitespace', '2.5e1 ', '2.5e1'],
    ['large finite exponent', '1e30', '1e30'], ['small finite exponent', '1e-30', '1e-30'],
    ['overflow', '1e1000', '3.4028235e38'],
    ['overflow whitespace', '1e1000 ', '3.4028235e38'],
    ['negative overflow whitespace', '-1e1000 ', '-3.4028235e38'],
    ['underflow whitespace', '1e-1000 ', '1e-1000'],
    ['extra number', '23 29', null], ['dimension', '23px', null],
    ['tokens separated by comment', '1/**/0', null],
    ['non-ASCII whitespace', '23\u00a0', null], ['vertical tab', '23\u000b', null],
    ['empty', '', null], ['whitespace only', '\t\r\n\f', null],
    ['calc', 'calc(1 + 2)', null], ['var', 'var(--number)', null],
    ['NaN', 'NaN', null], ['infinity identifier', 'Infinity', null], ['NUL', '\u000023', null],
  ];
  for (const [label, input, number] of corpus) {
    for (const [unit, property] of [['px', 'margin-left'], ['%', 'opacity'],
                                    ['s', 'transition-duration'], ['number', 'font-weight']]) {
      const suffix = unit === 'number' ? '' : unit;
      const element = document.createElement('div');
      const reference = document.createElement('div');
      document.body.append(element, reference);
      element.setAttribute('data-num', input);
      element.style.setProperty(property, 'attr(data-num ' + unit + ', 7' + suffix + ')');
      reference.style.setProperty('--unit-reference', (number === null ? '7' : number) + suffix);
      reference.style.setProperty(property, 'var(--unit-reference)');
      const actual = value(element, property), expected = value(reference, property);
      equal(actual, expected, label + ': ' + unit);
      element.remove(); reference.remove();
    }
  }
  for (const number of ['23', '23.0', '2.3e1']) {
    const element = document.createElement('div'), reference = document.createElement('div');
    document.body.append(element, reference);
    element.setAttribute('data-num', number + ' /**/');
    element.style.zIndex = 'attr(data-num number, 7)';
    reference.style.zIndex = number;
    equal(value(element, 'z-index'), value(reference, 'z-index'), 'integer token flag: ' + number);
    element.remove(); reference.remove();
  }
  const element = document.createElement('div'); document.body.appendChild(element);
  element.style.marginLeft = 'attr(data-num px, 7px)';
  for (const [input, expected] of [['23 ', '23px'], ['31/**/', '31px'], ['bad', '7px'], ['-0 ', '0px']]) {
    element.setAttribute('data-num', input);
    equal(value(element, 'margin-left'), expected, 'attribute mutation: ' + input);
  }
  element.setAttribute('data-num', '23 ');
  element.style.setProperty('--unit', 'px');
  element.style.marginLeft = 'attr(data-num var(--unit), 7px)';
  equal(value(element, 'margin-left'), '23px', 'computed unit argument');
  element.style.marginLeft = 'calc(attr(data-num px) + 1px)';
  equal(value(element, 'margin-left'), '24px', 'converted dimension in calc');
  for (const [input, expected] of [['23 ', '23px'], ['23 29', '11px']]) {
    element.setAttribute('data-num', input);
    element.style.setProperty('--loop', 'attr(data-num px, var(--loop))');
    element.style.marginLeft = 'var(--loop, 11px)';
    equal(value(element, 'margin-left'), expected, 'fallback cycle: ' + input);
  }
  const sheet = document.createElement('style');
  sheet.textContent = `
    #attr-number-units-pseudo::before { content: 'x'; margin-left: attr(data-num px, 7px); }
    @container style(1px < attr(data-num px, 0px) < 10px) { #attr-number-units-child { --hit: true; } }
  `;
  document.head.appendChild(sheet);
  const pseudo = document.createElement('div'); pseudo.id = 'attr-number-units-pseudo';
  document.body.appendChild(pseudo);
  pseudo.setAttribute('data-num', '23 ');
  equal(value(pseudo, 'margin-left', '::before'), '23px', 'pseudo dimension');
  pseudo.setAttribute('data-num', '37/**/');
  equal(value(pseudo, 'margin-left', '::before'), '37px', 'pseudo mutation');
  const container = document.createElement('main'), child = document.createElement('div');
  child.id = 'attr-number-units-child'; container.appendChild(child); document.body.appendChild(container);
  container.setAttribute('data-num', '5 ');
  equal(value(child, '--hit'), 'true', 'unit in container style query');
  container.setAttribute('data-num', '20/**/');
  equal(value(child, '--hit'), '', 'container query attribute mutation');
  return JSON.stringify({checks, failures});
})()
"#,
        )
        .expect("attr numeric unit probe should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 143, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
