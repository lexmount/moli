import { assertFixture, expectNetworkFailure } from "./harness";
import type { CaseSpec, SmokeMeta } from "./types";

export interface NetworkStorageBoundaryResult {
  status: "ready";
  facts: Array<{
    name: string;
    value: string;
  }>;
}

type BoundaryFact = NetworkStorageBoundaryResult["facts"][number];
type PlatformFrame = "platform-1" | "platform-2";
type CapturePlatformFrame = (name: PlatformFrame) => Promise<void>;

interface RedirectResult {
  token: string;
  firstMethod: string;
  firstBody: string;
  middleMethod: string;
  middleBody: string;
  finalMethod: string;
  finalBody: string;
  trace: string;
}

interface StreamPayload {
  token: string;
  items: string[];
  text: string;
}

interface XhrPayload {
  token: string;
  state: string;
  values: number[];
}

interface CookieEcho {
  path: string;
  cookieNames: string[];
}

// Allow storage initialization and event delivery time on shared CI runners.
const EVENT_TIMEOUT_MS = 60_000;

function fact(name: string, value: unknown): BoundaryFact {
  return { name, value: String(value) };
}

function tokenFor(meta: SmokeMeta, spec: CaseSpec): string {
  return `${meta.framework}-${spec.seed}-${spec.variant}`;
}

function platformLog(host: HTMLElement): HTMLOListElement {
  let log = host.querySelector("[data-platform-log]");
  if (!log) {
    log = document.createElement("ol");
    log.setAttribute("data-platform-log", "");
    host.append(log);
  }
  assertFixture(log instanceof HTMLOListElement, "platform log is an ordered list");
  return log;
}

async function capturePlatformStep(
  host: HTMLElement,
  capture: CapturePlatformFrame,
  name: PlatformFrame,
  label: string,
  details: string[],
): Promise<void> {
  const item = document.createElement("li");
  item.dataset.platformStep = name;
  item.textContent = `${label}:${details.join("|")}`;
  platformLog(host).append(item);
  host.dataset.lastPlatformStep = name;
  await capture(name);
}

function withTimeout<T>(promise: Promise<T>, label: string): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error(`timed out waiting for ${label}`)),
      EVENT_TIMEOUT_MS,
    );
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error: unknown) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}

function errorName(value: unknown): string {
  return value instanceof Error || value instanceof DOMException
    ? value.name
    : Object.prototype.toString.call(value);
}

async function appendSrcdocFrame(
  host: HTMLElement,
  title: string,
  body: string,
): Promise<HTMLIFrameElement> {
  const frame = document.createElement("iframe");
  frame.title = title;
  const loaded = withTimeout(
    new Promise<void>((resolve, reject) => {
      frame.addEventListener("load", () => resolve(), { once: true });
      frame.addEventListener(
        "error",
        () => reject(new Error(`${title} failed to load`)),
        { once: true },
      );
    }),
    `${title} load`,
  );
  frame.srcdoc = `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>${title}</title></head><body>${body}</body></html>`;
  host.append(frame);
  await loaded;
  assertFixture(frame.contentWindow, `${title} exposes contentWindow`);
  assertFixture(frame.contentDocument, `${title} exposes contentDocument`);
  return frame;
}

