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
  globalThis.__blobReadingProgress = { stage: "Range recursive subfragments", checks };
  for (const side of ["left", "right"]) {
    for (const depth of [1, 3]) {
      const root = document.createElement("div");
      const start = document.createElement("section"), middle = document.createElement("article");
      const end = document.createElement("footer"), marker = document.createElement("mark");
      const leaf = document.createElement("span"), frame = document.createElement("iframe");
      start.textContent = "start"; middle.textContent = "middle"; end.textContent = "end";
      marker.textContent = "marker";
      const loaded = new Promise(resolve => frame.addEventListener("load", resolve, { once: true }));
      frame.srcdoc = "<!doctype html><p>child</p>";
      leaf.append(marker, frame, document.createTextNode("leaf"));
      let partial = leaf;
      for (let i = 0; i < depth; i++) {
        const ancestor = document.createElement("div");
        ancestor.append(partial); partial = ancestor;
      }
      if (side === "left") start.replaceChildren(partial);
      else end.replaceChildren(partial);
      root.append(start, middle, end); document.body.append(root);
      await loaded;
      const range = document.createRange();
      range.setStart(side === "left" ? leaf : start, 0);
      range.setEnd(side === "right" ? leaf : end, side === "right" ? leaf.childNodes.length : 1);
      let calls = 0, observed;
      observeRemovalNavigationAbort(frame, () => {
        calls++;
        const markerRoot = marker.getRootNode();
        const middleRoot = middle.getRootNode();
        observed = { markerRoot: markerRoot.nodeType, markerDirectChild: marker.parentNode === markerRoot,
          markerFirst: markerRoot.firstChild === marker,
          partialTextInOuterFragment: side === "right" ? middleRoot.lastChild?.textContent : null,
          markerText: markerRoot.textContent, collapsed: range.collapsed };
      });
      try {
        const fragment = range.extractContents();
        checks.push({ name: `${side}:depth-${depth}:subfragment`,
          passed: calls === 1 && observed?.markerRoot === 11 && observed.markerDirectChild
            && observed.markerFirst && observed.markerText === "marker" && observed.collapsed
            && (side === "left" || observed.partialTextInOuterFragment === "")
            && fragment.textContent === (side === "left" ? "markerleafmiddleend" : "startmiddlemarkerleaf")
            && range.collapsed,
          detail: { calls, observed, text: fragment.textContent } });
      } catch (error) {
        checks.push({ name: `${side}:depth-${depth}:subfragment`, passed: false, detail: String(error.stack || error) });
      } finally { root.remove(); }
    }
  }
  {
    const root = document.createElement("div"), start = document.createElement("section");
    const end = document.createElement("footer"), leaf = document.createElement("span");
    start.textContent = "start";
    const frames = [0, 1, 2].map(() => document.createElement("iframe"));
    const loaded = frames.map(frame => new Promise(resolve => frame.addEventListener("load", resolve, { once: true })));
    frames.forEach(frame => { frame.srcdoc = "<!doctype html><p>child</p>"; });
    leaf.append(frames[2], document.createTextNode("end"));
    end.append(frames[0], frames[1], leaf); root.append(start, end); document.body.append(root);
    await Promise.all(loaded);
    const calls = [];
    frames.forEach((frame, index) => observeRemovalNavigationAbort(frame, () => { calls.push(index); }));
    const range = document.createRange(); range.setStart(start, 0); range.setEnd(leaf, leaf.childNodes.length);
    try {
      const fragment = range.extractContents();
      checks.push({ name: "right:ancestor-siblings:tree-order", passed: calls.join(",") === "0,1,2" && fragment.textContent === "startend",
        detail: { calls, text: fragment.textContent } });
    } catch (error) { checks.push({ name: "right:ancestor-siblings:tree-order", passed: false, detail: String(error.stack || error) }); }
    finally { root.remove(); }
  }
  globalThis.__uiEventResults = { checks, navigationAbortObserved: checks.some(check => check.detail?.calls === 1 || Array.isArray(check.detail?.calls) && check.detail.calls.length) };
})().catch(error => { globalThis.__uiEventResults = { checks: [{ name: "fixture", passed: false, detail: String(error.stack || error) }] }; });
