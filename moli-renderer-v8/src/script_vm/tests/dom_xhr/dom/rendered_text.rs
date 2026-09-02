use super::*;

#[test]
fn detached_dom_parser_tree_root_stops_on_tampered_parent_cycle() {
    let mut vm = new_storage_test_vm("https://detached-dom-parent-cycle.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><section id="a"><span id="b"></span></section></body></html>',
    'text/html'
  );
  const a = doc.getElementById('a');
  const b = doc.getElementById('b');
  Object.defineProperty(a, 'parentNode', {
    configurable: true,
    get() {
      return a;
    }
  });
  const found = a.querySelector('#b');
  return `${found === b}:${found && found.id}`;
})()
"#,
        )
        .expect("DOMParser canonical node lookup should not spin on parent cycles");

    assert_eq!(result, "true:b");
}

#[test]
fn dom_parser_html_elements_expose_inner_html() {
    let mut vm = new_storage_test_vm("https://dom-parser-inner-html.test/");

    let result = vm
            .eval(
                r#"
(() => {
  const doc = new DOMParser().parseFromString('<h1>Title</h1><p><strong>Body</strong></p>', 'text/html');
  const h1 = doc.querySelector('h1');
  return [
    Object.prototype.toString.call(doc),
    typeof doc.body.innerHTML,
    doc.body.innerHTML,
    h1.innerHTML,
    h1.textContent,
  ].join('|');
})()
"#,
            )
            .expect("dom parser html innerHTML should be readable");

    assert_eq!(
        result,
        "[object HTMLDocument]|string|<h1>Title</h1><p><strong>Body</strong></p>|Title|Title"
    );
}

#[test]
fn dom_parser_inner_text_is_html_only() {
    let mut vm = new_storage_test_vm("https://dom-parser-inner-text-scope.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parser = new DOMParser();
  const html = parser.parseFromString('<div id="html">\n  HTML  <span>Text</span>\n</div>', 'text/html').getElementById('html');
  const xml = parser.parseFromString('<root><child>XML</child></root>', 'text/xml').documentElement;
  const svg = parser.parseFromString('<svg xmlns="http://www.w3.org/2000/svg"><text>SVG</text></svg>', 'image/svg+xml').documentElement;
  return [
    'innerText' in html,
    html.innerText,
    'innerText' in xml,
    typeof xml.innerText,
    'innerText' in svg,
    typeof svg.innerText
  ].join('|');
})()
"#,
        )
        .expect("DOMParser detached innerText scope should evaluate");

    assert_eq!(
        result,
        "true|\n  HTML  Text\n|false|undefined|false|undefined"
    );
}

#[test]
fn selectedcontent_inner_text_falls_back_when_its_select_child_box_is_suppressed() {
    let mut vm = new_parsed_test_vm(
        "https://selectedcontent-inner-text.test/",
        "<!doctype html><html><body><div id=before>before</div></body></html>",
    );

    let result = vm
        .eval(
            r#"
void document.body.innerText;
const select = document.createElement('select');
select.innerHTML = '<button><selectedcontent id="selectedcontent">default</selectedcontent></button><option>one</option>';
select.style.appearance = 'base-select';
document.body.append(select);
const selectedcontent = document.getElementById('selectedcontent');
[
  selectedcontent.textContent,
  selectedcontent.innerText,
  selectedcontent.getClientRects().length
].join('|')
"#,
        )
        .expect("selectedcontent innerText should evaluate");

    assert_eq!(result, "one|one|0");
}

#[test]
fn live_inner_text_applies_inline_text_transform() {
    let mut vm = new_rendered_test_vm(
        "https://inner-text-transform.test/",
        r#"<!doctype html><html><body>
          <a id="upper" style="text-transform: uppercase">link<br>text</a>
          <a id="lower" style="text-transform: lowercase">LINK TEXT</a>
          <a id="cap" style="text-transform: capitalize">link text</a>
          <div id="inherit" style="text-transform: uppercase">outer <span>inner</span></div>
          <div id="reset" style="text-transform: uppercase">outer <span style="text-transform: none">inner</span></div>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
[
  document.getElementById('upper').innerText,
  document.getElementById('lower').innerText,
  document.getElementById('cap').innerText,
  document.getElementById('inherit').innerText,
  document.getElementById('reset').innerText
].join('|')
"#,
        )
        .expect("innerText text-transform cases should evaluate");

    assert_eq!(
        result,
        "LINK\nTEXT|link text|Link Text|OUTER INNER|OUTER inner"
    );
}

#[test]
fn live_inner_text_preserves_breaks_and_private_use_text_during_normalization() {
    let mut vm = new_rendered_test_vm(
        "https://inner-text-single-pass.test/",
        r#"<!doctype html><html><body>
          <div id="target" style="text-transform: uppercase">
            alpha <span>betaß</span><br>
            <br><span style="text-transform: none">  end </span>
          </div>
        </body></html>"#,
    );

    let result = vm
        .eval("document.getElementById('target').innerText")
        .expect("single-pass innerText should evaluate");

    assert_eq!(result, "ALPHA BETASS\n\n\u{E000} end");
}

