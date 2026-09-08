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
  ['child URL', href => new frame.contentWindow.URL(href)],
  ['child URL.parse', href => frame.contentWindow.URL.parse(href)],
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
const checkSetter = (href, property, value, expected) => {{
  for (const [name, create] of factories) {{
    const object = create(href);
    const params = object.searchParams;
    object[property] = value;
    assert(object.href === expected,
      `${{name}}: ${{property}} = ${{JSON.stringify(value)}}: expected ${{expected}}, got ${{object.href}}`);
    if (params) assert(object.searchParams === params, 'setters preserve the URLSearchParams object');
  }}
}};
{script}
return 'ok';
}})()"#
        ))
        .expect("URL component semantics should match across exposed interfaces");
    assert_eq!(result, "ok");
}

#[test]
fn url_components_encode_carets_only_in_hierarchical_paths() {
    assert_url_components(
        r#"
for (const [name, create] of factories) {
  for (const prefix of ['https://host', 'file://', 'custom:', 'custom://host']) {
    const object = create(prefix + '/a^b/%5e?^#^');
    assert(object.href === prefix + '/a%5Eb/%5e?^#^', name + ': path caret encoded once');
    assert(object.pathname === '/a%5Eb/%5e', name + ': pathname uses the path encode set');
    object.pathname = '/^/%5e';
    assert(object.href === prefix + '/%5E/%5e?^#^', name + ': pathname setter shares encoding');
    object.search = '^';
    object.hash = '^';
    assert(object.search === '?^' && object.hash === '#^', name + ': suffix carets are literal');
  }
  const opaque = create('data:a b^c?^#^');
  assert(opaque.href === 'data:a b^c?^#^', name + ': opaque path uses its own encode set');
  opaque.pathname = '/replacement^';
  assert(opaque.pathname === 'a b^c', name + ': opaque pathname is not mutable');
}
"#,
    );
}

#[test]
fn url_components_encode_the_final_opaque_space_during_parsing() {
    assert_url_components(
        r#"
for (const [name, create] of factories) {
  for (const [input, path] of [
    ['payload ', 'payload%20'], ['payload   ', 'payload  %20'],
    ['payload \t\n\r', 'payload%20'], ['payload \t \n \r', 'payload  %20'],
    ['payload%20', 'payload%20'], ['payload %3F', 'payload %3F'],
  ]) {
    for (const suffix of ['?', '#', '?q', '#h', '?q#h', '?#']) {
      const object = create('data:' + input + suffix);
      const expected = 'data:' + path + suffix;
      assert(object.href === expected, name + ': canonical href at construction: ' + expected);
      assert(object.pathname === path, name + ': canonical path before any setter');
      assert(new URL(object.href).href === expected, name + ': serialization is stable');
      object.href = 'custom:' + input + suffix;
      assert(object.href === 'custom:' + path + suffix, name + ': href replacement shares parsing');
    }
  }
}
"#,
    );
}

