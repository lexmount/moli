(async () => {
  function observeRemovalNavigationAbort(frame, callback) {
    const child = frame.contentWindow;
    const navigation = child.navigation;
    if (!navigation?.currentEntry) throw new Error("Navigation entry is required for removal reentrancy");
    navigation.addEventListener("navigate", event => {
      event.signal.addEventListener("abort", callback, { once: true });
      if (event.canIntercept) event.intercept({ handler: () => new child.Promise(() => {}) });
    }, { once: true });
    const pending = navigation.navigate("#range-removal-pending");
    pending.committed.catch(() => {});
    pending.finished.catch(() => {});
  }

  const checks = [];
  globalThis.__blobReadingProgress = { stage: "range contents reentrancy", checks };
  const modes = [
    "none", "remove-end", "move-end-before", "move-end-out",
    "remove-middle", "move-middle", "insert-middle", "remove-start",
  ];
  const cases = [
    ...["partial", "nested"].flatMap(boundary => modes.map(mode => [boundary, mode])),
    ["whole", "none"],
  ];

  for (const [boundary, mode] of cases) {
    const root = document.createElement("div");
    const stash = document.createElement("div");
    const start = document.createElement("section");
    const middle = document.createElement("article");
    const end = document.createElement("footer");
    const outside = document.createElement("aside");
    const startNode = document.createElement("span");
    const endNode = document.createElement("span");
    const late = document.createElement("b");
    startNode.textContent = "start";
    middle.textContent = "middle";
    endNode.textContent = "end";
    outside.textContent = "outside";
    late.textContent = "late";

    const frame = document.createElement("iframe");
    const loaded = new Promise(resolve => frame.addEventListener("load", resolve, { once: true }));
    frame.srcdoc = "<!doctype html><p>child</p>";
    start.append(startNode, frame);
    end.append(endNode);
    root.append(start, middle, end, outside);
    document.body.append(root, stash);
    await loaded;

    const range = document.createRange();
    if (boundary === "whole") {
      range.setStart(root, 0);
      range.setEnd(root, 3);
    } else if (boundary === "nested") {
      range.setStart(startNode, 0);
      range.setEnd(endNode, 1);
    } else {
      range.setStart(start, 0);
      range.setEnd(end, 1);
    }

    let calls = 0;
    let observed = null;
    observeRemovalNavigationAbort(frame, () => {
      calls++;
      observed = {
        collapsed: range.collapsed,
        startIsRoot: range.startContainer === root,
        endIsRoot: range.endContainer === root,
        startOffset: range.startOffset,
        endOffset: range.endOffset,
      };
      switch (mode) {
        case "remove-end": end.remove(); break;
        case "move-end-before": root.insertBefore(end, start); break;
        case "move-end-out": stash.append(end); break;
        case "remove-middle": middle.remove(); break;
        case "move-middle": stash.append(middle); break;
        case "insert-middle": root.insertBefore(late, end); break;
        case "remove-start": start.remove(); break;
      }
    });

    try {
      const fragment = range.extractContents();
      const facts = {
        calls,
        observed,
        fragment: fragment instanceof DocumentFragment,
        text: fragment?.textContent,
        middle: fragment?.contains(middle),
        outside: outside.parentNode === root,
        late: mode !== "insert-middle" || late.parentNode === root,
        collapsed: range.collapsed,
        startIsRoot: range.startContainer === root,
        endIsRoot: range.endContainer === root,
        startOffset: range.startOffset,
        endOffset: range.endOffset,
      };
      const beforeOffset = boundary === "whole" ? 0 : 1;
      const offset = mode === "remove-start" ? 0 :
        mode === "move-end-before" ? 2 : beforeOffset;
      const early = observed?.collapsed && observed.startIsRoot && observed.endIsRoot &&
        observed.startOffset === beforeOffset && observed.endOffset === beforeOffset;
      checks.push({
        name: `${boundary}: ${mode}`,
        passed: calls === 1 && early && facts.fragment && facts.text === "startmiddleend" &&
          facts.middle && facts.outside && facts.late && facts.collapsed &&
          facts.startIsRoot && facts.endIsRoot && facts.startOffset === offset &&
          facts.endOffset === offset,
        detail: facts,
      });
    } catch (error) {
      checks.push({ name: `${boundary}: ${mode}`, passed: false,
        detail: String(error.stack || error), calls, observed });
    } finally {
      root.remove();
      stash.remove();
    }
  }

  try {
    const xml = document.implementation.createDocument(null, "root");
    const doctype = xml.implementation.createDocumentType("root", "", "");
    xml.insertBefore(doctype, xml.documentElement);
    const range = xml.createRange();
    range.setStart(xml, 0);
    range.setEnd(xml, 2);
    let error;
    try { range.extractContents(); } catch (thrown) { error = thrown; }
    checks.push({
      name: "doctype rejection is atomic",
      passed: error?.name === "HierarchyRequestError" && xml.firstChild === doctype &&
        xml.childNodes.length === 2 && range.startContainer === xml &&
        range.endContainer === xml && range.startOffset === 0 && range.endOffset === 2,
    });
  } catch (error) {
    checks.push({ name: "doctype rejection is atomic", passed: false,
      detail: String(error.stack || error) });
  }
  try {
    const host = document.createElement("div");
    const [start, middle, end, outside] = ["section", "article", "footer", "aside"]
      .map(name => document.createElement(name));
    host.append(start, middle, end, outside);
    document.body.append(host);
    const range = document.createRange();
    range.setStart(host, 1);
    range.collapse(true);
    host.insertBefore(end, start);
    middle.remove();
    checks.push({
      name: "ordinary live boundary follows insertion before its offset",
      passed: range.collapsed && range.startContainer === host &&
        range.endContainer === host && range.startOffset === 2 && range.endOffset === 2,
    });
    host.remove();
  } catch (error) {
    checks.push({ name: "ordinary live boundary follows insertion before its offset",
      passed: false, detail: String(error.stack || error) });
  }
  globalThis.__uiEventResults = {
    checks,
    navigationAbortObserved: checks.some(check => check.detail?.calls === 1),
  };
})().catch(error => { globalThis.__uiEventResults = { checks: [{name: "fixture setup", passed: false, detail: String(error.stack || error)}] }; });
