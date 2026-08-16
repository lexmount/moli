import assert from "node:assert/strict";
import test from "node:test";

import {
  PLATFORM_DEFINITIONS,
  assertSupportedLibc,
  platformDefinitionFor,
} from "../lib/platform.js";

test("maps every supported Node platform to one native target", () => {
  const expected = new Map([
    ["linux:x64", "x86_64-unknown-linux-gnu"],
    ["darwin:x64", "x86_64-apple-darwin"],
    ["darwin:arm64", "aarch64-apple-darwin"],
    ["win32:x64", "x86_64-pc-windows-msvc"],
  ]);

  assert.equal(PLATFORM_DEFINITIONS.length, expected.size);
  for (const [key, target] of expected) {
    const [platform, arch] = key.split(":");
    assert.equal(platformDefinitionFor(platform, arch).target, target);
  }
});

test("keeps package aliases and release assets unique", () => {
  for (const field of ["id", "target", "package", "archive"]) {
    const values = PLATFORM_DEFINITIONS.map((definition) => definition[field]);
    assert.equal(new Set(values).size, values.length, `${field} must be unique`);
  }
});

test("rejects unsupported platforms without falling back to a wrong binary", () => {
  assert.throws(
    () => platformDefinitionFor("linux", "arm64"),
    /Unsupported platform: linux \(arm64\)/,
  );
  assert.throws(
    () => platformDefinitionFor("freebsd", "x64"),
    /Unsupported platform: freebsd \(x64\)/,
  );
});

test("rejects musl before attempting to run the glibc Linux package", () => {
  const linux = platformDefinitionFor("linux", "x64");
  assert.doesNotThrow(() => assertSupportedLibc(linux, "2.36"));
  assert.throws(
    () => assertSupportedLibc(linux, undefined),
    /this Moli package requires glibc/,
  );

  const darwin = platformDefinitionFor("darwin", "arm64");
  assert.doesNotThrow(() => assertSupportedLibc(darwin, undefined));
});
