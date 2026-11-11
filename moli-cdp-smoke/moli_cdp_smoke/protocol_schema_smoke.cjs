"use strict";

const assert = require("node:assert/strict");
const CDP = require("chrome-remote-interface");

async function main() {
  const endpoint = new URL(process.argv[2]);
  const fixture = process.argv[3];
  assert(fixture, "usage: protocol_schema_smoke.cjs ENDPOINT FIXTURE");
  const options = {host: endpoint.hostname, port: Number(endpoint.port)};
  const version = await CDP.Version(options);
  // No local schema or protocol override: CRI must fetch /json/protocol and
  // generate all command functions and event subscriptions from that response.
  const client = await CDP({...options, target: version.webSocketDebuggerUrl});
  let targetId;
  const unsubscribe = [];
  try {
    assert.equal(client.Page.navigate.parameters.url.type, "string");
    assert(client.Page.navigate.returns.some((field) => field.name === "frameId"));
    assert.equal(client.Page.loadEventFired.parameters.timestamp.$ref, "Network.MonotonicTime");
    assert.equal(client.Network.requestWillBeSent.parameters.request.$ref, "Request");
    assert.equal(client.Target.attachedToTarget.parameters.sessionId.$ref, "SessionID");
    assert.equal(typeof client.Browser.close, "function");

    ({targetId} = await client.Target.createTarget({url: "about:blank"}));
    const attached = new Promise((resolve) => {
      unsubscribe.push(client.Target.attachedToTarget(resolve));
    });
    const {sessionId} = await client.Target.attachToTarget({targetId, flatten: true});
    await client.Page.enable({}, sessionId);
    await client.Network.enable({}, sessionId);
    assert.equal((await attached).targetInfo.targetId, targetId);

    const url = `${fixture}/plain?client=protocol-schema`;
    const requests = [];
    unsubscribe.push(client.Network.requestWillBeSent(sessionId, (event) => {
      requests.push(event);
    }));
    const load = new Promise((resolve) => {
      unsubscribe.push(client.Page.loadEventFired(sessionId, resolve));
    });
    const navigation = await client.Page.navigate({url}, sessionId);
    assert.equal(navigation.errorText, undefined);
    assert.equal(typeof navigation.frameId, "string");
    let timeout;
    try {
      const event = await Promise.race([
        load,
        new Promise((_, reject) => {
          timeout = setTimeout(() => reject(new Error("timed out waiting for load event")), 7000);
        }),
      ]);
      assert.equal(typeof event.timestamp, "number");
    } finally {
      clearTimeout(timeout);
    }
    assert(requests.some((event) => event.request.url === url && event.type === "Document"));
    const {result, exceptionDetails} = await client.Runtime.evaluate({
      expression: "location.href", returnByValue: true,
    }, sessionId);
    assert.equal(exceptionDetails, undefined);
    assert.equal(result.value, url);
    process.stdout.write(JSON.stringify({ok: true, results: [{
      name: "protocol_schema_generated_client_navigation", ok: true,
      client: "chrome-remote-interface", schema: "/json/protocol",
      events: ["Target.attachedToTarget", "Network.requestWillBeSent", "Page.loadEventFired"],
    }]}));
  } finally {
    for (const remove of unsubscribe) remove();
    if (targetId) await client.Target.closeTarget({targetId});
    await client.close();
  }
}

main().catch((error) => {
  console.error(error.stack || String(error));
  process.exitCode = 1;
});
