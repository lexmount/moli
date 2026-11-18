/** Finds Moli in the run's environment, without starting it at config load. */

import { accessSync, constants, statSync } from 'node:fs';
import path from 'node:path';

const INSTALL_URL = 'https://github.com/lexmount/moli#direct-installation';
const BINARY_NAME = process.platform === 'win32' ? 'moli.exe' : 'moli';

/** Resolves MOLI_PATH, then PATH, then ~/.local/bin; names the install guide when none exists. */
export function findMoliBinary(env: Readonly<Record<string, string | undefined>>): string {
  const named = env['MOLI_PATH']?.trim();
  if (named !== undefined && named !== '') return named;
  const directories = (env['PATH'] ?? env['Path'] ?? '').split(path.delimiter).filter((directory) => directory !== '');
  const home = env['HOME'] ?? env['USERPROFILE'];
  const candidates = [
    ...directories.map((directory) => path.join(directory, BINARY_NAME)),
    ...(home === undefined ? [] : [path.join(home, '.local', 'bin', BINARY_NAME)]),
  ];
  const found = candidates.find(executableFile);
  if (found === undefined) {
    throw new Error(`moli is not on PATH or in ~/.local/bin; install it (${INSTALL_URL}) or set MOLI_PATH`);
  }
  return found;
}

/** Skips directories and files the current process cannot execute. */
function executableFile(file: string): boolean {
  try {
    accessSync(file, constants.X_OK);
    return statSync(file).isFile();
  } catch {
    return false;
  }
}