async function redirectMethodChain(
  host: HTMLElement,
  meta: SmokeMeta,
  spec: CaseSpec,
  capture: CapturePlatformFrame,
): Promise<BoundaryFact[]> {
  const token = tokenFor(meta, spec);
  const body = new URLSearchParams({ stage: "created", framework: meta.framework });
  const request = new Request(
    `/support/network/redirect-307?token=${encodeURIComponent(token)}`,
    {
      method: "POST",
      body,
      headers: { "X-Smoke-Trace": `redirect:${token}` },
    },
  );
  await capturePlatformStep(host, capture, "platform-1", "redirect-request", [
    request.method,
    body.toString(),
  ]);

  const response = await fetch(request);
  assertFixture(response.ok, `redirect response returned ${response.status}`);
  const result = (await response.json()) as RedirectResult;
  assertFixture(result.firstMethod === "POST", "307 received the original POST method");
  assertFixture(result.middleMethod === "POST", "307 preserved POST at the middle hop");
  assertFixture(result.finalMethod === "GET", "303 changed the final request to GET");
  assertFixture(result.firstBody === body.toString(), "first hop received the request body");
  assertFixture(result.middleBody === body.toString(), "307 replayed the request body");
  assertFixture(result.finalBody === "", "303 removed the final request body");
  assertFixture(result.trace === `redirect:${token}`, "same-origin redirects retained headers");
  await capturePlatformStep(host, capture, "platform-2", "redirect-result", [
    `${result.firstMethod}>${result.middleMethod}>${result.finalMethod}`,
    String(response.redirected),
  ]);

  return [
    fact("method-chain", `${result.firstMethod}>${result.middleMethod}>${result.finalMethod}`),
    fact("body-chain", `${result.firstBody}|${result.middleBody}|${result.finalBody}`),
    fact("redirected", response.redirected),
    fact("trace", result.trace),
  ];
}

async function streamedResponseClone(
  host: HTMLElement,
  meta: SmokeMeta,
  spec: CaseSpec,
  capture: CapturePlatformFrame,
): Promise<BoundaryFact[]> {
  const token = tokenFor(meta, spec);
  const response = await fetch(
    `/support/network/stream-payload?token=${encodeURIComponent(token)}`,
  );
  assertFixture(response.ok, `stream payload returned ${response.status}`);
  const clone = response.clone();
  const body = response.body;
  assertFixture(body, "network response exposes a ReadableStream body");
  await capturePlatformStep(host, capture, "platform-1", "stream-ready", [
    String(response.status),
    String(response.bodyUsed),
    String(body.locked),
  ]);

  const [left, right] = body.tee();
  const [leftText, rightText, clonePayload] = await Promise.all([
    new Response(left).text(),
    new Response(right).text(),
    clone.json() as Promise<StreamPayload>,
  ]);
  assertFixture(leftText === rightText, "tee branches produced identical bytes");
  const parsed = JSON.parse(leftText) as StreamPayload;
  assertFixture(parsed.token === token, "tee payload retained its token");
  assertFixture(clonePayload.text === "café-東京", "clone decoded Unicode JSON");
  assertFixture(
    JSON.stringify(parsed) === JSON.stringify(clonePayload),
    "clone and tee payloads agree",
  );
  await capturePlatformStep(host, capture, "platform-2", "stream-consumed", [
    parsed.items.join(","),
    String(new TextEncoder().encode(leftText).byteLength),
  ]);

  return [
    fact("tee-equal", leftText === rightText),
    fact("items", parsed.items.join("|")),
    fact("unicode", clonePayload.text),
    fact("bytes", new TextEncoder().encode(leftText).byteLength),
  ];
}

