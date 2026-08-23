use super::*;

#[test]
fn xpath_compiled_expressions_preserve_namespaces_brands_and_realms() {
    let mut vm = new_parsed_test_vm(
        "https://xpath-expression.test/",
        "<html><body></body></html>",
    );
    assert_eq!(
        vm.eval(include_str!("xpath_compiled_expressions.js"))
            .expect("compiled XPath expression fixture should evaluate"),
        "true"
    );
}

#[test]
fn xpath_evaluation_uses_context_document_after_resolver_callbacks() {
    let mut vm = new_parsed_test_vm("https://xpath-context.test/", "<html><body></body></html>");
    assert_eq!(
        vm.eval(include_str!("xpath_context_documents.js"))
            .expect("XPath context document fixture should evaluate"),
        "true"
    );
}

#[test]
fn xpath_evaluator_constructor_requires_new() {
    let mut vm = new_parsed_test_vm(
        "https://xpath-evaluator-constructor.test/",
        "<html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const evaluator = new XPathEvaluator();
  let bareCall;
  try {
    XPathEvaluator();
    bareCall = 'ok';
  } catch (error) {
    bareCall = error.name;
  }
  const host = document.createElement('div');
  const shadow = host.attachShadow({ mode: 'open' });
  const span = document.createElement('span');
  shadow.appendChild(span);
  document.body.appendChild(host);
  const shadowResult = evaluator.evaluate(
    '//span',
    span,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  );
  const parsed = new DOMParser().parseFromString(
    '<root><item id="detached"/></root>',
    'text/xml'
  );
  const detachedItem = parsed.documentElement.firstChild;
  const detachedResult = evaluator.evaluate(
    '//item',
    detachedItem,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  );
  return [
    evaluator instanceof XPathEvaluator,
    Object.getPrototypeOf(evaluator) === XPathEvaluator.prototype,
    Object.prototype.toString.call(evaluator),
    XPathEvaluator.name,
    XPathEvaluator.length,
    bareCall,
    typeof evaluator.evaluate,
    evaluator.evaluate.length,
    shadowResult.singleNodeValue === span,
    detachedResult.singleNodeValue === detachedItem
  ].join('|');
})()
"#,
        )
        .expect("XPathEvaluator constructor probe should evaluate");

    assert_eq!(
        result,
        "true|true|[object XPathEvaluator]|XPathEvaluator|0|TypeError|function|2|true|true"
    );
}

#[test]
fn xpath_evaluator_create_ns_resolver_returns_node_identity() {
    let mut vm = new_parsed_test_vm(
        "https://xpath-evaluator-create-ns-resolver.test/",
        "<!doctype html><html><body>text</body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const evaluator = new XPathEvaluator();
  const fragment = document.createDocumentFragment();
  const attribute = document.createAttribute('data-probe');
  const nodes = [
    document,
    fragment,
    document.doctype,
    document.body,
    document.body.firstChild,
    attribute
  ];
  const errorName = callback => {
    try {
      callback();
      return 'missing';
    } catch (error) {
      return error.name;
    }
  };
  return JSON.stringify({
    identities: nodes.map(node => evaluator.createNSResolver(node) === node),
    elementXml: evaluator.createNSResolver(document.body).lookupNamespaceURI('xml'),
    documentXml: evaluator.createNSResolver(new Document()).lookupNamespaceURI('xml'),
    length: evaluator.createNSResolver.length,
    missing: errorName(() => evaluator.createNSResolver()),
    primitive: errorName(() => evaluator.createNSResolver(1))
  });
})()
"#,
        )
        .expect("XPathEvaluator createNSResolver probe should evaluate");

    assert_eq!(
        result,
        r#"{"identities":[true,true,true,true,true,true],"elementXml":"http://www.w3.org/XML/1998/namespace","documentXml":null,"length":1,"missing":"TypeError","primitive":"TypeError"}"#
    );
}

