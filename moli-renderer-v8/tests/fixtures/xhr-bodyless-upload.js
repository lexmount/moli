function probeBodylessXhrUpload() {
  const cases = [
    ["GET null", "GET", [null], false],
    ["GET ignores body", "GET", ["payload"], false],
    ["HEAD ignores body", "HEAD", ["payload"], false],
    ["POST omitted", "POST", [], false],
    ["POST null", "POST", [null], false],
    ["POST undefined", "POST", [undefined], false],
    ["POST empty string", "POST", [""], true],
    ["POST payload", "POST", ["payload"], true]
  ];
  for (const [label, method, args, uploadExpected] of cases) {
    const xhr = new XMLHttpRequest();
    const events = [];
    for (const type of ["loadstart", "progress", "abort", "error", "load", "loadend"]) {
      xhr.upload.addEventListener(type, () => events.push(`upload:${type}:${xhr.readyState}`));
    }
    xhr.onloadstart = () => {
      events.push(`xhr:loadstart:${xhr.readyState}`);
      xhr.abort();
    };
    xhr.onabort = () => events.push(`xhr:abort:${xhr.readyState}`);
    xhr.onloadend = () => events.push(`xhr:loadend:${xhr.readyState}`);
    xhr.open(method, "/unused");
    xhr.send(...args);
    events.push(`returned:${xhr.readyState}`);
    const expected = ["xhr:loadstart:1"];
    if (uploadExpected) expected.push("upload:abort:4", "upload:loadend:4");
    expected.push("xhr:abort:4", "xhr:loadend:4", "returned:0");
    if (JSON.stringify(events) !== JSON.stringify(expected)) {
      throw new Error(`${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(events)}`);
    }
  }
  return "ok";
}