async function abortGatedResponse(
  host: HTMLElement,
  meta: SmokeMeta,
  spec: CaseSpec,
  capture: CapturePlatformFrame,
): Promise<BoundaryFact[]> {
  const token = tokenFor(meta, spec);
  const controller = new AbortController();
  const responseUrl = new URL(
    `/support/network/gated-response?token=${encodeURIComponent(token)}`,
    location.href,
  ).href;
  expectNetworkFailure({
    label: "gated-fetch-abort",
    url: responseUrl,
    type: "Fetch",
    canceled: true,
  });
  const response = await fetch(responseUrl, { signal: controller.signal });
  const body = response.body;
  assertFixture(body, "gated response exposes a body stream");
  const reader = body.getReader();
  const first = await reader.read();
  assertFixture(!first.done, "gated response produced a first chunk");
  const firstText = new TextDecoder().decode(first.value).trim();
  assertFixture(firstText === `first:${token}`, "first gated chunk is deterministic");
  await capturePlatformStep(host, capture, "platform-1", "gated-first-chunk", [
    firstText,
  ]);

  const reason = new DOMException("fixture stop", "AbortError");
  controller.abort(reason);
  let readFailure = "missing";
  try {
    await reader.read();
  } catch (error: unknown) {
    readFailure = errorName(error);
  }
  const release = await fetch(
    `/support/network/release-response?token=${encodeURIComponent(token)}`,
  );
  const releaseResult = (await release.json()) as { released: boolean; token: string };
  assertFixture(controller.signal.aborted, "AbortController published the aborted state");
  assertFixture(controller.signal.reason === reason, "AbortSignal retained the exact reason");
  assertFixture(readFailure === "AbortError", "pending reader rejected with AbortError");
  assertFixture(releaseResult.token === token, "release endpoint used the same gate token");
  await capturePlatformStep(host, capture, "platform-2", "gated-aborted", [
    readFailure,
    String(controller.signal.aborted),
    String(releaseResult.released),
  ]);

  return [
    fact("first-chunk", firstText),
    fact("read-failure", readFailure),
    fact("reason", errorName(controller.signal.reason)),
    fact("released", releaseResult.released),
  ];
}

async function xhrLifecycleHeaders(
  host: HTMLElement,
  meta: SmokeMeta,
  spec: CaseSpec,
  capture: CapturePlatformFrame,
): Promise<BoundaryFact[]> {
  const token = tokenFor(meta, spec);
  const request = new XMLHttpRequest();
  const observed = { loadstart: false, load: false, loadend: false, error: false };
  for (const name of ["loadstart", "load", "loadend", "error"] as const) {
    request.addEventListener(name, () => {
      observed[name] = true;
    });
  }
  const completed = withTimeout(
    new Promise<void>((resolve) => request.addEventListener("loadend", () => resolve(), { once: true })),
    "XHR loadend",
  );
  request.open(
    "GET",
    `/support/network/xhr-payload?token=${encodeURIComponent(token)}`,
  );
  request.responseType = "json";
  request.send();
  const stateAfterSend = request.readyState;
  await capturePlatformStep(host, capture, "platform-1", "xhr-sent", [
    String(stateAfterSend),
    request.responseType,
  ]);

  await completed;
  const payload = request.response as XhrPayload;
  assertFixture(request.status === 206, "XHR retained the 206 response status");
  assertFixture(request.readyState === XMLHttpRequest.DONE, "XHR reached DONE");
  assertFixture(payload.token === token, "XHR decoded the JSON response");
  assertFixture(observed.loadstart && observed.load && observed.loadend, "XHR lifecycle fired");
  assertFixture(!observed.error, "XHR did not report a network error");
  const trace = request.getResponseHeader("x-smoke-trace");
  assertFixture(trace === `xhr:${token}`, "XHR exposes case-insensitive response headers");
  await capturePlatformStep(host, capture, "platform-2", "xhr-loaded", [
    String(request.status),
    payload.values.join(","),
    trace,
  ]);

  return [
    fact("state-after-send", stateAfterSend),
    fact("final-state", request.readyState),
    fact("status", request.status),
    fact("values", payload.values.join("|")),
    fact("trace", trace),
  ];
}

