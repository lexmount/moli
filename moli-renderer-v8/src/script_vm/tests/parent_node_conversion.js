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
    ["iframe", () => {
      const frame = document.body.appendChild(document.createElement("iframe"));
      return frame.contentDocument;
    }],
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
  function errorFrom(callback) {
    try { callback(); } catch (error) { return error; }
  }
  function hierarchy(callback) {
    const error = errorFrom(callback);
    check(error && error.name === "HierarchyRequestError", "HierarchyRequestError required");
  }
  function observe(...targets) {
    const observer = new MutationObserver(() => {});
    for (const target of targets) observer.observe(target, {childList: true});
    return observer;
  }
  function invoke(target, method, borrowed, ...nodes) {
    if (!borrowed) return target[method](...nodes);
    const prototype = target.nodeType === 9 ? Document.prototype :
      target.nodeType === 11 ? DocumentFragment.prototype : Element.prototype;
    return prototype[method].call(target, ...nodes);
  }
  for (const [name, create] of factories) {
    for (const borrowed of [false, true]) {
      for (const method of ["append", "prepend", "replaceChildren"]) {
        const label = [name, borrowed, method].join("/");
        if (method !== "replaceChildren") run(label + "/single existing root", create, owner => {
          const before = [...owner.childNodes];
          const root = owner.documentElement;
          const observer = observe(owner);
          try {
            hierarchy(() => invoke(owner, method, borrowed, root));
            check(same(owner.childNodes, before) && root.parentNode === owner, "single node stays in place on failure");
            check(observer.takeRecords().length === 0, "no records before validation");
          } finally { observer.disconnect(); }
        });
        run(label + "/multi invalid roots", create, owner => {
          const root = owner.documentElement;
          const extra = owner.createElement("extra");
          const observer = observe(owner);
          try {
            hierarchy(() => invoke(owner, method, borrowed, root, extra));
            check(root.parentNode && root.parentNode.nodeType === 11 &&
              same(root.parentNode.childNodes, [root, extra]), "conversion must move both inputs to one fragment");
            check(owner.documentElement === null, "conversion precedes document validation");
            const records = observer.takeRecords();
            check(records.length === 1 && same(records[0].removedNodes, [root]), "one root removal record");
          } finally { observer.disconnect(); }
        });
        run(label + "/single invalid fragment", create, owner => {
          const fragment = owner.createDocumentFragment();
          const first = fragment.appendChild(owner.createElement("one"));
          const second = fragment.appendChild(owner.createElement("two"));
          const before = [...owner.childNodes];
          const observer = observe(owner, fragment);
          try {
            hierarchy(() => invoke(owner, method, borrowed, fragment));
            check(same(owner.childNodes, before) && same(fragment.childNodes, [first, second]), "original fragment stays intact");
            check(observer.takeRecords().length === 0, "invalid single fragment has no records");
          } finally { observer.disconnect(); }
        });
        run(label + "/duplicate arguments", create, owner => {
          const root = owner.documentElement;
          if (method === "prepend" && owner.doctype) {
            hierarchy(() => invoke(owner, method, borrowed, root, root));
            check(owner.documentElement === null && root.parentNode.nodeType === 11, "duplicate root moved before invalid prepend");
          } else {
            check(invoke(owner, method, borrowed, root, root) === undefined, "successful operation returns undefined");
            check(owner.documentElement === root, "duplicate input becomes one root");
          }
        });
        run(label + "/single doctype", create, owner => {
          for (const node of [...owner.childNodes]) owner.removeChild(node);
          const type = owner.implementation.createDocumentType("root", "", "");
          check(invoke(owner, method, borrowed, type) === undefined, "single doctype insertion succeeds");
          check(same(owner.childNodes, [type]), "doctype is inserted without a temporary fragment");
        });
        for (const parentKind of ["element", "fragment"]) {
          run(label + "/" + parentKind + "/no arguments", create, owner => {
            const parent = parentKind === "element" ? owner.createElement("parent") : owner.createDocumentFragment();
            const old = parent.appendChild(owner.createElement("old"));
            const observer = observe(parent);
            try {
              invoke(parent, method, borrowed);
              const records = observer.takeRecords();
              check(records.length === (method === "replaceChildren" ? 1 : 0), "empty input record count");
              if (method === "replaceChildren") {
                check(same(records[0].removedNodes, [old]) && records[0].addedNodes.length === 0,
                  "empty replacement only removes existing children");
              }
              invoke(parent, method, borrowed);
              check(observer.takeRecords().length === 0, "empty fragment insertion is not observable");
            } finally { observer.disconnect(); }
          });
          run(label + "/" + parentKind + "/late invalid argument", create, owner => {
            const parent = parentKind === "element" ? owner.createElement("parent") : owner.createDocumentFragment();
            const child = parent.appendChild(owner.createElement("child"));
            const attr = owner.createAttribute("test");
            let conversions = 0;
            const text = {toString() {
              conversions++;
              check(child.parentNode === parent, "all string conversions precede node movement");
              return "text";
            }};
            const observer = observe(parent);
            try {
              hierarchy(() => invoke(parent, method, borrowed, child, attr, text));
              check(conversions === 1, "later string argument must be converted before hierarchy validation");
              check(child.parentNode && child.parentNode.nodeType === 11 && child.parentNode !== parent,
                "earlier node moved before later invalid node");
              const records = observer.takeRecords();
              check(records.length === 1 && same(records[0].removedNodes, [child]), "earlier movement remains observable");
            } finally { observer.disconnect(); }
          });
          run(label + "/" + parentKind + "/conversion failure", create, owner => {
            const parent = parentKind === "element" ? owner.createElement("parent") : owner.createDocumentFragment();
            const child = parent.appendChild(owner.createElement("child"));
            const sentinel = {};
            const observer = observe(parent);
            try {
              const error = errorFrom(() => invoke(parent, method, borrowed, child,
                {toString() { throw sentinel; }}));
              check(error === sentinel && same(parent.childNodes, [child]), "propagate conversion error before mutations");
              check(observer.takeRecords().length === 0, "conversion failure has no records");
            } finally { observer.disconnect(); }
          });
          run(label + "/" + parentKind + "/strings and brands", create, owner => {
            const parent = parentKind === "element" ? owner.createElement("parent") : owner.createDocumentFragment();
            let reads = 0;
            const value = {get nodeType() { reads++; return 1; }, toString() { return "\ud800A\udc00"; }};
            invoke(parent, method, borrowed, value, "\udfff");
            check(reads === 0, "Node union uses native brands");
            check(parent.childNodes.length === 2 && parent.childNodes[0].data === "\ud800A\udc00" &&
              parent.childNodes[1].data === "\udfff", "DOMString code units survive conversion");
          });
          run(label + "/" + parentKind + "/batch records", create, owner => {
            const parent = parentKind === "element" ? owner.createElement("parent") : owner.createDocumentFragment();
            const old = parent.appendChild(owner.createElement("old"));
            const source = owner.createElement("source");
            const a = source.appendChild(owner.createElement("a"));
            const b = source.appendChild(owner.createElement("b"));
            const observer = observe(parent, source);
            try {
              invoke(parent, method, borrowed, a, b);
              const records = observer.takeRecords();
              check(records.length === 3 && records[0].target === source && records[1].target === source &&
                records[2].target === parent, "source removals precede one destination record");
              check(same(records[2].addedNodes, [a, b]), "destination insertion is batched");
              check(same(records[2].removedNodes, method === "replaceChildren" ? [old] : []), "replacement removed nodes");
            } finally { observer.disconnect(); }
          });
        }
      }
      run([name, borrowed, "replaceChildren", "current standard document replacement"].join("/"), create, owner => {
        const original = [...owner.childNodes];
        const root = owner.createElement("replacement");
        const observer = observe(owner);
        try {
          invoke(owner, "replaceChildren", borrowed, root);
          check(same(owner.childNodes, [root]), "existing document children are excluded from validity checks");
          const records = observer.takeRecords();
          check(records.length === 1 && same(records[0].removedNodes, original) &&
            same(records[0].addedNodes, [root]), "replace all queues one record");
        } finally { observer.disconnect(); }
      });
      run([name, borrowed, "replaceChildren", "conversion and prior records"].join("/"), create, owner => {
        const parent = owner.createElement("parent");
        const a = parent.appendChild(owner.createElement("a"));
        const b = parent.appendChild(owner.createElement("b"));
        const c = parent.appendChild(owner.createElement("c"));
        const observer = observe(parent);
        try {
          parent.removeChild(c);
          parent.appendChild(c);
          invoke(parent, "replaceChildren", borrowed, b, a);
          const records = observer.takeRecords();
          check(records.length === 5, "retain earlier records and both conversion removals");
          check(same(records[0].removedNodes, [c]) && same(records[1].addedNodes, [c]) &&
            same(records[2].removedNodes, [b]) && same(records[3].removedNodes, [a]) &&
            same(records[4].removedNodes, [c]) && same(records[4].addedNodes, [b, a]), "record boundaries and order");
        } finally { observer.disconnect(); }
      });
      run([name, borrowed, "replaceChildren", "single existing child and prior records"].join("/"), create, owner => {
        const parent = owner.createElement("parent");
        const a = parent.appendChild(owner.createElement("a"));
        const b = parent.appendChild(owner.createElement("b"));
        const c = parent.appendChild(owner.createElement("c"));
        const range = owner.createRange();
        range.selectNodeContents(parent);
        const observer = observe(parent);
        try {
          parent.removeChild(c);
          parent.appendChild(c);
          invoke(parent, "replaceChildren", borrowed, b);
          const records = observer.takeRecords();
          check(records.length === 3 && same(records[0].removedNodes, [c]) &&
            same(records[1].addedNodes, [c]), "previous operations retain their records");
          check(same(records[2].removedNodes, [a, b, c]) && same(records[2].addedNodes, [b]) &&
            records[2].previousSibling === null && records[2].nextSibling === null,
            "single input removal is folded into replacement");
          check(range.startContainer === parent && range.endContainer === parent &&
            range.startOffset === 0 && range.endOffset === 0, "replace-all updates range boundaries");
        } finally { observer.disconnect(); }
      });
      run([name, borrowed, "replaceChildren", "single external child"].join("/"), create, owner => {
        const parent = owner.createElement("parent");
        const old = parent.appendChild(owner.createElement("old"));
        const source = owner.createDocumentFragment();
        const before = source.appendChild(owner.createElement("before"));
        const input = source.appendChild(owner.createElement("input"));
        const observer = observe(parent, source);
        try {
          invoke(parent, "replaceChildren", borrowed, input);
          const records = observer.takeRecords();
          check(records.length === 2 && records[0].target === source &&
            same(records[0].removedNodes, [input]) && records[0].previousSibling === before &&
            records[0].nextSibling === null, "source removal remains observable before destination record");
          check(records[1].target === parent && same(records[1].removedNodes, [old]) &&
            same(records[1].addedNodes, [input]), "destination has one replacement record");
        } finally { observer.disconnect(); }
      });
    }
  }
  if (failures.length) throw new Error(failures.join("\n"));
  return true;
})()
