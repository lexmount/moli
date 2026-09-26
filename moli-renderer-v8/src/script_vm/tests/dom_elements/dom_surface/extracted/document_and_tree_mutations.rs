use super::*;

#[test]
fn attr_owner_document_tracks_element_adoption() {
    let mut vm = new_storage_test_vm("https://attr-owner-document.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const div = document.createElement("div");
  div.id = "target";
  const attr = div.attributes[0];
  const other = document.implementation.createHTMLDocument("");
  other.body.appendChild(div);
  const created = other.createAttribute("data-created");
  return [
    attr.ownerDocument === other,
    div.attributes[0].ownerDocument === other,
    created.ownerDocument === other
  ].join("|");
})()
"#,
        )
        .expect("Attr ownerDocument adoption probe should evaluate");

    assert_eq!(result, "true|true|true");
}
#[test]
fn parent_node_append_prepend_validate_before_fragment_conversion() {
    let mut vm = new_storage_test_vm("https://parent-node-validation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return "no-throw";
    } catch (error) {
      return error && error.name;
    }
  };
  const appendDoc = document.implementation.createHTMLDocument("title");
  const appendAncestor = probe(() => appendDoc.body.append(appendDoc.documentElement));
  const appendBodyPreserved = appendDoc.body && appendDoc.body.parentNode === appendDoc.documentElement;

  const prependDoc = document.implementation.createHTMLDocument("title");
  const prependAncestor = probe(() => prependDoc.body.prepend(prependDoc.documentElement));
  const prependBodyPreserved = prependDoc.body && prependDoc.body.parentNode === prependDoc.documentElement;

  const doc = document.implementation.createHTMLDocument("title");
  const otherDoc = document.implementation.createHTMLDocument("other");
  const documentNode = probe(() => doc.append(otherDoc));
  const doctypeIntoElement = probe(() => doc.createElement("a").append(doc.doctype));
  const duplicateDoctype = probe(() => doc.append(doc.doctype.cloneNode()));

  return [
    appendAncestor,
    appendBodyPreserved,
    prependAncestor,
    prependBodyPreserved,
    documentNode,
    doctypeIntoElement,
    duplicateDoctype
  ].join("|");
})()
"#,
        )
        .expect("ParentNode pre-insertion validation probe should evaluate");

    assert_eq!(
        result,
        "HierarchyRequestError|true|HierarchyRequestError|true|HierarchyRequestError|HierarchyRequestError|HierarchyRequestError"
    );
}
#[test]
fn small_static_nodelist_array_indexof_remains_fast_after_length_tamper() {
    let mut vm = new_parsed_test_vm(
        "https://example.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.createElement("div");
  for (let i = 0; i < 100; i++) {
    const span = document.createElement("span");
    span.className = "hit";
    root.appendChild(span);
  }
  document.body.appendChild(root);
  const nodes = root.querySelectorAll(".hit");
  const target = nodes[50];
  let before = -1;
  let after = -1;
  const { indexOf } = Array.prototype;
  for (let i = 0; i < 1000; i++) {
    before = indexOf.call(nodes, target);
  }
  Object.defineProperty(nodes, "length", { get() { return 10; } });
  for (let i = 0; i < 1000; i++) {
    after = indexOf.call(nodes, target);
  }
  return before + "|" + after;
})()
"#,
        )
        .expect("static NodeList Array#indexOf probe should evaluate");

    assert_eq!(result, "50|-1");
}
#[test]
fn main_document_static_nodelist_reads_preserve_wrapper_identity() {
    let markup = format!(
        "<!doctype html><html><body>{}</body></html>",
        "<span class=hit></span>".repeat(1_100)
    );
    let mut vm = new_parsed_test_vm("https://example.test/", &markup);

    let result = vm
        .eval(
            r#"
(() => {
  const nodes = document.querySelectorAll(".hit");
  let nodeTypeSum = 0;
  for (let round = 0; round < 2; round++) {
    for (let index = 0; index < nodes.length; index++) {
      nodeTypeSum += nodes[index].nodeType;
    }
  }
  return `${nodes.length}|${nodeTypeSum}|${nodes[0] === nodes[0]}`;
})()
"#,
        )
        .expect("large static NodeList iteration should evaluate");

    assert_eq!(result, "1100|2200|true");
}
#[test]
fn main_document_large_static_nodelist_indices_survive_isolate_gc() {
    let markup = format!(
        "<!doctype html><html><body>{}</body></html>",
        "<span class=hit></span>".repeat(1_100)
    );
    let mut vm = new_parsed_test_vm("https://example.test/", &markup);

    vm.eval("globalThis.__largeStaticNodes = document.querySelectorAll('.hit')")
        .expect("large static NodeList should be retained across turns");
    vm.collect_renderer_document_isolate_garbage()
        .expect("document isolate garbage collection should run");

    let result = vm
        .eval(
            r#"
(() => {
  let nodeTypeSum = 0;
  for (let index = 0; index < __largeStaticNodes.length; index++) {
    const node = __largeStaticNodes[index];
    if (node === undefined) return `missing:${index}`;
    nodeTypeSum += node.nodeType;
  }
  return `${__largeStaticNodes.length}|${nodeTypeSum}|${__largeStaticNodes[0] === __largeStaticNodes[0]}`;
})()
"#,
        )
        .expect("large static NodeList iteration after GC should evaluate");

    assert_eq!(result, "1100|1100|true");
}
#[test]
fn document_title_getter_walks_full_tree_and_setter_respects_head() {
    let mut vm = new_parsed_test_vm(
        "https://document-title.test/",
        "<!doctype html><html><head><title>ORIG</title></head><body></body></html>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const out = [];
  out.push('initial=' + document.title);
  document.title = 'UPDATED';
  out.push('updated=' + document.title);
  // Remove <head>; per spec the setter must be a no-op when no <title> in tree
  // and no <head> exists.
  const head = document.getElementsByTagName('head')[0];
  if (head) head.parentNode.removeChild(head);
  out.push('headRemoved=' + (document.getElementsByTagName('head').length === 0));
  document.title = 'SHOULD_NOT_APPLY';
  out.push('afterHeadGone=' + document.title);
  // Append a <title> under <body>; the getter must find it (first title in tree
  // order, regardless of whether it is under <head>).
  const t = document.createElement('title');
  t.appendChild(document.createTextNode('FROM_BODY'));
  document.body.appendChild(t);
  out.push('bodyTitle=' + document.title);
  // Now the setter has an existing title element to replace; head still absent.
  document.title = 'REPLACED_BODY';
  out.push('replaced=' + document.title);
  return out.join('|');
})()
"#,
        )
        .expect("document.title spec behavior should evaluate");
    assert_eq!(
        result,
        "initial=ORIG|updated=UPDATED|headRemoved=true|afterHeadGone=|bodyTitle=FROM_BODY|replaced=REPLACED_BODY"
    );
}
#[test]
fn document_title_normalizes_html_whitespace_and_uses_svg_namespace_rules() {
    let mut vm = new_parsed_test_vm(
        "https://document-title-semantics.test/",
        "<!doctype html><html><head><title> initial  title </title></head><body></body></html>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const SVG = 'http://www.w3.org/2000/svg';
  const HTML = 'http://www.w3.org/1999/xhtml';
  const out = [];

  out.push('initial=' + document.title);
  document.title = ' one\t\n  two\f\r three\u000bfour ';
  out.push('set=' + document.title);

  const htmlDoc = document.implementation.createHTMLDocument(' detached\t\n title ');
  out.push('detached=' + htmlDoc.title);
  const emptyHtmlDoc = document.implementation.createHTMLDocument('');
  out.push('emptyDetached=' + [
    emptyHtmlDoc.title,
    emptyHtmlDoc.head.firstChild.childNodes.length,
    emptyHtmlDoc.head.firstChild.firstChild.data
  ].join(','));

  const svgDoc = document.implementation.createDocument(SVG, 'svg', null);
  const oldChild = svgDoc.createElementNS(SVG, 'x-child');
  svgDoc.documentElement.appendChild(oldChild);
  svgDoc.title = ' svg\n title ';
  const svgTitle = svgDoc.documentElement.firstChild;
  out.push('svg=' + [
    svgDoc.title,
    svgTitle.namespaceURI,
    svgTitle.localName,
    svgTitle.textContent,
    svgTitle.nextSibling === oldChild
  ].join(','));

  const nestedSvgDoc = document.implementation.createDocument(SVG, 'svg', null);
  const group = nestedSvgDoc.createElementNS(SVG, 'g');
  const nestedTitle = nestedSvgDoc.createElementNS(SVG, 'title');
  nestedTitle.textContent = 'nested';
  group.appendChild(nestedTitle);
  nestedSvgDoc.documentElement.appendChild(group);
  nestedSvgDoc.title = 'direct';
  out.push('nested=' + [
    nestedSvgDoc.title,
    nestedTitle.textContent,
    nestedSvgDoc.documentElement.firstChild.localName
  ].join(','));

  const xmlDoc = document.implementation.createDocument(null, 'root', null);
  const foreignTitle = xmlDoc.createElementNS(HTML, 'title');
  foreignTitle.textContent = 'keep';
  xmlDoc.documentElement.appendChild(foreignTitle);
  xmlDoc.title = 'blocked';
  out.push('xml=' + [xmlDoc.title, foreignTitle.textContent].join(','));

  return out.join('|');
})()
"#,
        )
        .expect("Document.title namespace and normalization behavior should evaluate");
    assert_eq!(
        result,
        "initial=initial title|set=one two three\u{000b}four|detached=detached title|emptyDetached=,1,|svg=svg title,http://www.w3.org/2000/svg,title, svg\n title ,true|nested=direct,nested,title|xml=keep,keep"
    );
}
#[test]
fn detached_node_internal_slots_stay_hidden_before_and_after_live_pairing() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement('html'));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement('body'));
              }
              const doc = new DOMParser().parseFromString(
                '<html><body><main><span id="child">x</span></main></body></html>',
                'text/html'
              );
              const node = doc.getElementById('child');
              const host = document.body || document.documentElement || document;
              const snapshot = target => ({
                ownNamesHasState: Object.getOwnPropertyNames(target).includes("__moliDetachedState"),
                ownNamesHasDelegate: Object.getOwnPropertyNames(target).includes("__moliLiveDelegate"),
                ownKeysHasState: Reflect.ownKeys(target).includes("__moliDetachedState"),
                ownKeysHasDelegate: Reflect.ownKeys(target).includes("__moliLiveDelegate"),
                hasStateSlot: "__moliDetachedState" in target,
                hasDelegateSlot: "__moliLiveDelegate" in target,
                stateType: typeof target.__moliDetachedState,
                delegateType: typeof target.__moliLiveDelegate
              });
              const before = snapshot(node);
              host.appendChild(node);
              const after = {
                ...snapshot(node),
                ownerDocumentIsLive: node.ownerDocument === document,
                hostContains: host.contains(node),
                text: node.textContent
              };
              return JSON.stringify({ before, after });
            })()
            "#,
        )
        .expect("detached node private slots should remain hidden across live pairing");

    assert_eq!(
        result,
        r#"{"before":{"ownNamesHasState":false,"ownNamesHasDelegate":false,"ownKeysHasState":false,"ownKeysHasDelegate":false,"hasStateSlot":false,"hasDelegateSlot":false,"stateType":"undefined","delegateType":"undefined"},"after":{"ownNamesHasState":false,"ownNamesHasDelegate":false,"ownKeysHasState":false,"ownKeysHasDelegate":false,"hasStateSlot":false,"hasDelegateSlot":false,"stateType":"undefined","delegateType":"undefined","ownerDocumentIsLive":true,"hostContains":true,"text":"x"}}"#
    );
}
#[test]
fn adopt_node_updates_detached_subtree_owner_document() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const sourceDocument = document.implementation.createDocument(null, "root", null);
              const root = sourceDocument.documentElement;
              const child = sourceDocument.createElement("child");
              root.appendChild(child);
              const adopted = document.adoptNode(root);
              return JSON.stringify({
                returnedSameNode: adopted === root,
                detachedFromSource: sourceDocument.documentElement === null,
                rootParent: root.parentNode,
                childPreserved: root.firstChild === child,
                rootOwnerIsLive: root.ownerDocument === document,
                childOwnerIsLive: child.ownerDocument === document
              });
            })()
            "#,
        )
        .expect("detached adoptNode ownerDocument probe should evaluate");

    assert_eq!(
        result,
        r#"{"returnedSameNode":true,"detachedFromSource":true,"rootParent":null,"childPreserved":true,"rootOwnerIsLive":true,"childOwnerIsLive":true}"#
    );
}
#[test]
fn document_import_and_adopt_reject_shadow_roots() {
    let mut vm = new_storage_test_vm("https://shadow-root-import-adopt.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const detachedDocument = document.implementation.createHTMLDocument();
              const openHost = document.createElement("div");
              const openRoot = openHost.attachShadow({ mode: "open" });
              const closedHost = document.createElement("div");
              const closedRoot = closedHost.attachShadow({ mode: "closed" });

              function exceptionName(callback) {
                try {
                  callback();
                  return "none";
                } catch (error) {
                  return `${error.name}:${error.code}`;
                }
              }

              const fragment = document.createDocumentFragment();
              fragment.appendChild(document.createElement("span"));
              const importedFragment = detachedDocument.importNode(fragment, true);

              return [
                exceptionName(() => detachedDocument.importNode(openRoot)),
                exceptionName(() => detachedDocument.importNode(closedRoot)),
                exceptionName(() => detachedDocument.adoptNode(openRoot)),
                exceptionName(() => detachedDocument.adoptNode(closedRoot)),
                exceptionName(() => document.importNode(openRoot)),
                exceptionName(() => document.adoptNode(openRoot)),
                importedFragment.nodeType,
                importedFragment.firstChild.localName
              ].join("|");
            })()
            "#,
        )
        .expect("Document importNode/adoptNode should reject ShadowRoot nodes");

    assert_eq!(
        result,
        "NotSupportedError:9|NotSupportedError:9|HierarchyRequestError:3|HierarchyRequestError:3|NotSupportedError:9|HierarchyRequestError:3|11|span"
    );
}
#[test]
fn contextual_fragment_document_element_fragment_append_runs_scripts() {
    let mut vm = new_storage_test_vm("https://contextual-fragment-document-element-scripts.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__fragmentScriptRan = false;
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const range = document.createRange();
  range.selectNodeContents(html);
  const fragment = range.createContextualFragment(
    "<script>__fragmentScriptRan = true<\/script>"
  );
  const beforeShape = Array.from(fragment.childNodes)
    .map(node => `${node.nodeName}:${node.localName}:${node.childNodes.length}`)
    .join(",");
  const beforeAppend = __fragmentScriptRan;
  body.appendChild(fragment);
  return [
    beforeAppend,
    __fragmentScriptRan,
    fragment.childNodes.length,
    beforeShape,
    body.lastChild && body.lastChild.nodeName,
    body.lastChild && body.lastChild.childNodes.length
  ].join("|");
})()
"#,
        )
        .expect("contextual fragment documentElement script should evaluate after append");

    assert_eq!(result, "false|true|0|SCRIPT:script:1|SCRIPT|1");
}
#[test]
fn sandboxed_fragment_parsers_parse_noscript_when_scripting_disabled() {
    let mut vm = new_storage_test_vm("https://sandbox-contextual-fragment-noscript.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.sandbox = "allow-same-origin";
  (document.body || document.documentElement || document).appendChild(iframe);
  const doc = iframe.contentDocument;
  iframe.contentWindow.didRunScript = false;
  const contextualHtml =
    "<script>window.didRunScript = true<\/script>" +
    "<noscript><div id=contextual-nos></div>";
  const fragment = doc.createRange().createContextualFragment(contextualHtml);
  doc.body.appendChild(fragment);

  const inner = doc.createElement("div");
  inner.innerHTML = "<noscript><div id=inner-nos></div></noscript>";
  doc.body.appendChild(inner);

  const unsafe = doc.createElement("div");
  unsafe.setHTMLUnsafe("<noscript><div id=unsafe-nos></div></noscript>");
  doc.body.appendChild(unsafe);

  return [
    iframe.contentWindow.didRunScript,
    doc.getElementById("contextual-nos") !== null,
    doc.getElementById("inner-nos") !== null,
    doc.getElementById("unsafe-nos") !== null
  ].join("|");
})()
"#,
        )
        .expect("sandboxed fragment noscript probe should evaluate");

    assert_eq!(result, "false|true|true|true");
}
#[test]
fn globally_disabled_fragment_parsers_parse_noscript_markup() {
    let mut vm = new_storage_test_vm("https://disabled-fragment-noscript.test/");
    vm.set_script_execution_disabled(true);

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  const inner = document.createElement("div");
  inner.innerHTML = "<noscript><span id=inner-fallback></span></noscript>";
  root.appendChild(inner);

  const unsafe = document.createElement("div");
  unsafe.setHTMLUnsafe("<noscript><span id=unsafe-fallback></span></noscript>");
  root.appendChild(unsafe);

  return [
    document.getElementById("inner-fallback") !== null,
    document.getElementById("unsafe-fallback") !== null
  ].join("|");
})()
"#,
        )
        .expect("globally disabled fragment noscript probe should evaluate");

    assert_eq!(result, "true|true");
}
#[test]
fn document_fragment_script_start_revalidates_after_an_earlier_script_mutates_the_batch() {
    let mut vm = new_storage_test_vm("https://document-fragment-script-revalidation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__documentFragmentRevalidation = [];
  const fragment = document.createDocumentFragment();
  const first = document.createElement('script');
  first.textContent = `
    __documentFragmentRevalidation.push('first');
    document.getElementById('later-script').remove();
  `;
  const later = document.createElement('script');
  later.id = 'later-script';
  later.textContent = "__documentFragmentRevalidation.push('stale-later')";
  fragment.append(first, later);
  (document.body || document.documentElement || document).appendChild(fragment);
  return __documentFragmentRevalidation.join('|');
})()
"#,
        )
        .expect("an earlier fragment script should invalidate a later start candidate");

    assert_eq!(result, "first");
}