async function cacheStorageRoundtrip(
  host: HTMLElement,
  meta: SmokeMeta,
  spec: CaseSpec,
  capture: CapturePlatformFrame,
): Promise<BoundaryFact[]> {
  const token = tokenFor(meta, spec);
  const cacheName = `frontend-smoke-${token}`;
  await caches.delete(cacheName);
  const cache = await caches.open(cacheName);
  const request = new Request(
    `/support/network/cache-item?token=${encodeURIComponent(token)}`,
  );
  const network = await fetch(request);
  assertFixture(network.ok, "cache seed network request succeeded");
  await cache.put(request, network.clone());
  const seeded = await cache.match(request);
  assertFixture(seeded, "Cache.match finds the seeded response");
  await capturePlatformStep(host, capture, "platform-1", "cache-seeded", [
    cacheName,
    String(seeded.status),
    seeded.headers.get("x-smoke-cache") ?? "missing",
  ]);

  const seededText = await seeded.text();
  const manualRequest = new Request(`/support/network/cache-item?manual=${token}`);
  await cache.put(
    manualRequest,
    new Response(`manual-cache:${token}`, {
      status: 201,
      statusText: "Created",
      headers: { "X-Smoke-Cache": "manual" },
    }),
  );
  const manual = await cache.match(manualRequest);
  assertFixture(manual, "Cache.match finds the manual response");
  const manualText = await manual.text();
  const visibleNames = (await caches.keys()).filter((name) => name === cacheName);
  const deleted = await caches.delete(cacheName);
  assertFixture(seededText === `network-cache:${token}`, "network response body survived cache.put");
  assertFixture(manual.status === 201, "manual cached response retained its status");
  assertFixture(manualText === `manual-cache:${token}`, "manual cache body roundtripped");
  assertFixture(deleted, "CacheStorage.delete removed the case cache");
  await capturePlatformStep(host, capture, "platform-2", "cache-read-deleted", [
    seededText,
    manualText,
    String(deleted),
  ]);

  return [
    fact("cache-names", visibleNames.join("|")),
    fact("network-body", seededText),
    fact("manual-status", manual.status),
    fact("manual-body", manualText),
    fact("deleted", deleted),
  ];
}

function requestResult<T>(request: IDBRequest<T>, label: string): Promise<T> {
  return withTimeout(
    new Promise<T>((resolve, reject) => {
      request.addEventListener("success", () => resolve(request.result), { once: true });
      request.addEventListener(
        "error",
        () => reject(request.error ?? new Error(`${label} failed`)),
        { once: true },
      );
    }),
    label,
  );
}

function transactionDone(transaction: IDBTransaction, label: string): Promise<void> {
  return withTimeout(
    new Promise<void>((resolve, reject) => {
      transaction.addEventListener("complete", () => resolve(), { once: true });
      transaction.addEventListener(
        "abort",
        () => reject(transaction.error ?? new Error(`${label} aborted`)),
        { once: true },
      );
      transaction.addEventListener(
        "error",
        () => reject(transaction.error ?? new Error(`${label} failed`)),
        { once: true },
      );
    }),
    label,
  );
}

function deleteDatabase(name: string): Promise<void> {
  return requestResult(indexedDB.deleteDatabase(name), `delete IndexedDB ${name}`).then(
    () => undefined,
  );
}

function openDatabase(
  name: string,
  version: number,
  upgrade: (database: IDBDatabase, transaction: IDBTransaction) => void,
): Promise<IDBDatabase> {
  const request = indexedDB.open(name, version);
  request.addEventListener("upgradeneeded", () => {
    assertFixture(request.transaction, `${name} upgrade has a transaction`);
    upgrade(request.result, request.transaction);
  });
  request.addEventListener("blocked", () => {
    throw new Error(`${name} open was blocked`);
  });
  return requestResult(request, `open IndexedDB ${name} v${version}`);
}

function collectCursor(index: IDBIndex): Promise<string[]> {
  return withTimeout(
    new Promise<string[]>((resolve, reject) => {
      const rows: string[] = [];
      const request = index.openCursor();
      request.addEventListener("error", () => reject(request.error), { once: true });
      request.addEventListener("success", () => {
        const cursor = request.result;
        if (!cursor) {
          resolve(rows);
          return;
        }
        const value = cursor.value as { id: number; state: string; score: number };
        rows.push(`${value.id}:${value.state}:${value.score}`);
        cursor.continue();
      });
    }),
    "IndexedDB cursor",
  );
}

