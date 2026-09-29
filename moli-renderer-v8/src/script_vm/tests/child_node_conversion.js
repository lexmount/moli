(() => {
  const check = (value, message) => { if (!value) throw new Error(message); };
  const same = (actual, expected) => actual.length === expected.length &&
    expected.every((node, index) => actual[index] === node);
  const failures = [];
  const factories = [
    ["HTML", () => document.implementation.createHTMLDocument("")],
    ["XML", () => document.implementation.createDocument(null, "root")],
    ["parsed HTML", () => new DOMParser().parseFromString("<!doctype html><body></body>", "text/html")],
    ["parsed XML", () => new DOMParser().parseFromString("<!DOCTYPE root><root/>", "application/xml")],
    ["iframe", () => document.body.appendChild(document.createElement("iframe")).contentDocument],
    ["main", () => document],
  ];
  function run(label, create, callback) {
    const owner = create();
    const original = [...owner.childNodes];
    const frame = owner !== document && owner.defaultView && owner.defaultView.frameElement;
    try { callback(owner); }
    catch (error) { failures.push(label + ": " + error.message); }
    finally {
      if (!same(owner.childNodes, original)) {
        for (const node of [...owner.childNodes]) owner.removeChild(node);
        for (const node of original) owner.appendChild(node);
      }
      if (frame) frame.remove();
    }
  }
  function errorFrom(callback) { try { callback(); } catch (error) { return error; } }
  function hierarchy(callback) {
    const error = errorFrom(callback);
    check(error && error.name === "HierarchyRequestError", "HierarchyRequestError required");
  }
  function invoke(target, method, borrowed, ...values) {
    if (!borrowed) return target[method](...values);
    const prototype = target.nodeType === 1 ? Element.prototype :
      target.nodeType === 10 ? DocumentType.prototype : CharacterData.prototype;
    return prototype[method].call(target, ...values);
  }
  function observe(...targets) {
    const observer = new MutationObserver(() => {});
    for (const target of targets) observer.observe(target, {childList: true});
    return observer;
  }
  function record(r, target, added, removed, before, after) {
    check(r && r.type === "childList" && r.target === target && same(r.addedNodes, added) &&
      same(r.removedNodes, removed) && r.previousSibling === before && r.nextSibling === after,
      "mutation record target, nodes and boundaries");
  }
  const kinds = [
    ["element", owner => owner.createElement("select")],
    ["text", owner => owner.createTextNode("text")],
    ["comment", owner => owner.createComment("comment")],
    ["PI", owner => owner.createProcessingInstruction("test", "value")],
    ["CDATA", owner => owner.adoptNode(document.implementation.createDocument(null, "root").createCDATASection("data"))],
  ];
  function setup(owner, kind) {
    const parent = owner.createElement("parent");
    const left = parent.appendChild(owner.createElement("left"));
    const target = parent.appendChild(kind(owner));
    const right = parent.appendChild(owner.createElement("right"));
    return {parent, left, target, right};
  }
  for (const [name, create] of factories) for (const borrowed of [false, true]) {
    for (const method of ["before", "after", "replaceWith"]) {
      const prefix = [name, borrowed, method].join("/");
      for (const [kindName, kind] of kinds) {
        const label = prefix + "/" + kindName;
        run(label + "/parentless argument conversion", create, owner => {
          const target = kind(owner), sentinel = {};
          check(errorFrom(() => invoke(target, method, borrowed, {toString() { throw sentinel; }})) === sentinel,
            "parentless receivers still convert arguments");
          let conversions = 0;
          invoke(target, method, borrowed, owner.createAttribute("bad"), {toString() { conversions++; return "x"; }});
          check(conversions === 1 && target.parentNode === null, "return after IDL conversion without building a fragment");
        });
        run(label + "/parent created during conversion", create, owner => {
          const target = kind(owner), parent = owner.createElement("parent");
          invoke(target, method, borrowed, {toString() { parent.appendChild(target); return "x"; }});
          const text = method === "before" || method === "replaceWith" ? parent.firstChild : parent.lastChild;
          check(text.nodeType === 3 && text.data === "x", "read the parent after string conversion");
          check(same(parent.childNodes, method === "before" ? [text, target] :
            method === "after" ? [target, text] : [text]), "conversion can attach the receiver");
        });
        run(label + "/conversion failure", create, owner => {
          const {parent, left, target, right} = setup(owner, kind), sentinel = {};
          const observer = observe(parent);
          try {
            check(errorFrom(() => invoke(target, method, borrowed, left, {toString() { throw sentinel; }})) === sentinel,
              "preserve the conversion exception");
            check(same(parent.childNodes, [left, target, right]) && observer.takeRecords().length === 0,
              "conversion failure precedes tree mutations");
          } finally { observer.disconnect(); }
        });
        run(label + "/invalid later node", create, owner => {
          const {parent, left, target, right} = setup(owner, kind);
          let conversions = 0;
          const observer = observe(parent);
          try {
            hierarchy(() => invoke(target, method, borrowed, left, owner.createAttribute("bad"), {toString() {
              conversions++; check(left.parentNode === parent, "all strings convert before movement"); return "x";
            }}));
            check(conversions === 1, "convert arguments after an invalid node");
            check(left.parentNode && left.parentNode.nodeType === 11 && same(left.parentNode.childNodes, [left]),
              "earlier input is moved into the temporary fragment");
            check(same(parent.childNodes, [target, right]), "failed conversion must not remove the receiver");
            const records = observer.takeRecords();
            check(records.length === 1, "one earlier movement record");
            record(records[0], parent, [], [left], null, target);
          } finally { observer.disconnect(); }
        });
        run(label + "/invalid single doctype", create, owner => {
          const {parent, left, target, right} = setup(owner, kind);
          const input = owner.implementation.createDocumentType("root", "", "");
          const observer = observe(parent);
          try {
            hierarchy(() => invoke(target, method, borrowed, input));
            check(same(parent.childNodes, [left, target, right]) && input.parentNode === null,
              "invalid single node preserves the receiver");
            check(observer.takeRecords().length === 0, "no mutation for an invalid single node");
          } finally { observer.disconnect(); }
        });
        run(label + "/single self", create, owner => {
          const {parent, left, target, right} = setup(owner, kind);
          const observer = observe(parent);
          try {
            invoke(target, method, borrowed, target);
            check(same(parent.childNodes, [left, target, right]), "self insertion retains tree order");
            const records = observer.takeRecords(); check(records.length === 2, "self insertion removes then inserts");
            record(records[0], parent, [], [target], left, right);
            record(records[1], parent, [target], [], left, right);
          } finally { observer.disconnect(); }
        });
        run(label + "/self among inputs", create, owner => {
          const {parent, left, target, right} = setup(owner, kind), extra = owner.createElement("extra");
          const observer = observe(parent);
          try {
            invoke(target, method, borrowed, target, extra);
            check(same(parent.childNodes, [left, target, extra, right]), "receiver remains in the input sequence");
            const records = observer.takeRecords(); check(records.length === 2, "conversion removal and batched insertion");
            record(records[0], parent, [], [target], left, right);
            record(records[1], parent, [target, extra], [], left, right);
          } finally { observer.disconnect(); }
        });
        run(label + "/viable siblings", create, owner => {
          const parent = owner.createElement("parent"), anchor = owner.createComment("anchor");
          const a = owner.createElement("a"), b = owner.createElement("b"), target = kind(owner);
          const c = owner.createElement("c"), d = owner.createElement("d");
          parent.append(anchor, a, b, target, c, d);
          invoke(target, method, borrowed, b, a, target, c);
          check(same(parent.childNodes, [anchor, b, a, target, c, d]), "viable sibling excludes every input before movement");
        });
        run(label + "/strings and brands", create, owner => {
          const {parent, target} = setup(owner, kind); let reads = 0;
          invoke(target, method, borrowed, {get nodeType() { reads++; return 1; }, toString() { return "\ud800x\udc00"; }}, "\udfff");
          const texts = [...parent.childNodes].filter(node => node.nodeType === 3 && node !== target);
          check(reads === 0 && texts.length === 2 && texts[0].data === "\ud800x\udc00" && texts[1].data === "\udfff",
            "Node union uses native brands and preserves DOMString code units");
        });
        run(label + "/no arguments", create, owner => {
          const {parent, left, target, right} = setup(owner, kind), observer = observe(parent);
          try {
            invoke(target, method, borrowed);
            const records = observer.takeRecords(); check(records.length === (method === "replaceWith" ? 1 : 0), "empty input record count");
            check(same(parent.childNodes, method === "replaceWith" ? [left, right] : [left, target, right]), "empty input tree");
            if (method === "replaceWith") record(records[0], parent, [], [target], left, right);
          } finally { observer.disconnect(); }
        });
        run(label + "/prior records and external inputs", create, owner => {
          const {parent, left, target, right} = setup(owner, kind), source = owner.createElement("source");
          const x = source.appendChild(owner.createElement("x")), y = source.appendChild(owner.createElement("y"));
          const observer = observe(parent, source);
          try {
            parent.removeChild(target); parent.insertBefore(target, right);
            invoke(target, method, borrowed, x, y);
            const records = observer.takeRecords(); check(records.length === 5, "preserve prior records and source removals");
            record(records[0], parent, [], [target], left, right);
            record(records[1], parent, [target], [], left, right);
            record(records[2], source, [], [x], null, y);
            record(records[3], source, [], [y], null, null);
            record(records[4], parent, [x, y], method === "replaceWith" ? [target] : [],
              method === "after" ? target : left, method === "before" ? target : right);
          } finally { observer.disconnect(); }
        });
        run(label + "/single fragment", create, owner => {
          const {parent, left, target, right} = setup(owner, kind), input = owner.createDocumentFragment();
          const x = input.appendChild(owner.createElement("x")), y = input.appendChild(owner.createElement("y"));
          const observer = observe(parent, input);
          try {
            invoke(target, method, borrowed, input);
            const records = observer.takeRecords(); check(records.length === 2, "single fragment removal is batched");
            record(records[0], input, [], [x, y], null, null);
            record(records[1], parent, [x, y], method === "replaceWith" ? [target] : [],
              method === "after" ? target : left, method === "before" ? target : right);
          } finally { observer.disconnect(); }
        });
      }
      run(prefix + "/Document root self", create, owner => {
        const root = owner.documentElement, original = [...owner.childNodes], observer = observe(owner);
        try {
          if (method === "replaceWith") {
            invoke(root, method, borrowed, root);
            check(observer.takeRecords().length === 2, "root self replacement removes then inserts");
          } else {
            hierarchy(() => invoke(root, method, borrowed, root));
            check(observer.takeRecords().length === 0, "invalid root reinsertion has no records");
          }
          check(same(owner.childNodes, original), "root self tree shape");
        } finally { observer.disconnect(); }
      });
      run(prefix + "/Document invalid single fragment", create, owner => {
        const root = owner.documentElement, original = [...owner.childNodes], input = owner.createDocumentFragment();
        const x = input.appendChild(owner.createElement("x")), y = input.appendChild(owner.createElement("y"));
        const observer = observe(owner, input);
        try {
          hierarchy(() => invoke(root, method, borrowed, input));
          check(same(owner.childNodes, original) && same(input.childNodes, [x, y]), "invalid single fragment is not dismantled");
          check(observer.takeRecords().length === 0, "invalid fragment leaves no records");
        } finally { observer.disconnect(); }
      });
      run(prefix + "/Document doctype", create, owner => {
        if (owner.doctype) owner.removeChild(owner.doctype);
        const root = owner.documentElement, input = owner.implementation.createDocumentType("root", "", "");
        if (method === "after") hierarchy(() => invoke(root, method, borrowed, input));
        else {
          invoke(root, method, borrowed, input);
          check(same(owner.childNodes, method === "before" ? [input, root] : [input]), "single doctype is not put in a fragment");
        }
      });
      run(prefix + "/DocumentType receiver conversion", create, owner => {
        const target = owner.implementation.createDocumentType("root", "", ""), sentinel = {};
        check(errorFrom(() => invoke(target, method, borrowed, {toString() { throw sentinel; }})) === sentinel,
          "DocumentType converts arguments without a parent");
      });
    }
  }
  let definition = 0;
  for (const borrowed of [false, true]) for (const method of ["before", "after", "replaceWith"]) {
    run(["main", borrowed, method, "custom element self reactions"].join("/"), () => document, owner => {
      const log = [], name = "child-conversion-self-" + definition++;
      customElements.define(name, class extends HTMLElement {
        connectedCallback() { log.push("connected"); }
        disconnectedCallback() { log.push("disconnected"); }
      });
      const parent = owner.body.appendChild(owner.createElement("div"));
      try {
        const target = parent.appendChild(owner.createElement(name));
        log.length = 0;
        invoke(target, method, borrowed, target);
        check(same(log, ["disconnected", "connected"]), "self movement runs removal and insertion reactions");
      } finally { parent.remove(); }
    });
  }
  if (failures.length) throw new Error(failures.join("\n"));
  return true;
})()