#[test]
fn document_xpath_queries_parse_webidl_arguments() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-webidl.test/path/index.html",
        r#"<html><body>
            <section><div id="first"></div><div id="second"></div></section>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.id ? value.id : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const ids = document.evaluate(
    "//div",
    document,
    null,
    XPathResult.ORDERED_NODE_SNAPSHOT_TYPE
  );
  const wrapped = document.evaluate(
    { toString() { return "//div[@id='second']"; } },
    document,
    null,
    { valueOf() { return XPathResult.FIRST_ORDERED_NODE_TYPE; } }
  );
  const existing = document.evaluate(
    "//div[@id='first']",
    document,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  );
  const withExisting = document.evaluate(
    "//div[@id='second']",
    document,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE,
    existing
  );
  const parsed = new DOMParser().parseFromString(
    "<main><div id='detached-first'></div><div id='detached-second'></div></main>",
    "text/html"
  );
  const detachedExisting = parsed.evaluate(
    "//div[@id='detached-first']",
    parsed,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  );
  const detachedWithExisting = parsed.evaluate(
    "//div[@id='detached-second']",
    parsed,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE,
    detachedExisting
  );
  const liveResolverNode = document.body.firstElementChild;
  const detachedResolverNode = parsed.documentElement;
  return JSON.stringify({
    first: ids.snapshotItem(0).id,
    wrappedExpressionAndType: wrapped.singleNodeValue.id,
    existingResultIgnored: withExisting !== existing,
    existingResultNode: withExisting.singleNodeValue.id,
    detachedExistingResultIgnored: detachedWithExisting !== detachedExisting,
    detachedExistingResultNode: detachedWithExisting.singleNodeValue.id,
    liveResolverIdentity: document.createNSResolver(liveResolverNode) === liveResolverNode,
    detachedResolverIdentity: parsed.createNSResolver(detachedResolverNode) === detachedResolverNode,
    createNSResolverLength: document.createNSResolver.length,
    missingResolverNode: probe(() => document.createNSResolver()),
    primitiveResolverNode: probe(() => document.createNSResolver(1)),
    missingExpression: probe(() => document.evaluate()),
    missingContext: probe(() => document.evaluate("//div")),
    symbolExpression: probe(() => document.evaluate(Symbol(), document)),
    symbolType: probe(() => document.evaluate("//div", document, null, Symbol())),
    primitiveExistingResult: probe(() => document.evaluate("//div", document, null, 0, 1)),
    symbolExistingResult: probe(() => document.evaluate("//div", document, null, 0, Symbol())),
    detachedPrimitiveExistingResult: probe(() => parsed.evaluate("//div", parsed, null, 0, 1)),
    unsupportedType: probe(() => document.evaluate("//div", document, null, 10)),
    missingSnapshotIndex: probe(() => ids.snapshotItem()),
    symbolSnapshotIndex: probe(() => ids.snapshotItem(Symbol())),
    outOfRangeSnapshotItem: ids.snapshotItem(ids.snapshotLength) === null,
    wrappedSnapshotIndex: ids.snapshotItem({ valueOf() { return 1; } }).id
  });
})()
"#,
        )
        .expect("document XPath WebIDL probe should evaluate");

    assert_eq!(
        result,
        r#"{"first":"first","wrappedExpressionAndType":"second","existingResultIgnored":true,"existingResultNode":"second","detachedExistingResultIgnored":true,"detachedExistingResultNode":"detached-second","liveResolverIdentity":true,"detachedResolverIdentity":true,"createNSResolverLength":1,"missingResolverNode":"throw:TypeError","primitiveResolverNode":"throw:TypeError","missingExpression":"throw:TypeError","missingContext":"throw:TypeError","symbolExpression":"throw:TypeError","symbolType":"throw:TypeError","primitiveExistingResult":"throw:TypeError","symbolExistingResult":"throw:TypeError","detachedPrimitiveExistingResult":"throw:TypeError","unsupportedType":"throw:NotSupportedError","missingSnapshotIndex":"throw:TypeError","symbolSnapshotIndex":"throw:TypeError","outOfRangeSnapshotItem":true,"wrappedSnapshotIndex":"second"}"#
    );
}

#[test]
fn document_xpath_evaluates_live_detached_context_nodes() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-live-context.test/path/index.html",
        r#"<html><body></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement("article");
  const span = document.createElement("span");
  span.id = "detached";
  span.setAttribute("data-kind", "target");
  span.textContent = "Detached text";
  host.appendChild(span);

  const nodeResult = document.evaluate(
    ".//span[@data-kind='target']",
    host,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  );
  const stringResult = document.evaluate(
    "string(.//span)",
    host,
    null,
    XPathResult.STRING_TYPE
  );

  return JSON.stringify({
    sameNode: nodeResult.singleNodeValue === span,
    stringValue: stringResult.stringValue
  });
})()
"#,
        )
        .expect("live detached XPath context should evaluate");

    assert_eq!(result, r#"{"sameNode":true,"stringValue":"Detached text"}"#);
}

#[test]
fn document_xpath_live_iterators_track_dom_mutations() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-iterator-mutation.test/path/index.html",
        r#"<html><body><div id="first"></div><div id="second"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.id ? value.id : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const iterator = document.evaluate(
    "//div",
    document,
    null,
    XPathResult.ORDERED_NODE_ITERATOR_TYPE
  );
  const snapshot = document.evaluate(
    "//div",
    document,
    null,
    XPathResult.ORDERED_NODE_SNAPSHOT_TYPE
  );
  const first = iterator.iterateNext().id;
  document.body.appendChild(document.createElement("div"));

  const attrIterator = document.evaluate(
    "//div[@id='first']",
    document,
    null,
    XPathResult.UNORDERED_NODE_ITERATOR_TYPE
  );
  document.getElementById("first").setAttribute("data-mutated", "yes");

  return JSON.stringify({
    first,
    iteratorInvalid: iterator.invalidIteratorState,
    iteratorAfterTreeMutation: probe(() => iterator.iterateNext()),
    snapshotInvalid: snapshot.invalidIteratorState,
    snapshotLength: snapshot.snapshotLength,
    snapshotSecond: snapshot.snapshotItem(1).id,
    attrIteratorInvalid: attrIterator.invalidIteratorState,
    attrIteratorAfterMutation: probe(() => attrIterator.iterateNext())
  });
})()
"#,
        )
        .expect("live XPath iterators should observe DOM mutations");

    assert_eq!(
        result,
        r#"{"first":"first","iteratorInvalid":true,"iteratorAfterTreeMutation":"throw:InvalidStateError","snapshotInvalid":false,"snapshotLength":2,"snapshotSecond":"second","attrIteratorInvalid":true,"attrIteratorAfterMutation":"throw:InvalidStateError"}"#
    );
}

