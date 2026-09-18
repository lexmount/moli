(() => {
  const html = "http://www.w3.org/1999/xhtml";
  const svg = "http://www.w3.org/2000/svg";
  const parser = new DOMParser();
  const contentType = Object.getOwnPropertyDescriptor(Document.prototype, "contentType").get;
  const factories = [
    ["HTML implementation", () => document.implementation.createHTMLDocument(""), "text/html"],
    ["HTML parser", () => parser.parseFromString("", "text/html"), "text/html"],
    ["XHTML no namespace", () => parser.parseFromString("<html/>", "application/xhtml+xml"), "application/xhtml+xml"],
    ["XHTML SVG root", () => parser.parseFromString('<svg xmlns="' + svg + '"/>', "application/xhtml+xml"), "application/xhtml+xml"],
    ["XHTML parser error", () => parser.parseFromString("", "application/xhtml+xml"), "application/xhtml+xml"],
    ["XML XHTML root", () => parser.parseFromString('<html xmlns="' + html + '"/>', "application/xml"), "application/xml"],
    ["text XML XHTML root", () => parser.parseFromString('<html xmlns="' + html + '"/>', "text/xml"), "text/xml"],
    ["SVG XHTML root", () => parser.parseFromString('<html xmlns="' + html + '"/>', "image/svg+xml"), "image/svg+xml"],
    ["XHTML implementation", () => document.implementation.createDocument(html, "html"), "application/xhtml+xml"],
    ["rootless XHTML implementation", () => document.implementation.createDocument(html, ""), "application/xhtml+xml"],
    ["SVG implementation", () => document.implementation.createDocument(svg, "svg"), "image/svg+xml"],
    ["XML implementation", () => document.implementation.createDocument(null, "root"), "application/xml"],
    ["Document constructor", () => new Document(), "application/xml"],
  ];
  const failures = [];
  let checks = 0;
  function equal(label, actual, expected) {
    checks++;
    if (actual !== expected) failures.push({label, actual, expected});
  }
  for (const [label, factory, type] of factories) {
    const doc = factory();
    const namespace = type === "text/html" || type === "application/xhtml+xml" ? html : null;
    function check(phase) {
      const prefix = label + ": " + phase;
      equal(prefix + " contentType", contentType.call(doc), type);
      const node = doc.createElement("MiXeD");
      equal(prefix + " namespace", node.namespaceURI, namespace);
      equal(prefix + " name", node.localName, type === "text/html" ? "mixed" : "MiXeD");
      equal(prefix + " owner", node.ownerDocument === doc, true);
      equal(prefix + " null namespace", doc.createElementNS(null, "MiXeD").namespaceURI, null);
      equal(prefix + " empty namespace", doc.createElementNS("", "MiXeD").namespaceURI, null);
      equal(prefix + " SVG namespace", doc.createElementNS(svg, "g").namespaceURI, svg);
      equal(prefix + " borrowed method", Document.prototype.createElement.call(doc, "x").namespaceURI, namespace);
    }
    check("initial");
    if (doc.documentElement) doc.removeChild(doc.documentElement);
    check("root removed");
    const root = doc.createElementNS(html, "html");
    doc.appendChild(root);
    check("HTML root");
    doc.replaceChild(doc.createElementNS(svg, "svg"), root);
    check("SVG root");
    function checkClones(phase) {
      for (const deep of [false, true]) {
        const clone = doc.cloneNode(deep);
        const node = clone.createElement("MiXeD");
        equal(label + ": " + phase + " clone " + deep + " contentType", clone.contentType, type);
        equal(label + ": " + phase + " clone " + deep + " namespace", node.namespaceURI, namespace);
        equal(label + ": " + phase + " clone " + deep + " owner", node.ownerDocument === clone, true);
      }
    }
    checkClones("SVG root");
    let reads = 0;
    for (const property of ["contentType", "documentElement"]) {
      Object.defineProperty(doc, property, {
        get() { reads++; return property === "contentType" ? "tampered/type" : null; }
      });
    }
    check("public getters replaced");
    checkClones("public getters replaced");
    equal(label + ": no public getter reads", reads, 0);
  }
  return {checks, failures};
})()