#[test]
fn url_components_opaque_path_is_stable_across_suffix_and_search_params_mutations() {
    assert_url_components(
        r#"
const base = 'data:payload  %20';
for (const [name, create] of factories) {
  for (const first of ['search', 'hash']) {
    const object = create('data:payload   ?q#h');
    const params = object.searchParams;
    object[first] = '';
    assert(object.href === base + (first === 'search' ? '#h' : '?q'), name + ': clearing one suffix');
    object[first === 'search' ? 'hash' : 'search'] = '';
    assert(object.href === base, name + ': clearing both suffixes preserves the path');
    object.search = '?';
    object.hash = '#';
    assert(object.href === base + '?#', name + ': empty suffixes keep their delimiters');
    if (params) {
      assert(object.searchParams === params, name + ': setters retain searchParams identity');
      params.append('q', 'value');
      assert(object.href === base + '?q=value#', name + ': append does not modify path');
      params.delete('q');
      assert(object.href === base + '#', name + ': delete removes only the query');
      params.sort();
      assert(object.href === base + '#', name + ': sorting empty params preserves the path');
      object.hash = '';
      assert(object.href === base, name + ': removing the final suffix preserves the path');
    }
  }
}
"#,
    );
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
fn url_components_protocol_setter_stops_at_the_first_colon() {
    assert_url_components(
        r#"
for (const value of ['HTTPS:any suffix', 'https::::', 'h\r\ntt\tps:ignored']) {
  checkSetter('http://example.test:443/path?q=value#frag', 'protocol', value,
    'https://example.test/path?q=value#frag');
}
checkSetter('data:text/plain,hello', 'protocol', 'custom:ignored', 'custom:text/plain,hello');
for (const value of ['', '1https', 'https ', 'https\0', 'custom:']) {
  checkSetter('http://example.test/path', 'protocol', value, 'http://example.test/path');
}
checkSetter('https://user:pass@example.test/path', 'protocol', 'file:',
  'https://user:pass@example.test/path');
checkSetter('file:///path', 'protocol', 'https:', 'file:///path');
"#,
    );
}

#[test]
fn url_components_host_setter_preserves_ports_and_accepts_partial_updates() {
    assert_url_components(
        r#"
const base = 'https://old.test:8443/path?q=value#frag';
for (const value of ['new.test', 'new.test:', 'new.test:invalid', 'new.test:65536']) {
  checkSetter(base, 'host', value, 'https://new.test:8443/path?q=value#frag');
}
for (const suffix of ['/discard', '?discard:99', '#discard', '\\discard']) {
  checkSetter(base, 'host', 'new.test' + suffix, 'https://new.test:8443/path?q=value#frag');
  checkSetter(base, 'host', 'new.test:443' + suffix, 'https://new.test/path?q=value#frag');
}
checkSetter(base, 'host', '[2001:db8::2]:123tail', 'https://[2001:db8::2]:123/path?q=value#frag');
checkSetter(base, 'host', '[::1]', 'https://[::1]:8443/path?q=value#frag');
checkSetter(base, 'host', 'new\t.\r\ntest', 'https://new.test:8443/path?q=value#frag');
for (const value of ['', 'bad host', 'user@new.test', '[::invalid]']) {
  checkSetter(base, 'host', value, base);
}
checkSetter('file://old.test/path', 'host', '', 'file:///path');
checkSetter('file://old.test/path', 'host', '\t\r\n', 'file:///path');
checkSetter('file://old.test/path', 'host', 'loc%41lhost', 'file:///path');
checkSetter('custom://old.test/path', 'host', '', 'custom:///path');
checkSetter('custom:/path', 'host', 'new.test', 'custom://new.test/path');
checkSetter('mailto:user@example.test', 'host', 'new.test', 'mailto:user@example.test');
for (const href of ['custom://user@old.test/path', 'custom://:pass@old.test/path', 'custom://old.test:123/path']) {
  for (const value of ['', '/discard', '#discard', '\t\r\n']) checkSetter(href, 'host', value, href);
}
"#,
    );
}

#[test]
fn url_components_hostname_setter_does_not_accept_ports_or_modify_other_components() {
    assert_url_components(
        r#"
const base = 'https://old.test:8443/path?q=value#frag';
for (const value of ['new.test', 'new.test/discard', 'new.test?discard', 'new.test#discard']) {
  checkSetter(base, 'hostname', value, 'https://new.test:8443/path?q=value#frag');
}
for (const value of ['new.test:443', 'new.test:', '[::1]:443', 'user@new.test', 'bad host']) {
  checkSetter(base, 'hostname', value, base);
}
checkSetter(base, 'hostname', '[2001:db8::2]', 'https://[2001:db8::2]:8443/path?q=value#frag');
checkSetter('file://old.test/path', 'hostname', '', 'file:///path');
checkSetter('file://old.test/path', 'hostname', '\t\r\n', 'file:///path');
checkSetter('custom://old.test/path', 'hostname', '', 'custom:///path');
checkSetter('data:payload', 'hostname', 'new.test', 'data:payload');
checkSetter('mailto:user@example.test', 'hostname', 'new.test', 'mailto:user@example.test');
"#,
    );
}

#[test]
fn url_components_port_setter_parses_digit_prefixes_without_clearing_invalid_values() {
    assert_url_components(
        r#"
const base = 'https://example.test:8443/path?q=value#frag';
for (const value of ['123tail', '123/path', '123?query', '123#hash', '\t1\n2\r3\t']) {
  checkSetter(base, 'port', value, 'https://example.test:123/path?q=value#frag');
}
for (const value of [null, undefined, 'invalid', '+123', '-1', '65536', ' 123', '\n\t\r']) {
  checkSetter(base, 'port', value, base);
}
for (const value of ['', '443']) {
  checkSetter(base, 'port', value, 'https://example.test/path?q=value#frag');
}
for (const href of ['file://example.test/path', 'custom:///path', 'custom:/path', 'data:payload']) {
  checkSetter(href, 'port', '123', href);
}
"#,
    );
}

#[test]
fn url_components_pathname_setter_respects_opaque_paths_and_scheme_delimiters() {
    assert_url_components(
        r#"
for (const href of ['mailto:user@example.test', 'data:payload', 'custom:payload']) {
  checkSetter(href, 'pathname', '/replacement', href);
}
checkSetter('https://example.test/old?q=value#frag', 'pathname', '\\one\\..\\two',
  'https://example.test/two?q=value#frag');
checkSetter('custom://example.test/old?q=value#frag', 'pathname', '\\one\\two',
  'custom://example.test/\\one\\two?q=value#frag');
checkSetter('custom://example.test/old?q=value#frag', 'pathname', '',
  'custom://example.test?q=value#frag');
checkSetter('custom:///old', 'pathname', '', 'custom://');
checkSetter('custom:///old', 'pathname', '\t\r\n', 'custom://');
checkSetter('https://example.test/old', 'pathname', '\t/next', 'https://example.test/next');
checkSetter('https://example.test/old', 'pathname', '\n\\next', 'https://example.test/next');
checkSetter('custom:/old', 'pathname', '', 'custom:/');
checkSetter('https://example.test/old', 'pathname', '', 'https://example.test/');
checkSetter('https://example.test/old?q=value#frag', 'pathname', '/?new#value',
  'https://example.test/%3Fnew%23value?q=value#frag');
"#,
    );
}

#[test]
fn url_components_setters_convert_input_before_reading_the_current_url() {
    assert_url_components(
        r#"
const cases = [
  ['protocol', 'https', 'https://new.test:8080/new?q=next#next'],
  ['host', 'host.test', 'http://host.test:8080/new?q=next#next'],
  ['hostname', 'host.test', 'http://host.test:8080/new?q=next#next'],
  ['port', '123', 'http://new.test:123/new?q=next#next'],
  ['pathname', '/replacement', 'http://new.test:8080/replacement?q=next#next'],
  ['username', 'user', 'http://user@new.test:8080/new?q=next#next'],
  ['password', 'pass', 'http://:pass@new.test:8080/new?q=next#next'],
  ['search', '?q=changed', 'http://new.test:8080/new?q=changed#next'],
  ['hash', '#changed', 'http://new.test:8080/new?q=next#changed'],
];
for (const [name, create] of factories) {
  for (const [property, value, expected] of cases) {
    const object = create('https://old.test/old?q=old#old');
    let conversions = 0;
    object[property] = {toString() {
      conversions++;
      object.href = 'http://new.test:8080/new?q=next#next';
      return value;
    }};
    assert(conversions === 1 && object.href === expected, `${name}: ${property} observes conversion side effects`);
    for (const href of ['data:payload', 'file:///path']) {
      const target = create(href);
      const error = new Error('conversion');
      let caught;
      try { target[property] = {toString() { throw error; }}; } catch (value) { caught = value; }
      assert(caught === error && target.href === href, `${name}: ${property} still converts on a non-settable URL`);
    }
    if (object.getAttribute) {
      object.href = 'https://[invalid';
      object[property] = {toString() {
        object.href = 'http://new.test:8080/new?q=next#next';
        return value;
      }};
      assert(object.href === expected, `${name}: conversion can repair an invalid href before parsing`);
    }
  }
}
"#,
    );
}

#[test]
fn hyperlink_component_setters_update_href_after_parser_rejection_but_not_for_opaque_paths() {
    assert_url_components(
        r#"
for (const tag of ['a', 'area']) {
  const object = document.body.appendChild(document.createElement(tag));
  const observer = new MutationObserver(() => {});
  for (const [property, value] of [
    ['protocol', '1invalid'], ['host', 'bad host'], ['hostname', 'bad host'], ['port', 'invalid']
  ]) {
    object.setAttribute('href', '../path');
    observer.observe(object, {attributes: true, attributeOldValue: true});
    object[property] = value;
    assert(object.getAttribute('href') === 'https://url-components.test/path', `${tag}: ${property} serializes the resolved href`);
    const records = observer.takeRecords();
    assert(records.length === 1 && records[0].attributeName === 'href' && records[0].oldValue === '../path',
      `${tag}: ${property} uses the normal href mutation path`);
    observer.disconnect();
  }
  for (const property of ['host', 'hostname', 'port', 'pathname']) {
    object.setAttribute('href', 'data:payload');
    observer.observe(object, {attributes: true});
    object[property] = 'ignored';
    assert(object.getAttribute('href') === 'data:payload' && observer.takeRecords().length === 0,
      `${tag}: ${property} leaves opaque URLs and attributes untouched`);
    observer.disconnect();
  }
}
"#,
    );
}

#[test]
fn hyperlink_component_setters_reparse_after_base_changes_and_adoption() {
    assert_url_components(
        r#"
for (const tag of ['a', 'area']) {
  const object = document.createElement(tag);
  object.href = 'relative?q=keep#frag';
  const base = document.createElement('base');
  base.href = 'https://base-changed.test/dir/';
  object.pathname = {toString() {
    document.head.appendChild(base);
    return '/replacement';
  }};
  assert(object.href === 'https://base-changed.test/replacement?q=keep#frag', `${tag}: conversion changes the document base`);
  base.remove();

  const owner = document.implementation.createHTMLDocument('adopt');
  const adoptedBase = owner.createElement('base');
  adoptedBase.href = 'https://adopted.test/dir/';
  owner.head.appendChild(adoptedBase);
  object.href = 'relative?q=keep#frag';
  object.pathname = {toString() {
    owner.body.appendChild(object);
    return '/replacement';
  }};
  assert(object.ownerDocument === owner && object.href === 'https://adopted.test/replacement?q=keep#frag',
    `${tag}: conversion adopts the node before URL parsing`);
}
"#,
    );
}

#[test]
fn url_hierarchical_path_mutations_keep_hosts_and_logical_paths_separate() {
    assert_url_components(
        r#"
for (const input of ['custom:/old?q#f', 'custom:/.//old?q#f']) {
  for (const value of ['//p', '/.//p', '/..//p']) {
    checkSetter(input, 'pathname', value, 'custom:/.//p?q#f');
  }
  checkSetter(input, 'pathname', 'p', 'custom:/p?q#f');
  checkSetter(input, 'pathname', '', 'custom:/?q#f');
}
checkSetter('custom:///old?q#f', 'pathname', '', 'custom://?q#f');
checkSetter('custom://host/old?q#f', 'pathname', '//p', 'custom://host//p?q#f');
checkSetter('custom:/.//p?q#f', 'host', 'h:77', 'custom://h:77//p?q#f');
for (const [name, create] of factories) {
  for (const hostname of ['h', '', '[::1]']) {
    const object = create('custom:/.//p?q#f');
    assert(object.pathname === '//p', name + ': prefix is not part of pathname');
    object.hostname = hostname;
    assert(object.href === 'custom://' + hostname + '//p?q#f', name + ': add authority');
    assert(object.pathname === '//p', name + ': authority change preserves path');
    assert(object.search === '?q' && object.hash === '#f', name + ': suffix offsets');
    assert(new URL(object.href).pathname === '//p', name + ': serialized URL round-trips');
    object.pathname = '';
    assert(object.href === 'custom://' + hostname + '?q#f', name + ': empty authority path');
  }
}
"#,
    );
}

#[test]
fn url_relative_resolution_distinguishes_special_slashes_and_non_special_backslashes() {
    assert_url_components(
        r#"
for (const [input, base, expected] of [
  ['///next.test/a/../p?q#f', 'https://user:pass@old.test:123/a/b', 'https://next.test/p?q#f'],
  ['///\\//\\//next.test/p', 'http://old.test/a/b', 'http://next.test/p'],
  ['\\p', 'custom://host/a/b', 'custom://host/a/\\p'],
  ['\\/p', 'custom://host/a/b', 'custom://host/a/\\/p'],
  ['\\\\p', 'custom://host/a/b', 'custom://host/a/\\\\p'],
  ['\\p', 'custom:/.//a/b', 'custom:/.//a/\\p'],
  ['///next', 'custom://host/a/b', 'custom:///next'],
  ['////next', 'custom:/a/b', 'custom:////next'],
]) {
  for (const constructor of [URL, frame.contentWindow.URL]) {
    assert(new constructor(input, base).href === expected, 'relative constructor: ' + input);
    assert(constructor.parse(input, base).href === expected, 'relative URL.parse: ' + input);
    assert(constructor.canParse(input, base), 'relative URL.canParse: ' + input);
  }
  for (const owner of [document, frame.contentDocument,
    document.implementation.createHTMLDocument('relative path')]) {
    const baseElement = owner.head.appendChild(owner.createElement('base'));
    baseElement.href = base;
    for (const tag of ['a', 'area']) {
      const link = owner.body.appendChild(owner.createElement(tag));
      link.href = input;
      assert(link.href === expected, tag + ': relative document base: ' + input);
      link.remove();
    }
    baseElement.remove();
  }
}
for (const constructor of [URL, frame.contentWindow.URL]) {
  assert(constructor.parse('///?q', 'https://host/') === null, 'empty special host fails');
  assert(!constructor.canParse('//\\host', 'custom://base/'), 'backslash in opaque host fails');
}
"#,
    );
}

#[test]
fn file_url_components_preserve_hosts_drive_letters_and_empty_path_segments() {
    assert_url_components(
        r#"
for (const [input, expected] of [
  ['file://server///?q#f', 'file://server///?q#f'],
  ['file://server/C|/path', 'file://server/C:/path'],
  ['file:///w|/m', 'file:///w:/m'],
  ['file://localhost////foo', 'file://////foo'],
]) {
  for (const [name, create] of factories) {
    const object = create(input);
    assert(object.href === expected, `${name}: parsing ${input}`);
    assert(object.origin === 'null', `${name}: file origin stays opaque`);
    object.href = object.href;
    assert(object.href === expected, `${name}: file serialization round-trips`);
  }
}
checkSetter('file://monkey/', 'pathname', '\\\\', 'file://monkey//');
checkSetter('file:///unicorn', 'pathname', '//\\/', 'file://////');
checkSetter('file:///unicorn', 'pathname', '//monkey/..//', 'file://///');
checkSetter('file://host/old?q#f', 'pathname', 'C|/new', 'file://host/C:/new?q#f');
"#,
    );
}

#[test]
fn file_url_relative_resolution_matches_document_base_and_static_url_parsing() {
    assert_url_components(
        r#"
for (const [input, base, expected] of [
  ['/', 'file://host/C:/a/b', 'file://host/C:/'],
  ['C|/new', 'file://host/D:/a/b', 'file://host/C:/new'],
  ['/..//share//file', 'file://host/path', 'file://host//share//file'],
  ['..', 'file://host/a/C:/', 'file://host/a/'],
]) {
  for (const constructor of [URL, frame.contentWindow.URL]) {
    assert(new constructor(input, base).href === expected, 'relative URL constructor');
    assert(constructor.parse(input, base).href === expected, 'relative URL.parse');
    assert(constructor.canParse(input, base), 'relative URL.canParse');
  }
  for (const owner of [document, frame.contentDocument,
    document.implementation.createHTMLDocument('file base')]) {
    const baseElement = owner.head.appendChild(owner.createElement('base'));
    baseElement.href = base;
    for (const tag of ['a', 'area']) {
      const link = owner.body.appendChild(owner.createElement(tag));
      link.href = input;
      assert(link.href === expected, `${tag}: file document base`);
      link.remove();
    }
    baseElement.remove();
  }
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
