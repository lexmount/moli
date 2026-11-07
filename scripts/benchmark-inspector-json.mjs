#!/usr/bin/env node
// Requires Node.js 22+ for the built-in WebSocket client. No npm dependencies.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdir, open, readFile, writeFile } from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { parseArgs } from 'node:util';

const { values: args } = parseArgs({ options: {
  before: { type: 'string' }, after: { type: 'string' }, capture: { type: 'string' },
  verify: { type: 'string' },
  output: { type: 'string', default: 'target/inspector-json-benchmark' },
  rounds: { type: 'string', default: '5' },
} });
if (!args.capture && !args.verify && (!args.before || !args.after)) {
  throw new Error('Use --capture BINARY, --verify BINARY, or --before BINARY --after BINARY [--output DIR] [--rounds N]');
}
await mkdir(args.output, { recursive: true });

class Client {
  constructor(socket) {
    this.socket = socket;
    this.sequence = 1;
    this.pending = new Map();
    this.waiters = [];
    socket.addEventListener('message', ({ data }) => {
      const raw = String(data);
      const message = JSON.parse(raw);
      if (message.id !== undefined) {
        const waiter = this.pending.get(message.id);
        if (!waiter) return;
        this.pending.delete(message.id);
        clearTimeout(waiter.timer);
        if (message.error) waiter.reject(new Error(JSON.stringify(message.error)));
        else waiter.resolve({ message, raw });
      } else {
        for (const waiter of [...this.waiters]) {
          if (waiter.matches(message)) {
            this.waiters.splice(this.waiters.indexOf(waiter), 1);
            clearTimeout(waiter.timer);
            waiter.resolve({ message, raw });
          }
        }
      }
    });
    socket.addEventListener('close', () => {
      for (const waiter of [...this.pending.values(), ...this.waiters]) {
        clearTimeout(waiter.timer);
        waiter.reject(new Error('CDP closed with pending work'));
      }
      this.pending.clear();
      this.waiters = [];
    });
  }
  static async connect(url) {
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true });
      socket.addEventListener('error', reject, { once: true });
    });
    return new Client(socket);
  }
  call(method, params = {}, sessionId) {
    const id = this.sequence++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 30_000);
      this.pending.set(id, { resolve, reject, timer });
      this.socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  }
  event(method, sessionId, predicate = () => true) {
    return new Promise((resolve, reject) => {
      const waiter = { resolve, reject, matches: message => message.method === method && message.sessionId === sessionId && predicate(message) };
      waiter.timer = setTimeout(() => {
        this.waiters.splice(this.waiters.indexOf(waiter), 1);
        reject(new Error(`CDP event timeout: ${method}`));
      }, 30_000);
      this.waiters.push(waiter);
    });
  }
  evaluate(expression, sessionId, returnByValue = true) {
    return this.call('Runtime.evaluate', { expression, returnByValue, awaitPromise: true }, sessionId);
  }
  async nested(method, params, sessionId) {
    const id = this.sequence++;
    const reply = this.event('Target.receivedMessageFromTarget', undefined, message =>
      message.params.sessionId === sessionId && JSON.parse(message.params.message).id === id);
    await this.call('Target.sendMessageToTarget', { sessionId, message: JSON.stringify({ id, method, params }) });
    return JSON.parse((await reply).message.params.message);
  }
  close() { this.socket.close(); }
}

async function start(binary, name) {
  const listener = net.createServer();
  listener.listen(0, '127.0.0.1');
  await once(listener, 'listening');
  const port = listener.address().port;
  await new Promise(resolve => listener.close(resolve));
  const log = await open(path.join(args.output, `${name}.log`), 'w');
  const child = spawn(path.resolve(binary), ['serve', '--host', '127.0.0.1', '--port', String(port), '--log-level', 'error', '--http-no-proxy', '*'], { stdio: ['ignore', 'ignore', log.fd] });
  const exited = once(child, 'exit');
  const deadline = performance.now() + 20_000;
  let version;
  try {
    while (!version) {
      if (child.exitCode !== null) throw new Error(`${name} exited during startup: ${child.exitCode}`);
      try { const response = await fetch(`http://127.0.0.1:${port}/json/version`); if (response.ok) version = await response.json(); } catch { /* Wait for the listening endpoint. */ }
      if (performance.now() > deadline) throw new Error(`${name} did not become ready`);
      if (!version) await new Promise(resolve => setTimeout(resolve, 10));
    }
    const client = await Client.connect(version.webSocketDebuggerUrl);
    const target = (await client.call('Target.createTarget', { url: 'about:blank' })).message.result.targetId;
    const session = (await client.call('Target.attachToTarget', { targetId: target, flatten: true })).message.result.sessionId;
    await client.call('Page.enable', {}, session);
    const loaded = client.event('Page.loadEventFired', session);
    await client.call('Page.navigate', { url: 'data:text/html,<p>Inspector JSON benchmark</p>' }, session);
    await loaded;
    await client.call('Runtime.enable', {}, session);
    return { name, child, client, session, async stop() {
      client.close();
      child.kill('SIGTERM');
      const watchdog = setTimeout(() => child.kill('SIGKILL'), 5_000);
      await exited;
      clearTimeout(watchdog);
      await log.close();
    } };
  } catch (error) {
    child.kill('SIGKILL');
    await exited;
    await log.close();
    throw error;
  }
}