#[test]
fn document_xpath_detached_iterators_track_domparser_mutations() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-detached-iterator-mutation.test/path/index.html",
        r#"<html><body></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.id ? value.id : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const parsed = new DOMParser().parseFromString(
    "<main><div id='first'></div><div id='second'></div></main>",
    "text/html"
  );
  const iterator = parsed.evaluate(
    "//div",
    parsed,
    null,
    XPathResult.ORDERED_NODE_ITERATOR_TYPE
  );
  const snapshot = parsed.evaluate(
    "//div",
    parsed,
    null,
    XPathResult.ORDERED_NODE_SNAPSHOT_TYPE
  );
  const first = iterator.iterateNext().id;
  parsed.body.appendChild(parsed.createElement("div"));

  const attrIterator = parsed.evaluate(
    "//div[@id='first']",
    parsed,
    null,
    XPathResult.UNORDERED_NODE_ITERATOR_TYPE
  );
  parsed.getElementById("first").setAttribute("data-mutated", "yes");

  return JSON.stringify({
    first,
    iteratorInvalid: iterator.invalidIteratorState,
    iteratorAfterTreeMutation: probe(() => iterator.iterateNext()),
    snapshotInvalid: snapshot.invalidIteratorState,
    snapshotLength: snapshot.snapshotLength,
    snapshotSecond: snapshot.snapshotItem(1).id,
    attrIteratorInvalid: attrIterator.invalidIteratorState,
    attrIteratorAfterMutation: probe(() => attrIterator.iterateNext())
  });
})()
"#,
        )
        .expect("detached DOMParser XPath iterators should observe DOM mutations");

    assert_eq!(
        result,
        r#"{"first":"first","iteratorInvalid":true,"iteratorAfterTreeMutation":"throw:InvalidStateError","snapshotInvalid":false,"snapshotLength":2,"snapshotSecond":"second","attrIteratorInvalid":true,"attrIteratorAfterMutation":"throw:InvalidStateError"}"#
    );
}

#[test]
fn document_xpath_detached_object_tree_iterators_track_mutations() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-object-tree-iterator-mutation.test/path/index.html",
        r#"<html><body></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.id ? value.id : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const doc = document.implementation.createHTMLDocument("");
  const first = doc.createElement("div");
  first.id = "first";
  const second = doc.createElement("div");
  second.id = "second";
  const text = doc.createTextNode("before");
  second.appendChild(text);
  doc.body.appendChild(first);
  doc.body.appendChild(second);

  const treeIterator = doc.evaluate(
    "//div",
    doc,
    null,
    XPathResult.ORDERED_NODE_ITERATOR_TYPE
  );
  const snapshot = doc.evaluate(
    "//div",
    doc,
    null,
    XPathResult.ORDERED_NODE_SNAPSHOT_TYPE
  );
  const firstId = treeIterator.iterateNext().id;
  doc.body.appendChild(doc.createElement("div"));

  const attrIterator = doc.evaluate(
    "//div[@id='first']",
    doc,
    null,
    XPathResult.UNORDERED_NODE_ITERATOR_TYPE
  );
  first.setAttribute("data-mutated", "yes");

  const textIterator = doc.evaluate(
    "//div[text()='before']",
    doc,
    null,
    XPathResult.UNORDERED_NODE_ITERATOR_TYPE
  );
  text.data = "after";

  return JSON.stringify({
    firstId,
    treeInvalid: treeIterator.invalidIteratorState,
    treeAfterMutation: probe(() => treeIterator.iterateNext()),
    snapshotInvalid: snapshot.invalidIteratorState,
    snapshotLength: snapshot.snapshotLength,
    snapshotSecond: snapshot.snapshotItem(1).id,
    attrInvalid: attrIterator.invalidIteratorState,
    attrAfterMutation: probe(() => attrIterator.iterateNext()),
    textInvalid: textIterator.invalidIteratorState,
    textAfterMutation: probe(() => textIterator.iterateNext())
  });
})()
"#,
        )
        .expect("detached object-tree XPath iterators should observe DOM mutations");

    assert_eq!(
        result,
        r#"{"firstId":"first","treeInvalid":true,"treeAfterMutation":"throw:InvalidStateError","snapshotInvalid":false,"snapshotLength":2,"snapshotSecond":"second","attrInvalid":true,"attrAfterMutation":"throw:InvalidStateError","textInvalid":true,"textAfterMutation":"throw:InvalidStateError"}"#
    );
}

#[test]
fn document_xpath_result_accessors_are_type_specific() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-result-accessors.test/path/index.html",
        r#"<html><body><div id="first"></div><div id="second"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.id ? value.id : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const number = document.evaluate("count(//div)", document, null, XPathResult.NUMBER_TYPE);
  const string = document.evaluate("string(//div[@id='first']/@id)", document, null, XPathResult.STRING_TYPE);
  const boolean = document.evaluate("count(//div) = 2", document, null, XPathResult.BOOLEAN_TYPE);
  const nodeSetBoolean = document.evaluate("//div", document, null, XPathResult.BOOLEAN_TYPE);
  const emptyNodeSetBoolean = document.evaluate("//missing", document, null, XPathResult.BOOLEAN_TYPE);
  const single = document.evaluate("//div[@id='first']", document, null, XPathResult.FIRST_ORDERED_NODE_TYPE);
  const iterator = document.evaluate("//div", document, null, XPathResult.ORDERED_NODE_ITERATOR_TYPE);
  const snapshot = document.evaluate("//div", document, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE);
  const nodeSetBooleanInvalidBefore = nodeSetBoolean.invalidIteratorState;
  const nodeSetBooleanIterateBefore = probe(() => nodeSetBoolean.iterateNext());
  document.body.appendChild(document.createElement("div"));

  return JSON.stringify({
    numberValue: number.numberValue,
    stringValue: string.stringValue,
    booleanValue: boolean.booleanValue,
    nodeSetBooleanType: nodeSetBoolean.resultType,
    nodeSetBooleanValue: nodeSetBoolean.booleanValue,
    emptyNodeSetBooleanValue: emptyNodeSetBoolean.booleanValue,
    nodeSetBooleanInvalidBefore,
    nodeSetBooleanInvalidAfter: nodeSetBoolean.invalidIteratorState,
    nodeSetBooleanIterateBefore,
    nodeSetBooleanIterateAfter: probe(() => nodeSetBoolean.iterateNext()),
    singleNodeValue: single.singleNodeValue.id,
    snapshotLength: snapshot.snapshotLength,
    numberStringValue: probe(() => number.stringValue),
    stringNumberValue: probe(() => string.numberValue),
    booleanSingleNodeValue: probe(() => boolean.singleNodeValue),
    singleSnapshotLength: probe(() => single.snapshotLength),
    snapshotIterateNext: probe(() => snapshot.iterateNext()),
    iteratorSnapshotItem: probe(() => iterator.snapshotItem(0)),
    snapshotSingleNodeValue: probe(() => snapshot.singleNodeValue)
  });
})()
"#,
        )
        .expect("XPathResult type-specific accessors should evaluate");

    assert_eq!(
        result,
        r#"{"numberValue":2,"stringValue":"first","booleanValue":true,"nodeSetBooleanType":3,"nodeSetBooleanValue":true,"emptyNodeSetBooleanValue":false,"nodeSetBooleanInvalidBefore":false,"nodeSetBooleanInvalidAfter":false,"nodeSetBooleanIterateBefore":"throw:TypeError","nodeSetBooleanIterateAfter":"throw:TypeError","singleNodeValue":"first","snapshotLength":2,"numberStringValue":"throw:TypeError","stringNumberValue":"throw:TypeError","booleanSingleNodeValue":"throw:TypeError","singleSnapshotLength":"throw:TypeError","snapshotIterateNext":"throw:TypeError","iteratorSnapshotItem":"throw:TypeError","snapshotSingleNodeValue":"throw:TypeError"}"#
    );
}

