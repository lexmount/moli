import { readFileSync } from "node:fs";

const definitions = JSON.parse(
  readFileSync(new URL("../platforms.json", import.meta.url), "utf8"),
);

export const PLATFORM_DEFINITIONS = Object.freeze(
  definitions.map((definition) => Object.freeze(definition)),
);

export function platformDefinitionFor(platform, arch) {
  const definition = PLATFORM_DEFINITIONS.find(
    (candidate) =>
      candidate.platform === platform && candidate.arch === arch,
  );
  if (!definition) {
    throw new Error(`Unsupported platform: ${platform} (${arch})`);
  }
  return definition;
}

export function assertSupportedLibc(definition, glibcVersionRuntime) {
  if (
    definition.libc?.includes("glibc") &&
    (typeof glibcVersionRuntime !== "string" || glibcVersionRuntime.length === 0)
  ) {
    throw new Error(
      `Unsupported libc for ${definition.platform} (${definition.arch}): ` +
        "this Moli package requires glibc",
    );
  }
}