#[test]
fn inner_text_matches_chromium_structural_and_white_space_rules() {
    let mut vm = new_rendered_test_vm(
        "https://inner-text-structure.test/",
        "<!doctype html><html><body></body></html>",
    );

    vm
        .eval(
            r#"
(() => {
  const targets = [];
  const read = (html) => {
    const host = document.createElement('div');
    host.innerHTML = html;
    document.body.append(host);
    targets.push(host.querySelector('#x'));
    return targets.length - 1;
  };
  const readReplacedWithChild = (tag) => {
    const element = document.createElement(tag);
    element.append('abc');
    document.body.append(element);
    targets.push(element);
    return targets.length - 1;
  };
  const cases = {
    inline: read('<div id="x">a<span>b</span>c</div>'),
    nestedBlock: read('<div id="x">a<div>b</div>c</div>'),
    cssBlock: read('<div id="x"><span style="display:block">a</span><span style="display:block">b</span></div>'),
    paragraph: read('<div id="x">a<p>b</p>c</div>'),
    adjacentParagraphs: read('<div id="x"><p>a</p><p>b</p></div>'),
    emptyBlocks: read('<div id="x">a<div></div><div>b</div><div></div>c</div>'),
    inlineBlock: read('<div id="x">a<span style="display:inline-block">b</span>c</div>'),
    emptyInlineBlock: read('<div id="x">abc <span style="display:inline-block"></span> def</div>'),
    spacedInlineBlock: read('<div id="x">abc <span style="display:inline-block"> def </span> ghi</div>'),
    tightInlineBlock: read('<div id="x">123<span style="display:inline-block"> abc </span>def</div>'),
    imageLeadingSpace: read('<div id="x"><img> abc</div>'),
    imageTrailingSpace: read('<div id="x">abc <img></div>'),
    imageChild: readReplacedWithChild('img'),
    inputChild: readReplacedWithChild('input'),
    atomicBlockPair: read('<div id="x"><span style="display:inline-block"><div>a</div><div>b<br></div></span> <span style="display:inline-block"><div> <div>c</div><div>d</div> </div></span></div>'),
    atomicBlockPairNoBr: read('<div id="x"><span style="display:inline-block"><div>a</div><div>b</div></span> <span style="display:inline-block"><div> <div>c</div><div>d</div> </div></span></div>'),
    blockOfAtomicPairs: read('<div id="x"><div><span style="display:inline-block"><div>a</div><div>b<br></div></span> <span style="display:inline-block"><div> <div>c</div><div>d</div> </div></span></div>\n<div><span style="display:inline-block"><div>e</div></span></div></div>'),
    blockWhitespace: read('<div id="x"><div>a</div> <div>b</div></div>'),
    atomicWhitespace: read('<div id="x"><span style="display:inline-block">a</span> <span style="display:inline-block">b</span></div>'),
    flex: read('<div id="x">a<span style="display:flex">b</span>c</div>'),
    flexItems: read('<div id="x" style="display:flex"><span>a</span><span>b</span></div>'),
    gridItems: read('<div id="x" style="display:grid"><span>a</span><span>b</span></div>'),
    inlineFlexItems: read('<div id="x">x<span style="display:inline-flex"><span>a</span><span>b</span></span>y</div>'),
    inlineGridItems: read('<div id="x">x<span style="display:inline-grid"><span>a</span><span>b</span></span>y</div>'),
    normal: read('<div id="x" style="white-space:normal">  a \t b\n c  </div>'),
    pre: read('<div id="x" style="white-space:pre">  a \t b\n c  </div>'),
    preWrap: read('<div id="x" style="white-space:pre-wrap">  a \t b\n c  </div>'),
    preLine: read('<div id="x" style="white-space:pre-line">  a \t b\n c  </div>'),
    breakSpaces: read('<div id="x" style="white-space:break-spaces">  a \t b\n c  </div>'),
    inheritedPre: read('<div id="x" style="white-space:pre"> a <span> b\n c </span> d </div>'),
    overriddenNormal: read('<div id="x" style="white-space:pre"> a <span style="white-space:normal">  b\n c  </span> d </div>'),
    preElement: read('<pre id="x">  a \t b\n c  </pre>'),
    hr: read('<div id="x">a<hr><hr>b</div>'),
    rootBr: read('<br id="x">'),
    rubyRp: read('<div id="x"><ruby>abc<rp>(</rp><rt>def</rt><rp>)</rp></ruby></div>'),
    loneRp: read('<div id="x"><rp>abc</rp></div>'),
    renderedRp: read('<div id="x"><rp style="display:block">abc</rp>def</div>'),
    renderedScript: read('<div id="x">a<script style="display:block">b</script>c</div>'),
    renderedStyle: read('<div id="x">a<style style="display:block">b</style>c</div>'),
    textarea: read('<div id="x">a<textarea>b</textarea>c</div>'),
    canvas: read('<div id="x">a<canvas>b</canvas>c</div>'),
    svgStop: read('<div id="x"><svg><stop>abc</stop></svg></div>')
  };
globalThis.__readFixture = () => {
  return JSON.stringify(Object.fromEntries(
    Object.entries(cases).map(([name, index]) => [name, targets[index].innerText])
  ));
};
})()
"#,
        )
        .expect("Chromium-shaped structural innerText cases should evaluate");
    vm.publish_layout_for_test()
        .expect("publish prepared fixture");
    let result = vm
        .eval("__readFixture()")
        .expect("Chromium-shaped structural innerText cases should evaluate");

    assert_eq!(
        result,
        r#"{"inline":"abc","nestedBlock":"a\nb\nc","cssBlock":"a\nb","paragraph":"a\n\nb\n\nc","adjacentParagraphs":"a\n\nb","emptyBlocks":"a\nb\nc","inlineBlock":"abc","emptyInlineBlock":"abc  def","spacedInlineBlock":"abc def ghi","tightInlineBlock":"123abcdef","imageLeadingSpace":" abc","imageTrailingSpace":"abc ","imageChild":"","inputChild":"","atomicBlockPair":"a\nb\n\n \nc\nd","atomicBlockPairNoBr":"a\nb\n \nc\nd","blockOfAtomicPairs":"a\nb\n\n \nc\nd\ne","blockWhitespace":"a\nb","atomicWhitespace":"a b","flex":"a\nb\nc","flexItems":"a\nb","gridItems":"a\nb","inlineFlexItems":"x\na\nb\ny","inlineGridItems":"x\na\nb\ny","normal":"a b c","pre":"  a \t b\n c  ","preWrap":"  a \t b\n c  ","preLine":"a b\nc","breakSpaces":"  a \t b\n c  ","inheritedPre":" a  b\n c  d ","overriddenNormal":" a  b c  d ","preElement":"  a \t b\n c  ","hr":"a\nb","rootBr":"","rubyRp":"abcdef","loneRp":"","renderedRp":"abc\ndef","renderedScript":"a\nb\nc","renderedStyle":"a\nb\nc","textarea":"ac","canvas":"ac","svgStop":""}"#
    );
}

