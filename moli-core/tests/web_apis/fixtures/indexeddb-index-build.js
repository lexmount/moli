globalThis.indexBuildProbe = async function (name = "idb-index-build") {
  const checks = globalThis.indexBuildChecks = [];
  const check = (label, pass, actual) => checks.push({ label, pass, actual });
  const same = (label, actual, expected) => check(label,
    JSON.stringify(actual) === JSON.stringify(expected), actual);
  const connections = [];
  const open = (suffix, version, upgrade) => new Promise((resolve, reject) => {
    const request = indexedDB.open(`${name}-${suffix}`, version);
    request.onupgradeneeded = () => {
      connections.push(request.result);
      try { upgrade(request.result, request.transaction); }
      catch (error) { request.transaction.abort(); reject(error); }
    };
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      connections.push(request.result);
      resolve(request.result);
    };
  });
  const result = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const finished = tx => new Promise(resolve => {
    tx.oncomplete = () => resolve("complete");
    tx.onabort = () => resolve("abort");
  });
  const throws = (label, expected, callback) => {
    try { callback(); check(label, false, "accepted"); }
    catch (error) { same(label, error.name, expected); }
  };
  const failedBuild = async (label, { persisted = false, keyPath = "group",
    multiEntry = false, records = [{ group: "same", rank: 1 }, { group: "same", rank: 1 }],
    after = () => {}, explicitAbort = false } = {}) => {
    if (persisted) {
      const db = await open(label, 1, db => {
        const store = db.createObjectStore("s");
        store.createIndex("original", "rank");
        records.forEach((value, i) => store.put(value, i + 1));
      });
      db.close();
    }
    const log = [];
    const requests = [];
    let tx, store, index, db;
    const outcome = await new Promise((resolve, reject) => {
      const request = indexedDB.open(`${name}-${label}`, persisted ? 2 : 1);
      request.onupgradeneeded = () => {
        try {
          db = request.result;
          connections.push(db);
          tx = request.transaction;
          tx.oncomplete = () => log.push("tx:complete");
          tx.onabort = () => log.push(`tx:abort:${tx.error?.name || "null"}`);
          db.onabort = () => log.push(`db:abort:${tx.error?.name || "null"}`);
          tx.onerror = event => log.push(`tx:error:${event.target.error.name}`);
          db.onerror = event => log.push(`db:error:${event.target.error.name}`);
          store = persisted ? tx.objectStore("s") : db.createObjectStore("s");
          const watch = (request, prefix, precedesIndex = false) => {
            requests.push({ request, prefix, precedesIndex });
            request.onsuccess = () => log.push(`${prefix}:success`);
            request.onerror = event => {
              log.push(`${prefix}:error:${request.error.name}`);
              // Canceling pending request errors cannot rescue an aborted index build.
              event.preventDefault();
            };
            return request;
          };
          if (!persisted) records.forEach((value, i) => watch(store.put(value, i + 1), `put${i}`, true));
          index = store.createIndex("unique", keyPath, { unique: true, multiEntry });
          check(`${label}:returns-index`, index instanceof IDBIndex && index.objectStore === store &&
            index.unique && index.multiEntry === multiEntry, index.name);
          check(`${label}:synchronous-metadata`, store.indexNames.contains("unique") && tx.error === null,
            Array.from(store.indexNames));
          after({ db, tx, store, index, watch });
          log.push("upgrade:return");
        } catch (error) { reject(error); }
      };
      request.onerror = event => {
        event.preventDefault();
        log.push(`open:error:${request.error.name}`);
        check(`${label}:open-transaction-cleared`, request.transaction === null, request.transaction);
        resolve("error");
      };
      request.onsuccess = () => {
        connections.push(request.result);
        log.push("open:success");
        resolve("success");
      };
    });
    same(`${label}:open-fails`, outcome, "error");
    same(`${label}:transaction-error`, tx.error?.name || null, explicitAbort ? null : "ConstraintError");
    check(`${label}:error-realm`, explicitAbort ? tx.error === null : tx.error instanceof DOMException,
      tx.error?.constructor.name);
    check(`${label}:abort-before-open-error`, log.at(-3) === `tx:abort:${explicitAbort ? "null" : "ConstraintError"}` &&
      log.at(-2) === `db:abort:${explicitAbort ? "null" : "ConstraintError"}` &&
      log.at(-1) === "open:error:AbortError", log);
    for (const { request, prefix, precedesIndex } of requests) {
      const success = precedesIndex && !explicitAbort;
      check(`${label}:${prefix}:result`, request.readyState === "done" &&
        (success ? request.error === null : request.error?.name === "AbortError"),
        request.error?.name || "success");
    }
    same(`${label}:version-rollback`, db.version, persisted ? 1 : 0);
    throws(`${label}:index-invalid-after-abort`, "InvalidStateError", () => index.get("same"));
    if (label === "ordered") {
      same("ordered:event-order", log, ["upgrade:return", "put0:success", "put1:success",
        "late:error:AbortError", "tx:error:AbortError", "db:error:AbortError",
        "tx:abort:ConstraintError", "db:abort:ConstraintError", "open:error:AbortError"]);
    }
    if (persisted) {
      db.close();
      const restored = await open(label, 1, () => check(`${label}:unexpected-upgrade`, false));
      const read = restored.transaction("s");
      const done = finished(read);
      const store = read.objectStore("s");
      same(`${label}:metadata-rollback`, Array.from(store.indexNames), ["original"]);
      same(`${label}:data-rollback`, await result(store.getAll()), records);
      same(`${label}:read-complete`, await done, "complete");
      restored.close();
    }
  };
  try {
    await failedBuild("ordered", { after: ({ store, watch }) => watch(store.put({ group: "same" }, 3), "late") });
    await failedBuild("clear", { after: ({ store, watch }) => watch(store.clear(), "clear") });
    await failedBuild("delete-index", { after: ({ store, watch }) => {
      store.deleteIndex("unique");
      watch(store.put({ group: "different" }, 2), "replace");
      store.createIndex("unique", "rank");
    } });
    await failedBuild("delete-store", { after: ({ db }) => db.deleteObjectStore("s") });
    await failedBuild("multiple-failures", { after: ({ store }) => store.createIndex("other", "group", { unique: true }) });
    await failedBuild("compound", { keyPath: ["group", "rank"] });
    await failedBuild("multientry-conflict", { multiEntry: true,
      records: [{ group: ["same", "same"] }, { group: ["same"] }] });
    await failedBuild("explicit-abort", { explicitAbort: true, after: ({ tx }) => tx.abort() });
    await failedBuild("persisted", { persisted: true });
    await failedBuild("commit", { persisted: true, after: ({ tx }) => tx.commit() });
    await failedBuild("persisted-clear", { persisted: true, after: ({ store, watch }) => watch(store.clear(), "clear") });

    for (const unique of [false, true]) {
      for (const before of [false, true]) {
        const label = `multi-${unique}-${before}`;
        const db = await open(label, 1, db => {
          const store = db.createObjectStore("s", { keyPath: "id" });
          const create = () => store.createIndex("tags", "tags", { multiEntry: true, unique });
          if (!before) create();
          store.put({ id: 1, tags: [1, 1, "a", "a", ["a", 2], ["a", 2], null, undefined, {}, NaN] });
          store.put({ id: 2, tags: [3, 3, "b", "b"] });
          store.put({ id: 3, tags: [] });
          store.put({ id: 4 });
          store.put({ id: 5, tags: "scalar" });
          if (before) create();
        });
        const tx = db.transaction("s");
        const done = finished(tx);
        const index = tx.objectStore("s").index("tags");
        const values = await Promise.all([
          result(index.count()), result(index.count("a")), result(index.getAllKeys()),
          result(index.getAll("a")), result(index.getAllKeys(["a", 2]))
        ]);
        same(`${label}:count`, values[0], 6);
        same(`${label}:one-entry-per-record`, values[1], 1);
        same(`${label}:keys`, values[2], [1, 2, 1, 2, 5, 1]);
        same(`${label}:get-all-once`, values[3].map(record => record.id), [1]);
        same(`${label}:array-key-equality`, values[4], [1]);
        same(`${label}:read-complete`, await done, "complete");

        const cursorTx = db.transaction("s");
        const cursorDone = finished(cursorTx);
        const keys = await new Promise((resolve, reject) => {
          const entries = [];
          const request = cursorTx.objectStore("s").index("tags").openKeyCursor();
          request.onerror = () => reject(request.error);
          request.onsuccess = () => {
            const cursor = request.result;
            if (!cursor) { resolve(entries); return; }
            entries.push([cursor.key, cursor.primaryKey]);
            cursor.continue();
          };
        });
        same(`${label}:cursor-no-duplicates`, keys, [[1, 1], [3, 2], ["a", 1], ["b", 2], ["scalar", 5], [["a", 2], 1]]);
        same(`${label}:cursor-complete`, await cursorDone, "complete");

        const write = db.transaction("s", "readwrite");
        const written = finished(write);
        const store = write.objectStore("s");
        // Replacing the same primary key is allowed even in a unique index.
        const saved = await Promise.all([
          result(store.put({ id: 1, tags: ["a", "a", ["a", 2], ["a", 2]] })),
          result(store.add({ id: 6, tags: unique ? ["c", "c"] : ["a", "a"] }))
        ]);
        same(`${label}:write-results`, saved, [1, 6]);
        same(`${label}:write-complete`, await written, "complete");

        if (unique) {
          const conflict = db.transaction("s", "readwrite");
          const settled = finished(conflict);
          const conflicting = conflict.objectStore("s").add({ id: 7, tags: ["a", "a"] });
          const error = await new Promise(resolve => {
            conflicting.onsuccess = () => resolve(null);
            conflicting.onerror = event => { event.preventDefault(); resolve(conflicting.error.name); };
          });
          same(`${label}:cross-record-conflict`, error, "ConstraintError");
          same(`${label}:canceled-write-error`, await settled, "complete");
        }
        const verify = db.transaction("s");
        const verified = finished(verify);
        const current = verify.objectStore("s");
        const verifiedValues = await Promise.all([result(current.index("tags").getAllKeys("a")), result(current.get(7))]);
        same(`${label}:updated-keys`, verifiedValues[0], unique ? [1] : [1, 6]);
        check(`${label}:failed-write-absent`, verifiedValues[1] === undefined, verifiedValues[1]);
        same(`${label}:verify-complete`, await verified, "complete");
        db.close();
      }
    }

    for (const unique of [false, true]) {
      const db = await open(`missing-${unique}`, 1, db => {
        const store = db.createObjectStore("s");
        const records = [{}, {}, { group: null }, { group: null }, { group: {} }, { group: NaN },
          { group: 1 }, { group: "1" }, { group: [] }, { group: [1, 2] }];
        if (!unique) records.push({ group: 1 });
        records.forEach((value, i) => store.put(value, i));
        store.createIndex("index", "group", { unique });
      });
      const tx = db.transaction("s");
      const done = finished(tx);
      const keys = await result(tx.objectStore("s").index("index").getAllKeys());
      same(`missing-${unique}:only-valid-keys`, keys, unique ? [6, 7, 8, 9] : [6, 10, 7, 8, 9]);
      same(`missing-${unique}:complete`, await done, "complete");
      db.close();
    }
    return { state: checks.every(check => check.pass) ? "pass" : "fail", checks };
  } finally {
    for (const db of connections) db.close();
  }
};
