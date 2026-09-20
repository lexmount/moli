(() => {
  const parser = new DOMParser();
  const html = () => document.implementation.createHTMLDocument("source title");
  const factories = [
    ["HTML implementation", html],
    ["HTML parser", () => parser.parseFromString("<!doctype html><title>Source</title><p>text</p>", "text/html")],
    ["empty HTML", () => { const doc = html(); doc.replaceChildren(); return doc; }],
    ["custom HTML root", () => { const doc = html(); doc.replaceChild(doc.createElement("section"), doc.documentElement); return doc; }],
    ["doctype and comment", () => { const doc = html(); doc.documentElement.remove(); doc.append(doc.createComment("tail")); return doc; }],
    ["XHTML implementation", () => document.implementation.createDocument("http://www.w3.org/1999/xhtml", "html")],
    ["XML parser", () => parser.parseFromString("<root><child>text</child></root>", "application/xml")],
  ];
  let checks = 0;
  const failures = [];
  function equal(label, actual, expected) {
    checks++;
    if (actual !== expected) failures.push({label, actual, expected});
  }
  for (const [name, factory] of factories) {
    const source = factory();
    const original = Array.from(source.childNodes);
    for (const [mode, clone] of [
      ["omitted", () => source.cloneNode()],
      ["false", () => source.cloneNode(false)],
      ["undefined", () => source.cloneNode(undefined)],
    ]) {
      const copy = clone();
      const label = name + ": " + mode;
      equal(label + " children", copy.childNodes.length, 0);
      equal(label + " root", copy.documentElement, null);
      equal(label + " doctype", copy.doctype, null);
      equal(label + " HTML structure", copy.head === null && copy.body === null, true);
      equal(label + " owner", copy.ownerDocument, null);
      equal(label + " view", copy.defaultView, null);
      const node = copy.createElement("probe");
      let error = null;
      try { copy.appendChild(node); } catch (exception) { error = exception.name; }
      equal(label + " append error", error, null);
      equal(label + " appended count", copy.childNodes.length, 1);
      equal(label + " appended identity", copy.firstChild === node, true);
      equal(label + " appended owner", node.ownerDocument === copy, true);
    }
    const deep = source.cloneNode(true);
    equal(name + ": deep identity", deep !== source, true);
    equal(name + ": deep equality", deep.isEqualNode(source), true);
    equal(name + ": deep count", deep.childNodes.length, original.length);
    equal(name + ": deep children", Array.from(deep.childNodes).every((child, i) => child !== original[i] && child.ownerDocument === deep), true);
    equal(name + ": source unchanged", source.childNodes.length === original.length && original.every((child, i) => child === source.childNodes[i] && child.parentNode === source), true);
  }
  return {checks, failures};
})()
