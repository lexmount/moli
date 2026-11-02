#!/usr/bin/env node
"use strict";

const http = require("http");
const path = require("path");

const endpoint = process.argv[2];
const fixture = process.argv[3];
if (!endpoint || !fixture) {
  throw new Error("usage: chrome_remote_interface_smoke.cjs ENDPOINT FIXTURE");
}

const moduleName = process.env.CHROME_REMOTE_INTERFACE_MODULE || "chrome-remote-interface";
const CDP = require(moduleName);
const packageJson = require(
  path.isAbsolute(moduleName) || moduleName.startsWith(".")
    ? path.join(moduleName, "package.json")
    : `${moduleName}/package.json`,
);
const expectedVersion = process.env.CHROME_REMOTE_INTERFACE_VERSION || "0.34.0";
if (packageJson.version !== expectedVersion) {
  throw new Error(
    `chrome-remote-interface ${packageJson.version} is installed; expected ${expectedVersion}`,
  );
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function readJson(url) {
  return new Promise((resolve, reject) => {
    const request = http.get(url, {timeout: 5000}, (response) => {
      let body = "";
      response.setEncoding("utf8");
      response.on("data", (chunk) => (body += chunk));
      response.on("end", () => {
        try {
          resolve(JSON.parse(body));
        } catch (error) {
          reject(new Error(`invalid JSON from ${url}: ${error.message}`));
        }
      });
    });
    request.on("timeout", () => request.destroy(new Error(`timeout fetching ${url}`)));
    request.on("error", reject);
  });
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function waitFor(probe, label, timeoutMilliseconds = 7000) {
  const deadline = Date.now() + timeoutMilliseconds;
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      const value = await probe();
      if (value) return value;
    } catch (error) {
      lastError = error;
    }
    await delay(25);
  }
  throw new Error(
    `timed out waiting for ${label}${lastError ? `; last error: ${lastError.message}` : ""}`,
  );
}