async function indexedDbVersionedCursor(
  host: HTMLElement,
  meta: SmokeMeta,
  spec: CaseSpec,
  capture: CapturePlatformFrame,
): Promise<BoundaryFact[]> {
  const token = tokenFor(meta, spec);
  const name = `frontend-smoke-${token}`;
  await deleteDatabase(name);
  const first = await openDatabase(name, 1, (database) => {
    const store = database.createObjectStore("records", { keyPath: "id" });
    store.createIndex("by-state", "state");
  });
  let versionChange = "missing";
  first.addEventListener("versionchange", (event) => {
    versionChange = `${event.oldVersion}>${event.newVersion ?? "null"}`;
    first.close();
  });
  const seedTransaction = first.transaction("records", "readwrite");
  const seedDone = transactionDone(seedTransaction, "IndexedDB seed transaction");
  const seedStore = seedTransaction.objectStore("records");
  seedStore.put({ id: 1, state: "active", score: 8 });
  seedStore.put({ id: 2, state: "paused", score: 3 });
  seedStore.put({ id: 3, state: "active", score: 13 });
  await seedDone;
  await capturePlatformStep(host, capture, "platform-1", "indexeddb-v1-seeded", [
    name,
    String(first.version),
    "3",
  ]);

  const second = await openDatabase(name, 2, (database, transaction) => {
    transaction.objectStore("records").createIndex("by-score", "score");
    database.createObjectStore("metadata");
  });
  const writeTransaction = second.transaction("records", "readwrite");
  const writeDone = transactionDone(writeTransaction, "IndexedDB version 2 write");
  writeTransaction.objectStore("records").put({
    id: 4,
    state: "archived",
    score: 5,
  });
  await writeDone;
  const readTransaction = second.transaction("records", "readonly");
  const rows = await collectCursor(readTransaction.objectStore("records").index("by-state"));
  await transactionDone(readTransaction, "IndexedDB cursor transaction");
  assertFixture(versionChange === "1>2", "versionchange closed the version 1 connection");
  assertFixture(rows.length === 4, "cursor returned all versioned records");
  assertFixture(second.objectStoreNames.contains("metadata"), "version 2 created metadata store");
  second.close();
  await deleteDatabase(name);
  await capturePlatformStep(host, capture, "platform-2", "indexeddb-v2-read", [
    versionChange,
    rows.join(","),
  ]);

  return [
    fact("versionchange", versionChange),
    fact("version", 2),
    fact("stores", "metadata|records"),
    fact("cursor", rows.join("|")),
    fact("deleted", true),
  ];
}

