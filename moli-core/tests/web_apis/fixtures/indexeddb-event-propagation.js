globalThis.idbEventProbe = async function (name = "idb-event-propagation") {
  const checks = globalThis.idbEventChecks = [];
  const check = (label, pass, actual) => checks.push({ label, pass, actual });
  const same = (label, actual, expected) => check(label,
    JSON.stringify(actual) === JSON.stringify(expected), actual);
  const report = event => {
    if (String(event.message).includes("idb-listener-throw")) {
      event.preventDefault();
    }
  };
  addEventListener("error", report);
  const db = await new Promise((resolve, reject) => {
    const request = indexedDB.open(name, 1);
    request.onupgradeneeded = () => request.result.createObjectStore("s").put("seed", 0);
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve(request.result);
  });
  const transaction = (mode, body) => new Promise((resolve, reject) => {
    const tx = db.transaction("s", mode);
    tx.oncomplete = () => resolve({ tx, aborted: false });
    tx.onabort = () => resolve({ tx, aborted: true });
    try { body(tx, tx.objectStore("s"), reject); } catch (error) { reject(error); }
  });
  const read = key => new Promise((resolve, reject) => {
    const request = db.transaction("s").objectStore("s").get(key);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  try {
    for (const mode of ["uncanceled", "database-cancel", "handler-false", "listener-false", "throw-after-cancel"]) {
      const log = [];
      let pendingRequest, failedRequest, retainedEvent;
      let dbCapture, dbBubble;
      const outcome = await transaction("readwrite", (tx, store) => {
        store.put(mode, mode);
        failedRequest = store.add("duplicate", 0);
        const logListener = (label, phase) => event => {
          if (event.target !== failedRequest) return;
          log.push(label);
          const current = label.startsWith("db-") ? db : label.startsWith("tx-") ? tx : failedRequest;
          check(`${mode}:${label}:target`, event.target === failedRequest && event.currentTarget === current &&
            event.eventPhase === phase && event.bubbles && event.cancelable && event.isTrusted,
            [event.type, event.eventPhase]);
          // DOM dispatch builds the whole request -> transaction -> database path.
          // Chromium's IndexedDB dispatcher currently exposes only currentTarget.
          same(`${mode}:${label}:path`, event.composedPath().map(value => value === db ? "db" : value === tx ? "tx" : "request"),
            ["request", "tx", "db"]);
        };
        dbCapture = logListener("db-capture", 1);
        dbBubble = event => {
          logListener("db-bubble", 3)(event);
          if (mode === "database-cancel") event.preventDefault();
        };
        db.addEventListener("error", dbCapture, true);
        db.addEventListener("error", dbBubble);
        tx.addEventListener("error", logListener("tx-capture", 1), true);
        tx.addEventListener("error", logListener("tx-bubble", 3));
        failedRequest.addEventListener("error", logListener("request-capture", 2), true);
        failedRequest.addEventListener("error", () => { log.push("before-handler"); });
        failedRequest.onerror = event => {
          retainedEvent = event;
          log.push("handler");
          check(`${mode}:active`, store.get(0) instanceof IDBRequest, tx.error);
          pendingRequest = store.put("must-rollback-on-abort", mode + "-pending");
          pendingRequest.onerror = event => event.preventDefault();
          if (mode === "handler-false") return false;
          if (mode === "throw-after-cancel") {
            event.preventDefault();
            throw new Error("idb-listener-throw:error");
          }
        };
        failedRequest.addEventListener("error", () => {
          log.push("after-handler");
          // This also runs after an earlier listener throws.
          check(`${mode}:still-active`, store.get(0) instanceof IDBRequest, "active");
          if (mode === "listener-false") return false;
        });
      });
      db.removeEventListener("error", dbCapture, true);
      db.removeEventListener("error", dbBubble);
      const expectedAbort = !["database-cancel", "handler-false"].includes(mode);
      check(`${mode}:outcome`, outcome.aborted === expectedAbort, outcome.aborted);
      check(`${mode}:transaction-error`, expectedAbort
        ? outcome.tx.error?.name === (mode === "throw-after-cancel" ? "AbortError" : "ConstraintError")
        : outcome.tx.error === null, outcome.tx.error?.name);
      check(`${mode}:request-error`, failedRequest.error.name === "ConstraintError", failedRequest.error.name);
      check(`${mode}:pending`, pendingRequest.readyState === "done" &&
        (expectedAbort ? pendingRequest.error?.name === "AbortError" : pendingRequest.error === null),
        pendingRequest.error?.name);
      // Abort notifications for the queued requests also visit the database;
      // keep the primary error's propagation order distinct in this assertion.
      same(`${mode}:order`, log, ["db-capture", "tx-capture", "request-capture",
        "before-handler", "handler", "after-handler", "tx-bubble", "db-bubble"]);
      check(`${mode}:dispatch-cleanup`, retainedEvent.currentTarget === null && retainedEvent.eventPhase === 0 &&
        retainedEvent.composedPath().length === 0, retainedEvent.eventPhase);
      check(`${mode}:storage`, await read(mode) === (expectedAbort ? undefined : mode), expectedAbort);
    }

    let completedRequest;
    const completed = await transaction("readonly", (tx, store) => {
      completedRequest = store.get(0);
    });
    check("inherited-methods", completedRequest.addEventListener === EventTarget.prototype.addEventListener &&
      completed.tx.removeEventListener === EventTarget.prototype.removeEventListener &&
      db.dispatchEvent === EventTarget.prototype.dispatchEvent, "EventTarget");
    for (const [prototype, names] of [[IDBRequest.prototype, ["onsuccess", "onerror"]],
      [IDBOpenDBRequest.prototype, ["onblocked", "onupgradeneeded"]],
      [IDBTransaction.prototype, ["onerror", "onabort", "oncomplete"]],
      [IDBDatabase.prototype, ["onerror", "onabort", "onclose", "onversionchange"]]]) {
      for (const key of names) {
        const descriptor = Object.getOwnPropertyDescriptor(prototype, key);
        check(`handler:${key}`, typeof descriptor?.get === "function" && typeof descriptor?.set === "function" &&
          descriptor.enumerable && descriptor.configurable, key);
      }
    }
    // Finished transactions retain their event parents even if public properties are shadowed.
    Object.defineProperty(completed.tx, "db", { value: null });
    Object.defineProperty(completedRequest, "transaction", { value: null });
    const order = [];
    const controller = new AbortController();
    const stopped = () => order.push("aborted-listener");
    db.addEventListener("custom", () => order.push("db-capture"), { capture: true, once: true });
    completed.tx.addEventListener("custom", () => order.push("tx-capture"), { capture: true, once: true });
    completedRequest.addEventListener("custom", () => order.push("request-capture"), { capture: true, once: true });
    completedRequest.addEventListener("custom", stopped, { signal: controller.signal });
    controller.abort();
    const listenerObject = { handleEvent() { check("callback-object:this", this === listenerObject, "object"); order.push("request-object"); } };
    completedRequest.addEventListener("custom", listenerObject, { once: true });
    completedRequest.addEventListener("custom", event => { event.preventDefault(); order.push("passive"); }, { passive: true, once: true });
    completed.tx.addEventListener("custom", () => order.push("tx-bubble"), { once: true });
    db.addEventListener("custom", event => { event.preventDefault(); order.push("db-bubble"); }, { once: true });
    const custom = new Event("custom", { bubbles: true, cancelable: true });
    check("script:canceled", EventTarget.prototype.dispatchEvent.call(completedRequest, custom) === false,
      custom.defaultPrevented);
    same("script:path-order", order, ["db-capture", "tx-capture", "request-capture", "request-object", "passive", "tx-bubble", "db-bubble"]);
    check("script:untrusted", !custom.isTrusted && custom.eventPhase === 0 && custom.currentTarget === null,
      custom.isTrusted);
    const onceCount = order.length;
    completedRequest.dispatchEvent(new Event("custom", { bubbles: true }));
    check("listeners:once-and-signal", order.length === onceCount, order.length);

    const stopOrder = [];
    const stoppedEvent = new Event("stopped", { bubbles: true });
    db.addEventListener("stopped", () => stopOrder.push("db"), true);
    completed.tx.addEventListener("stopped", event => {
      stopOrder.push("tx-stop");
      event.stopImmediatePropagation();
    }, { capture: true, once: true });
    completedRequest.addEventListener("stopped", () => stopOrder.push("request"), { once: true });
    completedRequest.dispatchEvent(stoppedEvent);
    same("stop:first", stopOrder, ["db", "tx-stop"]);
    completedRequest.dispatchEvent(stoppedEvent);
    // DOM dispatch clears both stop flags, including for IndexedDB targets.
    same("stop:reset", stopOrder, ["db", "tx-stop", "db", "request"]);

    const explicit = await transaction("readwrite", (tx, store) => {
      store.put("rolled-back", "explicit");
      tx.abort();
    });
    check("explicit:error-null", explicit.aborted && explicit.tx.error === null, explicit.tx.error?.name);
    check("explicit:rollback", await read("explicit") === undefined, "missing");
    try { explicit.tx.abort(); check("finished:abort", false, "accepted"); }
    catch (error) { check("finished:abort", error.name === "InvalidStateError", error.name); }

    return { state: checks.every(result => result.pass) ? "pass" : "fail", checks };
  } finally {
    db.close();
    removeEventListener("error", report);
  }
};
