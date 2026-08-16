import assert from "node:assert/strict";
import { cp, chmod, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { platformDefinitionFor } from "../lib/platform.js";

const npmSourceRoot = fileURLToPath(new URL("..", import.meta.url));

async function createLauncherFixture(context, binarySource) {
  const fixtureRoot = await mkdtemp(path.join(os.tmpdir(), "moli-npm-launcher-"));
  context.after(() => rm(fixtureRoot, { recursive: true, force: true }));

  await cp(path.join(npmSourceRoot, "bin"), path.join(fixtureRoot, "bin"), {
    recursive: true,
  });
  await cp(path.join(npmSourceRoot, "lib"), path.join(fixtureRoot, "lib"), {
    recursive: true,
  });
  await cp(
    path.join(npmSourceRoot, "platforms.json"),
    path.join(fixtureRoot, "platforms.json"),
  );

  const definition = platformDefinitionFor(process.platform, process.arch);
  const fakeBinary = path.join(
    fixtureRoot,
    "vendor",
    definition.target,
    "bin",
    definition.binary,
  );
  await mkdir(path.dirname(fakeBinary), { recursive: true });
  await writeFile(fakeBinary, binarySource, "utf8");
  await chmod(fakeBinary, 0o755);
  return {
    fixtureRoot,
    launcher: path.join(fixtureRoot, "bin", "moli.js"),
  };
}

test(
  "launcher forwards arguments, stdio, and exit status",
  { skip: process.platform === "win32" },
  async (context) => {
    const { launcher } = await createLauncherFixture(
      context,
      [
        "#!/usr/bin/env node",
        "const result = {",
        "  args: process.argv.slice(2),",
        "};",
        "process.stdout.write(JSON.stringify(result));",
        "process.stderr.write('native stderr');",
        "process.exit(Number(process.env.MOLI_TEST_EXIT_CODE));",
        "",
      ].join("\n"),
    );

    const result = spawnSync(
      process.execPath,
      [launcher, "argument with spaces", "--flag"],
      {
        encoding: "utf8",
        env: { ...process.env, MOLI_TEST_EXIT_CODE: "23" },
      },
    );

    assert.equal(result.status, 23);
    assert.equal(result.stderr, "native stderr");
    assert.deepEqual(JSON.parse(result.stdout), {
      args: ["argument with spaces", "--flag"],
    });
  },
);

test(
  "launcher forwards termination signals and exits with the same signal",
  { skip: process.platform === "win32" },
  async (context) => {
    const { launcher } = await createLauncherFixture(
      context,
      [
        "#!/usr/bin/env node",
        "process.stdout.write('ready\\n');",
        "setInterval(() => {}, 1000);",
        "",
      ].join("\n"),
    );
    const child = spawn(process.execPath, [launcher], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    context.after(() => {
      if (child.exitCode === null && child.signalCode === null) {
        child.kill("SIGKILL");
      }
    });

    await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.stdout.once("data", (chunk) => {
        assert.equal(chunk.toString(), "ready\n");
        resolve();
      });
    });
    const exited = new Promise((resolve) => {
      child.once("exit", (...result) => resolve(result));
    });
    child.kill("SIGTERM");
    const [exitCode, signal] = await exited;

    assert.equal(exitCode, null);
    assert.equal(signal, "SIGTERM");
  },
);

test("launcher reports a missing optional dependency clearly", () => {
  const result = spawnSync(
    process.execPath,
    [path.join(npmSourceRoot, "bin", "moli.js"), "--version"],
    { encoding: "utf8" },
  );

  assert.equal(result.status, 1);
  const definition = platformDefinitionFor(process.platform, process.arch);
  assert.match(result.stderr, new RegExp(`Missing optional dependency ${definition.package}`));
  assert.match(result.stderr, /without omitting optional dependencies/);
});