#[test]
fn document_xpath_result_uses_browser_like_object_shape() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-result-shape.test/path/index.html",
        r#"<html><body><div id="first"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const iterator = document.evaluate(
    "//div",
    document,
    null,
    XPathResult.ORDERED_NODE_ITERATOR_TYPE
  );
  const proto = XPathResult.prototype;
  const descriptorShape = descriptor => [
    typeof descriptor.value,
    descriptor.value.name,
    descriptor.value.length,
    descriptor.enumerable,
    descriptor.configurable,
    descriptor.writable
  ];
  const iterateDescriptor = Object.getOwnPropertyDescriptor(proto, "iterateNext");
  const snapshotDescriptor = Object.getOwnPropertyDescriptor(proto, "snapshotItem");
  const tagDescriptor = Object.getOwnPropertyDescriptor(proto, Symbol.toStringTag);
  return JSON.stringify({
    constructorType: typeof XPathResult,
    constructorName: XPathResult.name,
    constructorLength: XPathResult.length,
    illegalConstructor: probe(() => new XPathResult()),
    tag: Object.prototype.toString.call(iterator),
    instanceofXPathResult: iterator instanceof XPathResult,
    prototypeMatch: Object.getPrototypeOf(iterator) === proto,
    constructorOnPrototype: proto.constructor === XPathResult,
    ownKeys: Object.keys(iterator),
    ownIterateNext: Object.prototype.hasOwnProperty.call(iterator, "iterateNext"),
    ownResultType: Object.prototype.hasOwnProperty.call(iterator, "resultType"),
    resultTypeValue: iterator.resultType,
    invalidIteratorStateValue: iterator.invalidIteratorState,
    prototypeIterateNext: descriptorShape(iterateDescriptor),
    prototypeSnapshotItem: descriptorShape(snapshotDescriptor),
    prototypeToStringTag: [
      tagDescriptor.value,
      tagDescriptor.enumerable,
      tagDescriptor.configurable,
      tagDescriptor.writable
    ],
    prototypeResultTypePresent: "resultType" in proto,
    constructorConstantsEnumerable: Object.keys(XPathResult).includes("ORDERED_NODE_ITERATOR_TYPE"),
    prototypeConstantsEnumerable: Object.keys(proto).includes("ORDERED_NODE_ITERATOR_TYPE"),
    prototypeMethodsEnumerable: [
      Object.keys(proto).includes("iterateNext"),
      Object.keys(proto).includes("snapshotItem")
    ],
    constructorConstant: XPathResult.ORDERED_NODE_ITERATOR_TYPE,
    prototypeConstant: proto.ORDERED_NODE_ITERATOR_TYPE,
    windowEnumerable: Object.keys(window).includes("XPathResult")
  });
})()
"#,
        )
        .expect("XPathResult object shape should evaluate");

    assert_eq!(
        result,
        r#"{"constructorType":"function","constructorName":"XPathResult","constructorLength":0,"illegalConstructor":"throw:TypeError","tag":"[object XPathResult]","instanceofXPathResult":true,"prototypeMatch":true,"constructorOnPrototype":true,"ownKeys":[],"ownIterateNext":false,"ownResultType":false,"resultTypeValue":5,"invalidIteratorStateValue":false,"prototypeIterateNext":["function","iterateNext",0,true,true,true],"prototypeSnapshotItem":["function","snapshotItem",0,true,true,true],"prototypeToStringTag":["XPathResult",false,true,false],"prototypeResultTypePresent":true,"constructorConstantsEnumerable":true,"prototypeConstantsEnumerable":true,"prototypeMethodsEnumerable":[true,true],"constructorConstant":5,"prototypeConstant":5,"windowEnumerable":false}"#
    );
}

