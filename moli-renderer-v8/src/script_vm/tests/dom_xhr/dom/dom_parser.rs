use super::*;

#[test]
fn dom_parser_non_object_new_target_prototype_uses_new_target_realm_default() {
    let mut vm = new_storage_test_vm("https://dom-parser-new-target.test/top.html");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const child = frame.contentWindow;

  const TopBad = new Function();
  TopBad.prototype = 7;
  const ChildBad = new child.Function();
  ChildBad.prototype = 7;

  const BoundChild = Function.prototype.bind.call(new child.Function());
  BoundChild.prototype = 7;
  const BoundTop = child.Function.prototype.bind.call(new Function());
  BoundTop.prototype = 7;

  const ProxyChild = new Proxy(new child.Function(), {});
  ProxyChild.prototype = 7;
  const ProxyTop = new child.Proxy(new Function(), {});
  ProxyTop.prototype = 7;

  let getterCount = 0;
  const GetterProxyChild = new Proxy(new child.Function(), {
    get(target, property, receiver) {
      if (property === 'prototype') {
        getterCount += 1;
        return 7;
      }
      return Reflect.get(target, property, receiver);
    }
  });

  const check = (parser, expectedPrototype) =>
    Object.getPrototypeOf(parser) === expectedPrototype;

  return JSON.stringify({
    directTop: check(
      Reflect.construct(child.DOMParser, [], TopBad),
      DOMParser.prototype
    ),
    directChild: check(
      Reflect.construct(DOMParser, [], ChildBad),
      child.DOMParser.prototype
    ),
    boundChild: check(
      Reflect.construct(DOMParser, [], BoundChild),
      child.DOMParser.prototype
    ),
    boundTop: check(
      Reflect.construct(child.DOMParser, [], BoundTop),
      DOMParser.prototype
    ),
    proxyChild: check(
      Reflect.construct(DOMParser, [], ProxyChild),
      child.DOMParser.prototype
    ),
    proxyTop: check(
      Reflect.construct(child.DOMParser, [], ProxyTop),
      DOMParser.prototype
    ),
    getterProxyChild: check(
      Reflect.construct(DOMParser, [], GetterProxyChild),
      child.DOMParser.prototype
    ),
    getterCount
  });
})()
"#,
        )
        .expect("DOMParser NewTarget realm prototype fallback probe should evaluate");

    assert_eq!(
        result,
        r#"{"directTop":true,"directChild":true,"boundChild":true,"boundTop":true,"proxyChild":true,"proxyTop":true,"getterProxyChild":true,"getterCount":1}"#
    );
}

#[test]
fn dom_parser_parse_from_string_parses_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://dom-parser-webidl-args.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function probe(callback) {
    try {
      const value = callback();
      return value && value.documentElement
        ? value.documentElement.tagName
        : String(value);
    } catch (error) {
      return 'throw:' + error.name;
    }
  }
  const parser = new DOMParser();
  const sourceObject = {
    toString() {
      return '<html><body><section id="from-object"></section></body></html>';
    }
  };
  return JSON.stringify({
    html: parser.parseFromString('<html><body><p></p></body></html>', 'text/html').documentElement.tagName,
    xml: parser.parseFromString('<root></root>', 'application/xml').documentElement.tagName,
    objectSource: parser.parseFromString(sourceObject, 'text/html').getElementById('from-object').tagName,
    nullSource: parser.parseFromString(null, 'text/html').body.textContent,
    missingSource: probe(() => parser.parseFromString()),
    missingType: probe(() => parser.parseFromString('<root></root>')),
    invalidType: probe(() => parser.parseFromString('<root></root>', 'TEXT/html')),
    symbolSource: probe(() => parser.parseFromString(Symbol(), 'text/html')),
    symbolType: probe(() => parser.parseFromString('<root></root>', Symbol())),
    throwingSource: probe(() => parser.parseFromString({
      toString() { throw new Error('source failed'); }
    }, 'text/html'))
  });
})()
"#,
        )
        .expect("DOMParser.parseFromString WebIDL argument probe should run");

    assert_eq!(
        result,
        r#"{"html":"HTML","xml":"root","objectSource":"SECTION","nullSource":"null","missingSource":"throw:TypeError","missingType":"throw:TypeError","invalidType":"throw:TypeError","symbolSource":"throw:TypeError","symbolType":"throw:TypeError","throwingSource":"throw:Error"}"#
    );
}

