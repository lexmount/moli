#!/usr/bin/env node
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(import.meta.url);

const PROXY_ENV_KEYS = [
  'HTTP_PROXY',
  'HTTPS_PROXY',
  'ALL_PROXY',
  'http_proxy',
  'https_proxy',
  'all_proxy',
];

function usage() {
  return [
    'Usage: node scripts/playwright-cdp-smoke.mjs',
    '',
    'Environment:',
    '  MOLI_BIN=/path/to/moli  Override the binary under test.',
    '  PLAYWRIGHT_CORE_MODULE=playwright-core  Override the Playwright module name/path.',
    '  MOLI_CDP_PORT=9222  Override the CDP port; defaults to a free local port.',
  ].join('\n');
}

function clearProxyEnv(env) {
  const next = { ...env };
  for (const key of PROXY_ENV_KEYS) {
    delete next[key];
  }
  next.NO_PROXY = '*';
  next.no_proxy = '*';
  return next;
}

for (const key of PROXY_ENV_KEYS) {
  delete process.env[key];
}
process.env.NO_PROXY = '*';
process.env.no_proxy = '*';

function loadPlaywright() {
  const moduleName = process.env.PLAYWRIGHT_CORE_MODULE || 'playwright-core';
  try {
    return require(moduleName);
  } catch (error) {
    throw new Error(
      `failed to load ${moduleName}. Install playwright-core in this checkout or set NODE_PATH/PLAYWRIGHT_CORE_MODULE.\n${usage()}\n${error.message}`,
    );
  }
}

function moliBinary() {
  if (process.env.MOLI_BIN) {
    return resolve(process.env.MOLI_BIN);
  }
  const candidates = [
    join(repoRoot, 'target', 'debug', 'moli'),
    join(repoRoot, 'target', 'release', 'moli'),
  ];
  for (const candidate of candidates) {
    if (existsSync(candidate)) {
      return candidate;
    }
  }
  throw new Error(
    'missing moli binary. Build one first with `cargo build -p moli`, or set MOLI_BIN.',
  );
}

async function reservePort() {
  const server = createServer();
  await new Promise((resolveListen, rejectListen) => {
    server.once('error', rejectListen);
    server.listen(0, '127.0.0.1', resolveListen);
  });
  const { port } = server.address();
  await new Promise((resolveClose, rejectClose) => {
    server.close(error => (error ? rejectClose(error) : resolveClose()));
  });
  return port;
}

async function listen(server) {
  await new Promise((resolveListen, rejectListen) => {
    server.once('error', rejectListen);
    server.listen(0, '127.0.0.1', resolveListen);
  });
  const { port } = server.address();
  return `http://127.0.0.1:${port}`;
}

function websocketAcceptKey(key) {
  return createHash('sha1')
    .update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
    .digest('base64');
}

function websocketFrame(opcode, payload) {
  const body = Buffer.from(payload);
  if (body.length < 126) {
    return Buffer.concat([Buffer.from([0x80 | opcode, body.length]), body]);
  }
  if (body.length <= 0xffff) {
    const header = Buffer.alloc(4);
    header[0] = 0x80 | opcode;
    header[1] = 126;
    header.writeUInt16BE(body.length, 2);
    return Buffer.concat([header, body]);
  }
  throw new Error(`fixture websocket frame too large: ${body.length}`);
}

function websocketTextFrame(text) {
  return websocketFrame(0x1, Buffer.from(text, 'utf8'));
}

function websocketCloseFrame(code = 1000, reason = '') {
  const reasonBytes = Buffer.from(reason, 'utf8');
  const payload = Buffer.alloc(2 + reasonBytes.length);
  payload.writeUInt16BE(code, 0);
  reasonBytes.copy(payload, 2);
  return websocketFrame(0x8, payload);
}

function drainWebSocketFrames(buffer, onFrame) {
  let offset = 0;
  while (buffer.length - offset >= 2) {
    const first = buffer[offset];
    const second = buffer[offset + 1];
    const opcode = first & 0x0f;
    const masked = (second & 0x80) !== 0;
    let payloadLength = second & 0x7f;
    let headerLength = 2;

    if (payloadLength === 126) {
      if (buffer.length - offset < 4) {
        break;
      }
      payloadLength = buffer.readUInt16BE(offset + 2);
      headerLength = 4;
    } else if (payloadLength === 127) {
      throw new Error('fixture websocket does not support 64-bit frames');
    }

    const maskLength = masked ? 4 : 0;
    const frameLength = headerLength + maskLength + payloadLength;
    if (buffer.length - offset < frameLength) {
      break;
    }

    const maskOffset = offset + headerLength;
    const payloadOffset = maskOffset + maskLength;
    const payload = Buffer.from(buffer.subarray(payloadOffset, payloadOffset + payloadLength));
    if (masked) {
      const mask = buffer.subarray(maskOffset, maskOffset + 4);
      for (let index = 0; index < payload.length; index += 1) {
        payload[index] ^= mask[index % 4];
      }
    }

    onFrame(opcode, payload);
    offset += frameLength;
  }

  return buffer.subarray(offset);
}

