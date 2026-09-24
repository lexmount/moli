#!/usr/bin/env node
import fs from 'node:fs';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';

import {
  activateXPathElement,
  runPuppeteerDomInteractionSmoke,
} from './puppeteer_dom_interactions.mjs';

const require = createRequire(import.meta.url);

function loadPuppeteer() {
  const moduleName = process.env.PUPPETEER_CORE_MODULE || 'puppeteer-core';
  try {
    return require(moduleName);
  } catch (error) {
    throw new Error(
      `failed to load ${moduleName}. Install puppeteer-core or set PUPPETEER_CORE_MODULE/NODE_PATH. ${error.message}`,
    );
  }
}

function trace(message) {
  if (process.env.MOLI_SMOKE_TRACE === '1') {
    console.error(`[puppeteer smoke] ${message}`);
  }
}

async function withTimeout(label, promise, timeoutMs = 10000) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs}ms`)), timeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

async function discoverWebSocket(endpoint) {
  const response = await fetch(`${endpoint.replace(/\/$/, '')}/json/version`);
  if (!response.ok) {
    throw new Error(`CDP discovery failed with HTTP ${response.status}`);
  }
  const payload = await response.json();
  if (typeof payload.webSocketDebuggerUrl !== 'string' || !payload.webSocketDebuggerUrl) {
    throw new Error(`CDP discovery response did not include webSocketDebuggerUrl: ${JSON.stringify(payload)}`);
  }
  return payload.webSocketDebuggerUrl;
}

async function listPageTargets(endpoint) {
  const response = await fetch(`${endpoint.replace(/\/$/, '')}/json/list`);
  if (!response.ok) {
    throw new Error(`CDP target discovery failed with HTTP ${response.status}`);
  }
  const payload = await response.json();
  if (!Array.isArray(payload)) {
    throw new Error(`CDP target discovery returned an invalid payload: ${JSON.stringify(payload)}`);
  }
  return payload.filter(target => target?.type === 'page' && typeof target.id === 'string');
}

async function createPageTarget(endpoint) {
  const response = await fetch(
    `${endpoint.replace(/\/$/, '')}/json/new?${encodeURIComponent('about:blank')}`,
    { method: 'PUT' },
  );
  if (!response.ok) {
    throw new Error(`CDP target creation failed with HTTP ${response.status}`);
  }
  const target = await response.json();
  if (target?.type !== 'page' || typeof target.id !== 'string') {
    throw new Error(`CDP target creation returned an invalid payload: ${JSON.stringify(target)}`);
  }
  return target;
}

async function closePageTarget(endpoint, targetId) {
  const response = await fetch(
    `${endpoint.replace(/\/$/, '')}/json/close/${encodeURIComponent(targetId)}`,
  );
  if (!response.ok) {
    throw new Error(`CDP target ${targetId} cleanup failed with HTTP ${response.status}`);
  }
}

async function pageByTargetId(browser, targetId, label) {
  const target = await withTimeout(
    `${label} target`,
    browser.waitForTarget(candidate => {
      return candidate.type() === 'page' && candidate._targetId === targetId;
    }, { timeout: 10000 }),
  );
  const page = await withTimeout(`${label} page`, target.page());
  if (!page) {
    throw new Error(`${label} target ${targetId} did not expose a Page`);
  }
  return page;
}

async function runPuppeteerReconnectSmoke(puppeteer, endpoint, browserWSEndpoint) {
  let pages = await listPageTargets(endpoint);
  let originalTargetCreated = false;
  if (pages.length === 0) {
    pages = [await createPageTarget(endpoint)];
    originalTargetCreated = true;
  }
  const originalTargetId = pages[0].id;
  // Materializing a second Page parks the original Page in Moli. Accessing the
  // original through Puppeteer then promotes it again, which is the lifecycle
  // required to exercise parent-session detach cleanup on disconnect.
  const temporaryTarget = await createPageTarget(endpoint);
  if (temporaryTarget.id === originalTargetId) {
    throw new Error(`CDP target creation reused the existing target id ${originalTargetId}`);
  }

  let firstBrowser;
  let replacementBrowser;
  let completed = false;
  try {
    firstBrowser = await withTimeout(
      'first Puppeteer reconnect probe connect',
      puppeteer.connect({ browserWSEndpoint, protocolTimeout: 10000 }),
    );
    const originalPage = await pageByTargetId(
      firstBrowser,
      originalTargetId,
      'first Puppeteer reconnect probe',
    );
    const firstResult = await withTimeout(
      'first Puppeteer reconnect probe evaluate',
      originalPage.evaluate(() => {
        globalThis.__moliPuppeteerReconnectMarker = 29;
        return 6 * 7;
      }),
    );
    if (firstResult !== 42) {
      throw new Error(`unexpected first Puppeteer reconnect probe result: ${firstResult}`);
    }
    await firstBrowser.disconnect();
    firstBrowser = undefined;

    replacementBrowser = await withTimeout(
      'replacement Puppeteer reconnect probe connect',
      puppeteer.connect({ browserWSEndpoint, protocolTimeout: 10000 }),
    );
    const replacementPage = await pageByTargetId(
      replacementBrowser,
      originalTargetId,
      'replacement Puppeteer reconnect probe',
    );
    const replacementResult = await withTimeout(
      'replacement Puppeteer reconnect probe evaluate',
      replacementPage.evaluate(() => ({
        answer: 6 * 7,
        marker: globalThis.__moliPuppeteerReconnectMarker,
      })),
    );
    if (replacementResult?.answer !== 42 || replacementResult?.marker !== 29) {
      throw new Error(
        `unexpected replacement Puppeteer reconnect probe result: ${JSON.stringify(replacementResult)}`,
      );
    }
    completed = true;
    return {
      existingTargetReused: true,
      firstEvaluation: firstResult,
      replacementEvaluation: replacementResult.answer,
    };
  } finally {
    if (replacementBrowser) {
      await replacementBrowser.disconnect().catch(() => {});
    }
    if (firstBrowser) {
      await firstBrowser.disconnect().catch(() => {});
    }
    await closePageTarget(endpoint, temporaryTarget.id).catch(error => {
      if (completed) {
        throw error;
      }
    });
    if (originalTargetCreated) {
      await closePageTarget(endpoint, originalTargetId).catch(error => {
        if (completed) {
          throw error;
        }
      });
    }
  }
}

async function createBrowserContext(browser) {
  if (typeof browser.createBrowserContext === 'function') {
    return await browser.createBrowserContext();
  }
  if (typeof browser.createIncognitoBrowserContext === 'function') {
    return await browser.createIncognitoBrowserContext();
  }
  return browser.defaultBrowserContext();
}

async function closeBrowserContext(context) {
  if (typeof context.close === 'function') {
    await context.close();
  }
}

async function main() {
  const [endpoint, fixture] = process.argv.slice(2);
  if (!endpoint || !fixture) {
    throw new Error('usage: node puppeteer_smoke.mjs <endpoint> <fixture>');
  }
  const puppeteer = loadPuppeteer();
  trace('loaded puppeteer-core');
  const browserWSEndpoint = await discoverWebSocket(endpoint);
  const isMoliEndpoint = browserWSEndpoint.endsWith('/devtools/browser/moli-browser');
  trace(`discovered ${browserWSEndpoint}`);
  const results = [];
  const record = (name, data = {}) => results.push({ name, ok: true, ...data });
  const reconnectResult = await runPuppeteerReconnectSmoke(
    puppeteer,
    endpoint,
    browserWSEndpoint,
  );
  record('puppeteer_existing_page_reconnect_runtime_context', reconnectResult);
  trace('reconnected to existing Page with a fresh Puppeteer session');
  const browser = await withTimeout(
    'puppeteer.connect',
    puppeteer.connect({ browserWSEndpoint, protocolTimeout: 10000 }),
  );
  trace('connected');

  let context;
  let browserCdp;
  let peerBrowserCdp;
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'moli-puppeteer-smoke-'));
  try {
    const browserTarget = browser.target();
    if (!browserTarget || typeof browserTarget.createCDPSession !== 'function') {
      throw new Error('Puppeteer browser target does not expose createCDPSession()');
    }
    browserCdp = await withTimeout(
      'browser.target().createCDPSession',
      browserTarget.createCDPSession(),
    );
    peerBrowserCdp = await withTimeout(
      'browser.target().createCDPSession peer',
      browserTarget.createCDPSession(),
    );
    const [browserVersion, initialTargets] = await withTimeout(
      'browser-target CDPSession commands',
      Promise.all([
        browserCdp.send('Browser.getVersion'),
        browserCdp.send('Target.getTargets'),
      ]),
    );
    if (typeof browserVersion.product !== 'string' || !browserVersion.product) {
      throw new Error(`unexpected Browser.getVersion response: ${JSON.stringify(browserVersion)}`);
    }
    if (!Array.isArray(initialTargets.targetInfos)) {
      throw new Error(`unexpected Target.getTargets response: ${JSON.stringify(initialTargets)}`);
    }
    record('puppeteer_browser_target_cdp_session_workflow', {
      product: browserVersion.product,
      initialTargetCount: initialTargets.targetInfos.length,
    });

    context = await withTimeout('createBrowserContext', createBrowserContext(browser));
    trace('created browser context');
    const page = await withTimeout('newPage', context.newPage());
    trace('created page');
    const cdp = await withTimeout('createCDPSession', page.target().createCDPSession());
    const networkEvents = [];
    for (const method of [
      'Network.requestWillBeSent',
      'Network.responseReceived',
      'Network.loadingFinished',
      'Network.loadingFailed',
    ]) {
      cdp.on(method, params => networkEvents.push({ method, params }));
    }
    await withTimeout('Network.enable', cdp.send('Network.enable'));
    trace('enabled Network');

    const response = await withTimeout(
      'page.goto(/plain)',
      page.goto(`${fixture}/plain`, {
        waitUntil: 'load',
        timeout: 10000,
      }),
    );
    trace('navigated to plain page');
    if (!response || response.status() !== 200) {
      throw new Error(`unexpected Puppeteer navigation response: ${response && response.status()}`);
    }
    const mainText = await withTimeout('page.$eval(main)', page.$eval('main', node => node.textContent));
    if (mainText !== 'plain ok') {
      throw new Error(`unexpected Puppeteer page text: ${mainText}`);
    }
    record('puppeteer_goto_plain');

    await withTimeout('page Runtime.enable', cdp.send('Runtime.enable'));
    const pageSessionWorkerConsole = [];
    cdp.on('Runtime.consoleAPICalled', event => {
      const value = event.args?.[0]?.value;
      if (typeof value === 'string' && value.startsWith('__puppeteer_worker_console_')) {
        pageSessionWorkerConsole.push(value);
      }
    });

    const workerCreated = withTimeout(
      'Puppeteer workercreated',
      new Promise(resolve => page.once('workercreated', resolve)),
    );
    await withTimeout('create DedicatedWorker', page.evaluate(() => {
      globalThis.__puppeteerDedicatedWorker = new Worker('/worker.js', {
        name: 'puppeteer-dedicated-worker',
      });
    }));
    const dedicatedWorker = await workerCreated;
    const dedicatedWorkerProbe = await withTimeout(
      'WebWorker.evaluate',
      dedicatedWorker.evaluate(() => ({
        name,
        pathname: self.location.pathname,
        selfEqualsGlobal: self === globalThis,
        hasDocument: typeof document !== 'undefined',
      })),
    );
    if (
      dedicatedWorkerProbe.name !== 'puppeteer-dedicated-worker'
      || dedicatedWorkerProbe.pathname !== '/worker.js'
      || dedicatedWorkerProbe.selfEqualsGlobal !== true
      || dedicatedWorkerProbe.hasDocument !== false
    ) {
      throw new Error(`unexpected Puppeteer DedicatedWorker probe: ${JSON.stringify(dedicatedWorkerProbe)}`);
    }
    const workerConsoleMessages = [];
    const workerConsoleReady = withTimeout(
      'DedicatedWorker Runtime.consoleAPICalled routing',
      new Promise(resolve => {
        dedicatedWorker.client.on('Runtime.consoleAPICalled', event => {
          const value = event.args?.[0]?.value;
          if (typeof value !== 'string' || !value.startsWith('__puppeteer_worker_console_')) {
            return;
          }
          workerConsoleMessages.push(value);
          if (workerConsoleMessages.length === 2) {
            resolve();
          }
        });
      }),
    );
    await withTimeout(
      'DedicatedWorker console evaluation',
      dedicatedWorker.evaluate(() => {
        console.log('__puppeteer_worker_console_first');
        console.log('__puppeteer_worker_console_second');
      }),
    );
    await workerConsoleReady;
    // A second command is an ordered barrier on the same worker session. It
    // also exposes accidental cursor replay after the first evaluation.
    await withTimeout('DedicatedWorker console delivery barrier', dedicatedWorker.evaluate(() => 1));
    if (
      JSON.stringify(workerConsoleMessages)
        !== JSON.stringify([
          '__puppeteer_worker_console_first',
          '__puppeteer_worker_console_second',
        ])
      || pageSessionWorkerConsole.length !== 0
    ) {
      throw new Error(
        `DedicatedWorker console must stay exact-once on its worker session: ${JSON.stringify({
          workerConsoleMessages,
          pageSessionWorkerConsole,
        })}`,
      );
    }
    const workerDestroyed = withTimeout(
      'Puppeteer workerdestroyed',
      new Promise(resolve => page.once('workerdestroyed', resolve)),
    );
    await withTimeout('terminate DedicatedWorker', page.evaluate(() => {
      globalThis.__puppeteerDedicatedWorker.terminate();
    }));
    const destroyedWorker = await workerDestroyed;
    if (destroyedWorker.url() !== dedicatedWorker.url()) {
      throw new Error(
        `Puppeteer workerdestroyed target mismatch: ${destroyedWorker.url()} != ${dedicatedWorker.url()}`,
      );
    }
    const replacementWorkerCreated = withTimeout(
      'Puppeteer replacement workercreated',
      new Promise(resolve => page.once('workercreated', resolve)),
    );
    await withTimeout('create replacement DedicatedWorker', page.evaluate(() => {
      globalThis.__puppeteerReplacementWorker = new Worker('/worker.js', {
        name: 'puppeteer-replacement-worker',
      });
    }));
    const replacementWorker = await replacementWorkerCreated;
    const replacementWorkerDestroyed = withTimeout(
      'Puppeteer navigation workerdestroyed',
      new Promise(resolve => page.once('workerdestroyed', resolve)),
    );
    await withTimeout(
      'navigate with live DedicatedWorker',
      page.goto(`${fixture}/plain?worker-replacement=1`, {
        waitUntil: 'load',
        timeout: 10000,
      }),
    );
    const navigationDestroyedWorker = await replacementWorkerDestroyed;
    if (navigationDestroyedWorker.url() !== replacementWorker.url()) {
      throw new Error(
        `navigation workerdestroyed target mismatch: ${navigationDestroyedWorker.url()} != ${replacementWorker.url()}`,
      );
    }
    record('puppeteer_dedicated_worker_target_runtime_lifecycle', {
      navigationDestroyed: true,
      workerConsoleMessages,
    });

    const fetchBody = await withTimeout('page.evaluate(fetch /api)', page.evaluate(async () => {
      return await fetch('/api').then(response => response.text());
    }));
    trace('evaluated fetch');
    if (fetchBody !== 'fixture api body') {
      throw new Error(`unexpected Puppeteer fetch body: ${fetchBody}`);
    }
    record('puppeteer_evaluate_fetch');

    const domInteractionResult = await withTimeout(
      'Puppeteer DOM interaction matrix',
      runPuppeteerDomInteractionSmoke(page),
      15000,
    );
    record('puppeteer_dom_selector_activation_workflow', domInteractionResult);

    const layoutClient = await page.createCDPSession();
    await layoutClient.send('Page.captureScreenshot');
    await layoutClient.detach();
    const nameHandle = await withTimeout('page.$(#name)', page.$('#name'));
    if (!nameHandle) {
      throw new Error('Puppeteer ElementHandle lookup for #name returned null');
    }
    try {
      const box = await withTimeout('elementHandle.boundingBox', nameHandle.boundingBox());
      if (!box || box.width <= 0 || box.height <= 0) {
        throw new Error(`unexpected Puppeteer ElementHandle bounding box: ${JSON.stringify(box)}`);
      }
      const handleValue = await withTimeout(
        'elementHandle.evaluate',
        nameHandle.evaluate(node => ({ tag: node.tagName, value: node.value })),
      );
      if (handleValue.tag !== 'INPUT' || handleValue.value !== 'puppeteer user') {
        throw new Error(`unexpected Puppeteer ElementHandle evaluation: ${JSON.stringify(handleValue)}`);
      }
      record('puppeteer_element_handle_workflow');
    } finally {
      await nameHandle.dispose();
    }

    let screenshotData = null;
    let screenshotError = null;
    try {
      screenshotData = await withTimeout(
        'page.screenshot',
        page.screenshot({ encoding: 'base64', fullPage: false, captureBeyondViewport: false }),
      );
    } catch (error) {
      screenshotError = error && error.message ? error.message : String(error);
    }
    if (
      screenshotError
      || typeof screenshotData !== 'string'
      || !Buffer.from(screenshotData, 'base64').subarray(0, 8).equals(
        Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
      )
    ) {
      throw new Error(`viewport screenshot must return a PNG: ${screenshotError}`);
    }
    record('puppeteer_screenshot_boundary_workflow', {
      boundary: 'viewport-paint-supported',
      captureBeyondViewport: false,
    });

    const dialogPromise = withTimeout('page dialog event', new Promise((resolve, reject) => {
      page.once('dialog', async dialog => {
        try {
          if (dialog.type() !== 'alert' || dialog.message() !== 'puppeteer alert') {
            throw new Error(`unexpected Puppeteer dialog: ${dialog.type()} ${dialog.message()}`);
          }
          await dialog.accept();
          resolve({ type: dialog.type(), message: dialog.message() });
        } catch (error) {
          reject(error);
        }
      });
    }));
    await withTimeout('page.evaluate(alert)', page.evaluate(() => alert('puppeteer alert')));
    await dialogPromise;
    record('puppeteer_dialog_alert');

    const consolePromise = withTimeout('page console event', new Promise((resolve, reject) => {
      page.once('console', async message => {
        try {
          const values = await Promise.all(message.args().map(argument => argument.jsonValue()));
          resolve({ type: message.type(), text: message.text(), values });
        } catch (error) {
          reject(error);
        }
      });
    }));
    await withTimeout(
      'page.evaluate(console.log)',
      page.evaluate(() => console.log('puppeteer console', 17, { source: 'smoke' })),
    );
    const consoleMessage = await consolePromise;
    if (
      consoleMessage.type !== 'log'
      || !consoleMessage.text.startsWith('puppeteer console 17')
      || JSON.stringify(consoleMessage.values) !== JSON.stringify(['puppeteer console', 17, { source: 'smoke' }])
    ) {
      throw new Error(`unexpected Puppeteer console event: ${JSON.stringify(consoleMessage)}`);
    }
    record('puppeteer_console_event_workflow');

    const uploadPath = path.join(tempDir, 'puppeteer upload.txt');
    fs.writeFileSync(uploadPath, 'puppeteer upload contents', 'utf8');
    await withTimeout('page.evaluate(upload fixture)', page.evaluate(() => {
      document.body.innerHTML = '<input id="upload" type="file">';
      globalThis.__puppeteerUploadEvents = [];
      const input = document.querySelector('#upload');
      input.addEventListener('input', () => __puppeteerUploadEvents.push('input'));
      input.addEventListener('change', () => __puppeteerUploadEvents.push('change'));
    }));
    const uploadHandle = await withTimeout('page.$(#upload)', page.$('#upload'));
    if (!uploadHandle) {
      throw new Error('Puppeteer upload input lookup returned null');
    }
    try {
      await withTimeout('ElementHandle.uploadFile', uploadHandle.uploadFile(uploadPath));
    } finally {
      await uploadHandle.dispose();
    }
    const uploadState = await withTimeout('read uploaded file', page.$eval('#upload', async input => {
      const file = input.files?.[0];
      if (!file) {
        return null;
      }
      const text = await new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(reader.result);
        reader.onerror = () => reject(reader.error || new Error('FileReader failed'));
        reader.readAsText(file);
      });
      return {
        name: file.name,
        size: file.size,
        text,
        events: globalThis.__puppeteerUploadEvents.slice(),
      };
    }));
    if (
      !uploadState
      || uploadState.name !== 'puppeteer upload.txt'
      || uploadState.text !== 'puppeteer upload contents'
      || JSON.stringify(uploadState.events) !== JSON.stringify(['input', 'change'])
    ) {
      throw new Error(`unexpected Puppeteer upload state: ${JSON.stringify(uploadState)}`);
    }
    record('puppeteer_upload_file_workflow', { size: uploadState.size });

    const downloadDir = path.join(tempDir, 'downloads');
    fs.mkdirSync(downloadDir);
    const peerDownloadEvents = [];
    const onPeerDownloadWillBegin = params => {
      peerDownloadEvents.push({ method: 'Browser.downloadWillBegin', params });
    };
    const onPeerDownloadProgress = params => {
      peerDownloadEvents.push({ method: 'Browser.downloadProgress', params });
    };
    peerBrowserCdp.on('Browser.downloadWillBegin', onPeerDownloadWillBegin);
    peerBrowserCdp.on('Browser.downloadProgress', onPeerDownloadProgress);
    const downloadBehavior = {
      behavior: 'allowAndName',
      downloadPath: downloadDir,
      eventsEnabled: true,
    };
    if (context.id) {
      downloadBehavior.browserContextId = context.id;
    }
    await withTimeout(
      'Browser.setDownloadBehavior',
      browserCdp.send('Browser.setDownloadBehavior', downloadBehavior),
    );
    await withTimeout(
      'page.goto(/download-page)',
      page.goto(`${fixture}/download-page`, { waitUntil: 'load', timeout: 10000 }),
    );
    const downloadEvents = [];
    let removeDownloadListeners;
    const downloadCompleted = new Promise(resolve => {
      const onWillBegin = params => {
        downloadEvents.push({ method: 'Browser.downloadWillBegin', params });
      };
      const onProgress = params => {
        downloadEvents.push({ method: 'Browser.downloadProgress', params });
        if (params.state === 'completed') {
          resolve(params);
        }
      };
      browserCdp.on('Browser.downloadWillBegin', onWillBegin);
      browserCdp.on('Browser.downloadProgress', onProgress);
      removeDownloadListeners = () => {
        browserCdp.off('Browser.downloadWillBegin', onWillBegin);
        browserCdp.off('Browser.downloadProgress', onProgress);
      };
    });
    let completedDownload;
    try {
      await withTimeout(
        'DOM activation download',
        page.$eval('#download', element => element.click()),
      );
      completedDownload = await withTimeout('Browser.downloadProgress completed', downloadCompleted);
    } finally {
      removeDownloadListeners?.();
    }
    await withTimeout(
      'peer browser session download event fence',
      peerBrowserCdp.send('Browser.getVersion'),
    );
    peerBrowserCdp.off('Browser.downloadWillBegin', onPeerDownloadWillBegin);
    peerBrowserCdp.off('Browser.downloadProgress', onPeerDownloadProgress);
    if (peerDownloadEvents.length !== 0) {
      throw new Error(
        `Puppeteer browser download events leaked to a peer session: ${JSON.stringify(peerDownloadEvents)}`,
      );
    }
    const willBeginIndex = downloadEvents.findIndex(event => event.method === 'Browser.downloadWillBegin');
    const completedIndex = downloadEvents.findIndex(event => {
      return event.method === 'Browser.downloadProgress' && event.params.state === 'completed';
    });
    if (willBeginIndex < 0 || completedIndex <= willBeginIndex) {
      throw new Error(`unexpected Puppeteer browser download event order: ${JSON.stringify(downloadEvents)}`);
    }
    const downloadPath = completedDownload.filePath || path.join(downloadDir, completedDownload.guid);
    const downloadBody = fs.readFileSync(downloadPath, 'utf8');
    if (downloadBody !== 'download contents') {
      throw new Error(`unexpected Puppeteer download body: ${JSON.stringify(downloadBody)}`);
    }
    record('puppeteer_browser_session_download_workflow', {
      guid: completedDownload.guid,
      eventCount: downloadEvents.length,
      peerEventCount: peerDownloadEvents.length,
    });

    await withTimeout('page.goto(/plain?reload)', page.goto(`${fixture}/plain?reload`, {
      waitUntil: 'domcontentloaded',
      timeout: 10000,
    }));
    const reloadResponse = await withTimeout('page.reload', page.reload({
      waitUntil: 'load',
      timeout: 10000,
    }));
    if (!reloadResponse || reloadResponse.status() !== 200) {
      throw new Error(`unexpected Puppeteer reload response: ${reloadResponse && reloadResponse.status()}`);
    }
    if (!page.url().includes('/plain?reload')) {
      throw new Error(`unexpected Puppeteer reload URL: ${page.url()}`);
    }
    record('puppeteer_reload_workflow');

    await withTimeout('page.evaluate(DOM activation navigation setup)', page.evaluate(() => {
      document.body.innerHTML = '<a id="nav" data-smoke="navigate" href="/plain?from=puppeteer-dom-activation">navigate</a>';
    }));
    const [domActivationNavigationResponse, xpathActivation] = await withTimeout(
      'XPath DOM activation navigation',
      Promise.all([
        page.waitForNavigation({ waitUntil: 'load', timeout: 10000 }),
        activateXPathElement(page, '//*[@id="nav" and @data-smoke="navigate"]'),
      ]),
    );
    if (xpathActivation.id !== 'nav' || xpathActivation.tag !== 'A') {
      throw new Error(`unexpected XPath activation: ${JSON.stringify(xpathActivation)}`);
    }
    if (!page.url().endsWith('/plain?from=puppeteer-dom-activation')) {
      throw new Error(`unexpected Puppeteer DOM activation navigation URL: ${page.url()}`);
    }
    record('puppeteer_dom_activation_navigation_workflow', {
      responseStatus: domActivationNavigationResponse && domActivationNavigationResponse.status(),
    });

    const [sameDocumentResponse] = await withTimeout(
      'page.waitForNavigation same-document hash',
      Promise.all([
        page.waitForNavigation({ waitUntil: 'load', timeout: 10000 }),
        page.evaluate(() => {
          location.hash = 'puppeteer-hash';
        }),
      ]),
    );
    if (sameDocumentResponse !== null) {
      throw new Error('Puppeteer same-document navigation should resolve with a null response');
    }
    if (!page.url().endsWith('/plain?from=puppeteer-dom-activation#puppeteer-hash')) {
      throw new Error(`unexpected Puppeteer same-document URL: ${page.url()}`);
    }
    record('puppeteer_same_document_navigation_workflow');

    const historyApiUrl = new URL(page.url());
    historyApiUrl.searchParams.set('history', 'puppeteer');
    historyApiUrl.hash = '';
    const [historyApiResponse] = await withTimeout(
      'page.waitForNavigation History API',
      Promise.all([
        page.waitForNavigation({ waitUntil: 'load', timeout: 10000 }),
        page.evaluate(url => {
          history.pushState({ smoke: true }, '', url);
        }, historyApiUrl.toString()),
      ]),
    );
    if (historyApiResponse !== null) {
      throw new Error('Puppeteer History API navigation should resolve with a null response');
    }
    if (page.url() !== historyApiUrl.toString()) {
      throw new Error(`unexpected Puppeteer History API URL: ${page.url()}`);
    }
    record('puppeteer_history_api_navigation_workflow');

    if (typeof browser.waitForTarget !== 'function') {
      throw new Error('Puppeteer Browser.waitForTarget is unavailable');
    }
    const popupUrl = `${fixture}/plain?popup=puppeteer`;
    const popupTargetPromise = withTimeout(
      'browser.waitForTarget(window.open popup)',
      browser.waitForTarget(target => target.type() === 'page' && target.url() === popupUrl, {
        timeout: 10000,
      }),
    );
    const popupReturnIsNull = await withTimeout(
      'page.evaluate(window.open)',
      page.evaluate(url => window.open(url, '_blank') === null, popupUrl),
    );
    if (popupReturnIsNull !== false) {
      throw new Error(`unexpected Puppeteer window.open return value: ${popupReturnIsNull}`);
    }
    const popupTarget = await popupTargetPromise;
    if (!popupTarget || popupTarget.url() !== popupUrl) {
      throw new Error(`unexpected Puppeteer popup target: ${popupTarget && popupTarget.url()}`);
    }
    record('puppeteer_popup_target_workflow', { returnedWindowProxy: true });

    await withTimeout('setRequestInterception', page.setRequestInterception(true));
    trace('enabled request interception');
    page.on('request', request => {
      if (request.url().endsWith('/api-puppeteer-fulfill')) {
        request.respond({
          status: 200,
          contentType: 'text/plain; charset=utf-8',
          body: 'puppeteer fulfilled body',
        });
      } else if (request.url().endsWith('/api-continue')) {
        request.continue({
          headers: {
            ...request.headers(),
            'x-smoke-route': 'puppeteer-continued',
          },
        });
      } else {
        request.continue();
      }
    });

    const fulfilled = await withTimeout('page.evaluate(fetch /api-puppeteer-fulfill)', page.evaluate(async () => {
      return await fetch('/api-puppeteer-fulfill').then(response => response.text());
    }));
    trace('evaluated intercepted respond fetch');
    if (fulfilled !== 'puppeteer fulfilled body') {
      throw new Error(`unexpected Puppeteer fulfilled body: ${fulfilled}`);
    }
    record('puppeteer_request_respond_fetch');

    const continued = await withTimeout('page.evaluate(fetch /api-continue)', page.evaluate(async () => {
      return await fetch('/api-continue').then(response => response.json());
    }));
    trace('evaluated intercepted continue fetch');
    if (continued.routeHeader !== 'puppeteer-continued') {
      throw new Error(`unexpected Puppeteer continue payload: ${JSON.stringify(continued)}`);
    }
    record('puppeteer_request_continue_fetch');

    const sawFetchNetwork = networkEvents.some(event => {
      return event.method === 'Network.requestWillBeSent'
        && event.params?.type === 'Fetch'
        && String(event.params?.request?.url || '').endsWith('/api-continue');
    });
    if (!sawFetchNetwork) {
      throw new Error(`missing Puppeteer CDPSession Fetch Network event: ${JSON.stringify(networkEvents.slice(-20))}`);
    }
    record('puppeteer_cdp_session_network_events', { networkEventCount: networkEvents.length });

    await withTimeout('page.bringToFront(position boundary)', page.bringToFront());
    await withTimeout('page.evaluate(position click fixture)', page.evaluate(() => {
      document.body.innerHTML = '<button id="position-click">position click</button>';
      globalThis.__puppeteerPositionClickCount = 0;
      document.querySelector('#position-click').addEventListener('click', () => {
        globalThis.__puppeteerPositionClickCount += 1;
      });
    }));
    const positionLayout = await page.createCDPSession();
    await positionLayout.send('Page.captureScreenshot');
    await positionLayout.detach();
    let positionClickError = null;
    try {
      await withTimeout('page.click(position boundary)', page.click('#position-click'));
    } catch (error) {
      positionClickError = error && error.message ? error.message : String(error);
    }
    const positionClickCount = await withTimeout(
      'read position click count',
      page.evaluate(() => globalThis.__puppeteerPositionClickCount),
    );
    let positionClickBoundary;
    if (positionClickError === null) {
      if (positionClickCount !== 1) {
        throw new Error(`position click must dispatch exactly once: count=${positionClickCount}`);
      }
      positionClickBoundary = 'layout-supported';
    } else if (isMoliEndpoint && positionClickError.includes('Input.dispatchMouseEvent is not supported')) {
      if (positionClickCount !== 0) {
        throw new Error(`Moli position click failed after mutating the page: ${positionClickCount}`);
      }
      positionClickBoundary = 'explicit-client-failure';
    } else {
      throw new Error(
        `position click failed unexpectedly: error=${positionClickError}; count=${positionClickCount}`,
      );
    }
    record('puppeteer_position_click_boundary', {
      boundary: positionClickBoundary,
      error: positionClickError,
    });

    console.log(JSON.stringify({ ok: true, results }));
  } finally {
    if (peerBrowserCdp) {
      await peerBrowserCdp.detach().catch(() => {});
    }
    if (browserCdp) {
      await browserCdp.detach().catch(() => {});
    }
    if (context) {
      await closeBrowserContext(context);
    }
    await browser.disconnect();
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

main().catch(error => {
  console.error(error && error.stack ? error.stack : String(error));
  process.exit(1);
});