#[test]
fn document_xpath_result_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-result-private-slots.test/path/index.html",
        r#"<html><body><div id="first"></div><div id="second"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      return callback();
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith("__moliXPath"))
    .sort();
  const iterator = document.evaluate(
    "//div",
    document,
    null,
    XPathResult.ORDERED_NODE_ITERATOR_TYPE
  );
  const snapshot = document.evaluate(
    "//div",
    document,
    null,
    XPathResult.ORDERED_NODE_SNAPSHOT_TYPE
  );
  const number = document.evaluate(
    "count(//div)",
    document,
    null,
    XPathResult.NUMBER_TYPE
  );
  const internalNamesBefore = {
    iterator: internalNames(iterator),
    snapshot: internalNames(snapshot),
    number: internalNames(number)
  };
  Object.assign(iterator, {
    __moliXPathType: XPathResult.NUMBER_TYPE,
    __moliXPathNumberValue: 99,
    __moliXPathNodes: [],
    __moliXPathIndex: 99
  });
  Object.assign(snapshot, {
    __moliXPathType: XPathResult.STRING_TYPE,
    __moliXPathStringValue: "spoofed",
    __moliXPathSnapshotLength: 99,
    __moliXPathNodes: []
  });
  Object.assign(number, {
    __moliXPathType: XPathResult.STRING_TYPE,
    __moliXPathStringValue: "spoofed"
  });
  const proto = XPathResult.prototype;
  const fake = {
    __moliXPathType: XPathResult.STRING_TYPE,
    __moliXPathStringValue: "fake",
    __moliXPathSnapshotLength: 9,
    __moliXPathNodes: [document.body],
    __moliXPathIndex: 0
  };
  return JSON.stringify({
    internalNamesBefore,
    iteratorType: iterator.resultType,
    iteratorInvalid: iterator.invalidIteratorState,
    iteratorNext: iterator.iterateNext().id,
    snapshotLength: snapshot.snapshotLength,
    snapshotFirst: snapshot.snapshotItem(0).id,
    numberValue: number.numberValue,
    fakeResultType: Object.getOwnPropertyDescriptor(proto, "resultType").get.call(fake),
    fakeStringValue: probe(() => Object.getOwnPropertyDescriptor(proto, "stringValue").get.call(fake)),
    fakeSnapshotItem: probe(() => proto.snapshotItem.call(fake, 0))
  });
})()
"#,
        )
        .expect("XPathResult private slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"internalNamesBefore":{"iterator":[],"snapshot":[],"number":[]},"iteratorType":5,"iteratorInvalid":false,"iteratorNext":"first","snapshotLength":2,"snapshotFirst":"first","numberValue":2,"fakeResultType":0,"fakeStringValue":"throw:TypeError","fakeSnapshotItem":"throw:TypeError"}"#
    );
}

#[test]
fn document_xpath_maps_attribute_node_results() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-attribute-result.test/path/index.html",
        r#"<html><body><section id="root" data-live="yes"></section></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.getElementById("root");
  const liveAttr = document.evaluate(
    "//@data-live",
    document,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  const parsed = new DOMParser().parseFromString(
    "<main><p id='p' data-detached='yes'></p></main>",
    "text/html"
  );
  const detachedAttr = parsed.evaluate(
    "//@data-detached",
    parsed,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  return JSON.stringify({
    liveName: liveAttr && liveAttr.name,
    liveValue: liveAttr && liveAttr.value,
    liveOwner: liveAttr && liveAttr.ownerElement === root,
    detachedName: detachedAttr && detachedAttr.name,
    detachedValue: detachedAttr && detachedAttr.value,
    detachedOwner: detachedAttr && detachedAttr.ownerElement === parsed.getElementById("p")
  });
})()
"#,
        )
        .expect("XPath attribute node results should evaluate");

    assert_eq!(
        result,
        r#"{"liveName":"data-live","liveValue":"yes","liveOwner":true,"detachedName":"data-detached","detachedValue":"yes","detachedOwner":true}"#
    );
}

#[test]
fn document_xpath_lang_uses_namespaced_attribute_local_names() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-lang.test/path/index.html",
        r#"<html><body></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const parsed = new DOMParser().parseFromString(
    "<root xml:lang='ja'><inherited/><specific xml:lang='en-US'/></root>",
    "text/xml"
  );
  const inherited = parsed.documentElement.firstChild;
  const specific = inherited.nextSibling;
  const evaluateLang = (expression, node) => parsed.evaluate(
    expression,
    node,
    null,
    XPathResult.BOOLEAN_TYPE
  ).booleanValue;

  return JSON.stringify({
    inheritedJapanese: evaluateLang('lang("ja")', inherited),
    specificEnglish: evaluateLang('lang("en")', specific),
    specificOverridesJapanese: evaluateLang('lang("ja")', specific)
  });
})()
"#,
        )
        .expect("XPath lang() should inspect namespaced attribute local names");

    assert_eq!(
        result,
        r#"{"inheritedJapanese":true,"specificEnglish":true,"specificOverridesJapanese":false}"#
    );
}