#[test]
fn inner_text_matches_chromium_table_and_select_rules() {
    let mut vm = new_rendered_test_vm(
        "https://inner-text-table-select.test/",
        "<!doctype html><html><body></body></html>",
    );

    vm
        .eval(
            r#"
(() => {
  const targets = [];
  const read = (html) => {
    const host = document.createElement('div');
    host.innerHTML = html;
    document.body.append(host);
    targets.push(host.querySelector('#x'));
    return targets.length - 1;
  };
  const cases = {
    table: read('<table id="x"><tbody><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></tbody></table>'),
    hiddenCell: read('<table id="x"><tbody><tr><td>a</td><td style="display:none">x</td><td>b</td></tr></tbody></table>'),
    preservedWhitespace: read('<div id="x"><table style="white-space:pre">  <tbody>  <tr>  <td>a</td>  </tr>  </tbody>  </table></div>'),
    visibilityHiddenFirst: read('<table id="x"><tbody><tr><td style="visibility:hidden">x</td><td>b</td></tr></tbody></table>'),
    visibilityHiddenMiddle: read('<table id="x"><tbody><tr><td>a</td><td style="visibility:hidden">x</td><td>b</td></tr></tbody></table>'),
    visibilityHiddenLast: read('<table id="x"><tbody><tr><td>a</td><td style="visibility:hidden">x</td></tr></tbody></table>'),
    hiddenFirstRow: read('<table id="x"><tbody><tr style="visibility:hidden"><td>x</td></tr><tr><td>b</td></tr></tbody></table>'),
    hiddenMiddleRow: read('<table id="x"><tbody><tr><td>a</td></tr><tr style="visibility:hidden"><td>x</td></tr><tr><td>b</td></tr></tbody></table>'),
    hiddenLastRow: read('<table id="x"><tbody><tr><td>a</td></tr><tr style="visibility:hidden"><td>x</td></tr></tbody></table>'),
    cssTable: read('<div id="x"><div style="display:table"><span style="display:table-cell">a</span>\n<span style="display:table-cell">b</span></div></div>'),
    cssInlineTable: read('<div id="x"><div style="display:inline-table"><span style="display:table-cell">a</span>\n<span style="display:table-cell">b</span></div></div>'),
    rowRoot: read('<table><tbody><tr id="x"><td>a</td><td>b</td></tr><tr><td>c</td></tr></tbody></table>'),
    inlineTable: read('<div id="x">a<table style="display:inline-table"><tbody><tr><td>x</td><td>y</td></tr><tr><td>z</td></tr></tbody></table>b</div>'),
    select: read('<div id="x">a<select><option>one</option><option>two</option></select>b</div>'),
    option: read('<option id="x">  one <span> two </span> </option>'),
    optgroup: read('<div id="x">a<select><optgroup label="g"><option>one</option><option>two</option></optgroup></select>b</div>'),
    emptyOptgroup: read('<div id="x">a<select><optgroup label="g"></optgroup></select>b</div>'),
    outsideOptgroup: read('<div id="x">a<optgroup>ignored</optgroup>bc</div>'),
    outsideOption: read('<div id="x">a<option>one</option>bc</div>')
  };
globalThis.__readFixture = () => {
  return JSON.stringify(Object.fromEntries(
    Object.entries(cases).map(([name, index]) => [name, targets[index].innerText])
  ));
};
})()
"#,
        )
        .expect("Chromium-shaped table/select innerText cases should evaluate");
    vm.publish_layout_for_test()
        .expect("publish prepared fixture");
    let result = vm
        .eval("__readFixture()")
        .expect("Chromium-shaped table/select innerText cases should evaluate");

    assert_eq!(
        result,
        r#"{"table":"a\tb\nc\td","hiddenCell":"a\tb","preservedWhitespace":"a","visibilityHiddenFirst":"b","visibilityHiddenMiddle":"a\tb","visibilityHiddenLast":"a\t","hiddenFirstRow":"b","hiddenMiddleRow":"a\nb","hiddenLastRow":"a\n","cssTable":"a\tb","cssInlineTable":"a\tb","rowRoot":"a\tb","inlineTable":"ax\ty\nzb","select":"a\none\ntwo\nb","option":"one two","optgroup":"a\none\ntwo\nb","emptyOptgroup":"a\nb","outsideOptgroup":"a\nignored\nbc","outsideOption":"a\none\nbc"}"#
    );
}

