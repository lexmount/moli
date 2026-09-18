(extraSources = []) => {
  const parser = new DOMParser();
  const html = markup => parser.parseFromString(markup, "text/html");
  const sources = [
    ["live HTML", document],
    ["standards HTML", html("<!doctype html><p>text</p>")],
    ["quirks HTML", html("<p>text</p>")],
    ["limited quirks HTML", html('<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Transitional//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd"><p>text</p>')],
    ["XHTML", parser.parseFromString('<html xmlns="http://www.w3.org/1999/xhtml"><body/></html>', "application/xhtml+xml")],
    ["XML", parser.parseFromString("<root/>", "application/xml")],
    ["SVG", parser.parseFromString('<svg xmlns="http://www.w3.org/2000/svg"/>', "image/svg+xml")],
    ["HTML implementation", document.implementation.createHTMLDocument("")],
    ["XML implementation", document.implementation.createDocument("urn:test", "root")],
    ...extraSources,
  ];
  const failures = [];
  let checks = 0;
  function equal(label, actual, expected) {
    checks++;
    if (actual !== expected) failures.push({label, actual, expected});
  }
  function relativeHref(doc, value = "assets/item?x=1#tail") {
    const link = doc.createElementNS("http://www.w3.org/1999/xhtml", "a");
    link.setAttribute("href", value);
    return link.href;
  }
  const properties = ["URL", "documentURI", "compatMode", "characterSet", "charset", "inputEncoding", "contentType"];
  for (const [name, source] of sources) {
    const expected = Object.fromEntries(properties.map(key => [key, source[key]]));
    const href = relativeHref(source);
    for (const deep of [false, true]) {
      const clone = source.cloneNode(deep);
      const repeated = clone.cloneNode(deep);
      for (const [generation, copy] of [["clone", clone], ["reclone", repeated]]) {
        const label = name + ": " + deep + " " + generation;
        for (const key of properties) equal(label + " " + key, copy[key], expected[key]);
        equal(label + " relative URL", relativeHref(copy), href);
        equal(label + " encoded URL query", relativeHref(copy, "https://clone-query.test/?ä"),
          "https://clone-query.test/?" + (expected.characterSet === "windows-1252" ? "%E4" : "%C3%A4"));
        equal(label + " no browsing context", copy.defaultView, null);
        equal(label + " independent identity", copy !== source && copy.ownerDocument === null, true);
        copy.replaceChildren();
        const node = copy.createElementNS("http://www.w3.org/1999/xhtml", "section");
        node.className = "CaSe";
        copy.appendChild(node);
        equal(label + " class collection mode", copy.getElementsByClassName("case").length, expected.compatMode === "BackCompat" ? 1 : 0);
        equal(label + " selector mode", copy.querySelector(".case") === node, expected.compatMode === "BackCompat");
      }
    }
    // Clone metadata is internal state: author properties must not be consulted.
    if (source !== document) {
      let reads = 0;
      for (const key of [...properties, "baseURI"]) {
        Object.defineProperty(source, key, {
          configurable: true,
          get() { reads++; throw new Error("public metadata getter: " + key); },
        });
      }
      for (const deep of [false, true]) {
        try {
          const copy = source.cloneNode(deep);
          for (const key of properties) equal(name + ": tampered " + deep + " " + key, copy[key], expected[key]);
        } catch (error) {
          equal(name + ": tampered clone error", error.message, null);
        }
      }
      equal(name + ": public getter reads", reads, 0);
    }
  }

  const source = html('<!doctype html><base href="https://clone-base.test/first/"><a href="item">link</a>');
  const shallow = source.cloneNode(false);
  const deep = source.cloneNode(true);
  equal("shallow base uses document URL", shallow.baseURI, source.URL);
  equal("shallow relative URL", relativeHref(shallow), new URL("assets/item?x=1#tail", source.URL).href);
  equal("deep base from copied element", deep.baseURI, "https://clone-base.test/first/");
  equal("deep relative URL", deep.querySelector("a").href, "https://clone-base.test/first/item");
  source.querySelector("base").href = "https://clone-base.test/second/";
  equal("cloned base remains independent", deep.baseURI, "https://clone-base.test/first/");
  deep.querySelector("base").remove();
  equal("removed base falls back to cloned URL", deep.baseURI, source.URL);
  equal("removed base changes relative resolution", deep.querySelector("a").href, new URL("item", source.URL).href);
  return {checks, failures};
}
