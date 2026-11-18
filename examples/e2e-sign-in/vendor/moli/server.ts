/** Owns the Moli process behind one browser lease. */

import { spawn, type ChildProcess } from 'node:child_process';
import { createServer } from 'node:net';
import { setTimeout as sleep } from 'node:timers/promises';

/** Settings supplied when the provider acquires a worker's browser. */
export interface MoliServeOptions {
  readonly binary: string;
  readonly env: Readonly<Record<string, string | undefined>>;
  readonly layout: boolean;
  readonly loadResources: boolean;
  readonly args: readonly string[];
  readonly startupTimeoutMs: number;
  readonly signal: AbortSignal;
}

/** The CDP endpoint and the process that serves it. */
export interface MoliServer {
  readonly cdpEndpoint: string;
  /** Moli-Version from /json/version, or unknown when the build omits it. */
  readonly version: string;
  /** Resolves after SIGTERM, or SIGKILL when the grace period expires. */
  stop(graceMs: number): Promise<void>;
}

const HOST = '127.0.0.1';
const POLL_MS = 50;
const PROBE_TIMEOUT_MS = 500;
const STDERR_CAP = 64 * 1024;

/** Starts a loopback CDP server, ending the process on a failed or cancelled startup. */
export async function serveMoli(options: MoliServeOptions): Promise<MoliServer> {
  throwIfCancelled(options.signal);
  const port = await availableMoliPort();
  throwIfCancelled(options.signal);
  const args = [
    'serve', '--host', HOST, '--port', String(port),
    ...(options.layout ? ['--layout'] : []),
    ...(options.loadResources ? ['--resource'] : []),
    ...options.args,
  ];
  const child = spawn(options.binary, args, {
    env: options.env,
    detached: process.platform !== 'win32',
    stdio: ['ignore', 'ignore', 'pipe'],
  });
  const stderr = collectMoliStderr(child);
  const exited = new Promise<never>((_, reject) => {
    child.once('error', (cause) => reject(new Error(`could not start ${options.binary}: ${cause.message}`)));
    child.once('exit', (code, signal) => {
      const details = stderr().trim();
      reject(new Error(`${options.binary} exited with ${signal ?? `code ${code}`} before its CDP server answered${details === '' ? '' : `:\n${details}`}`));
    });
  });
  const cdpEndpoint = `http://${HOST}:${port}`;
  const polling = new AbortController();
  const stop = (graceMs: number): Promise<void> => stopMoliProcess(child, graceMs);
  try {
    const version = await Promise.race([
      waitForMoliVersion(`${cdpEndpoint}/json/version`, options.startupTimeoutMs, AbortSignal.any([options.signal, polling.signal])),
      exited,
    ]);
    throwIfCancelled(options.signal);
    return { cdpEndpoint, version, stop };
  } catch (cause) {
    await stop(1_000);
    throw cause;
  } finally {
    polling.abort();
  }
}

/** Keeps a bounded prefix of stderr for startup errors. */
function collectMoliStderr(child: ChildProcess): () => string {
  const chunks: Buffer[] = [];
  let size = 0;
  child.stderr?.on('data', (chunk: Buffer) => {
    if (size >= STDERR_CAP) return;
    const kept = chunk.subarray(0, STDERR_CAP - size);
    chunks.push(kept);
    size += kept.length;
  });
  return () => Buffer.concat(chunks).toString('utf8');
}

/** Polls the actual CDP discovery route, rather than treating an open TCP port as ready. */
async function waitForMoliVersion(url: string, timeoutMs: number, signal: AbortSignal): Promise<string> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    throwIfCancelled(signal);
    const remaining = deadline - Date.now();
    if (remaining <= 0) throw new Error(`the Moli CDP server did not answer on ${url} within ${timeoutMs}ms`);
    try {
      const response = await fetch(url, { signal: AbortSignal.any([signal, AbortSignal.timeout(Math.min(PROBE_TIMEOUT_MS, remaining))]) });
      if (response.ok) {
        const body: unknown = await response.json();
        if (typeof body === 'object' && body !== null && 'webSocketDebuggerUrl' in body && typeof body.webSocketDebuggerUrl === 'string' && body.webSocketDebuggerUrl !== '') {
          return 'Moli-Version' in body && typeof body['Moli-Version'] === 'string' ? body['Moli-Version'] : 'unknown';
        }
      }
    } catch {
      // A listening process can still be starting its CDP handler.
    }
    await new Promise((resolve) => setTimeout(resolve, Math.min(POLL_MS, Math.max(0, deadline - Date.now()))));
  }
}

/** Names cancellation consistently, including a request cancelled before spawn. */
function throwIfCancelled(signal: AbortSignal): void {
  if (signal.aborted) throw new Error('cancelled before the Moli CDP server answered');
}

/** Signals the process group, escalating even when only a wrapper's descendants remain. */
async function stopMoliProcess(child: ChildProcess, graceMs: number): Promise<void> {
  if (child.pid === undefined) return;
  const exited = child.exitCode !== null || child.signalCode !== null
    ? Promise.resolve()
    : new Promise<void>((resolve) => child.once('exit', () => resolve()));
  signalMoliProcess(child, 'SIGTERM');
  const deadline = Date.now() + graceMs;
  while (moliProcessGroupRunning(child)) {
    const remaining = deadline - Date.now();
    if (remaining <= 0) {
      signalMoliProcess(child, 'SIGKILL');
      break;
    }
    await sleep(Math.min(POLL_MS, remaining));
  }
  await exited;
}

/** A POSIX group can outlive its leader; Windows tracks only the child. */
function moliProcessGroupRunning(child: ChildProcess): boolean {
  if (process.platform === 'win32') return child.exitCode === null && child.signalCode === null;
  if (child.pid === undefined) return false;
  try {
    process.kill(-child.pid, 0);
    return true;
  } catch {
    return false;
  }
}

/** Reaches a wrapper's child processes too on platforms with process groups. */
function signalMoliProcess(child: ChildProcess, signal: NodeJS.Signals): void {
  if (child.pid === undefined) return;
  try {
    if (process.platform === 'win32') child.kill(signal);
    else process.kill(-child.pid, signal);
  } catch {
    // The process may have exited between the check and the signal.
  }
}

/** Reserves an available loopback port long enough to learn its number. */
function availableMoliPort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const probe = createServer();
    probe.once('error', reject);
    probe.listen(0, HOST, () => {
      const address = probe.address();
      const port = typeof address === 'object' && address !== null ? address.port : undefined;
      probe.close(() => (port === undefined ? reject(new Error('no free port for Moli')) : resolve(port)));
    });
  });
}