async function storageEventMultiframe(
  host: HTMLElement,
  meta: SmokeMeta,
  spec: CaseSpec,
  capture: CapturePlatformFrame,
): Promise<BoundaryFact[]> {
  const token = tokenFor(meta, spec);
  const key = `frontend-smoke-${token}`;
  localStorage.removeItem(key);
  localStorage.setItem(key, "zero");
  const firstFrame = await appendSrcdocFrame(
    host,
    "storage receiver one",
    '<output id="storage-output">waiting-one</output>',
  );
  const secondFrame = await appendSrcdocFrame(
    host,
    "storage receiver two",
    '<output id="storage-output">waiting-two</output>',
  );
  const frameEvents: string[][] = [[], []];
  let resolveEvents: (() => void) | undefined;
  const allEvents = withTimeout(
    new Promise<void>((resolve) => {
      resolveEvents = resolve;
    }),
    "six storage events",
  );
  [firstFrame, secondFrame].forEach((frame, index) => {
    const childWindow = frame.contentWindow;
    const childDocument = frame.contentDocument;
    assertFixture(childWindow, `storage frame ${index + 1} exposes a window`);
    assertFixture(childDocument, `storage frame ${index + 1} exposes a document`);
    childWindow.addEventListener("storage", (event) => {
      if (event.key !== key) {
        return;
      }
      const transition = `${event.oldValue ?? "null"}>${event.newValue ?? "null"}`;
      frameEvents[index].push(transition);
      const output = childDocument.querySelector("#storage-output");
      if (output) {
        output.textContent = frameEvents[index].join("|");
      }
      if (frameEvents[0].length + frameEvents[1].length === 6) {
        resolveEvents?.();
      }
    });
  });
  await capturePlatformStep(host, capture, "platform-1", "storage-frames-ready", [
    key,
    localStorage.getItem(key) ?? "missing",
  ]);

  localStorage.setItem(key, "one");
  localStorage.setItem(key, "two");
  localStorage.removeItem(key);
  await allEvents;
  const expected = "zero>one|one>two|two>null";
  assertFixture(frameEvents[0].join("|") === expected, "first frame saw all storage events");
  assertFixture(frameEvents[1].join("|") === expected, "second frame saw all storage events");
  await capturePlatformStep(host, capture, "platform-2", "storage-events-delivered", [
    frameEvents[0].join(","),
    frameEvents[1].join(","),
  ]);

  return [
    fact("frame-one", frameEvents[0].join("|")),
    fact("frame-two", frameEvents[1].join("|")),
    fact("final-value", localStorage.getItem(key) ?? "null"),
    fact("events", frameEvents[0].length + frameEvents[1].length),
  ];
}

async function cookiePathHttpOnly(
  host: HTMLElement,
  meta: SmokeMeta,
  spec: CaseSpec,
  capture: CapturePlatformFrame,
): Promise<BoundaryFact[]> {
  const token = tokenFor(meta, spec).replaceAll("-", "_");
  const rootName = `smoke_root_${token}`;
  const scopedName = `smoke_scoped_${token}`;
  const httpName = `smoke_http_${token}`;
  document.cookie = `${rootName}=root; Path=/; SameSite=Lax`;
  document.cookie = `${scopedName}=scoped; Path=/support/network/scoped; SameSite=Lax`;
  const setResponse = await fetch(
    `/support/network/set-cookie?name=${encodeURIComponent(httpName)}`,
  );
  assertFixture(setResponse.ok, "HttpOnly cookie endpoint succeeded");
  const visibleAtPage = document.cookie.split(";").map((item) => item.trim());
  assertFixture(visibleAtPage.some((item) => item.startsWith(`${rootName}=`)), "root cookie is visible");
  assertFixture(!visibleAtPage.some((item) => item.startsWith(`${scopedName}=`)), "scoped cookie is hidden at the case path");
  assertFixture(!visibleAtPage.some((item) => item.startsWith(`${httpName}=`)), "HttpOnly cookie is hidden from document.cookie");
  await capturePlatformStep(host, capture, "platform-1", "cookies-seeded", [
    rootName,
    scopedName,
    httpName,
  ]);

  const [rootEcho, scopedEcho] = await Promise.all([
    fetch("/support/network/cookie-echo").then((response) => response.json() as Promise<CookieEcho>),
    fetch("/support/network/scoped/cookie-echo").then((response) => response.json() as Promise<CookieEcho>),
  ]);
  assertFixture(rootEcho.cookieNames.includes(rootName), "root request included root cookie");
  assertFixture(rootEcho.cookieNames.includes(httpName), "root request included HttpOnly cookie");
  assertFixture(!rootEcho.cookieNames.includes(scopedName), "root request excluded scoped cookie");
  assertFixture(scopedEcho.cookieNames.includes(rootName), "scoped request included root cookie");
  assertFixture(scopedEcho.cookieNames.includes(scopedName), "scoped request included scoped cookie");
  assertFixture(scopedEcho.cookieNames.includes(httpName), "scoped request included HttpOnly cookie");
  document.cookie = `${rootName}=deleted; Path=/; Max-Age=0; SameSite=Lax`;
  document.cookie = `${scopedName}=deleted; Path=/support/network/scoped; Max-Age=0; SameSite=Lax`;
  await fetch(
    `/support/network/set-cookie?name=${encodeURIComponent(httpName)}&clear=1`,
  );
  await capturePlatformStep(host, capture, "platform-2", "cookie-paths-observed", [
    rootEcho.cookieNames.filter((name) => name.startsWith("smoke_")).join(","),
    scopedEcho.cookieNames.filter((name) => name.startsWith("smoke_")).join(","),
  ]);

  return [
    fact("page-visible", rootName),
    fact("root-request", rootEcho.cookieNames.filter((name) => name.startsWith("smoke_")).join("|")),
    fact("scoped-request", scopedEcho.cookieNames.filter((name) => name.startsWith("smoke_")).join("|")),
    fact("http-only-visible", false),
  ];
}

