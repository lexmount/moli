import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { createServer } from 'node:http';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// The fixture and the protocol endpoint must both use direct loopback traffic.
for (const key of ['HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'http_proxy', 'https_proxy', 'all_proxy']) {
  delete process.env[key];
}
process.env.NO_PROXY = '*';
process.env.no_proxy = '*';
const { remote } = await import('webdriverio');
const repoRoot = fileURLToPath(new URL('../', import.meta.url));
const binary = resolve(process.env.MOLI_BIN || resolve(repoRoot, 'target/debug/moli'));

async function listen(server) {
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return server.address().port;
}

const fixture = createServer((req, res) => {
  res.setHeader('Content-Type', 'text/html; charset=utf-8');
  res.end(req.url === '/next'
    ? '<!doctype html><title>Second page</title><h1>Link navigation succeeded</h1>'
    : `<!doctype html><title>First page</title><h1>Ready</h1><h2>Untouched</h2><h3>Classic first</h3>
       <input id="name"><button id="submit" onclick="document.querySelector('#result').textContent='Hello '+document.querySelector('#name').value">Submit</button>
       <p id="result"></p><a id="next" href="/next">Next</a>`);
});
const fixturePort = await listen(fixture);
const reservation = createServer();
const port = await listen(reservation);
await new Promise(resolveClose => reservation.close(resolveClose));
const endpoint = `http://127.0.0.1:${port}`;
const fixtureUrl = `http://127.0.0.1:${fixturePort}`;
const server = spawn(binary, ['serve', '--port', String(port)], { stdio: ['ignore', 'pipe', 'pipe'] });
let serverOutput = '';
server.stdout.on('data', data => { serverOutput += data; });
server.stderr.on('data', data => { serverOutput += data; });
let serverError;
server.on('error', error => { serverError = error; });
const exited = new Promise(resolveExit => server.once('close', resolveExit));

async function ready() {
  const deadline = Date.now() + 15000;
  while (true) {
    if (serverError) throw serverError;
    try {
      const response = await fetch(`${endpoint}/status`, { signal: AbortSignal.timeout(1000) });
      assert.equal(response.status, 200);
      return;
    } catch (error) {
      if (server.exitCode !== null || Date.now() >= deadline) {
        throw new Error(`Moli did not become ready: ${serverOutput}`, { cause: error });
      }
      await new Promise(resolvePoll => setTimeout(resolvePoll, 25));
    }
  }
}

async function workflow(name, capabilities, expectedBidi) {
  const started = performance.now();
  const browser = await remote({
    hostname: '127.0.0.1', port, path: '/',
    logLevel: 'warn', connectionRetryCount: 0,
    connectionRetryTimeout: 10000, bidiResponseTimeout: 10000,
    capabilities: { browserName: 'moli', ...capabilities },
  });
  try {
    assert.equal(browser.isBidi, expectedBidi, `${name}: selected protocol`);
    assert.equal(typeof browser.capabilities.webSocketUrl === 'string', expectedBidi);
    await browser.url(`${fixtureUrl}/first`);
    assert.equal(await browser.$('h1').getText(), 'Ready');
    await browser.$('#name').setValue('Moli');
    await browser.$('#submit').click();
    assert.equal(await browser.$('#result').getText(), 'Hello Moli');
    let untouchedId;
    if (expectedBidi) {
      const { contexts } = await browser.browsingContextGetTree({});
      const context = contexts[0].context;
      const { nodes } = await browser.browsingContextLocateNodes({
        context, locator: { type: 'css', value: 'h1' },
      });
      const key = 'element-6066-11e4-a52e-4f735466cecf';
      const response = await fetch(`${endpoint}/session/${browser.sessionId}/execute/sync`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ script: 'return arguments[0];', args: [{ [key]: nodes[0].sharedId }] }),
      });
      assert.equal(response.status, 200);
      const { value } = await response.json();
      assert.equal(value[key], nodes[0].sharedId, `${name}: cross-protocol node identity`);
      const echoed = await browser.scriptCallFunction({
        target: { context }, functionDeclaration: 'function(node) { return node.textContent; }',
        arguments: [{ sharedId: value[key] }], awaitPromise: false,
      });
      assert.equal(echoed.result.value, 'Ready');
      const findClassicFirst = async () => {
        const response = await fetch(`${endpoint}/session/${browser.sessionId}/element`, {
          method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ using: 'css selector', value: 'h3' }),
        });
        assert.equal(response.status, 200);
        return (await response.json()).value[key];
      };
      const classicId = await findClassicFirst();
      const classicFirst = await browser.browsingContextLocateNodes({
        context, locator: { type: 'css', value: 'h3' },
      });
      const imported = await fetch(`${endpoint}/session/${browser.sessionId}/execute/sync`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          script: 'return arguments[0];', args: [{ [key]: classicFirst.nodes[0].sharedId }],
        }),
      });
      assert.equal(imported.status, 200);
      assert.equal((await imported.json()).value[key], classicId, `${name}: preserve existing Classic ID`);
      assert.equal(await findClassicFirst(), classicId, `${name}: repeated Classic lookup identity`);
      const untouched = await browser.browsingContextLocateNodes({
        context, locator: { type: 'css', value: 'h2' },
      });
      untouchedId = untouched.nodes[0].sharedId;
    }
    await browser.$('#next').click();
    await browser.waitUntil(async () => await browser.getTitle() === 'Second page', { timeout: 5000 });
    assert.equal(await browser.$('h1').getText(), 'Link navigation succeeded');
    if (untouchedId) {
      const response = await fetch(`${endpoint}/session/${browser.sessionId}/element/${untouchedId}/text`);
      assert.equal(response.status, 404);
      assert.equal((await response.json()).value.error, 'stale element reference');
    }
    console.log(JSON.stringify({ test: name, protocol: expectedBidi ? 'bidi' : 'classic', status: 'passed', milliseconds: Math.round(performance.now() - started) }));
  } finally {
    await browser.deleteSession();
  }
}

try {
  await ready();
  for (const webSocketUrl of [undefined, false, true]) {
    const capabilities = { browserName: 'moli', ...(webSocketUrl === undefined ? {} : { webSocketUrl }) };
    const response = await fetch(`${endpoint}/session`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ capabilities: { alwaysMatch: capabilities } }),
    });
    assert.equal(response.status, 200);
    const { value } = await response.json();
    assert.equal(typeof value.capabilities.webSocketUrl === 'string', webSocketUrl === true);
    assert.equal((await fetch(`${endpoint}/session/${value.sessionId}`, { method: 'DELETE' })).status, 200);
  }
  await workflow('webdriverio-classic-opt-out', { 'wdio:enforceWebDriverClassic': true }, false);
  await workflow('webdriverio-default-bidi', {}, true);
  await workflow('webdriverio-explicit-bidi', { 'wdio:enforceWebDriverClassic': true, webSocketUrl: true }, true);
} catch (error) {
  console.error(serverOutput);
  throw error;
} finally {
  server.kill('SIGTERM');
  await exited;
  fixture.closeAllConnections();
  await new Promise(resolveClose => fixture.close(resolveClose));
}