async function capture(server) {
  const dir = path.join(args.output, 'fixtures');
  await mkdir(dir, { recursive: true });
  const cases = [];
  for (const [name, size, iterations] of [['evaluate-1k', 1024, 1000], ['evaluate-100k', 102400, 200], ['evaluate-1m', 1048576, 25]]) {
    const expression = `'x'.repeat(${size})`;
    const response = await server.client.evaluate(expression, server.session);
    await writeFile(path.join(dir, `${name}.json`), response.raw);
    cases.push({ name, method: 'Runtime.evaluate', params: { expression, returnByValue: true }, iterations, bytes: Buffer.byteLength(response.raw) });
  }
  const response = await server.client.evaluate('Object.fromEntries(Array.from({length:1000}, (_,i)=>["property"+i,"x".repeat(32)]))', server.session, false);
  const objectId = response.message.result.result.objectId;
  const properties = await server.client.call('Runtime.getProperties', { objectId, ownProperties: true }, server.session);
  await writeFile(path.join(dir, 'properties.json'), properties.raw);
  cases.push({ name: 'properties', method: 'Runtime.getProperties', params: { objectId, ownProperties: true }, iterations: 100, bytes: Buffer.byteLength(properties.raw) });
  return cases;
}

async function verify(server) {
  const checks = [];
  for (const [expression, expected] of [
    ['String.fromCharCode(0xD800)', [0xD800]],
    ['String.fromCharCode(0xDFFF)', [0xDFFF]],
    ['String.fromCharCode(0xD83D,0xDE00)', [0xD83D,0xDE00]],
    [String.raw`'\\ud800'`, [92,117,100,56,48,48]],
    ['Promise.resolve(String.fromCharCode(0xD800))', [0xD800]],
    ['new Promise(resolve=>setTimeout(()=>resolve(String.fromCharCode(0xDFFF)),0))', [0xDFFF]],
  ]) {
    const { message } = await server.client.evaluate(expression, server.session);
    const value = message.result.result.value;
    const units = Array.from({ length: value.length }, (_, index) => value.charCodeAt(index));
    assert.deepEqual(units, expected, expression);
    checks.push({ expression, units });
  }
  const object = await server.client.evaluate('({value:String.fromCharCode(0xD800),[String.fromCharCode(0xDFFF)]:1})', server.session, false);
  const objectId = object.message.result.result.objectId;
  const properties = await server.client.call('Runtime.getProperties', { objectId, ownProperties: true }, server.session);
  assert.equal(properties.message.result.result.find(property => property.name === 'value').value.value.charCodeAt(0), 0xD800);
  assert.ok(properties.message.result.result.some(property => property.name.charCodeAt(0) === 0xDFFF));
  checks.push({ method: 'Runtime.getProperties', value: [0xD800], key: [0xDFFF] });
  const called = await server.client.call('Runtime.callFunctionOn', { objectId, functionDeclaration: 'function(){return this.value}', returnByValue: true, awaitPromise: true }, server.session);
  assert.equal(called.message.result.result.value.charCodeAt(0), 0xD800);
  checks.push({ method: 'Runtime.callFunctionOn', units: [0xD800] });
  const promise = await server.client.call('Runtime.evaluate', { expression: 'Promise.resolve(String.fromCharCode(0xDFFF))', returnByValue: false, awaitPromise: false }, server.session);
  const promiseObjectId = promise.message.result.result.objectId;
  const awaited = await server.client.call('Runtime.awaitPromise', { promiseObjectId, returnByValue: true }, server.session);
  assert.equal(awaited.message.result.result.value.charCodeAt(0), 0xDFFF);
  checks.push({ method: 'Runtime.awaitPromise', units: [0xDFFF] });
  const thrown = await server.client.evaluate('throw String.fromCharCode(0xD800)', server.session);
  assert.equal(thrown.message.result.exceptionDetails.exception.value.charCodeAt(0), 0xD800);
  checks.push({ result: 'exceptionDetails', units: [0xD800] });
  for (const objectId of [object.message.result.result.objectId, promiseObjectId]) {
    await server.client.call('Runtime.releaseObject', { objectId }, server.session);
  }
  await server.client.call('Runtime.enable', {}, server.session);
  const consoleEvent = server.client.event('Runtime.consoleAPICalled', server.session);
  await server.client.evaluate('console.log(String.fromCharCode(0xD800)); 1', server.session);
  assert.equal((await consoleEvent).message.params.args[0].value.charCodeAt(0), 0xD800);
  checks.push({ event: 'Runtime.consoleAPICalled', units: [0xD800] });
  await server.client.call('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: false, flatten: true }, server.session);
  const attached = server.client.event('Target.attachedToTarget', server.session, message => message.params.targetInfo.type === 'worker');
  await server.client.evaluate('globalThis.__rawWorker = new Worker(URL.createObjectURL(new Blob(["onmessage=()=>postMessage(1)"],{type:"text/javascript"}))); 1', server.session);
  const workerSession = (await attached).message.params.sessionId;
  await server.client.call('Runtime.enable', {}, workerSession);
  const worker = await server.client.evaluate('String.fromCharCode(0xD800)', workerSession);
  assert.equal(worker.message.result.result.value.charCodeAt(0), 0xD800);
  checks.push({ target: 'Worker', units: [0xD800] });
  const legacyTarget = (await server.client.call('Target.createTarget', { url: 'about:blank' })).message.result.targetId;
  const legacySession = (await server.client.call('Target.attachToTarget', { targetId: legacyTarget, flatten: false })).message.result.sessionId;
  const legacy = await server.client.nested('Runtime.evaluate', { expression: 'String.fromCharCode(0xD800)', returnByValue: true }, legacySession);
  assert.equal(legacy.result.result.value.charCodeAt(0), 0xD800);
  checks.push({ target: 'non-flat session', units: [0xD800] });
  await server.client.evaluate('globalThis.__rawWorker.terminate(); delete globalThis.__rawWorker; 1', server.session);
  await server.client.call('Target.closeTarget', { targetId: legacyTarget });
  await server.client.call('Target.setAutoAttach', { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }, server.session);
  return checks;
}

async function cpuTicks(pid) {
  try { const data = await readFile(`/proc/${pid}/stat`, 'utf8'); const fields = data.slice(data.lastIndexOf(')') + 2).split(' '); return Number(fields[11]) + Number(fields[12]); } catch { return null; }
}

const servers = [];
try {
  if (args.capture) {
    const server = await start(args.capture, 'capture');
    servers.push(server);
    const fixtures = await capture(server);
    await writeFile(path.join(args.output, 'fixtures.json'), JSON.stringify(fixtures, null, 2));
    console.log(JSON.stringify({ fixtures }, null, 2));
  } else if (args.verify) {
    const server = await start(args.verify, 'verify');
    servers.push(server);
    const checks = await verify(server);
    const report = { node: process.version, checks };
    await writeFile(path.join(args.output, 'verification.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } else {
    const before = await start(args.before, 'before'); servers.push(before);
    const after = await start(args.after, 'after'); servers.push(after);
    const checks = await verify(after);
    const beforeCases = await capture(before);
    const afterCases = await capture(after);
    const results = [];
    for (let round = 0; round < Number(args.rounds); ++round) {
      for (let index = 0; index < beforeCases.length; ++index) {
        for (const server of round % 2 === 0 ? [before, after] : [after, before]) {
          const test = (server === before ? beforeCases : afterCases)[index];
          for (let warm = 0; warm < 20; ++warm) await server.client.call(test.method, test.params, server.session);
          const cpu = await cpuTicks(server.child.pid);
          const start = performance.now();
          for (let iteration = 0; iteration < test.iterations; ++iteration) await server.client.call(test.method, test.params, server.session);
          results.push({ round, binary: server.name, case: test.name, iterations: test.iterations, elapsedMs: performance.now() - start, cpuTicks: cpu === null ? null : (await cpuTicks(server.child.pid)) - cpu });
        }
      }
    }
    const report = { node: process.version, checks, beforeCases, afterCases, results };
    await writeFile(path.join(args.output, 'results.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  }
} finally {
  for (const server of servers.reverse()) await server.stop();
}