#[test]
fn inner_text_projects_closed_details_rendered_subtree() {
    let mut vm = new_rendered_test_vm(
        "https://inner-text-details.test/",
        r#"<!doctype html><html><body>
          <details id="target"><summary><span id="summary-child">first</span></summary><summary id="second">second</summary><div id="hidden-child">details</div></details>
          <details id="no-summary"><div>details</div></details>
          <details id="nested" open><summary>outer</summary><details><summary>inner</summary><div>hidden</div></details><div>tail</div></details>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const hidden = document.getElementById('hidden-child');
  const values = {
    closed: target.innerText,
    summaryChild: document.getElementById('summary-child').innerText,
    secondSummary: document.getElementById('second').innerText,
    hiddenChild: hidden.innerText,
    hiddenCheckVisibility: hidden.checkVisibility(),
    noSummary: document.getElementById('no-summary').innerText,
    nested: document.getElementById('nested').innerText
  };
  target.open = true;
  values.open = target.innerText;
  values.openHiddenChild = hidden.innerText;
  target.open = false;
  values.reclosed = target.innerText;
  return JSON.stringify(values);
})()
"#,
        )
        .expect("closed details rendered-subtree cases should evaluate");

    assert_eq!(
        result,
        r#"{"closed":"first","summaryChild":"first","secondSummary":"","hiddenChild":"","hiddenCheckVisibility":false,"noSummary":"","nested":"outer\ninner\ntail","open":"first\nsecond\ndetails","openHiddenChild":"details","reclosed":"first"}"#
    );
}

#[test]
fn check_visibility_and_inner_text_use_computed_rendered_state() {
    let mut vm = new_rendered_test_vm(
        "https://rendered-state.test/",
        r#"<!doctype html><html><head><style>
          .hidden { display: none; }
          .upper { text-transform: uppercase; }
          #visibility-hidden { visibility: hidden; }
          #visibility-child { visibility: visible; }
          #transparent { opacity: 0; }
          #contents { display: contents; }
          #under-content-hidden { display: none; }
          #outer-display-none { display: none; }
        </style></head><body>
          <div id="card"><span class="upper">visible</span><span class="hidden">leak</span></div>
          <div id="hidden-root" class="hidden">hidden root</div>
          <div id="visibility-hidden">hidden visibility<span id="visibility-child">visible child</span></div>
          <div id="transparent">transparent</div>
          <div id="contents"><span>contents</span></div>
          <div id="content-hidden" style="content-visibility: hidden !important; content-visibility: visible">content hidden</div>
          <div id="outer-content-hidden" style="content-visibility: hidden"><span id="content-hidden-child">hidden child</span><span id="under-content-hidden">under content hidden</span></div>
          <div id="outer-display-none"><span id="under-display-none" style="content-visibility: hidden">under display none</span></div>
          <div id="shadow-host"><span>assigned</span><span slot="missing">unassigned leak</span></div>
          <div id="dynamic">dynamic</div>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const get = (id) => document.getElementById(id);
  const detached = new DOMParser().parseFromString(
    '<div id="x">\n  HTML  <span>Text</span>\n</div>',
    'text/html'
  ).getElementById('x');
  const detachedContentHidden = document.createElement('div');
  detachedContentHidden.setAttribute('style', 'content-visibility: hidden');
  detachedContentHidden.innerHTML = 'detached <span>content hidden</span>';
  const shadowHost = get('shadow-host');
  shadowHost.attachShadow({mode: 'open'}).innerHTML = 'shadow text<slot></slot>';
  const unassignedShadowChild = shadowHost.querySelector('[slot="missing"]');
  const dynamic = get('dynamic');
  const dynamicStates = [dynamic.checkVisibility()];
  dynamic.classList.add('hidden');
  dynamicStates.push(dynamic.checkVisibility());
  dynamic.classList.remove('hidden');
  dynamicStates.push(dynamic.checkVisibility());

  return JSON.stringify({
    shape: [typeof Element.prototype.checkVisibility, Element.prototype.checkVisibility.length],
    cardInnerText: get('card').innerText,
    hiddenRootInnerText: get('hidden-root').innerText,
    visibilityInnerText: get('visibility-hidden').innerText,
    detachedInnerText: detached.innerText,
    detachedContentHiddenInnerText: detachedContentHidden.innerText,
    contentHiddenInnerText: get('content-hidden').innerText,
    underContentHiddenInnerText: get('under-content-hidden').innerText,
    underDisplayNoneInnerText: get('under-display-none').innerText,
    shadowHostInnerText: shadowHost.innerText,
    checks: [
      get('card').checkVisibility(),
      get('hidden-root').checkVisibility(),
      get('visibility-hidden').checkVisibility(),
      get('visibility-hidden').checkVisibility({checkVisibilityCSS: true}),
      get('visibility-child').checkVisibility({visibilityProperty: true}),
      get('transparent').checkVisibility(),
      get('transparent').checkVisibility({checkOpacity: true}),
      get('contents').checkVisibility(),
      get('content-hidden').checkVisibility(),
      get('content-hidden-child').checkVisibility(),
      detachedContentHidden.checkVisibility(),
      unassignedShadowChild.checkVisibility()
    ],
    dynamicStates
  });
})()
"#,
        )
        .expect("computed rendered-state surfaces should evaluate");

    assert_eq!(
        result,
        r#"{"shape":["function",0],"cardInnerText":"VISIBLE","hiddenRootInnerText":"hidden root","visibilityInnerText":"visible child","detachedInnerText":"\n  HTML  Text\n","detachedContentHiddenInnerText":"detached content hidden","contentHiddenInnerText":"","underContentHiddenInnerText":"","underDisplayNoneInnerText":"under display none","shadowHostInnerText":"assigned","checks":[true,false,true,false,true,true,false,false,true,false,false,false],"dynamicStates":[true,false,true]}"#
    );
}