async function broadcastChannelRealmClone(
  host: HTMLElement,
  meta: SmokeMeta,
  spec: CaseSpec,
  capture: CapturePlatformFrame,
): Promise<BoundaryFact[]> {
  const token = tokenFor(meta, spec);
  const channelName = `frontend-smoke-${token}`;
  const frame = await appendSrcdocFrame(
    host,
    "broadcast receiver frame",
    '<output id="broadcast-output">waiting</output>',
  );
  const childWindow = frame.contentWindow;
  const childDocument = frame.contentDocument;
  assertFixture(childWindow, "broadcast frame exposes contentWindow");
  assertFixture(childDocument, "broadcast frame exposes contentDocument");
  const childRealm = childWindow as Window & typeof globalThis;
  const receiver = new BroadcastChannel(channelName);
  const childReceiver = new childRealm.BroadcastChannel(channelName);
  const sender = new BroadcastChannel(channelName);
  const topMessage = withTimeout(
    new Promise<MessageEvent>((resolve) => {
      receiver.addEventListener("message", (event) => resolve(event), { once: true });
    }),
    "top BroadcastChannel message",
  );
  const childMessage = withTimeout(
    new Promise<MessageEvent>((resolve) => {
      childReceiver.addEventListener("message", (event: MessageEvent) => resolve(event), {
        once: true,
      });
    }),
    "child BroadcastChannel message",
  );
  await capturePlatformStep(host, capture, "platform-1", "broadcast-channels-ready", [
    channelName,
    String(receiver.name === childReceiver.name),
  ]);

  sender.postMessage({
    label: "structured",
    list: [1, 2, 3],
    map: new Map<string, number>([["alpha", 7], ["beta", 11]]),
    set: new Set<string>(["north", "south"]),
  });
  const [topEvent, childEvent] = await Promise.all([topMessage, childMessage]);
  const topData = topEvent.data as {
    label: string;
    list: number[];
    map: Map<string, number>;
    set: Set<string>;
  };
  const childData = childEvent.data as typeof topData;
  assertFixture(topData.map instanceof Map, "top receiver reconstructed Map");
  assertFixture(childData.set instanceof childRealm.Set, "child receiver used the child Set realm");
  assertFixture(topData.map.get("beta") === 11, "structured Map retained entries");
  assertFixture(Array.from(childData.set).join("|") === "north|south", "structured Set retained values");
  const output = childDocument.querySelector("#broadcast-output");
  if (output) {
    output.textContent = `${childData.label}:${Array.from(childData.map).flat().join(":")}`;
  }
  receiver.close();
  childReceiver.close();
  sender.close();
  await capturePlatformStep(host, capture, "platform-2", "broadcast-delivered", [
    topData.list.join(","),
    Array.from(topData.map).flat().join(","),
    Array.from(childData.set).join(","),
  ]);

  return [
    fact("label", topData.label),
    fact("list", topData.list.join("|")),
    fact("map", Array.from(topData.map).map(([key, value]) => `${key}:${value}`).join("|")),
    fact("set", Array.from(childData.set).join("|")),
    fact("origins", `${topEvent.origin === location.origin}|${childEvent.origin === location.origin}`),
  ];
}

