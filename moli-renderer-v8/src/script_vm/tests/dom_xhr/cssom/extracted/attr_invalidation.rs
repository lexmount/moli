use super::*;

#[test]
fn style_attribute_value_is_shared_by_dom_readers() {
    let mut vm = new_storage_html_test_vm("https://style-attribute-readers.test/");
    assert_eq!(
        vm.eval(r#"
(() => {
  const el = document.createElement('div');
  const sheet = document.createElement('style');
  sheet.textContent = 'div[style*="height: 20px"] { color: rgb(0, 128, 0); }';
  document.head.append(sheet);
  document.body.append(el);
  if (el.hasAttribute('style') || el.matches('[style]')) throw new Error('missing style');
  const raw = 'width: 10px; /* preserve this */';
  el.setAttribute('style', raw);
  if (el.style.width !== '10px' || el.getAttribute('style') !== raw) throw new Error('raw style');
  const attr = el.getAttributeNode('style');
  el.style.height = '20px';
  const expected = 'width: 10px; height: 20px;';
  const values = [el.getAttribute('STYLE'), el.getAttributeNS(null, 'style'), attr.value,
    el.attributes.getNamedItemNS(null, 'style').value,
    el.cloneNode().getAttribute('style'), document.importNode(el).getAttribute('style')];
  if (values.some(value => value !== expected)) throw new Error(JSON.stringify(values));
  if (!el.matches('[style$="height: 20px;"]') || document.querySelector('[style]') !== el ||
      getComputedStyle(el).color !== 'rgb(0, 128, 0)') throw new Error('style selectors');
  for (const text of [el.outerHTML, new XMLSerializer().serializeToString(el)]) {
    if (!text.includes(`style="${expected}"`)) throw new Error(text);
  }
  el.style.cssText = '';
  if (!el.hasAttributeNS(null, 'style') || el.getAttribute('style') !== '' || !el.matches('[style=""]')) {
    throw new Error('empty style');
  }
  const xhtml = new DOMParser().parseFromString(
    '<div xmlns="http://www.w3.org/1999/xhtml" STYLE="case-sensitive"/>', 'application/xml'
  ).documentElement;
  if (xhtml.hasAttribute('style') || xhtml.getAttribute('STYLE') !== 'case-sensitive' ||
      xhtml.getAttributeNode('STYLE').value !== 'case-sensitive') {
    throw new Error('attribute names in XHTML must retain their case');
  }
  return 'ok';
})()
"#)
        .expect("DOM readers should observe the current inline style"),
        "ok"
    );
}

#[test]
fn attr_attribute_mutations_refresh_computed_values() {
    let mut vm = new_storage_html_test_vm("https://attr-invalidation.test/page.html");
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
  sheet.textContent = `
    .typed { z-index: attr(data-value type(<integer>), 7); }
    .raw { --text: attr(data-text, "missing"); }
    .inherited { --tone: attr(data-tone type(<color>), red); color: var(--tone); }
    .consumer { color: var(--tone); }
    .nested { z-index: attr(data-primary number, attr(data-secondary number, 5)); }
    .generated::before { content: "before"; color: attr(data-tone type(<color>), red); }
    .generated::after { content: "after"; color: attr(data-tone type(<color>), red); }
    .placeholder::placeholder { color: attr(data-tone type(<color>), red); }
  `;
  document.head.appendChild(sheet);
  const make = (className, tag = 'div') => {
    const element = document.createElement(tag);
    element.className = className;
    document.body.appendChild(element);
    return element;
  };
  const value = (element, property, pseudo) =>
    getComputedStyle(element, pseudo).getPropertyValue(property);
  for (const inline of [false, true]) {
    const element = make(inline ? '' : 'typed');
    if (inline) element.style.zIndex = 'attr(data-value type(<integer>), 7)';
    const label = inline ? 'inline' : 'stylesheet';
    equal(value(element, 'z-index'), '7', label + ': missing fallback');
    element.setAttribute('data-value', '11');
    equal(value(element, 'z-index'), '11', label + ': add attribute');
    element.setAttribute('data-value', '13');
    equal(value(element, 'z-index'), '13', label + ': change attribute');
    equal(value(element, 'z-index'), '13', label + ': repeated read');
    element.dataset.value = '17';
    equal(value(element, 'z-index'), '17', label + ': dataset mutation');
    element.setAttribute('data-value', 'invalid');
    equal(value(element, 'z-index'), '7', label + ': invalid value fallback');
    element.removeAttribute('data-value');
    equal(value(element, 'z-index'), '7', label + ': remove attribute');
    element.setAttribute('DATA-VALUE', '19');
    equal(value(element, 'z-index'), '19', label + ': HTML attribute casing');
  }
  const raw = make('raw');
  equal(value(raw, '--text'), '"missing"', 'raw: missing fallback');
  raw.setAttribute('data-text', 'first');
  equal(value(raw, '--text'), '"first"', 'raw: add');
  raw.setAttribute('data-text', 'second');
  equal(value(raw, '--text'), '"second"', 'raw: change');
  raw.setAttribute('data-text', '');
  equal(value(raw, '--text'), '""', 'raw: empty attribute');
  raw.removeAttribute('data-text');
  equal(value(raw, '--text'), '"missing"', 'raw: remove');

  const parent = make('inherited');
  const child = make('consumer');
  parent.appendChild(child);
  equal(value(child, 'color'), 'rgb(255, 0, 0)', 'inherited: missing parent attribute');
  parent.setAttribute('data-tone', 'blue');
  equal(value(child, 'color'), 'rgb(0, 0, 255)', 'inherited: child read before parent');
  equal(value(parent, 'color'), 'rgb(0, 0, 255)', 'inherited: parent read');
  child.setAttribute('data-tone', 'green');
  equal(value(child, 'color'), 'rgb(0, 0, 255)', 'inherited: substitution stays on parent');
  parent.removeAttribute('data-tone');
  equal(value(child, 'color'), 'rgb(255, 0, 0)', 'inherited: parent fallback restored');

  const nested = make('nested');
  equal(value(nested, 'z-index'), '5', 'nested: both attributes absent');
  nested.setAttribute('data-secondary', '23');
  equal(value(nested, 'z-index'), '23', 'nested: add fallback attribute');
  nested.setAttribute('data-primary', '29');
  equal(value(nested, 'z-index'), '29', 'nested: primary overrides fallback');
  nested.setAttribute('data-secondary', '31');
  equal(value(nested, 'z-index'), '29', 'nested: unused fallback mutation');
  nested.removeAttribute('data-primary');
  equal(value(nested, 'z-index'), '31', 'nested: fallback dependency reactivated');

  CSS.registerProperty({name: '--attr-index', syntax: '<integer>', inherits: true, initialValue: '3'});
  const registeredParent = make('');
  const registeredChild = make('');
  registeredParent.style.setProperty('--attr-index', 'attr(data-value type(<integer>), 7)');
  registeredChild.style.zIndex = 'var(--attr-index)';
  registeredParent.appendChild(registeredChild);
  equal(value(registeredChild, 'z-index'), '7', 'registered: missing parent attribute');
  registeredParent.setAttribute('data-value', '37');
  equal(value(registeredChild, 'z-index'), '37', 'registered: inherited computed value changes');
  registeredParent.removeAttribute('data-value');
  equal(value(registeredChild, 'z-index'), '7', 'registered: fallback restored');

  const generated = make('generated');
  const placeholder = make('placeholder', 'input');
  placeholder.placeholder = 'hint';
  for (const [element, pseudo, label] of [
    [generated, '::before', 'before'],
    [generated, '::after', 'after'],
    [placeholder, '::placeholder', 'placeholder'],
  ]) {
    equal(value(element, 'color', pseudo), 'rgb(255, 0, 0)', label + ': missing fallback');
    element.setAttribute('data-tone', 'blue');
    equal(value(element, 'color', pseudo), 'rgb(0, 0, 255)', label + ': originating attribute changes');
    element.removeAttribute('data-tone');
    equal(value(element, 'color', pseudo), 'rgb(255, 0, 0)', label + ': originating attribute removed');
  }
  return JSON.stringify({checks, failures});
})()
"#,
        )
        .expect("attr invalidation probe should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 43, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}

#[test]
fn dir_mutation_refreshes_auto_ancestor_and_attr_dependent_pseudo() {
    let mut vm = new_parsed_test_vm(
        "https://attr-dir-invalidation.test/",
        r#"<!doctype html>
        <style>#subject::before { content: "marker"; --authored-dir: attr(dir, "unset"); }</style>
        <div id="parent" dir="auto"><span id="subject">ת</span>abc</div>"#,
    );
    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.getElementById('parent');
  const subject = document.getElementById('subject');
  const read = () => [
    getComputedStyle(parent).direction,
    getComputedStyle(subject, '::before').getPropertyValue('--authored-dir'),
  ].join(':');
  const initial = read();
  subject.dir = 'ltr';
  const updated = read();
  subject.removeAttribute('dir');
  return [initial, updated, read()].join('|');
})()
"#,
        )
        .expect("dir mutation should update both style dependencies");
    assert_eq!(result, r#"rtl:"unset"|ltr:"ltr"|rtl:"unset""#);
}
#[test]
fn cssom_style_attribute_updates_value_selectors_after_retained_resolution() {
    let mut vm = new_parsed_test_vm(
        "https://cssom-style-attribute-selector.test/",
        r#"<!doctype html>
        <style>
          #probe { color: red; }
          #parent:has(#box[style*="blue"]) > #probe { color: green; }
        </style>
        <div id="parent"><div id="box"></div><span id="probe">text</span></div>"#,
    );
    assert_eq!(
        vm.eval(
            r#"
            const box = document.getElementById('box');
            const probe = document.getElementById('probe');
            const colors = [getComputedStyle(probe).color];
            box.style.color = 'blue';
            colors.push(getComputedStyle(probe).color);
            box.style.color = 'red';
            colors.push(getComputedStyle(probe).color);
            colors.join('/')
        "#
        )
        .unwrap(),
        "rgb(255, 0, 0)/rgb(0, 128, 0)/rgb(255, 0, 0)",
    );
}
