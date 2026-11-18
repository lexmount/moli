/** Moli as a BrowserProvider: one local CDP server for each worker slot. */

import type { BrowserLease, BrowserProvider, BrowserRequest } from '@e2e-dev/web';
import { findMoliBinary } from './binary.ts';
import { serveMoli, type MoliServer } from './server.ts';

const STARTUP_TIMEOUT_MS = 10_000;
const STOP_GRACE_MS = 5_000;

/** The binary and serve flags used for each worker's Moli process. */
export interface MoliOptions {
  /** The executable to start; otherwise MOLI_PATH, moli on PATH, or ~/.local/bin/moli. */
  readonly binary?: string | undefined;
  /** Enable on-demand layout and screenshots with --layout; default true. */
  readonly layout?: boolean | undefined;
  /** Fetch optional image, font, audio, video, media, and text-track resources with --resource; default true. */
  readonly loadResources?: boolean | undefined;
  /** Further moli serve flags, after the host, port, layout, and resource flags. */
  readonly args?: readonly string[] | undefined;
}

/**
 * Moli browsers for web({ browser: moli() }): starts one server on a free
 * loopback port per worker slot and stops it when the lease is released.
 * The binary is resolved from the run's environment when acquired.
 */
export function moli(options: MoliOptions = {}): BrowserProvider {
  const { layout = true, loadResources = true, args = [] } = options;
  const servers = new WeakMap<BrowserLease, MoliServer>();
  return {
    name: 'moli',
    async acquire(request: BrowserRequest): Promise<BrowserLease> {
      const server = await serveMoli({
        binary: options.binary ?? findMoliBinary(request.env),
        env: request.env,
        layout,
        loadResources,
        args,
        startupTimeoutMs: STARTUP_TIMEOUT_MS,
        signal: request.signal,
      });
      request.log(`moli ${server.version} at ${server.cdpEndpoint}`);
      const lease: BrowserLease = { id: server.cdpEndpoint, cdpEndpoint: server.cdpEndpoint };
      servers.set(lease, server);
      return lease;
    },
    async release(lease: BrowserLease): Promise<void> {
      await servers.get(lease)?.stop(STOP_GRACE_MS);
      servers.delete(lease);
    },
  };
}