#[test]
fn document_xpath_uses_namespace_resolver_callbacks() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-namespace-resolver.test/path/index.html",
        r#"<html><body><svg id="liveSvg"><g id="liveGroup"></g></svg></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const svgNs = "http://www.w3.org/2000/svg";
  const liveByFunction = document.evaluate(
    "//svg:svg",
    document,
    prefix => prefix === "svg" ? svgNs : null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  const parsed = new DOMParser().parseFromString(
    "<svg xmlns='http://www.w3.org/2000/svg' id='detachedSvg'><g id='detachedGroup'/></svg>",
    "image/svg+xml"
  );
  const detachedByObject = parsed.evaluate(
    "//svg:g",
    parsed,
    { lookupNamespaceURI(prefix) { return prefix === "svg" ? svgNs : null; } },
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const callError = { kind: "call" };
  const getError = { kind: "get" };
  const coercionError = { kind: "coercion" };
  const reported = [];
  const reportLabels = new Map([
    [callError, "call"],
    [getError, "get"],
    [coercionError, "coercion"]
  ]);
  const onError = event => {
    reported.push(reportLabels.get(event.error) || (event.error && event.error.name));
    event.preventDefault();
  };
  window.addEventListener("error", onError);

  const unresolvedPrefix = probe(() =>
    document.evaluate("//missing:svg", document, () => null)
  );
  const resolverCallException = probe(() =>
    document.evaluate("//svg:svg", document, () => { throw callError; })
  );
  const resolverGetException = probe(() =>
    document.evaluate("//svg:svg", document, {
      get lookupNamespaceURI() { throw getError; }
    })
  );
  const truthyNonCallable = probe(() =>
    document.evaluate("//svg:svg", document, { lookupNamespaceURI: {} })
  );
  const falsyNonCallable = probe(() =>
    document.evaluate("//svg:svg", document, {})
  );
  const undefinedResult = probe(() =>
    document.evaluate("//svg:svg", document, () => undefined)
  );
  const nullResult = probe(() =>
    document.evaluate("//svg:svg", document, () => null)
  );
  const numberResult = probe(() =>
    document.evaluate("//svg:svg", document, () => 0)
  );
  const booleanResult = probe(() =>
    document.evaluate("//svg:svg", document, () => false)
  );
  const symbolResult = probe(() =>
    document.evaluate("//svg:svg", document, () => Symbol())
  );
  const coercionException = probe(() =>
    document.evaluate("//svg:svg", document, () => ({
      toString() { throw coercionError; },
      valueOf() { throw new Error("valueOf must not be called"); }
    }))
  );
  const detachedUnresolvedPrefix = probe(() =>
    parsed.evaluate("//missing:svg", parsed, null)
  );
  const invalidResolver = probe(() =>
    document.evaluate("//svg:svg", document, 1)
  );
  window.removeEventListener("error", onError);

  return JSON.stringify({
    liveId: liveByFunction && liveByFunction.id,
    detachedId: detachedByObject && detachedByObject.id,
    unresolvedPrefix,
    resolverCallException,
    resolverGetException,
    truthyNonCallable,
    falsyNonCallable,
    undefinedResult,
    nullResult,
    numberResult,
    booleanResult,
    symbolResult,
    coercionException,
    detachedUnresolvedPrefix,
    invalidResolver,
    reported
  });
})()
"#,
        )
        .expect("XPath namespace resolver callbacks should evaluate");

    assert_eq!(
        result,
        r#"{"liveId":"liveSvg","detachedId":"detachedGroup","unresolvedPrefix":"throw:NamespaceError","resolverCallException":"throw:NamespaceError","resolverGetException":"throw:NamespaceError","truthyNonCallable":"throw:NamespaceError","falsyNonCallable":"throw:NamespaceError","undefinedResult":"throw:NamespaceError","nullResult":"throw:NamespaceError","numberResult":"ok","booleanResult":"ok","symbolResult":"throw:NamespaceError","coercionException":"throw:NamespaceError","detachedUnresolvedPrefix":"throw:NamespaceError","invalidResolver":"throw:TypeError","reported":["call","get","TypeError","TypeError","TypeError","coercion"]}"#
    );
}

#[test]
fn document_xpath_resolver_uses_webidl_callback_interface_semantics() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-resolver-callback-interface.test/",
        r#"<html><body><svg id="target"></svg></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const svgNs = "http://www.w3.org/2000/svg";
  const expression = "//svg:svg";

  let callableThis = "unset";
  let callableCalls = 0;
  let forbiddenOperationGets = 0;
  function callableResolver(prefix) {
    "use strict";
    callableThis = this;
    callableCalls++;
    return prefix === "svg" ? svgNs : null;
  }
  Object.defineProperty(callableResolver, "lookupNamespaceURI", {
    get() {
      forbiddenOperationGets++;
      throw new Error("the callable branch must not look up the operation");
    }
  });
  const callableResult = document.evaluate(
    expression,
    document,
    callableResolver,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  let operationGets = 0;
  let operationCalls = 0;
  let objectReceivers = 0;
  const objectResolver = {
    get lookupNamespaceURI() {
      operationGets++;
      return function(prefix) {
        operationCalls++;
        objectReceivers += this === objectResolver;
        return prefix === "svg" ? svgNs : null;
      };
    }
  };
  document.evaluate(expression, document, objectResolver);
  document.evaluate(expression, document, objectResolver);

  const replaceableResolver = {
    lookupNamespaceURI() {
      return null;
    }
  };
  let beforeReplacement;
  try {
    document.evaluate(expression, document, replaceableResolver);
    beforeReplacement = "ok";
  } catch (error) {
    beforeReplacement = error.name;
  }
  replaceableResolver.lookupNamespaceURI = () => svgNs;
  const afterReplacement = document.evaluate(
    expression,
    document,
    replaceableResolver,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  const revocable = Proxy.revocable(() => svgNs, {});
  const proxyBeforeRevoke = document.evaluate(
    expression,
    document,
    revocable.proxy,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;
  revocable.revoke();
  let reportedRevokedTypeError = false;
  const onError = event => {
    reportedRevokedTypeError = event.error instanceof TypeError;
    event.preventDefault();
  };
  window.addEventListener("error", onError);
  let revokedResult;
  try {
    document.evaluate(expression, document, revocable.proxy);
    revokedResult = "ok";
  } catch (error) {
    revokedResult = error.name;
  }
  window.removeEventListener("error", onError);

  return JSON.stringify({
    callableId: callableResult && callableResult.id,
    callableThisIsUndefined: callableThis === undefined,
    callableCalls,
    forbiddenOperationGets,
    operationGets,
    operationCalls,
    objectReceivers,
    beforeReplacement,
    afterReplacementId: afterReplacement && afterReplacement.id,
    proxyBeforeRevokeId: proxyBeforeRevoke && proxyBeforeRevoke.id,
    revokedResult,
    reportedRevokedTypeError
  });
})()
"#,
        )
        .expect("XPath callback-interface invocation semantics should evaluate");

    assert_eq!(
        result,
        r#"{"callableId":"target","callableThisIsUndefined":true,"callableCalls":1,"forbiddenOperationGets":0,"operationGets":2,"operationCalls":2,"objectReceivers":2,"beforeReplacement":"NamespaceError","afterReplacementId":"target","proxyBeforeRevokeId":"target","revokedResult":"NamespaceError","reportedRevokedTypeError":true}"#
    );
}

