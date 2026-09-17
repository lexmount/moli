globalThis.schemaValidationProbe = async function (name = "idb-schema-validation") {
  const checks = globalThis.schemaChecks = [];
  const check = (label, pass, actual) => checks.push({ label, pass, actual });
  const same = (label, actual, expected) => check(label,
    JSON.stringify(actual) === JSON.stringify(expected), actual);
  const throws = (label, expected, callback, realm = globalThis) => {
    try { callback(); check(label, false, "accepted"); }
    catch (error) { check(label, error.name === expected &&
      error instanceof (expected === "TypeError" ? realm.TypeError : realm.DOMException), error.name); }
  };
  const exactThrow = (label, sentinel, callback) => {
    try { callback(); check(label, false, "accepted"); }
    catch (error) { check(label, error === sentinel, String(error)); }
  };
  const connections = [];
  const open = (version, upgrade) => new Promise((resolve, reject) => {
    const request = indexedDB.open(name, version);
    request.onupgradeneeded = () => {
      connections.push(request.result);
      try { upgrade(request.result, request.transaction); }
      catch (error) { try { request.transaction.abort(); } catch {} reject(error); }
    };
    request.onsuccess = () => { connections.push(request.result); resolve({ db: request.result }); };
    request.onerror = event => { event.preventDefault(); resolve({ error: request.error }); };
  });
  const settled = tx => new Promise(resolve => {
    tx.oncomplete = () => resolve("complete");
    tx.onabort = () => resolve("abort");
  });
  const withModeGetter = (tx, getter, callback) => {
    const original = Object.getOwnPropertyDescriptor(tx, "mode");
    Object.defineProperty(tx, "mode", { configurable: true, get: getter });
    try { callback(); } finally {
      if (original) Object.defineProperty(tx, "mode", original);
      else delete tx.mode;
    }
  };
  let frame, other;
  if (typeof document !== "undefined") {
    frame = document.createElement("iframe");
    frame.srcdoc = "<!doctype html><p>schema realm</p>";
    const loaded = new Promise(resolve => frame.onload = resolve);
    document.documentElement.appendChild(frame);
    await loaded;
    other = frame.contentWindow;
  }
  let originalStore, deletedStore;
  try {
    const opened = await open(1, (db, tx) => {
      const store = originalStore = db.createObjectStore("base");
      store.createIndex("existing", "id");
      deletedStore = db.createObjectStore("deleted");
      db.deleteObjectStore("deleted");
      same("create-store:length", db.createObjectStore.length, 1);
      same("create-index:length", store.createIndex.length, 2);
      const valid = ["", "id", "true.$", "delete._", "my.k\u00f8i", "\u540d\u5b57.\u503c", "a0.b9",
        "a\u0301", "a\u200c\u200d", "\u2118", "\u309b", "\u{10400}.id", [""], ["id", "meta.name"]];
      valid.forEach((keyPath, i) => {
        const label = `valid-${i}`;
        try {
          same(`${label}:store`, db.createObjectStore(label, { keyPath }).keyPath, keyPath);
          same(`${label}:index`, store.createIndex(label, keyPath).keyPath, keyPath);
        } catch (error) { check(label, false, error.name); }
      });
      const invalid = [".", ".id", "id.", "id..name", "0id", "id.1", "with space", "id-name",
        "id[0]", "a,b", "\u0301a", "\u200ca", "\u200da", "a\0b", "a\nb", "a\ufeff", "\u{1f600}",
        "\ud800", "\udfff", "\\u0061", [], ["id", "bad path"], [["a", "b"]]];
      invalid.forEach((keyPath, i) => {
        const label = `invalid-${i}`;
        throws(`${label}:store`, "SyntaxError", () => db.createObjectStore(label, { keyPath, autoIncrement: true }));
        throws(`${label}:index`, "SyntaxError", () => store.createIndex(label, keyPath, { multiEntry: true }));
        check(`${label}:metadata`, !db.objectStoreNames.contains(label) && !store.indexNames.contains(label), "absent");
      });
      throws("store:syntax-before-name", "SyntaxError", () => db.createObjectStore("base", { keyPath: "bad path" }));
      throws("store:name-before-options", "ConstraintError", () => db.createObjectStore("base", { keyPath: "", autoIncrement: true }));
      throws("store:auto-increment-string", "InvalidAccessError", () => db.createObjectStore("fresh", { keyPath: "", autoIncrement: true }));
      throws("store:auto-increment-sequence", "InvalidAccessError", () => db.createObjectStore("fresh", { keyPath: ["id"], autoIncrement: true }));
      throws("index:name-before-syntax", "ConstraintError", () => store.createIndex("existing", "bad path"));
      throws("index:name-before-options", "ConstraintError", () => store.createIndex("existing", ["id"], { multiEntry: true }));
      throws("index:multi-entry", "InvalidAccessError", () => store.createIndex("fresh", ["id"], { multiEntry: true }));
      throws("deleted:before-syntax", "InvalidStateError", () => deletedStore.createIndex("fresh", "bad path"));
      throws("deleted:before-options", "InvalidStateError", () => deletedStore.createIndex("fresh", ["id"], { multiEntry: true }));
      throws("deleted:delete-index", "InvalidStateError", () => deletedStore.deleteIndex("missing"));

      const sentinel = new Error("schema conversion sentinel");
      let touched = 0;
      const poison = { toString() { touched++; throw sentinel; } };
      throws("index:arity-before-name", "TypeError", () => store.createIndex(poison));
      same("index:arity-no-conversion", touched, 0);
      for (const [label, receiver, method, args] of [
        ["database-plain", {}, db.createObjectStore, [poison]],
        ["database-inherited", Object.create(db), db.createObjectStore, [poison]],
        ["database-proxy", new Proxy(db, {}), db.createObjectStore, [poison]],
        ["database-delete", new Proxy(db, {}), db.deleteObjectStore, [poison]],
        ["store-plain", {}, store.createIndex, [poison, "id"]],
        ["store-inherited", Object.create(store), store.createIndex, [poison, "id"]],
        ["store-proxy", new Proxy(store, {}), store.createIndex, [poison, "id"]],
        ["store-delete", new Proxy(store, {}), store.deleteIndex, [poison]],
      ]) throws(`brand:${label}`, "TypeError", () => method.apply(receiver, args));
      same("brand:before-conversion", touched, 0);
      exactThrow("conversion:before-duplicate", sentinel, () => store.createIndex("existing", poison));
      const order = [];
      const iterable = () => ({
        get [Symbol.iterator]() {
          order.push("iterator-get");
          Object.defineProperty(this, Symbol.iterator, { get() { throw sentinel; } });
          return function* () { order.push("iterator-call"); yield { toString() { order.push("item"); return "id"; } }; };
        },
      });
      same("union:index-result", store.createIndex({ toString() { order.push("name"); return "union"; } }, iterable(), {
        get multiEntry() { order.push("multiEntry"); return false; },
        get unique() { order.push("unique"); return false; },
      }).keyPath, ["id"]);
      same("union:index-order-once", order.slice(), ["name", "iterator-get", "iterator-call", "item", "multiEntry", "unique"]);
      order.length = 0;
      same("union:store-result", db.createObjectStore({ toString() { order.push("name"); return "union"; } }, {
        get autoIncrement() { order.push("autoIncrement"); return false; },
        get keyPath() { order.push("keyPath"); return iterable(); },
      }).keyPath, ["id"]);
      same("union:store-order-once", order.slice(), ["name", "autoIncrement", "keyPath", "iterator-get", "iterator-call", "item"]);
      same("union:string-fallback", store.createIndex("fallback", { [Symbol.iterator]: null, toString() { return "id"; } }).keyPath, "id");
      same("union:string-object", store.createIndex("boxed", new String("xy")).keyPath, ["x", "y"]);
      same("union:set", store.createIndex("set", new Set(["id", "meta.name"])).keyPath, ["id", "meta.name"]);
      same("union:undefined-index", store.createIndex("undefined", undefined).keyPath, "undefined");
      same("union:null-index", store.createIndex("null", null).keyPath, "null");
      same("union:null-store", db.createObjectStore("null", { keyPath: null }).keyPath, null);
      throws("union:non-callable", "TypeError", () => store.createIndex("fresh", { [Symbol.iterator]: 1 }));
      throws("union:symbol", "TypeError", () => store.createIndex("fresh", Symbol()));
      exactThrow("dictionary:before-syntax", sentinel, () => store.createIndex("fresh", "bad path", { get unique() { throw sentinel; } }));
      let closed = false;
      const itemError = {
        [Symbol.iterator]() { return { next() { return { value: poison, done: false }; }, return() { closed = true; return {}; } }; },
      };
      exactThrow("union:item-exception", sentinel, () => store.createIndex("fresh", itemError));
      same("union:no-iterator-close", closed, false);
      throws("conversion:reentrant-name", "ConstraintError", () => store.createIndex("reentrant", {
        toString() { store.createIndex("reentrant", "id"); return "id"; },
      }));
      withModeGetter(tx, () => { throw sentinel; }, () => {
        check("state:native-mode", store.createIndex("native-mode", "id") instanceof IDBIndex, "created");
        store.deleteIndex("native-mode");
      });
      if (other) {
        const index = other.IDBObjectStore.prototype.createIndex.call(store, "cross", "id");
        check("realm:genuine-receiver", index.objectStore === store && index.name === "cross", "index");
        throws("realm:syntax", "SyntaxError", () => other.IDBObjectStore.prototype.createIndex.call(store, "fresh", "bad path"), other);
        throws("realm:constraint", "ConstraintError", () => other.IDBObjectStore.prototype.createIndex.call(store, "existing", "id"), other);
        throws("realm:arity", "TypeError", () => other.IDBObjectStore.prototype.createIndex.call(store, poison), other);
        throws("realm:conversion", "TypeError", () => other.IDBObjectStore.prototype.createIndex.call(store, "fresh", Symbol()), other);
        throws("realm:delete-index", "NotFoundError", () => other.IDBObjectStore.prototype.deleteIndex.call(store, "missing"), other);
        other.IDBObjectStore.prototype.deleteIndex.call(store, "cross");
        check("realm:deleted-index", !store.indexNames.contains("cross"), "absent");
        throws("realm:database", "SyntaxError", () => other.IDBDatabase.prototype.createObjectStore.call(db, "fresh", { keyPath: "bad path" }), other);
        throws("realm:receiver", "TypeError", () => other.IDBObjectStore.prototype.createIndex.call(new Proxy(store, {}), poison, "id"), other);
      }
    });
    if (opened.error) throw opened.error;
    const db = opened.db;
    throws("finished:index", "TransactionInactiveError", () => originalStore.createIndex("existing", ["bad path"], { multiEntry: true }));
    throws("finished:delete-index", "TransactionInactiveError", () => originalStore.deleteIndex("missing"));
    throws("finished:database", "InvalidStateError", () => db.createObjectStore("base", { keyPath: "bad path", autoIncrement: true }));
    throws("finished:delete-store", "InvalidStateError", () => db.deleteObjectStore("missing"));
    throws("finished:deleted", "InvalidStateError", () => deletedStore.createIndex("fresh", "bad path"));
    for (const mode of ["readonly", "readwrite"]) {
      const tx = db.transaction("base", mode);
      const done = settled(tx);
      const store = tx.objectStore("base");
      withModeGetter(tx, () => { throw new Error("public mode accessed"); }, () => {
        throws(`${mode}:mode-before-syntax`, "InvalidStateError", () => store.createIndex("existing", "bad path"));
        throws(`${mode}:mode-before-options`, "InvalidStateError", () => store.createIndex("existing", ["id"], { multiEntry: true }));
        throws(`${mode}:delete-index`, "InvalidStateError", () => store.deleteIndex("missing"));
      });
      same(`${mode}:complete`, await done, "complete");
    }
    db.close();
    const aborted = await open(2, (db, tx) => {
      const store = tx.objectStore("base");
      const fresh = db.createObjectStore("temporary");
      // Argument conversion can change transaction state before semantic checks.
      throws("conversion:abort-before-syntax", "TransactionInactiveError", () => store.createIndex("fresh", {
        toString() { tx.abort(); return "bad path"; },
      }));
      throws("abort:existing-store", "TransactionInactiveError", () => store.createIndex("existing", ["bad path"], { multiEntry: true }));
      throws("abort:new-store", "InvalidStateError", () => fresh.createIndex("fresh", "bad path"));
      throws("abort:database", "TransactionInactiveError", () => db.createObjectStore("base", { keyPath: "bad path" }));
      throws("abort:delete-store", "TransactionInactiveError", () => db.deleteObjectStore("missing"));
      throws("abort:delete-index", "TransactionInactiveError", () => store.deleteIndex("missing"));
    });
    same("abort:open-error", aborted.error?.name, "AbortError");
    const inactive = await open(2, (db, tx) => {
      const store = tx.objectStore("base");
      let keepAlive = true;
      const pump = () => {
        store.get(1).onsuccess = () => {
          if (keepAlive) pump();
          else {
            tx.commit();
            throws("committing:index", "TransactionInactiveError", () => store.createIndex("existing", "bad path"));
            throws("committing:database", "TransactionInactiveError", () => db.createObjectStore("base", { keyPath: "bad path" }));
          }
        };
      };
      pump();
      setTimeout(() => {
        throws("inactive:index", "TransactionInactiveError", () => store.createIndex("existing", ["bad path"], { multiEntry: true }));
        throws("inactive:database", "TransactionInactiveError", () => db.createObjectStore("base", { keyPath: "bad path" }));
        throws("inactive:delete-index", "TransactionInactiveError", () => store.deleteIndex("missing"));
        throws("inactive:delete-store", "TransactionInactiveError", () => db.deleteObjectStore("missing"));
        keepAlive = false;
      }, 0);
    });
    same("inactive:upgrade-completes", inactive.db?.version, 2);
    return { state: checks.every(check => check.pass) ? "pass" : "fail", checks };
  } finally {
    for (const db of connections) db.close();
    if (frame) frame.remove();
  }
};
