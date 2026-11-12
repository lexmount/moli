"use strict";

const assert = require("node:assert/strict");
const CDP = require("chrome-remote-interface");

async function verifyTracing(client) {
  assert.equal(typeof client.Tracing.dataCollected, "function");
  assert.equal(typeof client.Tracing.tracingComplete, "function");
  const results = [];
  for (const transferMode of ["ReportEvents", "ReturnAsStream"]) {
    const traceEvents = [];
    const removeData = client.Tracing.dataCollected(({value}) => traceEvents.push(...value));
    try {
      const completed = client.Tracing.tracingComplete();
      await client.Tracing.start({categories: "__metadata", transferMode});
      const syncId = `protocol-schema-${transferMode}`;
      await client.Tracing.recordClockSyncMarker({syncId});
      await client.Tracing.end();
      const complete = await completed;
      assert.equal(complete.dataLossOccurred, false);
      if (transferMode === "ReturnAsStream") {
        assert.equal(traceEvents.length, 0);
        assert.equal(typeof complete.stream, "string");
        const chunks = [];
        try {
          let eof = false;
          while (!eof) {
            const part = await client.IO.read({handle: complete.stream});
            chunks.push(Buffer.from(part.data, part.base64Encoded ? "base64" : "utf8"));
            eof = part.eof;
          }
        } finally {
          await client.IO.close({handle: complete.stream});
        }
        traceEvents.push(...JSON.parse(Buffer.concat(chunks).toString("utf8")).traceEvents);
      } else {
        assert.equal(complete.stream, undefined);
      }
      assert(traceEvents.some((event) => event.name === "clock_sync" && event.args?.sync_id === syncId));
      results.push({
        name: `protocol_schema_generated_client_tracing_${transferMode}`, ok: true,
        transferMode, traceEventCount: traceEvents.length,
      });
    } finally {
      removeData();
    }
  }
  return results;
}

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
    const tracingResults = await verifyTracing(client);
    process.stdout.write(JSON.stringify({ok: true, results: [{
      name: "protocol_schema_generated_client_navigation", ok: true,
      client: "chrome-remote-interface", schema: "/json/protocol",
      events: ["Target.attachedToTarget", "Network.requestWillBeSent", "Page.loadEventFired"],
    }, ...tracingResults]}));
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
