// Keep the benchmark's updater traffic local; this does not test release discovery.
// Requests dispatched before CDP attachment remain outside this fixture.
export async function isolateBenchmarkUpdates(context, version) {
  const evidence = { mode: "disabled-preference-and-local-release-response", interceptedRequests: 0 };
  await context.route("https://api.github.com/repos/bildhaus/milim/releases/latest", async (route) => {
    evidence.interceptedRequests += 1;
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: { "Access-Control-Allow-Origin": "*" },
      body: JSON.stringify({ tag_name: `v${version}`, assets: [] }),
    });
  });
  function disableAutomaticUpdates() {
    if (!["http:", "https:", "tauri:"].includes(location.protocol)) return true;
    localStorage.setItem("milim.local.updates", JSON.stringify({
      state: { automaticCheck: false, automaticDownload: false }, version: 1,
    }));
    return true;
  }
  await context.addInitScript(disableAutomaticUpdates);
  for (const page of context.pages()) {
    await page.waitForFunction(disableAutomaticUpdates, null, { timeout: 5_000 });
  }
  return evidence;
}