#[test]
fn content_visibility_only_locks_chromium_eligible_boxes() {
    let mut vm = new_rendered_test_vm(
        "https://content-visibility-applicability.test/",
        r#"<!doctype html><html><body>
          <span id="inline" style="content-visibility: hidden">inline visible</span>
          <span id="atomic" style="display: inline-block; content-visibility: hidden">atomic hidden</span>
          <div id="block" style="content-visibility: hidden">block hidden</div>
          <table id="table" style="content-visibility: hidden"><tbody><tr><td>table visible</td></tr></tbody></table>
          <table><tbody><tr id="row" style="content-visibility: hidden"><td>row visible</td></tr></tbody></table>
          <table><tbody><tr><td id="cell" style="content-visibility: hidden">cell hidden</td></tr></tbody></table>
          <table><caption id="caption" style="content-visibility: hidden">caption visible</caption><tbody><tr><td>caption body</td></tr></tbody></table>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const text = (id) => document.getElementById(id).innerText;
  return JSON.stringify({
    inline: text('inline'),
    atomic: text('atomic'),
    block: text('block'),
    table: text('table'),
    row: text('row'),
    cell: text('cell'),
    caption: text('caption')
  });
})()
"#,
        )
        .expect("content-visibility applicability cases should evaluate");

    assert_eq!(
        result,
        r#"{"inline":"inline visible","atomic":"","block":"","table":"table visible","row":"row visible","cell":"","caption":"caption visible"}"#
    );
}

#[test]
fn rendered_style_facts_refresh_after_synchronous_mutations() {
    let mut vm = new_rendered_test_vm(
        "https://rendered-style-mutation.test/",
        r#"<!doctype html><html><head><style>
          .hidden { display: none; }
          .upper { text-transform: uppercase; }
          .transparent { opacity: 0; }
          .invisible { visibility: hidden; }
        </style></head><body>
          <div id="target"><span>fresh value</span></div>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const values = [target.innerText, target.checkVisibility()];
  target.className = 'upper';
  values.push(target.innerText);
  target.className = 'transparent';
  values.push(target.checkVisibility({opacityProperty: true}));
  target.className = 'invisible';
  values.push(target.innerText, target.checkVisibility({visibilityProperty: true}));
  target.className = 'hidden';
  values.push(target.innerText, target.checkVisibility());
  target.className = '';
  target.hidden = true;
  values.push(target.innerText, target.checkVisibility());
  target.hidden = false;
  target.setAttribute('style', 'content-visibility: hidden');
  values.push(target.innerText, target.checkVisibility());
  target.setAttribute('style', 'text-transform: lowercase');
  values.push(target.innerText, target.checkVisibility());
  return JSON.stringify(values);
})()
"#,
        )
        .expect("rendered style reads should observe every synchronous mutation");

    assert_eq!(
        result,
        r#"["fresh value",true,"FRESH VALUE",false,"",false,"fresh value",false,"fresh value",false,"",true,"fresh value",true]"#
    );
}

#[test]
fn inner_text_reuses_retained_style_world_across_synchronous_reads() {
    let mut vm = new_parsed_test_vm(
        "https://inner-text-retained-style-world.test/",
        r#"<!doctype html><html><head><style>
          .upper { text-transform: uppercase; }
        </style></head><body><div id="target"></div></body></html>"#,
    );
    vm.eval(
        r#"
const target = document.getElementById('target');
for (let index = 0; index < 128; index++) {
  const child = document.createElement('span');
  child.textContent = 'a';
  target.appendChild(child);
}
"#,
    )
    .expect("innerText retained-style fixture should initialize");

    let update_materializations_before = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_before = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();
    let layout_before = vm.layout_pass_observability_for_test();
    publish_layout_for_test(&mut vm);
    let first = vm
        .eval(
            "(() => { const text = target.innerText; return [text.length, text[0], text[127]].join('|'); })()",
        )
        .expect("first innerText read should evaluate");
    let update_materializations_after_first = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_after_first = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();
    let layout_after_first = vm.layout_pass_observability_for_test();

    let repeated = vm
        .eval(
            "(() => { const text = target.innerText; return [text.length, text[0], text[127]].join('|'); })()",
        )
        .expect("repeated innerText read should evaluate");
    let update_materializations_after_repeated = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_after_repeated = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();
    let layout_after_repeated = vm.layout_pass_observability_for_test();

    let second = vm
        .eval(
            "target.className = 'upper'; (() => { const text = target.innerText; return [text.length, text[0], text[127]].join('|'); })()",
        )
        .expect("mutated innerText read should evaluate");
    let update_materializations_after_second = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_after_second = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();
    let layout_after_second = vm.layout_pass_observability_for_test();

    let stylesheet_mutation = vm
        .eval(
            "document.querySelector('style').textContent = '.upper { text-transform: lowercase; }'; (() => { const text = target.innerText; return [text.length, text[0], text[127]].join('|'); })()",
        )
        .expect("stylesheet-mutated innerText read should evaluate");
    let update_materializations_after_stylesheet_mutation = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_after_stylesheet_mutation = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();
    let layout_after_stylesheet_mutation = vm.layout_pass_observability_for_test();

    assert_eq!(first, "128|a|a");
    assert_eq!(repeated, "128|a|a");
    assert_eq!(second, "128|A|A");
    assert_eq!(stylesheet_mutation, "128|a|a");
    assert!(
        !layout_before.0
            && !layout_after_first.0
            && !layout_after_repeated.0
            && !layout_after_second.0
            && !layout_after_stylesheet_mutation.0
    );
    assert_eq!(layout_after_first.1, layout_before.1 + 1);
    assert_eq!(layout_after_repeated.1, layout_after_first.1);
    assert_eq!(layout_after_second.1, layout_after_first.1);
    assert_eq!(layout_after_stylesheet_mutation.1, layout_after_first.1);
    assert_eq!(
        update_materializations_after_first.saturating_sub(update_materializations_before),
        1,
        "one cold innerText traversal should materialize the initial style world"
    );
    assert_eq!(
        update_materializations_after_repeated.saturating_sub(update_materializations_after_first),
        0,
        "an unchanged generation should reuse its retained style world across getters"
    );
    assert_eq!(
        update_materializations_after_second.saturating_sub(update_materializations_after_repeated),
        0,
        "an element-only mutation must reuse the retained stylesheet world"
    );
    assert_eq!(
        update_materializations_after_stylesheet_mutation
            .saturating_sub(update_materializations_after_second),
        1,
        "a stylesheet mutation should materialize one incremental update"
    );
    assert_eq!(
        full_snapshots_after_first.saturating_sub(full_snapshots_before),
        1,
        "one cold traversal should build one full style-world snapshot"
    );
    assert_eq!(
        full_snapshots_after_repeated.saturating_sub(full_snapshots_after_first),
        0,
        "an unchanged generation should not rebuild a full style-world snapshot"
    );
    assert_eq!(
        full_snapshots_after_second.saturating_sub(full_snapshots_after_repeated),
        0,
        "an element-only mutation must not build a full style-world snapshot"
    );
    assert_eq!(
        full_snapshots_after_stylesheet_mutation.saturating_sub(full_snapshots_after_second),
        0,
        "an ordinary stylesheet mutation must stay on the incremental update path"
    );
}

