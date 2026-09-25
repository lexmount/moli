Promise.resolve().then(() => {
    opener.postMessage({kind: "then", title: window.document.title, self: window === window.parent}, "*");
    throw new Error("expected rejection");
}).catch(() => opener.postMessage({kind: "catch", title: window.document.title}, "*"));
(async () => {
    await 0;
    opener.postMessage({kind: "await", title: window.document.title}, "*");
})();
