async function runNetworkBodyProbe(scenario, url) {
  const response = await fetch(url);
  const body = scenario === "response" ? new Response(response.body) :
    new Request(url, { method: "POST", body: response.body, duplex: "half" });
  const text = await body.text();
  return { text };
}
