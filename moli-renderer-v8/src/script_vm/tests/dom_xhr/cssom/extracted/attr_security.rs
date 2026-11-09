use super::*;

#[test]
fn typed_attr_rejects_url_syntax_and_preserves_image_taint() {
    let mut vm = new_storage_html_test_vm("https://typed-attr-security.test/page.html");
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
  const source = 'https://attribute-value.test/private.png';
  const url = 'url(' + source + ')';
  const blocked = [
    ['URL token', '<url>', url],
    ['URL function', '<url>', 'url("' + source + '")'],
    ['escaped URL function', '<url>', String.raw`u\72l("${source}")`],
    ['URL space list', '<url>+', url + ' ' + url],
    ['URL comma list', '<url>#', url + ', ' + url],
    ['escaped URL type', String.raw`<u\72l>`, url],
    ['URL alternative', '<url> | <string>', url],
    ['quoted URL syntax', '"<url>"', url],
    ['string alternative cannot bypass URL ban', '<url> | <string>', '"allowed"'],
    ['earlier non-URL alternative cannot bypass URL ban', '<custom-ident> | <url>', 'allowed'],
    ['earlier literal cannot bypass URL ban', 'allowed | <url>', 'allowed'],
  ];
  for (const [label, syntax, value] of blocked) {
    const element = document.createElement('div');
    element.setAttribute('data-value', value);
    document.body.appendChild(element);
    const expression = 'attr(data-value type(' + syntax + '))';
    element.style.setProperty('--probe', expression);
    equal(element.style.getPropertyValue('--probe').includes('attr('), true, label + ': retained declaration');
    equal(CSS.supports('--probe', expression), true, label + ': valid declaration syntax');
    equal(getComputedStyle(element).getPropertyValue('--probe'), '', label + ': URL syntax invalid');
    element.style.setProperty('--probe', 'attr(data-value type(' + syntax + '), SAFE)');
    equal(getComputedStyle(element).getPropertyValue('--probe'), 'SAFE', label + ': invalid syntax fallback');
    element.remove();
  }
  const valid = [
    ['default string', '', url, JSON.stringify(url)],
    ['raw string', ' raw-string', url, JSON.stringify(url)],
    ['typed string', ' type(<string>)', JSON.stringify(source), JSON.stringify(source)],
    ['number', ' type(<number>)', '12', '12'],
    ['length', ' type(<length>)', '5px', '5px'],
    ['percentage', ' type(<percentage>)', '25%', '25%'],
    ['custom ident', ' type(<custom-ident>)', 'allowed', 'allowed'],
    ['universal token stream', ' type(*)', url, url],
    ['literal url keyword', ' type(url)', 'url', 'url'],
  ];
  for (const [label, type, value, expected] of valid) {
    const element = document.createElement('div');
    element.setAttribute('data-value', value);
    element.style.setProperty('--probe', 'attr(data-value' + type + ')');
    document.body.appendChild(element);
    equal(getComputedStyle(element).getPropertyValue('--probe'), expected, label);
    element.remove();
  }
  const images = [
    ['image URL', '<image>', url, 'url("' + source + '")'],
    ['image set URL', '<image>', 'image-set(' + url + ' 1x)', 'image-set(url("' + source + '") 1x)'],
    ['image list', '<image>#', 'linear-gradient(black, white), ' + url,
     'linear-gradient(black, white), url("' + source + '")'],
  ];
  for (const [label, syntax, value, expected] of images) {
    const element = document.createElement('div');
    element.setAttribute('data-value', value);
    document.body.appendChild(element);
    element.style.setProperty('--probe', 'attr(data-value type(' + syntax + '))');
    equal(getComputedStyle(element).getPropertyValue('--probe'), expected, label + ': valid image syntax');
    element.style.setProperty('--probe', 'attr(data-value type(' + syntax + '), SAFE)');
    equal(getComputedStyle(element).getPropertyValue('--probe'), expected, label + ': fallback not selected');
    element.style.backgroundImage = 'var(--probe)';
    equal(getComputedStyle(element).backgroundImage, 'none', label + ': image URL stays attr tainted');
    element.remove();
  }
  const element = document.createElement('div');
  document.body.appendChild(element);
  element.setAttribute('data-value', 'linear-gradient(black, white)');
  element.style.backgroundImage = 'attr(data-value type(<image>))';
  equal(getComputedStyle(element).backgroundImage,
        'linear-gradient(rgb(0, 0, 0), rgb(255, 255, 255))', 'gradient image remains usable');
  element.style.backgroundImage = url;
  equal(getComputedStyle(element).backgroundImage, 'url("' + source + '")', 'ordinary URL remains usable');
  element.setAttribute('data-value', url);
  element.style.setProperty('--probe', 'attr(data-value type(*))');
  element.style.backgroundImage = 'var(--probe)';
  equal(getComputedStyle(element).backgroundImage, 'none', 'universal token stream stays attr tainted');
  element.style.setProperty('--probe', 'attr(data-value type(<url>))');
  element.style.setProperty('--consumer', 'var(--probe, FALLBACK)');
  equal(getComputedStyle(element).getPropertyValue('--consumer'), 'FALLBACK', 'invalid typed URL triggers var fallback');
  CSS.registerProperty({name: '--registered-url', syntax: '<url>', inherits: false,
                        initialValue: 'url("https://safe.test/default.png")'});
  element.style.setProperty('--registered-url', 'attr(data-value type(<url>))');
  equal(getComputedStyle(element).getPropertyValue('--registered-url'),
        'url("https://safe.test/default.png")', 'registered URL keeps its initial value');
  element.remove();
  return JSON.stringify({checks, failures});
})()
"#,
        )
        .expect("typed attr URL security probe should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 67, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
