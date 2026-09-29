(() => {
  const check = (value, message) => { if (!value) throw new Error(message); };
  const same = (actual, expected) => actual.length === expected.length &&
    expected.every((node, index) => actual[index] === node);
  const frame = globalThis.__observerCallbackFrame || document.querySelector("iframe");
  const documents = [
    ["main", document],
    ["iframe", frame.contentDocument],
    ["HTML", document.implementation.createHTMLDocument("")],
    ["XML", document.implementation.createDocument(null, "root")],
    ["parsed HTML", new DOMParser().parseFromString("<body></body>", "text/html")],
    ["parsed XML", new DOMParser().parseFromString("<root/>", "application/xml")],
  ];
  const failures = [];
  let cases = 0;
  function run(label, callback) {
    cases++;
    try { callback(); } catch (error) { failures.push(label + ": " + error.message); }
  }
  function record(actual, target, added, removed, previous, next) {
    check(actual && actual.type === "childList" && actual.target === target, "record target");
    check(same(actual.addedNodes, added) && same(actual.removedNodes, removed), "record nodes");
    check(actual.previousSibling === previous && actual.nextSibling === next, "record siblings");
  }
  const replace = (parent, borrowed, ...args) => borrowed ?
    Node.prototype.replaceChild.call(parent, ...args) : parent.replaceChild(...args);
  for (const [name, owner] of documents) {
    for (const borrowed of [false, true]) {
      for (const location of ["detached", "attached", "fragment"]) {
        for (const kind of ["new", "other parent", "self", "previous", "next", "empty", "fragment", "foreign node", "foreign fragment"]) {
          run([name, borrowed, location, kind].join("/"), () => {
            const parent = location === "fragment" ? owner.createDocumentFragment() : owner.createElement("parent");
            const left = owner.createElement("left"), old = owner.createElement("old"), right = owner.createElement("right");
            const text = old.appendChild(owner.createTextNode("old contents"));
            parent.append(left, old, right);
            if (location === "attached") (owner.body || owner.documentElement).appendChild(parent);
            const foreign = kind.startsWith("foreign");
            const source = foreign ? (owner === document ? document.implementation.createHTMLDocument("") : document) : owner;
            const sourceParent = source.createElement("source");
            const input = kind === "self" ? old : kind === "previous" ? left : kind === "next" ? right :
              ["empty", "fragment", "foreign fragment"].includes(kind) ? source.createDocumentFragment() : source.createElement("input");
            const fragment = input.nodeType === 11;
            if (fragment && kind !== "empty") input.append(source.createElement("one"), source.createTextNode("two"));
            if (kind === "other parent" || kind === "foreign node") sourceParent.appendChild(input);
            const added = fragment ? [...input.childNodes] : [input];
            const cachedChildren = parent.childNodes;
            const fragmentChildren = fragment ? input.childNodes : null;
            const range = owner.createRange(); range.selectNodeContents(text);
            const iterator = owner.createNodeIterator(parent, NodeFilter.SHOW_ALL);
            check(iterator.nextNode() === parent && iterator.nextNode() === left && iterator.nextNode() === old, "iterator setup");
            const xpath = location === "fragment" ? null : owner.evaluate("*", parent, null, XPathResult.UNORDERED_NODE_ITERATOR_TYPE, null);
            const observer = new MutationObserver(() => {});
            observer.observe(parent, {childList: true});
            observer.observe(sourceParent, {childList: true});
            if (fragment) observer.observe(input, {childList: true});
            try {
              const marker = owner.createComment("prior");
              parent.appendChild(marker); parent.removeChild(marker);
              check(replace(parent, borrowed, input, old) === old, "return original old child");
              const records = observer.takeRecords();
              const hasSourceRemoval = ["other parent", "foreign node", "self", "previous", "next"].includes(kind) || added.length > 0 && fragment;
              check(records.length === 3 + Number(hasSourceRemoval), "record count " + records.length);
              record(records[0], parent, [marker], [], right, null);
              record(records[1], parent, [], [marker], right, null);
              let index = 2;
              if (kind === "other parent" || kind === "foreign node") record(records[index++], sourceParent, [], [input], null, null);
              else if (kind === "self") record(records[index++], parent, [], [old], left, right);
              else if (kind === "previous") record(records[index++], parent, [], [left], null, old);
              else if (kind === "next") record(records[index++], parent, [], [right], old, null);
              else if (fragment && added.length) record(records[index++], input, [], added, null, null);
              // DOM replace snapshots previousSibling before adopting its input.
              record(records[index], parent, added, kind === "self" ? [] : [old], left, kind === "next" ? null : right);
              const expected = [...(kind === "previous" ? [] : [left]), ...added, ...(kind === "next" ? [] : [right])];
              check(same(cachedChildren, expected), "cached childNodes reflect replacement");
              check(old.parentNode === (kind === "self" ? parent : null), "old child parent");
              check(added.every(node => node.parentNode === parent && node.ownerDocument === owner), "inserted node parent and document");
              if (fragment) {
                check(fragmentChildren.length === 0, "fragment emptied");
                check(input.ownerDocument === owner, "replace adopts the fragment itself");
              }
              const offset = kind === "previous" ? 0 : 1;
              check(range.startContainer === parent && range.endContainer === parent &&
                range.startOffset === offset && range.endOffset === offset, "range follows removal");
              check(iterator.nextNode() === (added[0] || right), "iterator follows removal");
              if (xpath) check(xpath.invalidIteratorState, "XPath iterator invalidated");
              if (added.length) {
                parent.removeChild(added[0]);
                check(same(records[index].addedNodes, added), "record retains static nodes");
              }
            } finally {
              observer.disconnect();
              if (parent.parentNode) parent.parentNode.removeChild(parent);
            }
          });
        }
      }
      run([name, borrowed, "validation precedes mutation"].join("/"), () => {
        const parent = owner.createElement("parent"), old = parent.appendChild(owner.createElement("old"));
        const source = document.implementation.createHTMLDocument("");
        const input = source.body.appendChild(source.createElement("input"));
        const observer = new MutationObserver(() => {});
        observer.observe(parent, {childList: true, subtree: true});
        observer.observe(source.body, {childList: true});
        try {
          for (const [target, args, expected] of [
            [parent, [input, owner.createComment("missing")], "NotFoundError"],
            [parent, [parent, old], "HierarchyRequestError"],
            [old.firstChild || owner.createTextNode("leaf"), [input, old], "HierarchyRequestError"],
            [parent, [input, null], "TypeError"],
            [parent, [null, old], "TypeError"],
            [parent, [input], "TypeError"],
          ]) {
            let error;
            try { replace(target, borrowed, ...args); } catch (caught) { error = caught; }
            check(error && error.name === expected, "validation error " + expected);
            check(parent.firstChild === old && old.parentNode === parent, "target not mutated");
            check(input.parentNode === source.body && input.ownerDocument === source, "input not removed or adopted");
            check(observer.takeRecords().length === 0, "failed replacement queues no records");
          }
        } finally { observer.disconnect(); }
      });
      for (const kind of ["duck", "proxy", "inherited", "revoked"]) {
        for (const parameter of ["new", "old"]) {
          run([name, borrowed, "Node brand", kind, parameter].join("/"), () => {
            const parent = owner.createElement("parent"), old = parent.appendChild(owner.createElement("old"));
            const input = owner.createElement("select");
            let reads = 0;
            const revoked = Proxy.revocable(input, {}); revoked.revoke();
            const invalid = kind === "duck" ? {get nodeType() { reads++; return 3; }, data: "fake"} :
              kind === "proxy" ? new Proxy(input, {get(target, key, receiver) { reads++; return Reflect.get(target, key, receiver); }}) :
              kind === "inherited" ? Object.create(input) : revoked.proxy;
            const observer = new MutationObserver(() => {}); observer.observe(parent, {childList: true});
            try {
              let error;
              try { replace(parent, borrowed, parameter === "new" ? invalid : input, parameter === "old" ? invalid : old); }
              catch (caught) { error = caught; }
              check(error && error.name === "TypeError", "unbranded Node argument rejected");
              check(reads === 0, "argument conversion must not read author properties");
              check(parent.firstChild === old && input.parentNode === null, "unbranded input cannot mutate the tree");
              check(observer.takeRecords().length === 0, "unbranded input queues no records");
            } finally { observer.disconnect(); }
          });
        }
      }
      run([name, borrowed, "genuine native select wrapper"].join("/"), () => {
        // Windowless select elements use Moli's registered native Proxy.
        const html = localName => owner.createElementNS("http://www.w3.org/1999/xhtml", localName);
        const parent = owner.createElement("parent"), old = parent.appendChild(html("select"));
        const input = html("select"); input.appendChild(html("option"));
        check(replace(parent, borrowed, input, old) === old, "return native wrapper");
        check(parent.firstChild === input && input.options.length === 1, "accept native brands");
      });
      run([name, borrowed, "IDL validation before hierarchy"].join("/"), () => {
        const parent = owner.createElement("parent");
        let error;
        try { replace(parent, borrowed, parent, null); } catch (caught) { error = caught; }
        check(error && error.name === "TypeError", "convert second argument before ancestor check");
      });
      if (name === "main" || name === "iframe") {
        for (const method of ["appendChild", "insertBefore"]) {
          run([name, borrowed, method, "shared argument validation"].join("/"), () => {
            const parent = owner.createElement("parent"), child = parent.appendChild(owner.createComment("child"));
            const input = owner.createElement("input");
            const call = (target, args) => borrowed ? Node.prototype[method].call(target, ...args) : target[method](...args);
            let reads = 0;
            const revoked = Proxy.revocable(input, {}); revoked.revoke();
            const invalid = [
              {get nodeType() { reads++; return 3; }, data: "fake"},
              Object.create(input), revoked.proxy,
              new Proxy(input, {get() { reads++; throw new Error("proxy trap"); }}),
            ];
            const observer = new MutationObserver(() => {}); observer.observe(parent, {childList: true});
            try {
              const reject = (target, args, expected) => {
                let error;
                try { call(target, args); } catch (caught) { error = caught; }
                check(error && error.name === expected, "expected " + expected + ", got " + error?.name);
                check(reads === 0 && parent.firstChild === child && input.parentNode === null, "invalid input has no side effects");
                check(observer.takeRecords().length === 0, "invalid insertion queues no records");
              };
              for (const value of invalid) {
                reject(parent, [value, child], "TypeError");
                if (method === "insertBefore") reject(parent, [input, value], "TypeError");
              }
              if (method === "insertBefore") {
                const attribute = owner.createAttribute("x");
                reject(parent, [attribute], "TypeError");
                reject(parent, [attribute, null], "HierarchyRequestError");
                reject(parent, [attribute, owner.createComment("missing")], "NotFoundError");
                reject(parent, [attribute, owner.createAttribute("missing")], "NotFoundError");
                reject(owner.createTextNode("leaf"), [attribute, owner.createAttribute("missing")], "HierarchyRequestError");
                reject(owner.createTextNode("leaf"), [attribute, invalid[0]], "TypeError");
              }
            } finally { observer.disconnect(); }
          });
        }
      }
      for (const parentKind of ["element", "fragment", "document", "text"]) {
        for (const oldKind of ["child", "missing", "attribute", "null", "proxy", "revoked"]) {
          run([name, borrowed, "Attr validation", parentKind, oldKind].join("/"), () => {
            const parent = parentKind === "document" ? owner : parentKind === "text" ? owner.createTextNode("leaf") :
              parentKind === "fragment" ? owner.createDocumentFragment() : owner.createElement("parent");
            const child = parentKind === "document" ? owner.documentElement : owner.createComment("child");
            if (parentKind === "element" || parentKind === "fragment") parent.appendChild(child);
            const inputOwner = document.implementation.createHTMLDocument("");
            const inputElement = inputOwner.createElement("input");
            inputElement.setAttribute("x", "retained");
            const input = inputElement.getAttributeNode("x");
            const revoked = Proxy.revocable(child, {}); revoked.revoke();
            let reads = 0;
            const old = oldKind === "child" ? child : oldKind === "missing" ? owner.createComment("missing") :
              oldKind === "attribute" ? owner.createAttribute("old") : oldKind === "null" ? null :
                oldKind === "revoked" ? revoked.proxy : new Proxy(child, {get() { reads++; throw new Error("proxy trap"); }});
            const expected = ["null", "proxy", "revoked"].includes(oldKind) ? "TypeError" :
              parentKind === "text" || oldKind === "child" ? "HierarchyRequestError" : "NotFoundError";
            const children = [...parent.childNodes];
            const observer = new MutationObserver(() => {});
            observer.observe(parent, {childList: true});
            observer.observe(inputElement, {attributes: true});
            try {
              let error;
              try { replace(parent, borrowed, input, old); } catch (caught) { error = caught; }
              check(error && error.name === expected, "expected " + expected + ", got " + error?.name);
              check(reads === 0, "conversion does not inspect a Proxy argument");
              check(same(parent.childNodes, children), "invalid replacement preserves children");
              check(input.ownerElement === inputElement && input.ownerDocument === inputOwner && input.value === "retained", "Attr ownership preserved");
              check(observer.takeRecords().length === 0, "invalid replacement has no mutation records");
            } finally { observer.disconnect(); }
          });
        }
      }
    }
  }
  for (const borrowed of [false, true]) {
    for (const attached of [false, true]) {
      for (const empty of [false, true]) {
        run(["ShadowRoot replacement", borrowed, attached, empty].join("/"), () => {
          const host = document.createElement("div"), shadow = host.attachShadow({mode: "open"});
          const destination = document.body.appendChild(document.createElement("div"));
          const old = destination.appendChild(document.createElement("i"));
          const reactions = [];
          const tag = "replacement-shadow-" + cases;
          customElements.define(tag, class extends HTMLElement {
            connectedCallback() { reactions.push("connected"); }
            disconnectedCallback() { reactions.push("disconnected"); }
          });
          const input = empty ? null : shadow.appendChild(document.createElement(tag));
          if (attached) document.body.appendChild(host);
          reactions.length = 0;
          try {
            check(replace(destination, borrowed, shadow, old) === old, "ShadowRoot replacement return");
            check(host.shadowRoot === shadow && shadow.ownerDocument === document, "source ShadowRoot preserved");
            check(shadow.childNodes.length === 0 && destination.firstChild === input, "ShadowRoot children moved");
            check(same(reactions, empty ? [] : attached ? ["disconnected", "connected"] : ["connected"]), "ordinary removal and insertion reactions");
            const button = shadow.appendChild(document.createElement("button"));
            check(button.isConnected === attached, "new child has actual source connectivity");
            button.focus();
            check(shadow.activeElement === (attached ? button : null), "focus agrees with source connectivity");
            if (!attached) document.body.appendChild(host);
            button.focus();
            check(shadow.activeElement === button && document.activeElement === host, "original ShadowRoot remains usable after connection");
          } finally { host.remove(); destination.remove(); }
        });
      }
    }
  }
  // Keep the iframe live throughout the matrix before testing removal of the
  // main document's root, which disconnects its browsing context.
  for (const [name, owner] of [...documents].reverse()) {
    for (const borrowed of [false, true]) {
      run([name, borrowed, "document root self"].join("/"), () => {
        const root = owner.documentElement, children = [...owner.childNodes];
        const previous = root.previousSibling, next = root.nextSibling;
        const observer = new MutationObserver(() => {}); observer.observe(owner, {childList: true});
        try {
          check(replace(owner, borrowed, root, root) === root, "root self return");
          const records = observer.takeRecords(); check(records.length === 2, "root self removes and inserts");
          record(records[0], owner, [], [root], previous, next);
          record(records[1], owner, [root], [], previous, next);
          check(same(owner.childNodes, children), "document shape preserved");
        } finally { observer.disconnect(); }
      });
    }
  }
  globalThis.__nodeReplacementResults = {cases, failures};
  if (failures.length) throw new Error(failures.join("\n"));
  return true;
})()