#[test]
fn dom_parser_prototype_parse_from_string_is_declared_operation() {
    let mut vm = new_storage_test_vm("https://dom-parser-prototype-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parser = new DOMParser();
  const descriptor = Object.getOwnPropertyDescriptor(DOMParser.prototype, 'parseFromString');
  const html = parser.parseFromString(
    '<html><body><main id="root"></main></body></html>',
    'text/html'
  );
  const xml = DOMParser.prototype.parseFromString.call(
    parser,
    '<root><child /></root>',
    'application/xml'
  );
  return JSON.stringify({
    descriptor: [
      typeof descriptor?.value,
      descriptor?.value?.name,
      descriptor?.value?.length,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable
    ].join(':'),
    own: Object.hasOwn(parser, 'parseFromString'),
    enumerable: Object.keys(DOMParser.prototype).join(','),
    behavior: [
      html.getElementById('root').tagName,
      xml.documentElement.tagName,
      parser instanceof DOMParser
    ].join(':')
  });
})()
"#,
        )
        .expect("DOMParser prototype method descriptor probe should run");

    assert_eq!(
        result,
        r#"{"descriptor":"function:parseFromString:2:true:true:true","own":false,"enumerable":"parseFromString","behavior":"MAIN:root:true"}"#
    );
}

#[test]
fn dom_parser_xml_text_content_excludes_processing_instruction_descendants() {
    let mut vm = new_storage_test_vm("https://dom-parser-xml-text-content.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    "<root><?start name=p?>Unchanged<!--ignored--><![CDATA[ kept ]]></root>",
    "application/xml"
  );
  const root = doc.documentElement;
  return JSON.stringify({
    rootText: root.textContent,
    piText: root.firstChild.textContent,
    commentText: root.childNodes[2].textContent
  });
})()
"#,
        )
        .expect("DOMParser XML textContent processing instruction probe should run");

    assert_eq!(
        result,
        r#"{"rootText":"Unchanged kept ","piText":"name=p","commentText":"ignored"}"#
    );
}

#[test]
fn dom_parser_inner_html_supports_html_sanitizer_roundtrip_path() {
    let mut vm = new_storage_test_vm("https://dom-parser-sanitizer-path.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function sanitizeLikePage(html) {
    const doc = new DOMParser().parseFromString(html, 'text/html');
    return doc.body.innerHTML;
  }
  return [
    sanitizeLikePage('Plain title'),
    sanitizeLikePage('<p>A</p><p><strong>B</strong></p>'),
    sanitizeLikePage('<img src="x.png" img_width="10"><p>caption</p>'),
  ].join('|');
})()
"#,
        )
        .expect("dom parser body.innerHTML should support sanitizer roundtrips");

    assert_eq!(
        result,
        "Plain title|<p>A</p><p><strong>B</strong></p>|<img src=\"x.png\" img_width=\"10\"><p>caption</p>"
    );
}

#[test]
fn dom_parser_body_and_elements_expose_class_list_for_sanitizer_walks() {
    let mut vm = new_storage_test_vm("https://dom-parser-class-list.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body class="article-body"><pre class="highlight language-js">const x = 1;</pre></body></html>',
    'text/html'
  );
  const body = doc.body;
  const pre = body.querySelector('pre');
  return JSON.stringify({
    bodyTag: Object.prototype.toString.call(body.classList),
    bodyContains: body.classList.contains('article-body'),
    preContainsHighlight: pre.classList.contains('highlight'),
    preContainsLanguage: pre.classList.contains('language-js'),
    stable: body.classList === body.classList,
    preStable: pre.classList === pre.classList,
    item0: pre.classList.item(0),
    item1: pre.classList.item(1),
    length: pre.classList.length
  });
})()
"#,
        )
        .expect("DOMParser detached body.classList should stay available for sanitizer walks");

    assert_eq!(
        result,
        r#"{"bodyTag":"[object DOMTokenList]","bodyContains":true,"preContainsHighlight":true,"preContainsLanguage":true,"stable":true,"preStable":true,"item0":"highlight","item1":"language-js","length":2}"#
    );
}