async function main() {
  const discovery = await readJson(`${endpoint.replace(/\/$/, "")}/json/version`);
  const browserWebSocket = discovery.webSocketDebuggerUrl;
  assert(typeof browserWebSocket === "string" && browserWebSocket, "missing browser websocket");
  const isMoli = typeof discovery["Moli-Version"] === "string";
  const results = [];
  const record = (name, data = {}) => results.push({name, ok: true, ...data});

  const client = await CDP({target: browserWebSocket, local: true});
  let browserContextId = null;
  const targetIds = [];
  try {
    const send = (method, params = {}, sessionId) => client.send(method, params, sessionId);
    const evaluate = async (sessionId, expression) => {
      const response = await send(
        "Runtime.evaluate",
        {expression, awaitPromise: true, returnByValue: true},
        sessionId,
      );
      if (response.exceptionDetails) {
        throw new Error(`Runtime.evaluate failed: ${JSON.stringify(response.exceptionDetails)}`);
      }
      return response.result ? response.result.value : undefined;
    };
    const waitReady = (sessionId, expectedPath) =>
      waitFor(
        async () =>
          await evaluate(
            sessionId,
            `document.readyState === "complete" && location.pathname === ${JSON.stringify(expectedPath)}`,
          ),
        `document ${expectedPath}`,
      );
    const navigate = async (sessionId, url) => {
      const response = await send("Page.navigate", {url}, sessionId);
      assert(!response.errorText, `Page.navigate failed: ${response.errorText}`);
      await waitReady(sessionId, new URL(url).pathname);
    };
    const attach = async (targetId) => {
      const {sessionId} = await send("Target.attachToTarget", {targetId, flatten: true});
      assert(typeof sessionId === "string" && sessionId, "attach returned no sessionId");
      await send("Page.enable", {}, sessionId);
      await send("Runtime.enable", {}, sessionId);
      await send("Network.enable", {}, sessionId);
      return sessionId;
    };
    const createPage = async (url) => {
      const response = await send("Target.createTarget", {url, browserContextId});
      assert(typeof response.targetId === "string", "createTarget returned no targetId");
      targetIds.push(response.targetId);
      const sessionId = await attach(response.targetId);
      await waitReady(sessionId, new URL(url).pathname);
      return {targetId: response.targetId, sessionId};
    };

    const version = await send("Browser.getVersion");
    assert(version.product === discovery.Browser, "live Browser.getVersion identity mismatch");
    ({browserContextId} = await send("Target.createBrowserContext"));
    assert(typeof browserContextId === "string" && browserContextId, "missing browserContextId");

    const firstUrl = `${fixture}/plain?client=cri-first`;
    const first = await createPage(firstUrl);
    const targets = await send("Target.getTargets");
    assert(
      targets.targetInfos.some((target) => target.targetId === first.targetId),
      "browser session cannot observe the page target",
    );
    record("cri_browser_page_session_binding", {
      clientVersion: packageJson.version,
      product: version.product,
    });

    await evaluate(
      first.sessionId,
      `localStorage.setItem("external-shared", "cri-local");
       sessionStorage.setItem("external-private", "cri-first");
       globalThis.__externalPageMarker = "first";`,
    );
    const second = await createPage(`${fixture}/plain?client=cri-second`);
    const secondStorage = await evaluate(
      second.sessionId,
      `({
        local: localStorage.getItem("external-shared"),
        session: sessionStorage.getItem("external-private"),
        marker: globalThis.__externalPageMarker || null,
      })`,
    );
    assert(secondStorage.local === "cri-local", `localStorage was not shared: ${JSON.stringify(secondStorage)}`);
    assert(secondStorage.session === null, `sessionStorage leaked to a second page: ${JSON.stringify(secondStorage)}`);
    assert(secondStorage.marker === null, `page realm leaked to a second page: ${JSON.stringify(secondStorage)}`);
    await evaluate(
      second.sessionId,
      `sessionStorage.setItem("external-private", "cri-second");
       globalThis.__externalPageMarker = "second";`,
    );
    const firstStorage = await evaluate(
      first.sessionId,
      `({
        local: localStorage.getItem("external-shared"),
        session: sessionStorage.getItem("external-private"),
        marker: globalThis.__externalPageMarker,
      })`,
    );
    assert(
      firstStorage.local === "cri-local" &&
        firstStorage.session === "cri-first" &&
        firstStorage.marker === "first",
      `first page storage/realm changed: ${JSON.stringify(firstStorage)}`,
    );
    record("cri_multi_page_storage_isolation");

    await navigate(first.sessionId, `${fixture}/history-a?client=cri`);
    await navigate(first.sessionId, `${fixture}/history-b?client=cri`);
    const history = await send("Page.getNavigationHistory", {}, first.sessionId);
    const expectedHistoryUrls = [
      firstUrl,
      `${fixture}/history-a?client=cri`,
      `${fixture}/history-b?client=cri`,
    ];
    const historyUrls = history.entries.map((entry) => entry.url);
    assert(
      JSON.stringify(historyUrls) === JSON.stringify(expectedHistoryUrls),
      `direct-target history mismatch: ${JSON.stringify(history.entries)}`,
    );
    assert(history.currentIndex === 2, `direct-target currentIndex was ${history.currentIndex}`);
    assert(
      history.entries[0].transitionType === "auto_toplevel",
      `direct-target initial transition was ${history.entries[0].transitionType}`,
    );
    const historyA = history.entries.find((entry) => new URL(entry.url).pathname === "/history-a");
    assert(historyA, `history-a entry missing: ${JSON.stringify(history.entries)}`);
    await send("Page.navigateToHistoryEntry", {entryId: historyA.id}, first.sessionId);
    await waitReady(first.sessionId, "/history-a");
    record("cri_navigation_history_workflow", {entryUrls: historyUrls});

    await navigate(first.sessionId, `${fixture}/iframe?client=cri`);
    const frameTree = await waitFor(async () => {
      const tree = (await send("Page.getFrameTree", {}, first.sessionId)).frameTree;
      return tree.childFrames && tree.childFrames.length === 1 ? tree : null;
    }, "child frame tree");
    const childFrame = frameTree.childFrames[0].frame;
    assert(childFrame.parentId === frameTree.frame.id, `wrong child parentId: ${JSON.stringify(childFrame)}`);
    const world = await send(
      "Page.createIsolatedWorld",
      {frameId: childFrame.id, worldName: "cri-external-smoke"},
      first.sessionId,
    );
    const childTextResponse = await send(
      "Runtime.evaluate",
      {
        expression: "document.body.textContent.trim()",
        contextId: world.executionContextId,
        returnByValue: true,
      },
      first.sessionId,
    );
    const childText = childTextResponse.result && childTextResponse.result.value;
    assert(String(childText).includes("child body text"), `wrong child frame text: ${JSON.stringify(childText)}`);
    record("cri_frame_tree_isolated_world", {childFrameId: childFrame.id});

    await navigate(first.sessionId, `${fixture}/plain?client=cri-fetch`);
    const routeUrl = `${fixture}/external-client-cri`;
    const networkEvents = {request: false, response: false, finished: false};
    let routeRequestId = null;
    const requestHandler = (params) => {
      if (params.request && params.request.url === routeUrl) {
        routeRequestId = params.requestId;
        networkEvents.request = true;
      }
    };
    const responseHandler = (params) => {
      if (params.response && params.response.url === routeUrl) networkEvents.response = true;
    };
    const finishedHandler = (params) => {
      if (params.requestId === routeRequestId) networkEvents.finished = true;
    };
    client.on(`Network.requestWillBeSent.${first.sessionId}`, requestHandler);
    client.on(`Network.responseReceived.${first.sessionId}`, responseHandler);
    client.on(`Network.loadingFinished.${first.sessionId}`, finishedHandler);
    let fulfillResolve;
    let fulfillReject;
    const fulfilled = new Promise((resolve, reject) => {
      fulfillResolve = resolve;
      fulfillReject = reject;
    });
    const pausedHandler = (params, eventSessionId) => {
      if (!params.request || params.request.url !== routeUrl) return;
      routeRequestId = params.networkId || null;
      send(
        "Fetch.fulfillRequest",
        {
          requestId: params.requestId,
          responseCode: 200,
          responseHeaders: [{name: "Content-Type", value: "application/json"}],
          body: Buffer.from('{"source":"cri"}', "utf8").toString("base64"),
        },
        first.sessionId,
      ).then(() => fulfillResolve(eventSessionId), fulfillReject);
    };
    client.on(`Fetch.requestPaused.${first.sessionId}`, pausedHandler);
    try {
      await send(
        "Fetch.enable",
        {patterns: [{urlPattern: "*external-client-cri*", requestStage: "Request"}]},
        first.sessionId,
      );
      const body = await evaluate(
        first.sessionId,
        `fetch(${JSON.stringify(routeUrl)}).then(response => response.text())`,
      );
      const eventSessionId = await fulfilled;
      assert(eventSessionId === first.sessionId, `Fetch event routed to ${eventSessionId}`);
      assert(body === '{"source":"cri"}', `wrong fulfilled body: ${JSON.stringify(body)}`);
      await waitFor(
        () => networkEvents.request && networkEvents.response && networkEvents.finished,
        "fulfilled request Network lifecycle",
      );
    } finally {
      await send("Fetch.disable", {}, first.sessionId);
      client.removeListener(`Fetch.requestPaused.${first.sessionId}`, pausedHandler);
      client.removeListener(`Network.requestWillBeSent.${first.sessionId}`, requestHandler);
      client.removeListener(`Network.responseReceived.${first.sessionId}`, responseHandler);
      client.removeListener(`Network.loadingFinished.${first.sessionId}`, finishedHandler);
    }
    record("cri_fetch_fulfill_network_lifecycle");

    await navigate(first.sessionId, `${fixture}/plain?client=cri-position`);
    const point = await evaluate(
      first.sessionId,
      `(() => {
        document.body.innerHTML = '<button id="position">position</button>';
        globalThis.__externalPositionClicks = 0;
        document.querySelector('#position').addEventListener('click', () => __externalPositionClicks += 1);
        const rect = document.querySelector('#position').getBoundingClientRect();
        return {x: rect.left + rect.width / 2, y: rect.top + rect.height / 2, width: rect.width, height: rect.height};
      })()`,
    );
    let positionError = null;
    try {
      await send(
        "Input.dispatchMouseEvent",
        {type: "mousePressed", x: point.x || 1, y: point.y || 1, button: "left", clickCount: 1},
        first.sessionId,
      );
      await send(
        "Input.dispatchMouseEvent",
        {type: "mouseReleased", x: point.x || 1, y: point.y || 1, button: "left", clickCount: 1},
        first.sessionId,
      );
    } catch (error) {
      positionError = String(error && error.message ? error.message : error);
    }
    const clickCount = await evaluate(first.sessionId, "globalThis.__externalPositionClicks");
    if (isMoli) {
      assert(
        positionError && /not supported|unsupported|layout hit testing/i.test(positionError),
        `Moli position click did not return an explicit capability error: ${positionError}`,
      );
      assert(clickCount === 0, `unsupported position click mutated the DOM: ${clickCount}`);
    } else {
      assert(!positionError, `Chromium position click failed: ${positionError}`);
      assert(point.width > 0 && point.height > 0, `Chromium returned an empty button rect: ${JSON.stringify(point)}`);
      assert(clickCount === 1, `Chromium position click count was ${clickCount}`);
    }
    record("cri_position_click_capability_boundary", {
      supported: !isMoli,
      clickCount,
    });

    return {ok: true, results};
  } finally {
    if (browserContextId) {
      try {
        await client.send("Target.disposeBrowserContext", {browserContextId});
      } catch {
        for (const targetId of targetIds) {
          try {
            await client.send("Target.closeTarget", {targetId});
          } catch {}
        }
      }
    }
    await client.close();
  }
}

main()
  .then((payload) => process.stdout.write(JSON.stringify(payload)))
  .catch((error) => {
    process.stderr.write(`${error && error.stack ? error.stack : error}\n`);
    process.exitCode = 1;
  });