async function blobUrlStreamRevoke(
  host: HTMLElement,
  meta: SmokeMeta,
  spec: CaseSpec,
  capture: CapturePlatformFrame,
): Promise<BoundaryFact[]> {
  const token = tokenFor(meta, spec);
  const text = `blob:${token}:café:東京`;
  const blob = new Blob([text.slice(0, 8), text.slice(8)], {
    type: "text/plain;charset=utf-8",
  });
  const url = URL.createObjectURL(blob);
  const response = await fetch(url);
  const clone = response.clone();
  await capturePlatformStep(host, capture, "platform-1", "blob-fetched", [
    String(blob.size),
    blob.type,
    String(response.status),
  ]);

  const [responseText, bytes, streamText] = await Promise.all([
    response.text(),
    clone.arrayBuffer(),
    new Response(blob.stream()).text(),
  ]);
  URL.revokeObjectURL(url);
  expectNetworkFailure({
    label: "revoked-blob-fetch",
    url,
    type: "Fetch",
    canceled: false,
  });
  let revokedFetch = "resolved";
  try {
    await fetch(url);
  } catch (error: unknown) {
    revokedFetch = errorName(error);
  }
  assertFixture(responseText === text, "Blob URL fetch returned exact Unicode text");
  assertFixture(streamText === text, "Blob.stream returned exact Unicode text");
  assertFixture(bytes.byteLength === blob.size, "Blob fetch byte length matches Blob.size");
  assertFixture(revokedFetch === "TypeError", "revoked Blob URL fetch rejected with TypeError");
  await capturePlatformStep(host, capture, "platform-2", "blob-revoked", [
    responseText,
    String(bytes.byteLength),
    revokedFetch,
  ]);

  return [
    fact("text", responseText),
    fact("stream-text", streamText),
    fact("size", blob.size),
    fact("bytes", bytes.byteLength),
    fact("revoked-fetch", revokedFetch),
  ];
}

export async function runNetworkStorageBoundaryCase(
  host: HTMLElement,
  meta: SmokeMeta,
  spec: CaseSpec,
  capture: CapturePlatformFrame,
): Promise<NetworkStorageBoundaryResult> {
  host.replaceChildren();
  host.dataset.platformCase = spec.slug;
  let facts: BoundaryFact[];
  switch (spec.slug) {
    case "redirect-method-chain":
      facts = await redirectMethodChain(host, meta, spec, capture);
      break;
    case "streamed-response-clone":
      facts = await streamedResponseClone(host, meta, spec, capture);
      break;
    case "abort-gated-response":
      facts = await abortGatedResponse(host, meta, spec, capture);
      break;
    case "xhr-lifecycle-headers":
      facts = await xhrLifecycleHeaders(host, meta, spec, capture);
      break;
    case "cache-storage-roundtrip":
      facts = await cacheStorageRoundtrip(host, meta, spec, capture);
      break;
    case "indexeddb-versioned-cursor":
      facts = await indexedDbVersionedCursor(host, meta, spec, capture);
      break;
    case "storage-event-multiframe":
      facts = await storageEventMultiframe(host, meta, spec, capture);
      break;
    case "cookie-path-http-only":
      facts = await cookiePathHttpOnly(host, meta, spec, capture);
      break;
    case "broadcastchannel-realm-clone":
      facts = await broadcastChannelRealmClone(host, meta, spec, capture);
      break;
    case "blob-url-stream-revoke":
      facts = await blobUrlStreamRevoke(host, meta, spec, capture);
      break;
    default:
      throw new Error(`unknown network/storage boundary case: ${spec.slug}`);
  }
  host.dataset.platformComplete = "true";
  return { status: "ready", facts };
}
