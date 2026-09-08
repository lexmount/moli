use super::*;

fn assert_url_components(script: &str) {
    let mut vm = new_parsed_test_vm(
        "https://url-components.test/base/index.html",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm
        .eval(&format!(
            r#"(() => {{
const assert = (condition, message) => {{ if (!condition) throw new Error(message); }};
const frame = document.body.appendChild(document.createElement('iframe'));
const factories = [
  ['URL', href => new URL(href)],
  ['URL.parse', href => URL.parse(href)],
];
for (const [name, doc] of [
  ['main', document],
  ['child', frame.contentDocument],
  ['detached', document.implementation.createHTMLDocument('components')],
]) {{
  for (const tag of ['a', 'area']) {{
    for (const connected of [false, true]) {{
      factories.push([`${{name}}/${{tag}}/${{connected}}`, href => {{
        const node = doc.createElement(tag);
        node.href = href;
        if (connected) doc.body.appendChild(node);
        return node;
      }}]);
    }}
  }}
}}
{script}
return 'ok';
}})()"#
        ))
        .expect("URL component semantics should match across exposed interfaces");
    assert_eq!(result, "ok");
}

#[test]
fn url_components_empty_getters_preserve_href() {
    assert_url_components(
        r#"
const cases = [
  ['', '', ''], ['?', '', ''], ['#', '', ''], ['?#', '', ''],
  ['?q=%23#frag', '?q=%23', '#frag'], ['??', '??', ''], ['##', '', '##'],
  ['?%3F#%23', '?%3F', '#%23'], ['?%20#%20', '?%20', '#%20'],
];
for (const [name, create] of factories) {
  for (const base of ['https://example.test/path', 'file:///tmp/item', 'mailto:user@example.test']) {
    for (const [suffix, search, hash] of cases) {
      const href = base + suffix;
      const object = create(href);
      assert(object.search === search, `${name}: search for ${href}`);
      assert(object.hash === hash, `${name}: hash for ${href}`);
      assert(object.href === href, `${name}: getters preserve href including empty delimiters`);
    }
  }
}
"#,
    );
}

#[test]
fn url_components_setters_strip_only_one_delimiter() {
    assert_url_components(
        r#"
const base = 'https://example.test/path';
for (const [name, create] of factories) {
  for (const [property, delimiter] of [['search', '?'], ['hash', '#']]) {
    for (const value of [delimiter.repeat(2), delimiter.repeat(3) + 'payload', 'payload', '%23%3F']) {
      const object = create(base);
      const expected = value.startsWith(delimiter) ? value : delimiter + value;
      object[property] = value;
      assert(object[property] === expected, `${name}: ${property} retains payload in ${value}`);
      assert(object.href === base + expected, `${name}: setter preserves remaining delimiters`);
    }
  }
  const object = create(base + '#keep');
  const params = object.searchParams;
  object.search = '??q=value';
  assert(object.href === base + '??q=value#keep', `${name}: query update retains fragment`);
  if (params) {
    assert(object.searchParams === params && params.get('?q') === 'value', 'live URLSearchParams observes the unstripped query payload');
    params.set('?q', 'next');
    assert(object.search === '?%3Fq=next' && object.hash === '#keep', 'URLSearchParams encodes the literal query marker');
  }
}
"#,
    );
}

#[test]
fn url_components_clearing_distinguishes_empty_from_absent() {
    assert_url_components(
        r#"
const base = 'https://example.test/path';
for (const [name, create] of factories) {
  const object = create(base + '?q=value#frag');
  const params = object.searchParams;
  object.search = '?';
  assert(object.search === '' && object.href === base + '?#frag', `${name}: empty query preserves its delimiter`);
  if (params) assert(params.size === 0, 'empty query clears live URLSearchParams');
  object.hash = '#';
  assert(object.hash === '' && object.href === base + '?#', `${name}: empty fragment preserves its delimiter`);
  object.search = '';
  assert(object.search === '' && object.href === base + '#', `${name}: clearing query preserves empty fragment`);
  object.hash = '';
  assert(object.hash === '' && object.href === base, `${name}: clearing fragment removes its delimiter`);
}
"#,
    );
}

#[test]
fn location_components_empty_getters_preserve_document_urls() {
    assert_url_components(
        r#"
const cases = [
  ['', '', ''], ['?', '', ''], ['#', '', ''], ['?#', '', ''],
  ['??query##fragment', '??query', '##fragment'], ['?%3F#%23', '?%3F', '#%23'],
];
for (const [suffix, search, hash] of cases) {
  const href = 'https://url-components.test/base/index.html' + suffix;
  history.replaceState(null, '', href);
  assert(location.search === search && location.hash === hash, `main Location components for ${suffix}`);
  assert(location.href === href && document.URL === href, 'Location getters preserve the document URL');
  const child = document.createElement('iframe');
  child.src = 'about:blank' + suffix;
  document.body.appendChild(child);
  const childLocation = child.contentWindow.location;
  assert(childLocation.search === search && childLocation.hash === hash, `child Location components for ${suffix}`);
  assert(childLocation.href === 'about:blank' + suffix, 'child Location getters preserve delimiters');
  child.remove();
}
"#,
    );
}