#[test]
fn inner_text_new_sources_wait_for_a_fresh_paint_layout() {
    let mut vm = new_rendered_test_vm(
        "https://inner-text-latest-layout.test/",
        "<!doctype html><html><body><div id=target><span>a</span></div></body></html>",
    );
    vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 320,
        inner_height: 200,
        device_pixel_ratio: 1.0,
        ..Default::default()
    }))
    .expect("innerText viewport should match the paint layout");
    let passes_before = vm.layout_pass_observability_for_test().1;
    let cache_before = vm.layout_snapshot_cache_observability_for_test();

    assert_eq!(
        vm.eval("document.getElementById('target').innerText")
            .expect("the published innerText read should evaluate"),
        "a"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, passes_before);

    assert_eq!(
        vm.eval(
            "const added = document.createElement('span'); added.textContent = 'b'; target.append(added); target.innerText",
        )
        .expect("the warm innerText read should evaluate"),
        "a",
        "a text source absent from the latest frozen layout tree remains unrendered until refresh"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, passes_before);

    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 200, 1.0))
        .expect("fresh paint layout should succeed")
        .expect("the fixture should have a layout root");
    assert_eq!(vm.layout_pass_observability_for_test().1, passes_before + 1);
    assert_eq!(
        vm.eval("target.innerText")
            .expect("innerText should read the refreshed geometry snapshot"),
        "ab"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, passes_before + 1);

    let cache_after = vm.layout_snapshot_cache_observability_for_test();
    assert_eq!(cache_after.0, cache_before.0 + 3);
    assert_eq!(cache_after.1, cache_before.1);
    assert_eq!(cache_after.2, cache_before.2 + 1);
}

#[test]
fn inner_text_updates_device_in_place_without_full_style_world_snapshots() {
    let mut vm = new_parsed_test_vm(
        "https://inner-text-emulated-media-style-world.test/",
        r#"<!doctype html><html><head><style>
          @media print { #target { text-transform: uppercase; } }
        </style></head><body><div id="target">mixed</div></body></html>"#,
    );

    let update_materializations_before = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_before = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();
    publish_layout_for_test(&mut vm);
    assert_eq!(
        vm.eval("document.getElementById('target').innerText")
            .expect("screen innerText read should evaluate"),
        "mixed"
    );
    let update_materializations_after_screen = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_after_screen = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        media: Some("print".to_owned()),
        ..Default::default()
    });
    assert_eq!(
        vm.eval("document.getElementById('target').innerText")
            .expect("print innerText read should evaluate"),
        "MIXED"
    );
    let update_materializations_after_print = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_after_print = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides::default());
    assert_eq!(
        vm.eval("document.getElementById('target').innerText")
            .expect("restored screen innerText read should evaluate"),
        "mixed"
    );
    let update_materializations_after_restore = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_after_restore = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();

    assert_eq!(
        update_materializations_after_screen.saturating_sub(update_materializations_before),
        1
    );
    assert_eq!(
        update_materializations_after_print.saturating_sub(update_materializations_after_screen),
        1
    );
    assert_eq!(
        update_materializations_after_restore.saturating_sub(update_materializations_after_print),
        1
    );
    assert_eq!(
        full_snapshots_after_screen.saturating_sub(full_snapshots_before),
        1
    );
    assert_eq!(
        full_snapshots_after_print.saturating_sub(full_snapshots_after_screen),
        0,
        "a device update must preserve the retained style world"
    );
    assert_eq!(
        full_snapshots_after_restore.saturating_sub(full_snapshots_after_print),
        0,
        "restoring the device must remain an incremental update"
    );
}

