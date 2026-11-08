globalThis.idbCommittingProbe = async function (name = "idb-committing-lifecycle") {
  const checks = globalThis.idbCommittingChecks = [];
  const check = (label, pass, actual) => checks.push({ label, pass, actual });
  const same = (label, actual, expected) => check(label,
    JSON.stringify(actual) === JSON.stringify(expected), actual);
  const errors = [];
  const report = event => {
    if (String(event.message).includes("idb-listener-throw")) {
      errors.push(event.error?.message || event.message);
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
  const throws = (label, expected, callback) => {
    try { callback(); check(label, false, "accepted"); }
    catch (error) { check(label, error.name === expected, error.name); }
  };
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
    const explicit = await transaction("readwrite", (tx, store) => {
      store.put("rolled-back", "explicit");
      tx.abort();
    });
    check("explicit:error-null", explicit.aborted && explicit.tx.error === null, explicit.tx.error?.name);
    check("explicit:rollback", await read("explicit") === undefined, "missing");
    try { explicit.tx.abort(); check("finished:abort", false, "accepted"); }
    catch (error) { check("finished:abort", error.name === "InvalidStateError", error.name); }

    const successThrow = await transaction("readwrite", (tx, store) => {
      const request = store.put("rolled-back", "throw-success");
      request.onsuccess = () => { throw new Error("idb-listener-throw:success"); };
      request.addEventListener("success", () => {
        check("success-throw:remaining-listener-active", store.get(0) instanceof IDBRequest, "active");
      });
    });
    check("success-throw:aborted", successThrow.aborted && successThrow.tx.error?.name === "AbortError", successThrow.tx.error?.name);
    check("success-throw:rollback", await read("throw-success") === undefined, "missing");

    const synthetic = await transaction("readwrite", (tx, store) => {
      const request = store.put("saved", "synthetic");
      request.addEventListener("error", () => { throw new Error("idb-listener-throw:synthetic"); });
      request.dispatchEvent(new Event("error", { bubbles: true, cancelable: true }));
    });
    check("synthetic:does-not-abort", !synthetic.aborted && synthetic.tx.error === null, synthetic.tx.error?.name);
    check("synthetic:committed", await read("synthetic") === "saved", "saved");

    for (const timing of ["before-success", "in-success"]) {
      const outcome = await transaction("readwrite", (tx, store) => {
        const request = store.put("saved", timing);
        request.onsuccess = () => {
          if (timing === "in-success") tx.commit();
          throws(`${timing}:callback-inactive`, "TransactionInactiveError", () => store.get(0));
          throw new Error("idb-listener-throw:" + timing);
        };
        request.addEventListener("success", () => {
          throws(`${timing}:remaining-listener-inactive`, "TransactionInactiveError", () => store.get(0));
        });
        if (timing === "before-success") {
          tx.commit();
          throws("commit:immediately-inactive", "TransactionInactiveError", () => store.get(0));
          throws("commit:cannot-commit-twice", "InvalidStateError", () => tx.commit());
          throws("commit:cannot-abort", "InvalidStateError", () => tx.abort());
        }
      });
      check(`${timing}:committed`, !outcome.aborted && outcome.tx.error === null, outcome.tx.error?.name);
      check(`${timing}:storage`, await read(timing) === "saved", timing);
    }

    for (const timing of ["before-error", "in-error"]) {
      let pendingRequest;
      const outcome = await transaction("readwrite", (tx, store) => {
        store.put("saved", timing);
        const request = store.add("duplicate", 0);
        pendingRequest = store.get(0);
        pendingRequest.onerror = event => event.preventDefault();
        request.onerror = event => {
          event.preventDefault();
          check(`${timing}:error-during-dispatch`, timing === "before-error"
            ? tx.error?.name === "ConstraintError" && request.error?.name === "AbortError"
            : tx.error === null && request.error?.name === "ConstraintError",
            [tx.error?.name, request.error?.name]);
          if (timing === "in-error") tx.commit();
          throws(`${timing}:callback-inactive`, "TransactionInactiveError", () => store.get(0));
          throw new Error("idb-listener-throw:" + timing);
        };
        if (timing === "before-error") tx.commit();
      });
      const shouldAbort = timing === "before-error";
      check(`${timing}:outcome`, outcome.aborted === shouldAbort, outcome.aborted);
      check(`${timing}:error`, shouldAbort ? outcome.tx.error?.name === "ConstraintError" : outcome.tx.error === null,
        outcome.tx.error?.name);
      check(`${timing}:pending`, shouldAbort ? pendingRequest.error?.name === "AbortError" : pendingRequest.error === null,
        pendingRequest.error?.name);
      check(`${timing}:storage`, await read(timing) === (shouldAbort ? undefined : "saved"), shouldAbort);
    }

    let timer;
    await transaction("readwrite", (tx, store) => {
      const request = store.put("saved", "auto-commit");
      timer = new Promise(resolve => {
        request.onsuccess = () => setTimeout(() => {
          throws("auto-commit:cannot-abort", "InvalidStateError", () => tx.abort());
          resolve();
        }, 0);
      });
    });
    await timer;
    return { state: checks.every(result => result.pass) ? "pass" : "fail", checks };
  } finally {
    db.close();
    removeEventListener("error", report);
  }
};
