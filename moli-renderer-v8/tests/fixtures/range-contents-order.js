(() => {
  const checks = [];
  globalThis.__blobReadingProgress = { stage: "Range partial-content order", checks };
  for (const side of ["left", "right"]) {
    for (const nested of [false, true]) {
      for (const moveMiddle of [false, true]) {
        const root = document.createElement("div"), stash = document.createElement("div");
        const start = document.createElement("section"), middle = document.createElement("article");
        const end = document.createElement("footer"), outside = document.createElement("aside");
        const startNode = document.createElement("span"), endNode = document.createElement("span");
        startNode.textContent = "start"; endNode.textContent = "end";
        middle.textContent = "middle"; outside.textContent = "outside";
        const startText = startNode.firstChild;
        const frame = document.createElement("iframe"); frame.src = "about:blank";
        start.append(startNode); end.append(endNode);
        if (side === "left") (nested ? startNode : start).append(frame);
        else (nested ? endNode : end).prepend(frame);
        root.append(start, middle, end, outside); document.body.append(root, stash);
        const range = document.createRange();
        range.setStart(nested ? startNode : start, 0);
        const endContainer = nested ? endNode : end;
        range.setEnd(endContainer, endContainer.childNodes.length);
        let calls = 0, observed;
        frame.contentWindow.addEventListener("unload", () => {
          calls++;
          observed = { leftRoot: startText.getRootNode().nodeType,
            middleRoot: middle.getRootNode().nodeType, collapsed: range.collapsed };
          if (moveMiddle) stash.append(middle);
        });
        try {
          const fragment = range.extractContents();
          const movedAfterExtraction = side === "right" && moveMiddle;
          const facts = { calls, observed, fragment: fragment instanceof DocumentFragment,
            text: fragment?.textContent, middleInFragment: fragment?.contains(middle),
            middleInStash: middle.parentNode === stash, outside: outside.parentNode === root,
            collapsed: range.collapsed, startRoot: range.startContainer === root,
            startOffset: range.startOffset, endRoot: range.endContainer === root, endOffset: range.endOffset };
          checks.push({ name: `${side}:${nested ? "nested" : "direct"}:${moveMiddle ? "move" : "observe"}`,
            passed: calls === 1 && observed?.leftRoot === 11 && observed.middleRoot === (side === "left" ? 9 : 11)
              && observed.collapsed && facts.fragment && facts.text === (movedAfterExtraction ? "startend" : "startmiddleend")
              && facts.middleInFragment === !movedAfterExtraction && facts.middleInStash === movedAfterExtraction
              && facts.outside && facts.collapsed && facts.startRoot && facts.endRoot
              && facts.startOffset === 1 && facts.endOffset === 1,
            detail: facts });
        } catch (error) { checks.push({ name: `${side}:${nested}:${moveMiddle}`, passed: false, detail: String(error.stack || error) }); }
        finally { root.remove(); stash.remove(); }
      }
    }
  }
  for (const method of ["cloneContents", "deleteContents"]) {
    const root = document.createElement("div");
    root.innerHTML = "<section>start</section><article>middle</article><footer>end</footer><aside>outside</aside>";
    document.body.append(root);
    const range = document.createRange(); range.setStart(root.firstChild, 0); range.setEnd(root.children[2], 1);
    try {
      const fragment = range[method]();
      checks.push({ name: method + " control", passed: method === "cloneContents"
        ? fragment instanceof DocumentFragment && fragment.textContent === "startmiddleend" && root.textContent === "startmiddleendoutside" && !range.collapsed
        : fragment === undefined && root.textContent === "outside" && range.collapsed && range.startContainer === root && range.startOffset === 1 });
    } catch (error) { checks.push({ name: method + " control", passed: false, detail: String(error.stack || error) }); }
    finally { root.remove(); }
  }
  globalThis.__uiEventResults = { checks, unloadObserved: checks.some(check => check.detail?.calls === 1) };
})();
