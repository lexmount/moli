// Kept callable independently so the redirect contract can also be checked
// against Chromium with the same Playwright client and HTTP fixture.
export async function checkFulfilledRedirectRouting(page, fixture) {
  const assertEqual = (actual, expected, label) => {
    if (actual !== expected) throw new Error(`${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  };
  // Playwright intentionally continues redirected requests instead of
  // invoking page.route again, even though CDP pauses every redirect hop.
  // Keep this in sync with Chromium and the page.route API contract.
  let redirectSourceCalls = 0;
  let redirectDestinationCalls = 0;
  await page.route('**/route-redirect-source', route => {
    redirectSourceCalls++;
    return route.fulfill({ status: 302, headers: { location: '/route-redirect-destination' } });
  });
  await page.route('**/route-redirect-destination', route => {
    redirectDestinationCalls++;
    return route.fulfill({ status: 200, body: 'unexpected second route handler' });
  });
  const redirectedFetch = await page.evaluate(async () => {
    const response = await fetch('/route-redirect-source');
    return { status: response.status, url: response.url, redirected: response.redirected, body: await response.text() };
  });
  assertEqual(redirectSourceCalls, 1, 'fulfilled redirect source route count');
  assertEqual(redirectDestinationCalls, 0, 'redirect destination route follows Playwright contract');
  assertEqual(redirectedFetch.status, 404, 'redirect destination backend status');
  assertEqual(redirectedFetch.body, 'redirect destination backend', 'redirect destination backend body');
  assertEqual(redirectedFetch.url, `${fixture}/route-redirect-destination`, 'redirect final URL');
  assertEqual(redirectedFetch.redirected, true, 'redirect response flag');
  await page.unroute('**/route-redirect-source');
  await page.unroute('**/route-redirect-destination');
}