#[test]
fn dom_parser_elements_expose_dataset_has_attribute_and_element_traversal() {
    let mut vm = new_storage_test_vm("https://dom-parser-dataset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body data-root="article">' +
      'text<!--gap-->' +
      '<pre id="code" data-eeimg="1" data-tex="inline"></pre>' +
      '<figure id="target"></figure>' +
      '<span id="tail"></span>' +
    '</body></html>',
    'text/html'
  );
  const body = doc.body;
  const pre = doc.getElementById('code');
  const figure = doc.getElementById('target');
  const dataset = pre.dataset;
  dataset.imageSrc = 'https://img.test/a.png';
  dataset.imageStatus = 'ready';
  return JSON.stringify({
    bodyHasRoot: body.hasAttribute('data-root'),
    bodyMissing: body.hasAttribute('data-missing'),
    datasetTag: Object.prototype.toString.call(dataset),
    stable: dataset === pre.dataset,
    eeimg: dataset.eeimg,
    tex: dataset.tex,
    imageSrc: dataset.imageSrc,
    attrImageSrc: pre.getAttribute('data-image-src'),
    keys: Object.keys(dataset).sort(),
    firstElementChild: body.firstElementChild && body.firstElementChild.id,
    lastElementChild: body.lastElementChild && body.lastElementChild.id,
    childElementCount: body.childElementCount,
    preNext: pre.nextElementSibling && pre.nextElementSibling.id,
    figurePrev: figure.previousElementSibling && figure.previousElementSibling.id,
    tailPrev: doc.getElementById('tail').previousElementSibling && doc.getElementById('tail').previousElementSibling.id
  });
})()
"#,
        )
        .expect("DOMParser detached elements should expose dataset and element traversal");

    assert_eq!(
        result,
        r#"{"bodyHasRoot":true,"bodyMissing":false,"datasetTag":"[object DOMStringMap]","stable":true,"eeimg":"1","tex":"inline","imageSrc":"https://img.test/a.png","attrImageSrc":"https://img.test/a.png","keys":["eeimg","imageSrc","imageStatus","tex"],"firstElementChild":"code","lastElementChild":"tail","childElementCount":3,"preNext":"target","figurePrev":"code","tailPrev":"target"}"#
    );
}

#[test]
fn child_shadow_root_legacy_wpt_attributes_and_methods() {
    let mut vm = new_storage_test_vm("https://child-shadow-root-legacy-wpt.test/");

    let result = eval_with_layout_publications(
        &mut vm,
        r#"
(function* () {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const doc = frame.contentWindow.document;
  const host = doc.createElement('div');
  doc.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const input = doc.createElement('input');
  shadow.appendChild(input);
  input.focus();
  const activeTag = shadow.activeElement && shadow.activeElement.tagName;

  const span = doc.createElement('span');
  span.innerHTML = 'Some text';
  shadow.appendChild(span);
  const innerBefore = shadow.innerHTML.toLowerCase();
  shadow.innerHTML = '<input type="text" id="inputId"><div id="divId">new text</div>';
  const innerAfter = shadow.innerHTML.toLowerCase();

  const styleHost = doc.createElement('div');
  doc.body.appendChild(styleHost);
  const styleRoot = styleHost.attachShadow({ mode: 'open' });
  const emptyStyleLength = styleRoot.styleSheets.length;
  styleRoot.appendChild(doc.createElement('style'));
  const styleLength = styleRoot.styleSheets.length;

  const selectionHost = doc.createElement('div');
  doc.body.appendChild(selectionHost);
  const selectionRoot = selectionHost.attachShadow({ mode: 'open' });
  const selected = doc.createElement('span');
  selected.innerHTML = 'Some text';
  selectionRoot.appendChild(selected);
  const range = doc.createRange();
  range.setStart(selected.firstChild, 0);
  range.setEnd(selected.firstChild, 3);
  const selection = selectionRoot.getSelection();
  selection.removeAllRanges();
  selection.addRange(range);
yield; // Publish this scene before reading its geometry.
  const selectedText = selectionRoot.getSelection().toString();

  let cloneError = '';
  try {
    selectionRoot.cloneNode();
  } catch (error) {
    cloneError = error.name + ':' + error.code;
  }

  return [
    activeTag,
    innerBefore,
    innerAfter,
    shadow.querySelector('#inputId') && shadow.querySelector('#inputId').id,
    emptyStyleLength,
    styleLength,
    selectedText,
    cloneError
  ].join('|');
})()
"#,
    )
    .expect("child ShadowRoot legacy WPT surface should evaluate");

    assert_eq!(
        result,
        "INPUT|<input><span>some text</span>|<input type=\"text\" id=\"inputid\"><div id=\"divid\">new text</div>|inputId|0|1|Som|NotSupportedError:9"
    );
}