#[test]
fn same_document_history_url_mutations_preserve_style_world() {
    let mut vm = new_rendered_test_vm(
        "https://inner-text-document-url-style-world.test/start/index.html",
        r#"<!doctype html><html><head><style>
          #target { text-transform: uppercase; background-image: url(asset.png); }
          #hash-target:target { text-transform: lowercase; }
        </style></head><body>
          <div id="target">mixed</div>
          <div id="hash-target">HASH</div>
        </body></html>"#,
    );

    assert_eq!(
        vm.eval("document.getElementById('target').innerText")
            .expect("initial innerText read should evaluate"),
        "MIXED"
    );
    assert_eq!(
        vm.eval("getComputedStyle(document.getElementById('target')).backgroundImage")
            .expect("initial stylesheet URL should evaluate"),
        r#"url("https://inner-text-document-url-style-world.test/start/asset.png")"#
    );
    let document = vm.document_handle_for_test();
    let stylist_identity = vm.retained_stylist_identity_for_document_for_test(document);
    let rebuilds = vm.retained_style_system_rebuild_count_for_document_for_test(document);
    let updates = vm.retained_style_system_update_count_for_document_for_test(document);
    let update_materializations_after_initial = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_after_initial = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();

    assert_eq!(
        vm.eval(
            "history.pushState(null, '', '/next/path'); document.getElementById('target').innerText"
        )
        .expect("innerText after same-document URL mutation should evaluate"),
        "MIXED"
    );
    assert_eq!(
        vm.eval("document.baseURI")
            .expect("base URI after pushState should evaluate"),
        "https://inner-text-document-url-style-world.test/next/path"
    );
    assert_eq!(
        vm.eval("getComputedStyle(document.getElementById('target')).backgroundImage")
            .expect("existing stylesheet URL after pushState should evaluate"),
        r#"url("https://inner-text-document-url-style-world.test/start/asset.png")"#,
        "an existing inline stylesheet keeps its processing-time base URL"
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
              const dynamic = document.createElement('div');
              dynamic.style.backgroundImage = 'url(new.png)';
              document.body.appendChild(dynamic);
              return getComputedStyle(dynamic).backgroundImage;
            })()"#
        )
        .expect("new inline declaration after pushState should evaluate"),
        r#"url("https://inner-text-document-url-style-world.test/next/new.png")"#,
        "new style declarations must observe the updated Document base URL"
    );
    assert_eq!(
        vm.eval(
            "history.replaceState(null, '', '?view=compact'); document.getElementById('target').innerText"
        )
        .expect("innerText after same-document query mutation should evaluate"),
        "MIXED"
    );
    assert_eq!(
        vm.eval("document.baseURI")
            .expect("base URI after replaceState should evaluate"),
        "https://inner-text-document-url-style-world.test/next/path?view=compact"
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
              const target = document.getElementById('target');
              for (let index = 0; index < 64; index += 1) {
                history.replaceState(null, '', `?view=${index}`);
                void target.innerText;
              }
              return target.innerText;
            })()"#
        )
        .expect("repeated history URL and layout observations should evaluate"),
        "MIXED"
    );
    assert_eq!(
        vm.eval(
            "history.replaceState(null, '', '#hash-target'); document.getElementById('hash-target').innerText"
        )
        .expect(":target style after same-document fragment mutation should evaluate"),
        "hash"
    );

    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(document),
        stylist_identity,
        "same-Document history mutations must retain the Stylist"
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds,
        "same-Document history mutations must not rebuild the style world"
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(document),
        updates,
        "path, query, and fragment changes need no retained style-world update"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_update_materializations_for_test(),
        update_materializations_after_initial,
        "history URL mutations must not materialize stylesheet inputs"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_full_snapshots_for_test(),
        full_snapshots_after_initial,
        "history URL mutations must not materialize a full style snapshot"
    );
}

#[test]
fn inner_text_reuses_one_document_style_world_across_shadow_scopes() {
    let mut vm = new_parsed_test_vm(
        "https://inner-text-shadow-style-world.test/",
        r#"<!doctype html><html><body>
          <div id="host"><span>assigned</span><span slot="missing">hidden</span></div>
        </body></html>"#,
    );
    vm.eval(
        r#"
const host = document.getElementById('host');
host.attachShadow({mode: 'open'}).innerHTML =
  '<style>::slotted(span) { text-transform: uppercase; }</style><slot></slot>';
"#,
    )
    .expect("shadow innerText retained-style fixture should initialize");
    let update_materializations_before = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_before = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();
    let layout_before = vm.layout_pass_observability_for_test();
    publish_layout_for_test(&mut vm);

    let first_text = vm
        .eval("host.innerText")
        .expect("shadow innerText read should evaluate");
    let update_materializations_after_first = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_after_first = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();
    let layout_after_first = vm.layout_pass_observability_for_test();
    let second_text = vm
        .eval("host.innerText")
        .expect("repeated shadow innerText read should evaluate");
    let update_materializations_after_second = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_after_second = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();
    let layout_after_second = vm.layout_pass_observability_for_test();

    assert_eq!(first_text, "ASSIGNED");
    assert_eq!(second_text, "ASSIGNED");
    assert!(!layout_before.0 && !layout_after_first.0 && !layout_after_second.0);
    assert_eq!(layout_after_first.1, layout_before.1 + 1);
    assert_eq!(layout_after_second.1, layout_after_first.1);
    assert_eq!(
        update_materializations_after_first.saturating_sub(update_materializations_before),
        1,
        "layout and rendered-text collection must share one document TreeScope universe"
    );
    assert_eq!(
        update_materializations_after_second.saturating_sub(update_materializations_after_first),
        0,
        "an unchanged document TreeScope universe must reuse its retained style world"
    );
    assert_eq!(
        full_snapshots_after_first.saturating_sub(full_snapshots_before),
        1,
        "one cold observation must build one full style-world snapshot"
    );
    assert_eq!(
        full_snapshots_after_second.saturating_sub(full_snapshots_after_first),
        0,
        "the rendered-text collector must reuse the unchanged retained style world"
    );
}

