// Capture real V8 deep serialization, rather than approximating the CDP schema.
// Usage: node scripts/capture-runtime-deep-responses.mjs OUTPUT_DIRECTORY
import { Session } from "node:inspector";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

const directory = process.argv[2];
if (!directory) throw new Error("expected output directory");
await mkdir(directory, { recursive: true });
const session = new Session();
session.connect();
const post = (method, params) => new Promise((resolve, reject) => {
  session.post(method, params, (error, result) => error ? reject(error) : resolve(result));
});
const cases = [];
try {
  for (const depth of [0, 10, 20, 40, 80]) {
    const expression = `(() => { let value = "x".repeat(1024 * 1024); for (let i = 0; i < ${depth}; i++) value = [value]; return value; })()`;
    const params = { expression, objectGroup: `deep-${depth}`, serializationOptions: { serialization: "deep", maxDepth: 100 } };
    const result = await post("Runtime.evaluate", params);
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    const wire = JSON.stringify({ id: depth + 1, result });
    const file = `deep-${depth}.json`;
    await writeFile(join(directory, file), wire);
    const ids = new Set();
    const stack = [result];
    while (stack.length) {
      const value = stack.pop();
      if (!value || typeof value !== "object") continue;
      for (const [key, child] of Object.entries(value)) {
        if (["objectId", "promiseObjectId", "errorObjectId"].includes(key) && typeof child === "string") ids.add(child);
        else if (typeof child === "object") stack.push(child);
      }
    }
    cases.push({ file, depth, bytes: Buffer.byteLength(wire), expected_ids: [...ids].sort(), params });
    await post("Runtime.releaseObjectGroup", { objectGroup: params.objectGroup });
  }
} finally {
  session.disconnect();
}
await writeFile(join(directory, "manifest.json"), JSON.stringify({ node: process.version, v8: process.versions.v8, cases }, null, 2) + "\n");