#[test]
fn document_xpath_resolver_uses_callback_relevant_realm() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-resolver-realm.test/",
        r#"<html><body><svg id="target"></svg></body></html>"#,
    );

    vm.eval(
        r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.srcdoc = "<!doctype html><html><body></body></html>";
  document.body.appendChild(iframe);
  globalThis.__xpathResolverRealmFrame = iframe;
  return "ready";
})()
"#,
    )
    .expect("cross-Realm XPath resolver setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const other = globalThis.__xpathResolverRealmFrame.contentWindow;
  const expression = "//svg:svg";
  const missingOperation = new other.Object();
  let reported = null;
  const onError = event => {
    reported = {
      relevantTypeError:
        event.error instanceof other.TypeError &&
        !(event.error instanceof TypeError),
      targetIsResolverWindow: event.currentTarget === other
    };
    event.preventDefault();
  };
  other.addEventListener("error", onError);
  let evaluationFailure;
  try {
    document.evaluate(expression, document, missingOperation);
    evaluationFailure = "ok";
  } catch (error) {
    evaluationFailure = {
      name: error.name,
      evaluatorRealm:
        error instanceof DOMException &&
        !(error instanceof other.DOMException)
    };
  }
  other.removeEventListener("error", onError);

  globalThis.__xpathResolverExpectedRealm = other;
  globalThis.__xpathResolverCallFacts = [];
  const crossRealmCallable = other.Function(
    "prefix",
    `"use strict";
     parent.__xpathResolverCallFacts = [
       this === undefined,
       globalThis === parent.__xpathResolverExpectedRealm,
       prefix
     ];
     return "http://www.w3.org/2000/svg";`
  );
  const resolved = document.evaluate(
    expression,
    document,
    crossRealmCallable,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  return JSON.stringify({
    evaluationFailure,
    reported,
    resolvedId: resolved && resolved.id,
    callFacts: globalThis.__xpathResolverCallFacts
  });
})()
"#,
        )
        .expect("cross-Realm XPath resolver invocation should evaluate");

    assert_eq!(
        result,
        r#"{"evaluationFailure":{"name":"NamespaceError","evaluatorRealm":true},"reported":{"relevantTypeError":true,"targetIsResolverWindow":true},"resolvedId":"target","callFacts":[true,true,"svg"]}"#
    );
}

#[test]
fn dom_selector_queries_parse_webidl_strings() {
    let mut vm = new_parsed_test_vm(
        "https://dom-selector-webidl.test/path/index.html",
        r#"<html><body>
            <section id="root">
              <div id="target" class="alpha beta" name="box"><span class="alpha"></span></div>
              <input id="field" name="field">
            </section>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r##"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      if (value && value.id) return value.id;
      if (value && typeof value.length === "number") return `length:${value.length}`;
      return String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const target = document.getElementById("target");
  const parsed = new DOMParser().parseFromString(
    "<main><p id='p' class='x'><span class='x'></span></p></main>",
    "text/html"
  );
  const detachedP = parsed.getElementById("p");
  return JSON.stringify({
    idObject: document.getElementById({ toString() { return "target"; } }).id,
    selectorObject: document.querySelector({ toString() { return "#target"; } }).id,
    selectorAllObject: document.querySelectorAll({ toString() { return ".alpha"; } }).length,
    matchesObject: target.matches({ toString() { return ".alpha"; } }),
    closestObject: target.closest({ toString() { return "section"; } }).id,
    tagObject: document.getElementsByTagName({ toString() { return "div"; } }).length,
    classObject: document.getElementsByClassName({ toString() { return "alpha"; } }).length,
    nameObject: document.getElementsByName({ toString() { return "field"; } })[0].id,
    tagNsObject: document.getElementsByTagNameNS(
      { toString() { return "*"; } },
      { toString() { return "div"; } }
    ).length,
    missingId: probe(() => document.getElementById()),
    symbolId: probe(() => document.getElementById(Symbol())),
    missingSelector: probe(() => document.querySelector()),
    symbolSelectorAll: probe(() => document.querySelectorAll(Symbol())),
    missingMatches: probe(() => target.matches()),
    symbolClosest: probe(() => target.closest(Symbol())),
    missingTag: probe(() => document.getElementsByTagName()),
    symbolClass: probe(() => document.getElementsByClassName(Symbol())),
    symbolName: probe(() => document.getElementsByName(Symbol())),
    missingNsLocal: probe(() => document.getElementsByTagNameNS("*")),
    symbolNs: probe(() => document.getElementsByTagNameNS(Symbol(), "div")),
    detachedIdObject: parsed.getElementById({ toString() { return "p"; } }).id,
    detachedSelectorObject: parsed.querySelector({ toString() { return "#p"; } }).id,
    detachedSelectorAllObject: detachedP.querySelectorAll({ toString() { return ".x"; } }).length,
    detachedTagObject: detachedP.getElementsByTagName({ toString() { return "span"; } }).length,
    detachedMissingId: probe(() => parsed.getElementById()),
    detachedSymbolSelector: probe(() => parsed.querySelector(Symbol())),
    detachedMissingSelectorAll: probe(() => detachedP.querySelectorAll()),
    detachedSymbolTag: probe(() => detachedP.getElementsByTagName(Symbol()))
  });
})()
"##,
        )
        .expect("DOM selector WebIDL probe should evaluate");

    assert_eq!(
        result,
        r#"{"idObject":"target","selectorObject":"target","selectorAllObject":2,"matchesObject":true,"closestObject":"root","tagObject":1,"classObject":2,"nameObject":"field","tagNsObject":1,"missingId":"throw:TypeError","symbolId":"throw:TypeError","missingSelector":"throw:TypeError","symbolSelectorAll":"throw:TypeError","missingMatches":"throw:TypeError","symbolClosest":"throw:TypeError","missingTag":"throw:TypeError","symbolClass":"throw:TypeError","symbolName":"throw:TypeError","missingNsLocal":"throw:TypeError","symbolNs":"throw:TypeError","detachedIdObject":"p","detachedSelectorObject":"p","detachedSelectorAllObject":1,"detachedTagObject":1,"detachedMissingId":"throw:TypeError","detachedSymbolSelector":"throw:TypeError","detachedMissingSelectorAll":"throw:TypeError","detachedSymbolTag":"throw:TypeError"}"#
    );
}

