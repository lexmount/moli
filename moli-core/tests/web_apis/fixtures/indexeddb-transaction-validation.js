globalThis.transactionValidationProbe = async function (name = "transaction-validation") {
  const checks = globalThis.transactionValidationChecks = [];
  const check = (label, pass, actual) => checks.push({ label, pass, actual });
  const throws = (label, name, operation) => {
    try {
      const result = operation();
      if (result instanceof IDBTransaction) result.abort();
      check(label, false, "accepted");
    } catch (error) {
      const constructor = name === "TypeError" ? TypeError : DOMException;
      check(label, error.name === name && error instanceof constructor, error.name);
    }
  };
  const exactThrow = (label, sentinel, operation) => {
    try {
      operation();
      check(label, false, "accepted");
    } catch (error) {
      check(label, error === sentinel, String(error));
    }
  };
  let openSucceeded = false;
  const database = await new Promise((resolve, reject) => {
    const open = indexedDB.open(name, 1);
    open.onerror = () => reject(open.error);
    open.onupgradeneeded = () => {
      const db = open.result;
      const upgrade = open.transaction;
      const store = db.createObjectStore("a");
      for (const name of ["b", "", "null", "undefined"]) db.createObjectStore(name);
      for (const names of ["a", "missing", []]) {
        for (const mode of ["readonly", "readwrite", "versionchange"]) {
          throws(`upgrade:${JSON.stringify(names)}:${mode}`, "InvalidStateError",
            () => db.transaction(names, mode));
        }
      }
      throws("upgrade:invalid-enum", "TypeError", () => db.transaction([], "bogus"));
      queueMicrotask(() => {
        throws("upgrade:microtask", "InvalidStateError", () => db.transaction("a"));
      });
      let keepAlive = true;
      const pump = () => {
        const request = store.get(1);
        request.onerror = () => reject(request.error);
        request.onsuccess = () => {
          if (keepAlive) {
            pump();
          } else {
            upgrade.commit();
            throws("upgrade:committing", "InvalidStateError", () => db.transaction("a"));
          }
        };
      };
      pump();
      setTimeout(() => {
        // The pending request keeps the upgrade live across a separate task.
        throws("upgrade:inactive-request", "TransactionInactiveError", () => store.get(1));
        throws("upgrade:inactive", "InvalidStateError", () => db.transaction("a"));
        keepAlive = false;
        upgrade.oncomplete = () => {
          check("upgrade:transaction-during-complete", open.transaction === upgrade, "transaction");
          try {
            const tx = db.transaction("a");
            check("upgrade:complete", tx instanceof IDBTransaction, tx.mode);
            tx.objectStore("a").get(1).onsuccess = () => {
              check("upgrade:open-before-request", openSucceeded, openSucceeded);
            };
          } catch (error) {
            check("upgrade:complete", false, error.name);
          }
        };
      }, 0);
      store.get(2).onsuccess = () => {
        throws("upgrade:request-callback", "InvalidStateError", () => db.transaction("a"));
      };
    };
    open.onsuccess = () => {
      openSucceeded = true;
      check("upgrade:transaction-after-complete", open.transaction === null, "transaction");
      resolve(open.result);
    };
  });

  check("method:length", IDBDatabase.prototype.transaction.length === 1,
    IDBDatabase.prototype.transaction.length);
  const sentinel = {};
  let touched = false;
  const poisonNames = { get [Symbol.iterator]() { touched = true; throw sentinel; } };
  const revoked = Proxy.revocable(database, {});
  revoked.revoke();
  for (const [label, receiver] of [
    ["plain", {}], ["inherited", Object.create(database)],
    ["proxy", new Proxy(database, {})], ["revoked", revoked.proxy],
  ]) {
    throws(`receiver:${label}`, "TypeError",
      () => IDBDatabase.prototype.transaction.call(receiver, poisonNames));
  }
  check("receiver:before-conversion", !touched, touched);
  throws("required-argument", "TypeError", () => database.transaction());
  for (const names of ["missing", []]) {
    for (const mode of ["readonly", "readwrite", "versionchange"]) {
      throws(`scope:${JSON.stringify(names)}:${mode}`,
        names === "missing" ? "NotFoundError" : "InvalidAccessError",
        () => database.transaction(names, mode));
    }
  }
  for (const mode of [null, "bogus", "READONLY"]) {
    throws(`enum:${String(mode)}`, "TypeError", () => database.transaction([], mode));
  }
  throws("versionchange:valid-scope", "TypeError", () => database.transaction("a", "versionchange"));
  for (const name of [null, undefined, ""]) {
    const tx = database.transaction(name);
    check(`string:${String(name)}`, tx.objectStoreNames[0] === String(name), tx.objectStoreNames[0]);
  }

  const order = [];
  const names = {
    get [Symbol.iterator]() {
      order.push("iterator-get");
      return function* () {
        order.push("iterator-call");
        yield { toString() { order.push("name"); return "b"; } };
        yield "a";
        yield "b";
      };
    },
  };
  const mode = { toString() { order.push("mode"); return "readonly"; } };
  const transaction = database.transaction(names, mode);
  check("conversion:order-once", order.join() === "iterator-get,iterator-call,name,mode", order.join());
  check("scope:deduplicated", Array.from(transaction.objectStoreNames).join() === "a,b",
    Array.from(transaction.objectStoreNames).join());
  let modeTouched = false;
  const poisonMode = { toString() { modeTouched = true; throw sentinel; } };
  exactThrow("conversion:iterator-exception", sentinel, () => database.transaction(poisonNames, poisonMode));
  check("conversion:scope-before-mode", !modeTouched, modeTouched);
  const nonIterable = { [Symbol.iterator]: null, toString() { return "a"; } };
  check("conversion:string-fallback", database.transaction(nonIterable).objectStoreNames[0] === "a", "a");
  throws("conversion:non-callable-iterator", "TypeError",
    () => database.transaction({ [Symbol.iterator]: 1, toString() { return "a"; } }));
  const twice = {
    get [Symbol.iterator]() {
      Object.defineProperty(this, Symbol.iterator, { get() { throw sentinel; } });
      return function* () { yield "a"; };
    },
  };
  try {
    check("conversion:cached-iterator", database.transaction(twice).mode === "readonly", "readonly");
  } catch (error) {
    check("conversion:cached-iterator", false, String(error));
  }
  const first = database.transaction("a", "readwrite");
  first.objectStore("a").put("value", 1);
  throws("queued:missing-scope", "NotFoundError", () => database.transaction("missing", "readwrite"));
  throws("queued:empty-scope", "InvalidAccessError", () => database.transaction([], "readwrite"));
  const second = database.transaction(["a", "a"], "readwrite");
  check("queued:deduplicated", second.objectStoreNames.length === 1, second.objectStoreNames.length);
  const done = new Promise((resolve, reject) => {
    second.oncomplete = resolve;
    second.onabort = () => reject(second.error || new Error("queued transaction aborted"));
  });
  const read = second.objectStore("a").get(1);
  read.onsuccess = () => check("queued:record", read.result === "value", read.result);

  exactThrow("conversion:mode-exception", sentinel, () => database.transaction("missing", poisonMode));
  const closeMode = { toString() { database.close(); return "versionchange"; } };
  throws("conversion:closes-connection", "InvalidStateError", () => database.transaction("missing", closeMode));
  throws("closed:valid-enum", "InvalidStateError", () => database.transaction([], "versionchange"));
  throws("closed:invalid-enum", "TypeError", () => database.transaction([], "bogus"));
  throws("closed:null-enum", "TypeError", () => database.transaction([], null));
  await done;
  return { state: checks.every(result => result.pass) ? "pass" : "fail", checks };
};