#[test]
fn paint_layout_reuses_one_style_world_across_many_empty_shadow_roots() {
    crate::style_engine::reset_author_source_text_parse_count_for_test();
    let mut vm = new_parsed_test_vm(
        "https://paint-empty-shadow-style-world.test/",
        r#"<!doctype html><html><head><style>
          body { color: rgb(1, 2, 3); }
        </style></head><body><main id="light">light</main></body></html>"#,
    );
    vm.eval(
        r#"
const body = document.body;
for (let index = 0; index < 64; index += 1) {
  const host = document.createElement('section');
  body.appendChild(host);
  const shadow = host.attachShadow({mode: index % 2 === 0 ? 'open' : 'closed'});
  const target = document.createElement('span');
  target.textContent = `shadow-${index}`;
  shadow.appendChild(target);
  if (index % 8 === 0) {
    const nestedHost = document.createElement('article');
    shadow.appendChild(nestedHost);
    nestedHost.attachShadow({mode: 'open'}).append(`nested-${index}`);
  }
}
"#,
    )
    .expect("empty shadow-root paint fixture should initialize");

    let document = vm.document_handle_for_test();
    let rebuilds_before = vm.retained_style_system_rebuild_count_for_document_for_test(document);
    let update_materializations_before = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_before = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();
    let document_scopes_before = vm
        ._context_host
        .borrow()
        .style_world_document_scope_materializations_for_test();
    let shadow_scopes_before = vm
        ._context_host
        .borrow()
        .style_world_shadow_scope_materializations_for_test();

    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
        .expect("first empty shadow-root paint layout should succeed")
        .expect("the fixture should have a layout root");
    let rebuilds_after_first =
        vm.retained_style_system_rebuild_count_for_document_for_test(document);
    let update_materializations_after_first = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_after_first = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();
    let document_scopes_after_first = vm
        ._context_host
        .borrow()
        .style_world_document_scope_materializations_for_test();
    let shadow_scopes_after_first = vm
        ._context_host
        .borrow()
        .style_world_shadow_scope_materializations_for_test();
    let updates_after_first = vm.retained_style_system_update_count_for_document_for_test(document);
    let parses_after_first = crate::style_engine::author_source_text_parse_count_for_test();

    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
        .expect("second empty shadow-root paint layout should succeed")
        .expect("the fixture should have a layout root");

    assert_eq!(
        rebuilds_after_first.saturating_sub(rebuilds_before),
        1,
        "entering and leaving empty shadow roots must not replace the retained style system",
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds_after_first,
        "an unchanged fresh paint layout must retain the same document style system",
    );
    assert_eq!(
        update_materializations_after_first.saturating_sub(update_materializations_before),
        1
    );
    assert_eq!(
        full_snapshots_after_first.saturating_sub(full_snapshots_before),
        1
    );
    assert_eq!(
        document_scopes_after_first.saturating_sub(document_scopes_before),
        1,
        "the initial paint should materialize the Document scope once"
    );
    assert_eq!(
        shadow_scopes_after_first.saturating_sub(shadow_scopes_before),
        72,
        "64 direct and 8 nested ShadowRoots should each materialize once"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_update_materializations_for_test(),
        update_materializations_after_first,
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_full_snapshots_for_test(),
        full_snapshots_after_first,
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_document_scope_materializations_for_test(),
        document_scopes_after_first,
        "a clean screenshot must not recollect Document stylesheets"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_shadow_scope_materializations_for_test(),
        shadow_scopes_after_first,
        "a clean screenshot must not recollect ShadowRoot stylesheets"
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(document),
        updates_after_first,
        "a clean screenshot must not flush the retained style world"
    );
    assert_eq!(
        crate::style_engine::author_source_text_parse_count_for_test(),
        parses_after_first,
        "a clean screenshot must not parse author stylesheets"
    );
}

#[test]
fn document_import_node_clones_dom_parser_svg_snapshot() {
    let mut vm = new_parsed_test_vm(
        "https://dom-parser-svg-import.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
(() => {
  const parsed = new DOMParser().parseFromString(
    "<symbol xmlns='http://www.w3.org/2000/svg' id='icon' viewBox='0 0 1 1'><path d='M0 0h1v1'/></symbol>",
    "image/svg+xml"
  );
  const imported = document.importNode(parsed.documentElement, true);
  document.body.appendChild(imported);
  return [
    imported.namespaceURI,
    imported.localName,
    imported.getAttribute("id"),
    imported.firstChild && imported.firstChild.localName,
    document.querySelector("#icon path").getAttribute("d")
  ].join("|");
})()
"##,
        )
        .expect("DOMParser SVG snapshot import should evaluate");

    assert_eq!(
        result,
        "http://www.w3.org/2000/svg|symbol|icon|path|M0 0h1v1"
    );
}