#[test]
fn dom_selector_id_escapes_do_not_match_lone_surrogate_ids() {
    let mut vm = new_parsed_test_vm(
        "https://dom-selector-surrogate-escapes.test/path/index.html",
        r#"<html><body></body></html>"#,
    );

    let result = vm
        .eval(
            r##"
(() => {
  const container = document.createElement("div");
  document.body.appendChild(container);

  const replacementFirst = document.createElement("span");
  replacementFirst.id = "\u{fffd}surrogateFirst";
  container.appendChild(replacementFirst);

  const surrogateFirst = document.createElement("span");
  surrogateFirst.id = "\ud83dsurrogateFirst";
  container.appendChild(surrogateFirst);

  const replacementSecond = document.createElement("span");
  replacementSecond.id = "surrogateSecond\u{fffd}";
  container.appendChild(replacementSecond);

  const surrogateSecond = document.createElement("span");
  surrogateSecond.id = "surrogateSecond\udd11";
  container.appendChild(surrogateSecond);

  return JSON.stringify({
    escapedHighMatchesReplacement: container.querySelector("#\\d83d surrogateFirst") === replacementFirst,
    escapedHighDoesNotMatchSurrogate: container.querySelector("#\\d83d surrogateFirst") !== surrogateFirst,
    escapedLowMatchesReplacement: container.querySelector("#surrogateSecond\\dd11") === replacementSecond,
    escapedLowDoesNotMatchSurrogate: container.querySelector("#surrogateSecond\\dd11") !== surrogateSecond
  });
})()
"##,
        )
        .expect("surrogate selector probe should evaluate");

    assert_eq!(
        result,
        r#"{"escapedHighMatchesReplacement":true,"escapedHighDoesNotMatchSurrogate":true,"escapedLowMatchesReplacement":true,"escapedLowDoesNotMatchSurrogate":true}"#
    );
}

#[test]
fn get_elements_by_tag_name_ns_matches_null_namespace_elements() {
    let mut vm = new_parsed_test_vm(
        "https://tag-name-ns-null.test/path/index.html",
        r#"<html><body><section id="root"></section></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.getElementById("root");
  const nullNs = document.createElementNS(null, "widget");
  nullNs.id = "nullNs";
  const htmlWidget = document.createElement("widget");
  htmlWidget.id = "htmlWidget";
  root.append(nullNs, htmlWidget);

  const xml = document.implementation.createDocument(null, "root");
  const detachedNullNs = xml.createElementNS(null, "widget");
  detachedNullNs.setAttribute("id", "detachedNullNs");
  const detachedHtmlNs = xml.createElementNS("http://www.w3.org/1999/xhtml", "widget");
  detachedHtmlNs.setAttribute("id", "detachedHtmlNs");
  xml.documentElement.append(detachedNullNs, detachedHtmlNs);

  const ids = collection => Array.from(collection).map(node => node.id).join(",");
  return JSON.stringify({
    liveNullNs: ids(document.getElementsByTagNameNS(null, "widget")),
    liveEmptyNs: ids(document.getElementsByTagNameNS("", "widget")),
    liveWildcardNs: ids(document.getElementsByTagNameNS("*", "widget")),
    liveElementNullNs: ids(root.getElementsByTagNameNS(null, "widget")),
    liveCaseSensitive: document.getElementsByTagNameNS(null, "WIDGET").length,
    detachedNullNs: ids(xml.getElementsByTagNameNS(null, "widget")),
    detachedEmptyNs: ids(xml.getElementsByTagNameNS("", "widget")),
    detachedWildcardNs: ids(xml.getElementsByTagNameNS("*", "widget")),
    detachedElementNullNs: ids(xml.documentElement.getElementsByTagNameNS(null, "widget")),
    detachedCaseSensitive: xml.getElementsByTagNameNS(null, "WIDGET").length
  });
})()
"#,
        )
        .expect("null namespace getElementsByTagNameNS probe should evaluate");

    assert_eq!(
        result,
        r#"{"liveNullNs":"nullNs","liveEmptyNs":"nullNs","liveWildcardNs":"nullNs,htmlWidget","liveElementNullNs":"nullNs","liveCaseSensitive":0,"detachedNullNs":"detachedNullNs","detachedEmptyNs":"detachedNullNs","detachedWildcardNs":"detachedNullNs,detachedHtmlNs","detachedElementNullNs":"detachedNullNs","detachedCaseSensitive":0}"#
    );
}
