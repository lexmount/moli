#!/usr/bin/env node

import { spawn } from "node:child_process";
import { existsSync, realpathSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  assertSupportedLibc,
  platformDefinitionFor,
} from "../lib/platform.js";

const require = createRequire(import.meta.url);
const packageRoot = realpathSync(
  path.join(path.dirname(fileURLToPath(import.meta.url)), ".."),
);

function executableFor(definition) {
  let vendorRoot;
  try {
    const packageJson = require.resolve(`${definition.package}/package.json`);
    vendorRoot = path.join(path.dirname(packageJson), "vendor");
  } catch {
    // This fallback makes source checkouts and assembled package smoke tests
    // possible without weakening the normal optional-dependency lookup.
    vendorRoot = path.join(packageRoot, "vendor");
  }

  const executable = path.join(
    vendorRoot,
    definition.target,
    "bin",
    definition.binary,
  );
  if (existsSync(executable)) {
    return executable;
  }

  throw new Error(
    `Missing optional dependency ${definition.package}. ` +
      "Reinstall @lexmount/moli without omitting optional dependencies.",
  );
}

async function run() {
  const definition = platformDefinitionFor(process.platform, process.arch);
  const runtimeReport = process.report?.getReport?.();
  assertSupportedLibc(definition, runtimeReport?.header?.glibcVersionRuntime);
  const child = spawn(executableFor(definition), process.argv.slice(2), {
    stdio: "inherit",
  });

  const forwardedSignals =
    process.platform === "win32"
      ? ["SIGINT", "SIGTERM"]
      : ["SIGINT", "SIGTERM", "SIGHUP"];
  const signalHandlers = new Map();
  for (const signal of forwardedSignals) {
    const handler = () => {
      if (!child.killed) {
        child.kill(signal);
      }
    };
    signalHandlers.set(signal, handler);
    process.on(signal, handler);
  }

  let result;
  try {
    result = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", (exitCode, signal) => {
        resolve(signal ? { signal } : { exitCode: exitCode ?? 1 });
      });
    });
  } finally {
    for (const [signal, handler] of signalHandlers) {
      process.off(signal, handler);
    }
  }

  if ("signal" in result) {
    process.kill(process.pid, result.signal);
    return;
  }
  process.exitCode = result.exitCode;
}

try {
  await run();
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  console.error(`moli: ${message}`);
  process.exitCode = 1;
}
