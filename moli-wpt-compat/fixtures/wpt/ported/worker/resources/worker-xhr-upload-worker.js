self.onmessage = function (event) {
  const data = event.data || {};
  const payload = data.payload;
  const kind = data.kind || "payload";
  const xhr = new XMLHttpRequest();
  const uploadEvents = [];
  const uploadOrder = [];
  const xhrEvents = [];

  ["loadstart", "progress", "load", "abort", "loadend"].forEach(function (type) {
    xhr.upload.addEventListener(type, function (uploadEvent) {
      uploadOrder.push("listener-before:" + uploadEvent.type);
    });
    xhr.upload["on" + type] = function (uploadEvent) {
      uploadOrder.push("handler:" + uploadEvent.type);
    };
    xhr.upload.addEventListener(type, function (uploadEvent) {
      uploadEvents.push(
        [
          uploadEvent.type,
          uploadEvent.target === xhr.upload,
          uploadEvent.currentTarget === xhr.upload,
          uploadEvent.lengthComputable,
          uploadEvent.loaded,
          uploadEvent.total,
        ].join(":"),
      );
      uploadOrder.push("listener-after:" + uploadEvent.type);
      if (
        (kind === "abort-from-loadstart" && uploadEvent.type === "loadstart") ||
        (kind === "abort-from-progress" && uploadEvent.type === "progress")
      ) {
        xhr.abort();
      }
    });
  });

  xhr.onabort = function () {
    xhrEvents.push("abort");
  };
  xhr.onload = function () {
    xhrEvents.push("load");
  };
  xhr.onloadend = function () {
    xhrEvents.push("loadend");
    // An abort from the final upload progress callback fires XHR loadend
    // before the upload's remaining events. Collect after that task finishes.
    setTimeout(() => {
      postMessage({
        kind,
        status: xhr.status,
        response: xhr.response,
        uploadEvents,
        uploadOrder,
        xhrEvents,
      });
      close();
    }, 0);
  };
  xhr.open("POST", "/wpt/runtime/xhr/echo-body");
  xhr.responseType = "json";
  if (kind === "payload") {
    xhr.setRequestHeader("Content-Type", "text/plain;charset=utf-8");
    xhr.send(payload);
  } else if (kind === "blob") {
    xhr.send(new Blob(["blob-upload"], { type: "text/plain" }));
  } else if (kind === "buffer") {
    xhr.send(new Uint8Array([65, 66, 67, 0, 68]));
  } else if (kind === "formdata") {
    const data = new FormData();
    data.append("alpha", "one");
    data.append("file", new Blob(["blob-body"], { type: "text/plain" }));
    xhr.send(data);
  } else if (kind === "none") {
    xhr.send();
  } else if (kind === "null") {
    xhr.send(null);
  } else if (kind === "empty-string") {
    xhr.send("");
  } else if (kind === "abort-from-loadstart") {
    xhr.send("abort=upload");
  } else if (kind === "abort-from-progress") {
    xhr.send("abort=upload");
  } else {
    postMessage({
      kind,
      error: "unknown send kind: " + kind,
      uploadEvents,
      uploadOrder,
      xhrEvents,
    });
    close();
  }
};
