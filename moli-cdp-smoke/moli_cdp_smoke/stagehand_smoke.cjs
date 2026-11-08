#!/usr/bin/env node
"use strict";

const http = require("http");
const path = require("path");

const realStdoutWrite = process.stdout.write.bind(process.stdout);
process.stdout.write = (chunk, encoding, callback) =>
  process.stderr.write(chunk, encoding, callback);

const endpoint = process.argv[2];
const fixture = process.argv[3];
if (!endpoint || !fixture) {
  throw new Error("usage: stagehand_smoke.cjs ENDPOINT FIXTURE");
}

const moduleName = process.env.STAGEHAND_MODULE || "@browserbasehq/stagehand";
const packageJson = require(
  path.isAbsolute(moduleName) || moduleName.startsWith(".")
    ? path.join(moduleName, "package.json")
    : `${moduleName}/package.json`,
);
const expectedVersion = process.env.STAGEHAND_VERSION || "3.7.0";
if (packageJson.version !== expectedVersion) {
  throw new Error(`Stagehand ${packageJson.version} is installed; expected ${expectedVersion}`);
}
const {Stagehand} = require(moduleName);

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

async function main() {
  const discovery = await readJson(`${endpoint.replace(/\/$/, "")}/json/version`);
  const browserWebSocket = discovery.webSocketDebuggerUrl;
  assert(typeof browserWebSocket === "string" && browserWebSocket, "missing browser websocket");
  const isMoli = typeof discovery["Moli-Version"] === "string";
  const results = [];
  const record = (name, data = {}) => results.push({name, ok: true, ...data});
  const createdPages = [];

  const stagehand = new Stagehand({
    env: "LOCAL",
    localBrowserLaunchOptions: {cdpUrl: browserWebSocket},
    verbose: 0,
    disablePino: true,
  });
  try {
    await stagehand.init();
    const page = await stagehand.context.newPage();
    createdPages.push(page);
    const liveVersion = await page.sendCDP("Browser.getVersion");
    assert(liveVersion.product === discovery.Browser, "live Stagehand transport identity mismatch");
    record("stagehand_explicit_cdp_binding", {
      clientVersion: packageJson.version,
      product: liveVersion.product,
    });

    await page.goto(`${fixture}/plain?client=stagehand`, {
      waitUntil: "load",
      timeoutMs: 10000,
    });
    await page.evaluate(`(() => {
      document.body.innerHTML = '<input id="name"><div id="shadow-host"></div>';
      globalThis.__stagehandFillEvents = [];
      document.querySelector('#name').addEventListener('input', event => {
        __stagehandFillEvents.push({type: event.type, value: event.target.value});
      });
      const root = document.querySelector('#shadow-host').attachShadow({mode: 'open'});
      setTimeout(() => {
        const marker = document.createElement('span');
        marker.id = 'shadow-value';
        marker.textContent = 'shadow ready';
        root.appendChild(marker);
      }, 50);
      return 'ready';
    })()`);
    await page.locator("#name").fill("stagehand value");
    const inputValue = await page.locator("#name").inputValue();
    const fillEvents = await page.evaluate("globalThis.__stagehandFillEvents");
    assert(inputValue === "stagehand value", `Stagehand fill returned ${JSON.stringify(inputValue)}`);
    assert(
      Array.isArray(fillEvents) && fillEvents.some((event) => event.value === "stagehand value"),
      `Stagehand fill did not dispatch an observable input event: ${JSON.stringify(fillEvents)}`,
    );
    await page.waitForSelector("#shadow-value", {
      state: "attached",
      timeout: 5000,
      pierceShadow: true,
    });
    const shadowText = await page.locator("#shadow-value").textContent();
    assert(shadowText === "shadow ready", `Stagehand shadow lookup returned ${JSON.stringify(shadowText)}`);
    record("stagehand_fill_wait_shadow_workflow");

    await page.evaluate(`localStorage.setItem("external-shared", "stagehand-local");
                         sessionStorage.setItem("external-private", "stagehand-first");
                         globalThis.__externalPageMarker = "first";`);
    const second = await stagehand.context.newPage();
    createdPages.push(second);
    await second.goto(`${fixture}/plain?client=stagehand-second`, {
      waitUntil: "load",
      timeoutMs: 10000,
    });
    const secondState = await second.evaluate(`({
      local: localStorage.getItem("external-shared"),
      session: sessionStorage.getItem("external-private"),
      marker: globalThis.__externalPageMarker || null,
    })`);
    assert(
      secondState.local === "stagehand-local" &&
        secondState.session === null &&
        secondState.marker === null,
      `Stagehand second-page state mismatch: ${JSON.stringify(secondState)}`,
    );
    await second.evaluate(`sessionStorage.setItem("external-private", "stagehand-second");
                           globalThis.__externalPageMarker = "second";`);
    const firstState = await page.evaluate(`({
      local: localStorage.getItem("external-shared"),
      session: sessionStorage.getItem("external-private"),
      marker: globalThis.__externalPageMarker,
    })`);
    assert(
      firstState.local === "stagehand-local" &&
        firstState.session === "stagehand-first" &&
        firstState.marker === "first",
      `Stagehand first-page state changed: ${JSON.stringify(firstState)}`,
    );
    record("stagehand_multi_page_storage_isolation");

    await page.goto(`${fixture}/history-a?client=stagehand`, {
      waitUntil: "load",
      timeoutMs: 10000,
    });
    await page.goto(`${fixture}/history-b?client=stagehand`, {
      waitUntil: "load",
      timeoutMs: 10000,
    });
    await page.goBack({waitUntil: "load", timeoutMs: 10000});
    const historyPath = await page.evaluate("location.pathname");
    assert(historyPath === "/history-a", `Stagehand goBack reached ${JSON.stringify(historyPath)}`);
    record("stagehand_navigation_history_workflow");

    await page.goto(`${fixture}/iframe?client=stagehand`, {
      waitUntil: "load",
      timeoutMs: 10000,
    });
    const frames = page.frames();
    assert(frames.length === 2, `Stagehand frame registry returned ${frames.length} frames`);
    const childFrame = frames.find((frame) => frame.frameId !== page.mainFrameId());
    assert(childFrame, "Stagehand frame registry has no child frame");
    const childText = await childFrame.evaluate("document.body.textContent.trim()");
    const childInput = await page.deepLocator("iframe >> input").inputValue();
    assert(String(childText).includes("child body text"), `Stagehand child frame text was ${JSON.stringify(childText)}`);
    assert(childInput === "inner", `Stagehand deep frame locator returned ${JSON.stringify(childInput)}`);
    record("stagehand_frame_registry_deep_locator", {frameCount: frames.length});

    await page.goto(`${fixture}/plain?client=stagehand-network`, {
      waitUntil: "load",
      timeoutMs: 10000,
    });
    await page.sendCDP("Network.enable");
    await page.setExtraHTTPHeaders({"X-Smoke-Post": "stagehand"});
    const echoed = await page.evaluate(`fetch(${JSON.stringify(`${fixture}/api-echo`)}, {
      method: 'POST',
      headers: {'Content-Type': 'text/plain'},
      body: 'stagehand-body',
    }).then(response => response.json())`);
    assert(
      echoed.method === "POST" &&
        echoed.body === "stagehand-body" &&
        echoed.customHeader === "stagehand",
      `Stagehand Network header/body mismatch: ${JSON.stringify(echoed)}`,
    );
    record("stagehand_network_headers_fetch_workflow");

    await page.evaluate(`(() => {
      document.body.innerHTML = '<button id="position">position</button>';
      globalThis.__externalPositionClicks = 0;
      document.querySelector('#position').addEventListener('click', () => __externalPositionClicks += 1);
      return 'ready';
    })()`);
    let positionError = null;
    try {
      await page.locator("#position").click();
    } catch (error) {
      positionError = String(error && error.message ? error.message : error);
    }
    const clickCount = await page.evaluate("globalThis.__externalPositionClicks");
    if (isMoli) {
      assert(
        positionError && /not supported|unsupported|layout hit testing/i.test(positionError),
        `Moli Stagehand click did not return an explicit capability error: ${positionError}`,
      );
      assert(clickCount === 0, `unsupported Stagehand click mutated the DOM: ${clickCount}`);
    } else {
      assert(!positionError, `Chromium Stagehand click failed: ${positionError}`);
      assert(clickCount === 1, `Chromium Stagehand click count was ${clickCount}`);
    }
    record("stagehand_position_click_capability_boundary", {
      supported: !isMoli,
      clickCount,
    });

    return {ok: true, results};
  } finally {
    for (const page of createdPages.reverse()) {
      try {
        await page.close();
      } catch {}
    }
    await stagehand.close().catch(() => {});
  }
}

main()
  .then((payload) => {
    realStdoutWrite(JSON.stringify(payload), () => process.exit(0));
  })
  .catch((error) => {
    process.stderr.write(`${error && error.stack ? error.stack : error}\n`);
    process.exit(1);
  });
