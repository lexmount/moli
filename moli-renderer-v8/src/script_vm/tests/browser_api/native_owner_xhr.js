(async function nativeOwnerXhrProbe() {
  function probe(worker) {
    const checks = [];
    const record = (name, actual, expected) =>
      checks.push({ name, actual, passed: actual === expected });
    const xhr = new XMLHttpRequest();
    xhr.open("GET", "data:text/plain,owner", true);
    record("async-open", xhr.readyState, XMLHttpRequest.OPENED);
    xhr.responseType = "arraybuffer";
    record("async-response-type", xhr.responseType, "arraybuffer");
    xhr.abort();
    record("async-abort-before-send", xhr.readyState, XMLHttpRequest.OPENED);
    const sync = new XMLHttpRequest();
    sync.open("GET", "data:text/plain,owner", false);
    record("sync-open", sync.readyState, XMLHttpRequest.OPENED);
    let error = null;
    try {
      sync.responseType = "arraybuffer";
    } catch (exception) {
      error = exception.name;
    }
    record(
      "sync-response-type-error",
      error,
      worker ? null : "InvalidAccessError",
    );
    record(
      "sync-response-type",
      sync.responseType,
      worker ? "arraybuffer" : "",
    );
    sync.abort();
    return checks;
  }
  const results = { window: probe(false) };
  for (const kind of ["dedicated", "shared"]) {
    const source =
      "const probe=" +
      probe.toString() +
      ";" +
      (kind === "dedicated"
        ? "postMessage(probe(true));"
        : "onconnect=event=>event.ports[0].postMessage(probe(true));");
    const url = URL.createObjectURL(
      new Blob([source], { type: "text/javascript" }),
    );
    let worker;
    try {
      results[kind] = await new Promise((resolve, reject) => {
        worker = kind === "dedicated" ? new Worker(url) : new SharedWorker(url);
        worker.onerror = (event) => reject(new Error(event.message));
        const target = kind === "dedicated" ? worker : worker.port;
        target.onmessage = (event) => resolve(event.data);
        target.onmessageerror = () => reject(new Error("messageerror"));
        if (kind === "shared") target.start();
      });
    } finally {
      if (worker) {
        if (kind === "dedicated") worker.terminate();
        else worker.port.close();
      }
      URL.revokeObjectURL(url);
    }
  }
  return {
    total: Object.values(results).reduce((n, checks) => n + checks.length, 0),
    failures: Object.entries(results).flatMap(([realm, checks]) =>
      checks.filter((row) => !row.passed).map((row) => ({ realm, ...row })),
    ),
    results,
  };
});