function startFixtureServer() {
  const profileRequests = new Map();
  const server = createServer((req, res) => {
    const url = new URL(req.url || '/', 'http://127.0.0.1');
    const send = (status, contentType, body) => {
      res.writeHead(status, {
        'content-type': contentType,
        'cache-control': 'no-store',
      });
      res.end(body);
    };

    if (url.pathname === '/favicon.ico') {
      res.writeHead(204);
      res.end();
      return;
    }

    if (url.pathname === '/plain') {
      send(200, 'text/html; charset=utf-8', '<!doctype html><main>plain ok</main>');
      return;
    }

    if (url.pathname === '/iframe') {
      send(
        200,
        'text/html; charset=utf-8',
        '<!doctype html><main>parent</main><iframe src="/child"></iframe>',
      );
      return;
    }

    if (url.pathname === '/child') {
      send(200, 'text/html; charset=utf-8', '<!doctype html><body>child body text</body>');
      return;
    }

    if (url.pathname === '/wait-for-function') {
      send(
        200,
        'text/html; charset=utf-8',
        '<!doctype html><body><script>setTimeout(() => { globalThis.__ready = true; }, 50);</script></body>',
      );
      return;
    }

    if (url.pathname === '/set-cookie') {
      res.writeHead(200, {
        'content-type': 'text/html; charset=utf-8',
        'cache-control': 'no-store',
        'set-cookie': 'serverCookie=server; Path=/; SameSite=Lax',
      });
      res.end('<!doctype html><main>cookie set</main>');
      return;
    }

    if (url.pathname === '/echo-cookie') {
      send(200, 'text/plain; charset=utf-8', req.headers.cookie || '');
      return;
    }

    if (url.pathname === '/profile-headers') {
      const token = url.searchParams.get('token') || '';
      profileRequests.set(token, {
        userAgent: req.headers['user-agent'] || null,
        acceptLanguage: req.headers['accept-language'] || null,
        extraHeader: req.headers['x-moli-profile-smoke'] || null,
        referer: req.headers.referer || null,
      });
      send(200, 'text/html; charset=utf-8', '<!doctype html><main>profile headers captured</main>');
      return;
    }

    if (url.pathname === '/profile-result') {
      const token = url.searchParams.get('token') || '';
      send(
        200,
        'application/json; charset=utf-8',
        JSON.stringify(profileRequests.get(token) || null),
      );
      return;
    }

    if (url.pathname === '/redirect-start') {
      res.writeHead(302, {
        location: '/redirect-final',
        'cache-control': 'no-store',
        'set-cookie': 'redirectCookie=redirect; Path=/; SameSite=Lax',
      });
      res.end();
      return;
    }

    if (url.pathname === '/redirect-final') {
      send(200, 'text/html; charset=utf-8', '<!doctype html><main>redirect final</main>');
      return;
    }

    if (url.pathname === '/history-a') {
      send(200, 'text/html; charset=utf-8', '<!doctype html><main>history a</main>');
      return;
    }

    if (url.pathname === '/history-b') {
      send(200, 'text/html; charset=utf-8', '<!doctype html><main>history b</main>');
      return;
    }

    if (url.pathname === '/document-continue') {
      send(
        200,
        'text/html; charset=utf-8',
        `<!doctype html><main>${req.headers['x-smoke-nav-route'] || 'missing-nav-route-header'}</main>`,
      );
      return;
    }

    if (url.pathname === '/api') {
      send(500, 'text/plain; charset=utf-8', 'route did not intercept /api');
      return;
    }

    if (url.pathname === '/api-continue') {
      send(
        200,
        'application/json; charset=utf-8',
        JSON.stringify({
          method: req.method,
          routeHeader: req.headers['x-smoke-route'] || null,
        }),
      );
      return;
    }

    if (url.pathname === '/api-abort') {
      send(200, 'text/plain; charset=utf-8', 'route abort did not intercept /api-abort');
      return;
    }

    if (url.pathname === '/parser-script-page') {
      send(
        200,
        'text/html; charset=utf-8',
        '<!doctype html><body><script src="/parser-script.js"></script><main>parser script page</main></body>',
      );
      return;
    }

    if (url.pathname === '/parser-script.js') {
      send(
        200,
        'text/javascript; charset=utf-8',
        'globalThis.__smokeParserScriptValue = "parser script loaded";',
      );
      return;
    }

    if (url.pathname === '/worker.js') {
      send(
        200,
        'text/javascript; charset=utf-8',
        [
          'self.onmessage = async event => {',
          '  if (event.data && event.data.kind === "fetch") {',
          '    try {',
          '      const response = await fetch(event.data.url);',
          '      const text = await response.text();',
          '      self.postMessage({ kind: "fetch", ok: true, status: response.status, text });',
          '    } catch (error) {',
          '      self.postMessage({',
          '        kind: "fetch",',
          '        ok: false,',
          '        error: `${error?.constructor?.name || "Error"}:${error?.message || String(error)}`,',
          '      });',
          '    }',
          '    return;',
          '  }',
          '  if (event.data && event.data.kind === "xhr") {',
          '    const xhr = new XMLHttpRequest();',
          '    xhr.open("GET", event.data.url, true);',
          '    xhr.onload = () => {',
          '      self.postMessage({ kind: "xhr", ok: true, status: xhr.status, text: xhr.responseText });',
          '    };',
          '    xhr.onerror = () => {',
          '      self.postMessage({ kind: "xhr", ok: false, error: `NetworkError:${xhr.status}` });',
          '    };',
          '    xhr.send();',
          '    return;',
          '  }',
          '  self.postMessage({',
          '    echoed: event.data,',
          '    pathname: self.location.pathname,',
          '    selfEqualsGlobal: self === globalThis,',
          '  });',
          '};',
        ].join('\n'),
      );
      return;
    }

    if (url.pathname === '/download-page') {
      send(
        200,
        'text/html; charset=utf-8',
        [
          '<!doctype html>',
          '<a id="download" href="/download">download</a>',
          '<a id="slow-download" href="/slow-download" download>slow download</a>',
        ].join(''),
      );
      return;
    }

    if (url.pathname === '/worker-route-continue') {
      send(
        200,
        'application/json; charset=utf-8',
        JSON.stringify({
          method: req.method,
          routeHeader: req.headers['x-smoke-worker-route'] || null,
        }),
      );
      return;
    }

    if (url.pathname === '/worker-route-fulfill') {
      send(500, 'text/plain; charset=utf-8', 'worker route did not fulfill /worker-route-fulfill');
      return;
    }

    if (url.pathname === '/worker-route-abort') {
      send(200, 'text/plain; charset=utf-8', 'worker route did not abort /worker-route-abort');
      return;
    }

    if (url.pathname === '/download') {
      res.writeHead(200, {
        'content-type': 'text/plain; charset=utf-8',
        'cache-control': 'no-store',
        'content-disposition': 'attachment; filename="smoke-download.txt"',
      });
      res.end('download contents');
      return;
    }

    if (url.pathname === '/slow-download') {
      res.writeHead(200, {
        'content-type': 'text/plain; charset=utf-8',
        'cache-control': 'no-store',
        'content-disposition': 'attachment; filename="slow-smoke-download.txt"',
      });
      res.write('slow download prefix\n');
      const timer = setTimeout(() => {
        if (!res.destroyed) {
          res.end('slow download tail\n');
        }
      }, 5_000);
      res.on('close', () => clearTimeout(timer));
      return;
    }

    send(404, 'text/plain; charset=utf-8', `missing fixture: ${url.pathname}`);
  });

  server.on('upgrade', (req, socket) => {
    const url = new URL(req.url || '/', 'http://127.0.0.1');
    if (url.pathname !== '/ws-echo') {
      socket.write('HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n');
      socket.destroy();
      return;
    }

    const key = req.headers['sec-websocket-key'];
    if (typeof key !== 'string') {
      socket.write('HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n');
      socket.destroy();
      return;
    }

    const protocols = String(req.headers['sec-websocket-protocol'] || '')
      .split(',')
      .map(protocol => protocol.trim())
      .filter(Boolean);
    const selectedProtocol = protocols.includes('smoke') ? 'smoke' : null;
    const headers = [
      'HTTP/1.1 101 Switching Protocols',
      'Upgrade: websocket',
      'Connection: Upgrade',
      `Sec-WebSocket-Accept: ${websocketAcceptKey(key)}`,
    ];
    if (selectedProtocol) {
      headers.push(`Sec-WebSocket-Protocol: ${selectedProtocol}`);
    }
    socket.write(`${headers.join('\r\n')}\r\n\r\n`);

    let buffered = Buffer.alloc(0);
    socket.on('data', chunk => {
      buffered = Buffer.concat([buffered, chunk]);
      buffered = drainWebSocketFrames(buffered, (opcode, payload) => {
        if (opcode === 0x1) {
          socket.write(websocketTextFrame(`echo:${payload.toString('utf8')}`));
        } else if (opcode === 0x8) {
          socket.write(websocketCloseFrame(1000, 'bye'));
          socket.end();
        } else if (opcode === 0x9) {
          socket.write(websocketFrame(0xA, payload));
        }
      });
    });
    socket.on('error', () => {});
  });

  return server;
}

async function waitForCdpServer(endpoint, processHandle, logs) {
  const deadline = Date.now() + 10_000;
  let lastError = '';
  while (Date.now() < deadline) {
    if (processHandle.exitCode !== null) {
      throw new Error(`moli serve exited early with ${processHandle.exitCode}\n${logs()}`);
    }
    try {
      const response = await fetch(`${endpoint}/json/version/`, { signal: AbortSignal.timeout(500) });
      if (response.ok) {
        return;
      }
      lastError = `HTTP ${response.status}`;
    } catch (error) {
      lastError = error.message;
    }
    await new Promise(resolveSleep => setTimeout(resolveSleep, 100));
  }
  throw new Error(`timed out waiting for moli serve at ${endpoint}: ${lastError}\n${logs()}`);
}

