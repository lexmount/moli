self.onmessage = async function (event) {
  const mode = event.data && event.data.mode;

  if (mode === "filelist") {
    let constructError = null;
    try {
      new FileList();
    } catch (error) {
      constructError = error && error.name;
    }
    postMessage({
      ctorOwn: Object.prototype.hasOwnProperty.call(self, "FileList"),
      ctorType: typeof FileList,
      ctorName: FileList.name,
      constructError: constructError,
      itemType: typeof FileList.prototype.item,
      lengthGetterType: typeof Object.getOwnPropertyDescriptor(
        FileList.prototype,
        "length",
      ).get,
      iterType: typeof FileList.prototype[Symbol.iterator],
    });
    close();
    return;
  }

  if (mode === "blob") {
    const response = new Response("hello from worker blob", {
      headers: { "Content-Type": "text/plain;charset=utf-8" },
    });
    const blob = await response.blob();
    const blobUrl = URL.createObjectURL(
      new Blob(
        [
          [
            "globalThis.__lmBlobImport = {",
            "  imported: true,",
            "  objectUrlType: typeof URL.createObjectURL",
            "};",
          ].join("\n"),
        ],
        { type: "text/javascript" },
      ),
    );
    importScripts(blobUrl);
    URL.revokeObjectURL(blobUrl);
    postMessage({
      blobCtor: typeof Blob,
      blobTag: Object.prototype.toString.call(blob),
      size: blob.size,
      type: blob.type,
      text: await blob.text(),
      blobUrlPrefix: blobUrl.startsWith("blob:" + location.origin + "/"),
      imported: globalThis.__lmBlobImport && globalThis.__lmBlobImport.imported,
      objectUrlType:
        globalThis.__lmBlobImport && globalThis.__lmBlobImport.objectUrlType,
    });
    close();
    return;
  }

  if (mode === "file-reader") {
    const file = new File(["hello worker file"], "note.txt", {
      type: "text/plain",
      lastModified: 123,
    });
    const fileText = await file.text();
    const reader = new FileReader();
    const events = [];
    reader.onloadstart = function () {
      events.push("loadstart");
    };
    reader.addEventListener("progress", function () {
      events.push("progress");
    });
    reader.onload = function () {
      events.push("load:" + reader.result);
    };
    reader.onloadend = function () {
      events.push("loadend:" + reader.readyState);
      postMessage({
        fileCtor: typeof File,
        readerCtor: typeof FileReader,
        fileTag: Object.prototype.toString.call(file),
        fileInstanceofBlob: file instanceof Blob,
        readerInstanceofEventTarget: reader instanceof EventTarget,
        fileName: file.name,
        fileLastModified: file.lastModified,
        fileType: file.type,
        fileText: fileText,
        constants: [FileReader.EMPTY, FileReader.LOADING, FileReader.DONE],
        events: events,
      });
      close();
    };
    events.push("before:" + reader.readyState);
    reader.readAsText(file);
    events.push("after:" + reader.readyState + ":" + String(reader.result === null));
    return;
  }

  if (mode === "file-reader-abort") {
    const reader = new FileReader();
    const events = [];
    reader.onload = function () {
      events.push("load");
    };
    reader.onabort = function () {
      events.push("abort");
    };
    reader.onloadend = function () {
      events.push("loadend");
    };
    reader.readAsText(new File(["cancel"], "cancel.txt"));
    reader.abort();
    setTimeout(function () {
      postMessage({
        readerReadyState: reader.readyState,
        resultIsNull: reader.result === null,
        events: events,
      });
      close();
    }, 0);
    return;
  }

  postMessage({ mode: mode, error: "unknown mode" });
  close();
};