function startMoliServe(port) {
  let logBuffer = '';
  const appendLog = data => {
    const text = data.toString();
    logBuffer += text;
    if (process.env.MOLI_SMOKE_TRACE_BG === '1') {
      process.stderr.write(text);
    }
    if (logBuffer.length > 24_000) {
      logBuffer = logBuffer.slice(-24_000);
    }
  };
  const child = spawn(
    moliBinary(),
    ['serve', '--host', '127.0.0.1', '--port', String(port), '--log-level', 'warn'],
    {
      cwd: repoRoot,
      env: clearProxyEnv(process.env),
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  );
  child.stdout.on('data', appendLog);
  child.stderr.on('data', appendLog);
  return {
    child,
    logs: () => logBuffer.trim(),
  };
}

async function stopProcess(child) {
  if (!child || child.exitCode !== null) {
    return;
  }
  child.kill('SIGTERM');
  await Promise.race([
    new Promise(resolveExit => child.once('exit', resolveExit)),
    new Promise(resolveTimeout => setTimeout(resolveTimeout, 2_000)),
  ]);
  if (child.exitCode === null) {
    child.kill('SIGKILL');
  }
}

function assertEqual(actual, expected, label) {
  if (actual !== expected) {
    throw new Error(`${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function record(results, name, data = {}) {
  results.push({ name, ok: true, ...data });
}

function traceStep(label) {
  if (process.env.MOLI_SMOKE_TRACE === '1') {
    console.error(`[smoke] ${label}`);
  }
}

async function waitUntil(predicate, label, timeoutMs = 5_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (predicate()) {
      return;
    }
    await new Promise(resolveSleep => setTimeout(resolveSleep, 25));
  }
  throw new Error(`timed out waiting for ${label}`);
}

async function runWorkerCommand(page, command, timeout = 5_000) {
  await page.evaluate(command => {
    globalThis.__smokeWorkerResult = null;
    globalThis.__smokeWorkerError = null;
    const worker = new Worker('/worker.js');
    worker.onmessage = event => {
      if (globalThis.__smokeTraceWorker === true) {
        console.warn(`[smoke-worker] onmessage ${JSON.stringify(event.data)}`);
      }
      globalThis.__smokeWorkerResult = event.data;
      worker.terminate();
    };
    worker.onerror = event => {
      if (globalThis.__smokeTraceWorker === true) {
        console.warn(`[smoke-worker] onerror ${event.message || 'unknown worker error'}`);
      }
      globalThis.__smokeWorkerError = event.message || 'unknown worker error';
      worker.terminate();
    };
    worker.postMessage(command);
  }, command);
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const state = await page.evaluate(() => ({
      result: globalThis.__smokeWorkerResult,
      error: globalThis.__smokeWorkerError,
    }));
    if (state.error !== null) {
      throw new Error(`worker failed: ${state.error}`);
    }
    if (state.result !== null) {
      return state.result;
    }
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  const workerFailure = await page.evaluate(() => globalThis.__smokeWorkerError);
  if (workerFailure !== null) {
    throw new Error(`worker failed: ${workerFailure}`);
  }
  throw new Error(`timed out waiting for worker result after ${timeout}ms`);
}

async function runBrowserContextProfileSmoke(browser, fixture, results) {
  const profileUserAgent = 'MoliProfileSmoke/1.0';
  const profileContext = await browser.newContext({
    userAgent: profileUserAgent,
    locale: 'zh-CN',
    timezoneId: 'Asia/Shanghai',
    extraHTTPHeaders: {
      'x-moli-profile-smoke': 'context-extra-header',
    },
  });
  try {
    const profilePage = await profileContext.newPage();
    const profileReferer = `${fixture}/profile-referer`;
    const profileToken = `profile-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    await profilePage.goto(`${fixture}/profile-headers?token=${encodeURIComponent(profileToken)}`, {
      waitUntil: 'load',
      timeout: 10_000,
      referer: profileReferer,
    });
    const profileHeadersResponse = await fetch(
      `${fixture}/profile-result?token=${encodeURIComponent(profileToken)}`,
    );
    const profileHeaders = await profileHeadersResponse.json();
    if (!profileHeaders) {
      throw new Error(`profile fixture did not capture browser request for token ${profileToken}`);
    }
    assertEqual(profileHeaders.userAgent, profileUserAgent, 'profile context User-Agent header');
    if (!String(profileHeaders.acceptLanguage || '').toLowerCase().includes('zh-cn')) {
      throw new Error(`profile context Accept-Language header missing zh-CN: ${profileHeaders.acceptLanguage}`);
    }
    assertEqual(
      profileHeaders.extraHeader,
      'context-extra-header',
      'profile context extra HTTP header',
    );
    assertEqual(profileHeaders.referer, profileReferer, 'profile context goto referer header');
    const profileRuntime = await profilePage.evaluate(() => ({
      userAgent: navigator.userAgent,
      language: navigator.language,
      languages: navigator.languages,
      timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    }));
    assertEqual(profileRuntime.userAgent, profileUserAgent, 'profile context navigator.userAgent');
    assertEqual(profileRuntime.language, 'zh-CN', 'profile context navigator.language');
    assertEqual(profileRuntime.languages?.[0], 'zh-CN', 'profile context navigator.languages[0]');
    assertEqual(profileRuntime.timeZone, 'Asia/Shanghai', 'profile context timezone');
    record(results, 'browser_context_profile_overrides');
  } finally {
    await profileContext.close();
  }
}

async function runPopupRouteEvaluateSmoke(browser, fixture, results) {
  const context = await browser.newContext();
  try {
    const page = await context.newPage();
    await page.goto(`${fixture}/plain`, { waitUntil: 'load', timeout: 10_000 });

    const popupUrl = `${fixture}/plain?popup=wait-for-event`;
    const popupPromise = page.waitForEvent('popup', { timeout: 10_000 });
    await page.evaluate(url => window.open(url, '_blank'), popupUrl);
    const popup = await popupPromise;
    await waitUntil(() => popup.url() === popupUrl, 'popup waitForEvent URL');
    assertEqual(popup.url(), popupUrl, 'popup waitForEvent URL');
    await popup.goto(popupUrl, { waitUntil: 'load', timeout: 10_000 });
    assertEqual(await popup.textContent('main', { timeout: 5_000 }), 'plain ok', 'popup initial text');

    await popup.route('**/popup-api', route =>
      route.fulfill({
        status: 200,
        contentType: 'application/json; charset=utf-8',
        body: JSON.stringify({ source: 'popup route', ok: true }),
      }),
    );
    const payload = await popup.evaluate(async () => {
      const response = await fetch('/popup-api');
      return {
        url: location.href,
        mainText: document.querySelector('main')?.textContent,
        api: await response.json(),
      };
    });
    assertEqual(payload?.url, popupUrl, 'popup evaluate URL');
    assertEqual(payload?.mainText, 'plain ok', 'popup evaluate document text');
    assertEqual(payload?.api?.source, 'popup route', 'popup route fulfilled source');
    assertEqual(payload?.api?.ok, true, 'popup route fulfilled ok');
    await popup.unroute('**/popup-api');
    await popup.close();
    record(results, 'popup_wait_for_event_route_evaluate');
  } finally {
    await context.close();
  }
}

async function runSmoke({ chromium, endpoint, fixture }) {
  const results = [];
  const tempDir = mkdtempSync(join(tmpdir(), 'moli-pw-smoke-'));
  const browser = await chromium.connectOverCDP(endpoint, { timeout: 10_000 });
  try {
    let browserCdp = null;
    try {
      browserCdp = await browser.newBrowserCDPSession();
    } catch (error) {
      if (process.env.MOLI_SMOKE_TRACE === '1') {
        traceStep(`browser_cdp:unavailable:${error?.message || String(error)}`);
      }
    }
    if (browserCdp && process.env.MOLI_SMOKE_TRACE === '1') {
      browserCdp.on('Target.attachedToTarget', event => {
        traceStep(
          `browser_cdp:attached:${event.targetInfo?.targetId || 'unknown'}:${event.sessionId || 'no-session'}`,
        );
      });
      browserCdp.on('Target.detachedFromTarget', event => {
        traceStep(
          `browser_cdp:detached:${event.targetId || 'unknown'}:${event.sessionId || 'no-session'}`,
        );
      });
    }
    record(results, 'connect_over_cdp', { browserContexts: browser.contexts().length });

    const context = await browser.newContext({ acceptDownloads: true });
    record(results, 'browser_new_context');

    const page = await context.newPage();
    if (process.env.MOLI_SMOKE_TRACE === '1') {
      page.on('console', message => {
        traceStep(`page_console:${message.type()}:${message.text()}`);
      });
    }
    const cdp = await context.newCDPSession(page);
    const websocketEvents = [];
    const subresourceNetworkEvents = [];
    for (const method of [
      'Network.webSocketCreated',
      'Network.webSocketWillSendHandshakeRequest',
      'Network.webSocketHandshakeResponseReceived',
      'Network.webSocketFrameSent',
      'Network.webSocketFrameReceived',
    ]) {
      cdp.on(method, params => websocketEvents.push({ method, params }));
    }
    for (const method of [
      'Network.requestWillBeSent',
      'Network.responseReceived',
      'Network.loadingFinished',
      'Network.loadingFailed',
    ]) {
      cdp.on(method, params => subresourceNetworkEvents.push({ method, params }));
    }
    await cdp.send('Network.enable');

    await page.goto(`${fixture}/plain`, { waitUntil: 'load', timeout: 10_000 });
    assertEqual(await page.textContent('main'), 'plain ok', 'plain page text');
    record(results, 'new_page_goto_plain');

    const second = await context.newPage();
    await second.goto(`${fixture}/plain`, { waitUntil: 'load', timeout: 10_000 });
    assertEqual(await second.textContent('main'), 'plain ok', 'second page text');
    record(results, 'second_page_same_context');
    await second.close();

    await page.goto(`${fixture}/iframe`, { waitUntil: 'load', timeout: 10_000 });
    const child = page.frames().find(frame => frame.url().includes('/child'));
    if (!child) {
      throw new Error(`missing child frame; frames=${page.frames().map(frame => frame.url()).join(', ')}`);
    }
    assertEqual((await child.textContent('body', { timeout: 5_000 })).trim(), 'child body text', 'child frame text');
    record(results, 'iframe_child_text_content', { frameCount: page.frames().length });

    await page.goto(`${fixture}/wait-for-function`, {
      waitUntil: 'domcontentloaded',
      timeout: 10_000,
    });
    await page.waitForFunction(() => globalThis.__ready === true, null, { timeout: 5_000 });
    record(results, 'wait_for_function_timer');

    await context.addCookies([{ name: 'manualCookie', value: 'manual', url: fixture }]);
    await page.goto(`${fixture}/echo-cookie`, { waitUntil: 'load', timeout: 10_000 });
    const manualCookieEcho = await page.textContent('body');
    if (!manualCookieEcho.includes('manualCookie=manual')) {
      throw new Error(`manual cookie was not sent: ${manualCookieEcho}`);
    }
    await page.goto(`${fixture}/set-cookie`, { waitUntil: 'load', timeout: 10_000 });
    const serverCookies = await context.cookies(fixture);
    if (!serverCookies.some(cookie => cookie.name === 'serverCookie' && cookie.value === 'server')) {
      throw new Error(`server Set-Cookie did not reach browser context: ${JSON.stringify(serverCookies)}`);
    }
    await page.goto(`${fixture}/echo-cookie`, { waitUntil: 'load', timeout: 10_000 });
    const serverCookieEcho = await page.textContent('body');
    if (!serverCookieEcho.includes('serverCookie=server')) {
      throw new Error(`server cookie was not sent back: ${serverCookieEcho}`);
    }
    record(results, 'cookie_profile_round_trip');

    const redirectResponse = await page.goto(`${fixture}/redirect-start`, {
      waitUntil: 'load',
      timeout: 10_000,
    });
    assertEqual(page.url(), `${fixture}/redirect-final`, 'redirect final page URL');
    assertEqual(redirectResponse?.url(), `${fixture}/redirect-final`, 'redirect final response URL');
    assertEqual(redirectResponse?.status(), 200, 'redirect final response status');
    assertEqual(await page.textContent('main'), 'redirect final', 'redirect final text');
    record(results, 'redirect_final_response');

    await page.goto(`${fixture}/history-a`, { waitUntil: 'load', timeout: 10_000 });
    await page.goto(`${fixture}/history-b`, { waitUntil: 'load', timeout: 10_000 });
    const backResponse = await page.goBack({ waitUntil: 'load', timeout: 10_000 });
    assertEqual(page.url(), `${fixture}/history-a`, 'history goBack URL');
    assertEqual(backResponse?.url(), `${fixture}/history-a`, 'history goBack response URL');
    assertEqual(await page.textContent('main'), 'history a', 'history goBack text');
    const forwardResponse = await page.goForward({ waitUntil: 'load', timeout: 10_000 });
    assertEqual(page.url(), `${fixture}/history-b`, 'history goForward URL');
    assertEqual(forwardResponse?.url(), `${fixture}/history-b`, 'history goForward response URL');
    assertEqual(await page.textContent('main'), 'history b', 'history goForward text');
    record(results, 'history_back_forward');

    traceStep('history_back_forward_after_target_switch:start');
    const parkedPage = await context.newPage();
    parkedPage.on('close', () => traceStep('history_back_forward_after_target_switch:parked_page_close_event'));
    traceStep('history_back_forward_after_target_switch:new_page_created');
    await parkedPage.goto(`${fixture}/plain`, { waitUntil: 'load', timeout: 10_000 });
    traceStep('history_back_forward_after_target_switch:parked_page_loaded');
    await page.bringToFront();
    traceStep('history_back_forward_after_target_switch:brought_to_front');
    const parkedBackResponse = await page.goBack({ waitUntil: 'load', timeout: 10_000 });
    traceStep('history_back_forward_after_target_switch:went_back');
    assertEqual(page.url(), `${fixture}/history-a`, 'history goBack URL after target switch');
    assertEqual(
      parkedBackResponse?.url(),
      `${fixture}/history-a`,
      'history goBack response URL after target switch',
    );
    const parkedForwardResponse = await page.goForward({ waitUntil: 'load', timeout: 10_000 });
    traceStep('history_back_forward_after_target_switch:went_forward');
    assertEqual(page.url(), `${fixture}/history-b`, 'history goForward URL after target switch');
    assertEqual(
      parkedForwardResponse?.url(),
      `${fixture}/history-b`,
      'history goForward response URL after target switch',
    );
    traceStep('history_back_forward_after_target_switch:closing_parked_page');
    const parkedPageClosePromise = parkedPage.close();
    if (process.env.MOLI_SMOKE_TRACE === '1') {
      const closeProbe = await Promise.race([
        parkedPageClosePromise.then(() => 'resolved'),
        new Promise(resolve => setTimeout(() => resolve('timeout'), 1_000)),
      ]);
      traceStep(
        `history_back_forward_after_target_switch:close_probe:${closeProbe}:pages=${context.pages().length}`,
      );
    }
    await parkedPageClosePromise;
    traceStep('history_back_forward_after_target_switch:parked_page_closed');
    record(results, 'history_back_forward_after_target_switch');
    traceStep('history_back_forward_after_target_switch:done');

    traceStep('document_route_matrix:start');
    await page.route('**/document-fulfill', route =>
      route.fulfill({
        status: 200,
        contentType: 'text/html; charset=utf-8',
        body: '<!doctype html><main>document fulfilled body</main>',
      }),
    );
    const fulfilledDocumentResponse = await page.goto(`${fixture}/document-fulfill`, {
      waitUntil: 'load',
      timeout: 10_000,
    });
    assertEqual(
      fulfilledDocumentResponse?.status(),
      200,
      'document route fulfill response status',
    );
    assertEqual(
      await page.textContent('main'),
      'document fulfilled body',
      'document route fulfill body',
    );
    record(results, 'route_fulfill_document');

    await page.unroute('**/document-fulfill');
    await page.route('**/document-continue', route => {
      const headers = {
        ...route.request().headers(),
        'x-smoke-nav-route': 'continued-document',
      };
      return route.continue({ headers });
    });
    const documentContinueStartIndex = subresourceNetworkEvents.length;
    const continuedDocumentResponse = await page.goto(`${fixture}/document-continue`, {
      waitUntil: 'load',
      timeout: 10_000,
    });
    assertEqual(
      continuedDocumentResponse?.status(),
      200,
      'document route continue response status',
    );
    assertEqual(
      await page.textContent('main'),
      'continued-document',
      'document route continue body',
    );
    await waitUntil(() => {
      const events = subresourceNetworkEvents.slice(documentContinueStartIndex);
      const request = events.find(
        event =>
          event.method === 'Network.requestWillBeSent' &&
          event.params?.type === 'Document' &&
          event.params?.request?.url === `${fixture}/document-continue`,
      );
      const requestId = request?.params?.requestId;
      return (
        requestId &&
        events.some(
          event =>
            event.method === 'Network.responseReceived' &&
            event.params?.requestId === requestId &&
            event.params?.type === 'Document' &&
            event.params?.response?.status === 200,
        ) &&
        events.some(
          event =>
            event.method === 'Network.loadingFinished' &&
            event.params?.requestId === requestId,
        )
      );
    }, 'Document route continue Network events on auxiliary CDP session');
    record(results, 'route_continue_document');

    await page.unroute('**/document-continue');
    await page.route('**/document-abort', route => route.abort('blockedbyclient'));
    const documentAbortStartIndex = subresourceNetworkEvents.length;
    let abortedDocumentError = null;
    try {
      await page.goto(`${fixture}/document-abort`, {
        waitUntil: 'load',
        timeout: 10_000,
      });
    } catch (error) {
      abortedDocumentError = String(error?.message || error);
    }
    if (!abortedDocumentError || !abortedDocumentError.includes('ERR_BLOCKED_BY_CLIENT')) {
      throw new Error(
        `document route abort should reject navigation with ERR_BLOCKED_BY_CLIENT, got ${abortedDocumentError}`,
      );
    }
    await waitUntil(() => {
      const events = subresourceNetworkEvents.slice(documentAbortStartIndex);
      const request = events.find(
        event =>
          event.method === 'Network.requestWillBeSent' &&
          event.params?.type === 'Document' &&
          event.params?.request?.url === `${fixture}/document-abort`,
      );
      const requestId = request?.params?.requestId;
      return (
        requestId &&
        events.some(
          event =>
            event.method === 'Network.loadingFailed' &&
            event.params?.requestId === requestId &&
            event.params?.errorText === 'net::ERR_BLOCKED_BY_CLIENT',
        )
      );
    }, 'Document route abort Network.loadingFailed on auxiliary CDP session');
    record(results, 'route_abort_document');
    await page.unroute('**/document-abort');
    traceStep('document_route_matrix:done');

    traceStep('post_history_switch_plain_goto:start');
    await page.route('**/api', route =>
      route.fulfill({
        status: 200,
        contentType: 'text/plain; charset=utf-8',
        body: 'fulfilled body',
      }),
    );
    await page.goto(`${fixture}/plain`, { waitUntil: 'load', timeout: 10_000 });
    traceStep('post_history_switch_plain_goto:done');
    const fetched = await page.evaluate(async () => await fetch('/api').then(response => response.text()));
    assertEqual(fetched, 'fulfilled body', 'route fulfilled fetch body');
    record(results, 'route_fulfill_fetch');

    const xhrFulfilled = await page.evaluate(async () => {
      return await new Promise(resolve => {
        const xhr = new XMLHttpRequest();
        xhr.open('GET', '/api', true);
        xhr.onload = () => {
          resolve({
            phase: 'load',
            status: xhr.status,
            text: xhr.responseText,
          });
        };
        xhr.onerror = () => {
          resolve({
            phase: 'error',
            status: xhr.status,
            readyState: xhr.readyState,
          });
        };
        xhr.send();
      });
    });
    assertEqual(xhrFulfilled?.phase, 'load', 'route fulfilled xhr phase');
    assertEqual(xhrFulfilled?.status, 200, 'route fulfilled xhr status');
    assertEqual(xhrFulfilled?.text, 'fulfilled body', 'route fulfilled xhr body');
    record(results, 'route_fulfill_xhr');

    await page.route('**/api-continue', route => {
      const headers = {
        ...route.request().headers(),
        'x-smoke-route': 'continued',
      };
      return route.continue({ headers });
    });
    const continued = await page.evaluate(async () => await fetch('/api-continue').then(response => response.json()));
    assertEqual(continued.routeHeader, 'continued', 'route continue request header');
    record(results, 'route_continue_fetch');

    const xhrContinued = await page.evaluate(async () => {
      return await new Promise(resolve => {
        const xhr = new XMLHttpRequest();
        xhr.open('POST', '/api-continue', true);
        xhr.onload = () => {
          resolve({
            phase: 'load',
            status: xhr.status,
            payload: xhr.responseText,
          });
        };
        xhr.onerror = () => {
          resolve({
            phase: 'error',
            status: xhr.status,
            readyState: xhr.readyState,
          });
        };
        xhr.send('payload');
      });
    });
    assertEqual(xhrContinued?.phase, 'load', 'route continue xhr phase');
    assertEqual(xhrContinued?.status, 200, 'route continue xhr status');
    assertEqual(
      xhrContinued?.payload,
      JSON.stringify({ method: 'POST', routeHeader: 'continued' }),
      'route continue xhr body',
    );
    record(results, 'route_continue_xhr');

    await page.route('**/api-abort', route => route.abort('blockedbyclient'));
    const aborted = await page.evaluate(async () => {
      try {
        await fetch('/api-abort');
        return 'resolved';
      } catch (error) {
        return `${error?.constructor?.name || 'Error'}:${error?.message || String(error)}`;
      }
    });
    if (!aborted.startsWith('TypeError:')) {
      throw new Error(`route abort should reject fetch with TypeError, got ${aborted}`);
    }
    record(results, 'route_abort_fetch');

    const xhrAborted = await page.evaluate(async () => {
      return await new Promise(resolve => {
        const xhr = new XMLHttpRequest();
        const events = [];
        xhr.addEventListener('load', () => events.push('load'));
        xhr.addEventListener('error', () => events.push('error'));
        xhr.addEventListener('abort', () => events.push('abort'));
        xhr.addEventListener('loadend', () => {
          resolve({
            events,
            status: xhr.status,
            readyState: xhr.readyState,
          });
        });
        xhr.open('GET', '/api-abort', true);
        xhr.send();
      });
    });
    if (!xhrAborted?.events?.includes('error')) {
      throw new Error(`route abort xhr should emit error, got ${JSON.stringify(xhrAborted)}`);
    }
    if (xhrAborted?.events?.includes('load')) {
      throw new Error(`route abort xhr should not emit load, got ${JSON.stringify(xhrAborted)}`);
    }
    assertEqual(xhrAborted?.status, 0, 'route abort xhr status');
    record(results, 'route_abort_xhr');
    await page.unroute('**/api');
    await page.unroute('**/api-continue');
    await page.unroute('**/api-abort');
    traceStep('page_route_matrix:done');
    const subresourceStartIndex = subresourceNetworkEvents.length;
    const networkObserved = await page.evaluate(async () => await fetch('/api-continue').then(response => response.json()));
    assertEqual(networkObserved.routeHeader, null, 'unrouted fetch should reach fixture server');
    await waitUntil(() => {
      const events = subresourceNetworkEvents.slice(subresourceStartIndex);
      const request = events.find(
        event =>
          event.method === 'Network.requestWillBeSent' &&
          event.params?.type === 'Fetch' &&
          event.params?.request?.url === `${fixture}/api-continue`,
      );
      const requestId = request?.params?.requestId;
      return (
        requestId &&
        events.some(
          event =>
            event.method === 'Network.responseReceived' &&
            event.params?.requestId === requestId &&
            event.params?.response?.status === 200,
        ) &&
        events.some(event => event.method === 'Network.loadingFinished' && event.params?.requestId === requestId)
      );
    }, 'Fetch Network events on auxiliary CDP session');
    record(results, 'fetch_network_events');
    traceStep('fetch_network_events:done');

    const xhrSubresourceStartIndex = subresourceNetworkEvents.length;
    const xhrObserved = await page.evaluate(async () => {
      return await new Promise(resolve => {
        const xhr = new XMLHttpRequest();
        xhr.open('GET', '/api-continue', true);
        xhr.onload = () => resolve(JSON.parse(xhr.responseText));
        xhr.send();
      });
    });
    assertEqual(xhrObserved.routeHeader, null, 'unrouted xhr should reach fixture server');
    assertEqual(xhrObserved.method, 'GET', 'unrouted xhr should keep original method');
    await waitUntil(() => {
      const events = subresourceNetworkEvents.slice(xhrSubresourceStartIndex);
      const request = events.find(
        event =>
          event.method === 'Network.requestWillBeSent' &&
          event.params?.type === 'XHR' &&
          event.params?.request?.url === `${fixture}/api-continue`,
      );
      const requestId = request?.params?.requestId;
      return (
        requestId &&
        events.some(
          event =>
            event.method === 'Network.responseReceived' &&
            event.params?.requestId === requestId &&
            event.params?.response?.status === 200,
        ) &&
        events.some(event => event.method === 'Network.loadingFinished' && event.params?.requestId === requestId)
      );
    }, 'XHR Network events on auxiliary CDP session');
    record(results, 'xhr_network_events');
    traceStep('xhr_network_events:done');

    const parserScriptStartIndex = subresourceNetworkEvents.length;
    await page.goto(`${fixture}/parser-script-page`, { waitUntil: 'load', timeout: 10_000 });
    assertEqual(
      await page.evaluate(() => globalThis.__smokeParserScriptValue),
      'parser script loaded',
      'parser script executed before load completion',
    );
    let parserScriptRequestId = null;
    await waitUntil(() => {
      const events = subresourceNetworkEvents.slice(parserScriptStartIndex);
      const request = events.find(
        event =>
          event.method === 'Network.requestWillBeSent' &&
          event.params?.type === 'Script' &&
          event.params?.request?.url === `${fixture}/parser-script.js`,
      );
      const response = events.find(
        event =>
          event.method === 'Network.responseReceived' &&
          event.params?.type === 'Script' &&
          event.params?.response?.url === `${fixture}/parser-script.js`,
      );
      const requestId = request?.params?.requestId;
      if (
        !requestId ||
        !response ||
        !events.some(
          event =>
            event.method === 'Network.loadingFinished' && event.params?.requestId === requestId,
        )
      ) {
        return false;
      }
      parserScriptRequestId = requestId;
      return true;
    }, 'parser Script Network events on auxiliary CDP session');
    const parserScriptBody = await cdp.send('Network.getResponseBody', {
      requestId: parserScriptRequestId,
    });
    assertEqual(
      parserScriptBody.body,
      'globalThis.__smokeParserScriptValue = "parser script loaded";',
      'parser script response body',
    );
    record(results, 'parser_script_network_events');
    traceStep('parser_script_network_events:done');

    const blockedWebSocketUrl = `${fixture.replace(/^http:/, 'ws:')}/ws-blocked`;
    const blockedWebSocketStartIndex = subresourceNetworkEvents.length;
    await cdp.send('Network.setBlockedURLs', { urls: [`${blockedWebSocketUrl}*`] });
    const blockedWebSocketResult = await page.evaluate(async url => {
      return await new Promise(resolve => {
        const socket = new WebSocket(url);
        const finish = value => resolve(value);
        const timer = setTimeout(() => {
          socket.close();
          finish(`timeout:${socket.readyState}`);
        }, 5_000);
        socket.onopen = () => {
          clearTimeout(timer);
          finish('open');
        };
        socket.onerror = () => {
          clearTimeout(timer);
          finish(`error:${socket.readyState}`);
        };
        socket.onclose = event => {
          clearTimeout(timer);
          finish(`close:${event.code}:${event.wasClean}`);
        };
      });
    }, blockedWebSocketUrl);
    if (blockedWebSocketResult === 'open' || blockedWebSocketResult.startsWith('timeout:')) {
      throw new Error(`blocked WebSocket should fail, got ${blockedWebSocketResult}`);
    }
    await waitUntil(() => {
      const events = subresourceNetworkEvents.slice(blockedWebSocketStartIndex);
      const request = events.find(
        event =>
          event.method === 'Network.requestWillBeSent' &&
          event.params?.type === 'WebSocket' &&
          event.params?.request?.url === blockedWebSocketUrl,
      );
      const requestId = request?.params?.requestId;
      return (
        requestId &&
        events.some(
          event =>
            event.method === 'Network.loadingFailed' &&
            event.params?.requestId === requestId &&
            event.params?.errorText === 'net::ERR_BLOCKED_BY_CLIENT',
        )
      );
    }, 'blocked WebSocket Network.loadingFailed on auxiliary CDP session');
    await cdp.send('Network.setBlockedURLs', { urls: [] });
    record(results, 'blocked_websocket_network_events');
    traceStep('blocked_websocket_network_events:done');

    await page.goto(`${fixture}/plain`, { waitUntil: 'load', timeout: 10_000 });
    traceStep('worker_postmessage_round_trip:start');
    await page.evaluate(enabled => {
      globalThis.__smokeTraceWorker = enabled;
    }, process.env.MOLI_SMOKE_TRACE === '1');
    const workerResult = await runWorkerCommand(page, 'worker ping');
    assertEqual(workerResult?.echoed, 'worker ping', 'worker echoed message');
    assertEqual(workerResult?.pathname, '/worker.js', 'worker location pathname');
    assertEqual(workerResult?.selfEqualsGlobal, true, 'worker global self identity');
    record(results, 'worker_postmessage_round_trip');
    traceStep('worker_postmessage_round_trip:done');

    traceStep('worker_route_fulfill_fetch:start');
    await context.route('**/worker-route-fulfill', route =>
      (traceStep(`worker_route_fulfill_fetch:route_hit:${route.request().url()}`),
      route.fulfill({
        status: 200,
        contentType: 'text/plain; charset=utf-8',
        body: 'worker fulfilled body',
      })),
    );
    const workerFetchFulfill = await runWorkerCommand(page, {
      kind: 'fetch',
      url: '/worker-route-fulfill',
    });
    assertEqual(workerFetchFulfill?.ok, true, 'worker fetch route fulfill ok');
    assertEqual(workerFetchFulfill?.status, 200, 'worker fetch route fulfill status');
    assertEqual(workerFetchFulfill?.text, 'worker fulfilled body', 'worker fetch route fulfill body');
    record(results, 'worker_route_fulfill_fetch');
    traceStep('worker_route_fulfill_fetch:done');

    traceStep('worker_route_continue_xhr:start');
    await context.route('**/worker-route-continue', route => {
      traceStep(`worker_route_continue_xhr:route_hit:${route.request().url()}`);
      const headers = {
        ...route.request().headers(),
        'x-smoke-worker-route': 'continued-from-worker',
      };
      return route.continue({ headers });
    });
    const workerXhrContinue = await runWorkerCommand(page, {
      kind: 'xhr',
      url: '/worker-route-continue',
    });
    assertEqual(workerXhrContinue?.ok, true, 'worker xhr route continue ok');
    assertEqual(workerXhrContinue?.status, 200, 'worker xhr route continue status');
    assertEqual(
      workerXhrContinue?.text,
      JSON.stringify({ method: 'GET', routeHeader: 'continued-from-worker' }),
      'worker xhr route continue body',
    );
    record(results, 'worker_route_continue_xhr');
    traceStep('worker_route_continue_xhr:done');

    traceStep('worker_route_abort_fetch:start');
    await context.route('**/worker-route-abort', route => {
      traceStep(`worker_route_abort_fetch:route_hit:${route.request().url()}`);
      return route.abort('blockedbyclient');
    });
    const workerFetchAbort = await runWorkerCommand(page, {
      kind: 'fetch',
      url: '/worker-route-abort',
    });
    assertEqual(workerFetchAbort?.ok, false, 'worker fetch route abort should reject');
    if (!String(workerFetchAbort?.error || '').startsWith('TypeError:')) {
      throw new Error(
        `worker fetch route abort should reject with TypeError, got ${JSON.stringify(workerFetchAbort)}`,
      );
    }
    record(results, 'worker_route_abort_fetch');
    await context.unroute('**/worker-route-fulfill');
    await context.unroute('**/worker-route-continue');
    await context.unroute('**/worker-route-abort');
    traceStep('worker_route_abort_fetch:done');

    const websocketResult = await page.evaluate(async () => {
      return await new Promise((resolve, reject) => {
        const url = new URL('/ws-echo', location.href);
        url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
        const socket = new WebSocket(url.href, 'smoke');
        const timer = setTimeout(() => {
          socket.close();
          reject(new Error(`websocket timed out at readyState=${socket.readyState}`));
        }, 5_000);
        socket.onopen = () => socket.send('websocket ping');
        socket.onmessage = event => {
          clearTimeout(timer);
          const result = {
            data: event.data,
            protocol: socket.protocol,
            readyStateBeforeClose: socket.readyState,
          };
          socket.close(1000, 'done');
          resolve(result);
        };
        socket.onerror = () => {
          clearTimeout(timer);
          reject(new Error(`websocket error at readyState=${socket.readyState}`));
        };
      });
    });
    assertEqual(websocketResult?.data, 'echo:websocket ping', 'websocket echoed message');
    assertEqual(websocketResult?.protocol, 'smoke', 'websocket selected protocol');
    assertEqual(websocketResult?.readyStateBeforeClose, 1, 'websocket ready state before close');
    record(results, 'websocket_echo_round_trip');

    await waitUntil(
      () => websocketEvents.some(event => event.method === 'Network.webSocketFrameReceived'),
      'Network.webSocketFrameReceived',
    );
    const expectedWebsocketUrl = `${fixture.replace(/^http:/, 'ws:')}/ws-echo`;
    const websocketCreated = websocketEvents.find(
      event => event.method === 'Network.webSocketCreated' && event.params?.url === expectedWebsocketUrl,
    );
    const websocketRequestId = websocketCreated?.params?.requestId;
    if (!websocketRequestId) {
      throw new Error(`missing Network.webSocketCreated event: ${JSON.stringify(websocketEvents)}`);
    }
    assertEqual(websocketCreated.params.url, expectedWebsocketUrl, 'websocket CDP created URL');
    const websocketHandshake = websocketEvents.find(
      event =>
        event.method === 'Network.webSocketHandshakeResponseReceived' &&
        event.params.requestId === websocketRequestId,
    );
    assertEqual(websocketHandshake?.params?.response?.status, 101, 'websocket CDP handshake status');
    const websocketFrameSent = websocketEvents.find(
      event =>
        event.method === 'Network.webSocketFrameSent' &&
        event.params.requestId === websocketRequestId,
    );
    assertEqual(websocketFrameSent?.params?.response?.opcode, 1, 'websocket CDP sent opcode');
    assertEqual(websocketFrameSent?.params?.response?.payloadLength, 14, 'websocket CDP sent payload length');
    const websocketFrameReceived = websocketEvents.find(
      event =>
        event.method === 'Network.webSocketFrameReceived' &&
        event.params.requestId === websocketRequestId,
    );
    assertEqual(websocketFrameReceived?.params?.response?.opcode, 1, 'websocket CDP received opcode');
    assertEqual(
      websocketFrameReceived?.params?.response?.payloadLength,
      19,
      'websocket CDP received payload length',
    );
    record(results, 'websocket_network_events', { websocketEventCount: websocketEvents.length });

    await page.setContent('<main id="set-content">set content ok</main>');
    assertEqual(await page.textContent('#set-content'), 'set content ok', 'setContent text');
    record(results, 'set_content_static_dom');

    await page.setContent(
      '<main id="set-content-inline">inline content ok</main><script>window.__moliSetContentInlineRan = (window.__moliSetContentInlineRan || 0) + 1;</script>',
    );
    assertEqual(await page.textContent('#set-content-inline'), 'inline content ok', 'setContent inline text');
    assertEqual(
      await page.evaluate(() => window.__moliSetContentInlineRan),
      1,
      'setContent inline script ran',
    );
    record(results, 'set_content_inline_script');

    const uploadFile = join(tempDir, 'upload.txt');
    writeFileSync(uploadFile, 'upload contents');
    await page.setContent('<input id="upload" type="file">');
    await page.setInputFiles('#upload', uploadFile);
    const uploaded = await page.evaluate(() => {
      const file = document.querySelector('#upload')?.files?.[0];
      return file ? { name: file.name, size: file.size } : null;
    });
    assertEqual(uploaded?.name, 'upload.txt', 'uploaded file name');
    assertEqual(uploaded?.size, 'upload contents'.length, 'uploaded file size');
    record(results, 'set_input_files');

    await page.setContent('<input id="chooser" type="file" multiple>');
    const chooserSurface = await page
      .locator('#chooser')
      .evaluate(input =>
        [
          input instanceof HTMLInputElement,
          input.constructor && input.constructor.name,
          typeof input.type,
          input.type,
          input.multiple,
        ].join('|'),
      );
    assertEqual(
      chooserSurface,
      'true|HTMLInputElement|string|file|true',
      'file chooser input surface before click',
    );
    const [fileChooser] = await Promise.all([
      page.waitForEvent('filechooser', { timeout: 10_000 }),
      page.click('#chooser'),
    ]);
    await fileChooser.setFiles(uploadFile);
    const chooserFiles = await page.evaluate(() =>
      Array.from(document.querySelector('#chooser')?.files || []).map(file => ({
        name: file.name,
        size: file.size,
      })),
    );
    assertEqual(chooserFiles.length, 1, 'file chooser selected file count');
    assertEqual(chooserFiles[0]?.name, 'upload.txt', 'file chooser selected file name');
    assertEqual(
      chooserFiles[0]?.size,
      'upload contents'.length,
      'file chooser selected file size',
    );
    record(results, 'file_chooser_set_files');

    await page.setContent(`
      <button id="open-chooser" onclick="document.getElementById('picker-script').showPicker()">open chooser</button>
      <input id="picker-script" type="file" multiple>
    `);
    const [scriptedChooser] = await Promise.all([
      page.waitForEvent('filechooser', { timeout: 10_000 }),
      page.click('#open-chooser'),
    ]);
    await scriptedChooser.setFiles(uploadFile);
    const scriptedChooserFiles = await page.evaluate(() =>
      Array.from(document.querySelector('#picker-script')?.files || []).map(file => ({
        name: file.name,
        size: file.size,
      })),
    );
    assertEqual(scriptedChooserFiles.length, 1, 'scripted file chooser selected file count');
    assertEqual(
      scriptedChooserFiles[0]?.name,
      'upload.txt',
      'scripted file chooser selected file name',
    );
    assertEqual(
      scriptedChooserFiles[0]?.size,
      'upload contents'.length,
      'scripted file chooser selected file size',
    );
    record(results, 'file_chooser_show_picker');

    await page.setContent(`<a id="go" href="${fixture}/plain">go</a>`);
    await Promise.all([
      page.waitForNavigation({ waitUntil: 'load', timeout: 10_000 }),
      page.click('#go'),
    ]);
    assertEqual(await page.textContent('main'), 'plain ok', 'click navigation target text');
    record(results, 'click_navigation');

    await page.goto(`${fixture}/download-page`, { waitUntil: 'load', timeout: 10_000 });
    const [download] = await Promise.all([
      page.waitForEvent('download', { timeout: 10_000 }),
      page.click('#download'),
    ]);
    assertEqual(download.suggestedFilename(), 'smoke-download.txt', 'download suggested filename');
    const downloadPath = await download.path();
    assertEqual(readFileSync(downloadPath, 'utf8'), 'download contents', 'download artifact contents');
    record(results, 'download_event_and_artifact');

    // saveAs starts directly in the download event handler. The object URL
    // is revoked before the protocol command delivering that event returns.
    const blobSaved = page.waitForEvent('download', { timeout: 10_000 }).then(async blobDownload => {
      const savedPath = join(tempDir, 'blob-payload.bin');
      await blobDownload.saveAs(savedPath);
      assertEqual(blobDownload.suggestedFilename(), 'blob-payload.bin', 'Blob suggested filename');
      assertEqual(await blobDownload.failure(), null, 'Blob download completion');
      assertEqual(readFileSync(savedPath).toString('hex'), '00ff80410a', 'Blob download binary contents');
    });
    await page.evaluate(() => {
      const url = URL.createObjectURL(new Blob([new Uint8Array([0, 255, 128, 65, 10])], {
        type: 'application/octet-stream',
      }));
      const link = document.createElement('a');
      link.href = url;
      link.download = 'blob-payload.bin';
      document.body.append(link);
      link.click();
      URL.revokeObjectURL(url);
      link.remove();
    });
    await blobSaved;
    record(results, 'blob_download_save_as_after_url_revocation');

    await page.goto(`${fixture}/download-page`, { waitUntil: 'load', timeout: 10_000 });
    const [slowDownload] = await Promise.all([
      page.waitForEvent('download', { timeout: 10_000 }),
      page.click('#slow-download'),
    ]);
    assertEqual(
      slowDownload.suggestedFilename(),
      'slow-smoke-download.txt',
      'slow download suggested filename',
    );
    await slowDownload.cancel();
    assertEqual(await slowDownload.failure(), 'canceled', 'slow download cancel failure state');
    record(results, 'download_cancel');

    await context.close();
    await runBrowserContextProfileSmoke(browser, fixture, results);
    await runPopupRouteEvaluateSmoke(browser, fixture, results);
    return results;
  } finally {
    await browser.close().catch(() => {});
    rmSync(tempDir, { recursive: true, force: true });
  }
}

async function main() {
  const { chromium } = loadPlaywright();
  const fixtureServer = startFixtureServer();
  let serve = null;
  const results = [];
  try {
    const fixture = await listen(fixtureServer);
    const cdpPort = process.env.MOLI_CDP_PORT
      ? Number.parseInt(process.env.MOLI_CDP_PORT, 10)
      : await reservePort();
    if (!Number.isInteger(cdpPort) || cdpPort <= 0 || cdpPort > 65_535) {
      throw new Error(`invalid MOLI_CDP_PORT: ${process.env.MOLI_CDP_PORT}`);
    }

    serve = startMoliServe(cdpPort);
    const endpoint = `http://127.0.0.1:${cdpPort}`;
    await waitForCdpServer(endpoint, serve.child, serve.logs);

    results.push(...(await runSmoke({ chromium, endpoint, fixture })));
    console.log(JSON.stringify({ ok: true, endpoint, fixture, results }, null, 2));
  } catch (error) {
    console.error(JSON.stringify({ ok: false, error: String(error?.stack || error), results }, null, 2));
    process.exitCode = 1;
  } finally {
    await stopProcess(serve?.child);
    await new Promise(resolveClose => fixtureServer.close(() => resolveClose()));
  }
}

await main();
