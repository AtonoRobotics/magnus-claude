// Opening a page, again when the machine's network changed under the load.
// Chromium aborts a navigation in flight with net::ERR_NETWORK_CHANGED when
// the machine's addresses change (anything on a shared build box bringing an
// interface up or down is enough); the page was never answered, and Chromium
// leaves it to the caller to try again. Release run 42032928589 (#171) failed
// two console tests at their first page this way.
//
// Playwright does not always name that error: when Chromium answers the
// abort with its own error page, the load is reported as "interrupted by
// another navigation to chrome-error://chromewebdata/" (#218, runs
// 42139753250 and 42146521203). So the page's own request is watched, and
// its failure is what decides: net::ERR_NETWORK_CHANGED, however the load
// was reported, is loaded again. Any other failure is the test's to report
// at once, naming the request's own error, and a network that never settles
// fails after `tries` loads.
export async function goto(page, url, tries = 3) {
  for (let i = 1; ; i++) {
    let failed = "";
    const watch = r => {
      try {
        if (r.isNavigationRequest() && r.frame() === page.mainFrame()) failed = r.failure()?.errorText ?? "";
      } catch {}
    };
    page.on?.("requestfailed", watch);
    try {
      return await page.goto(url);
    } catch (e) {
      const said = String(e?.message ?? e);
      const changed = said.includes("net::ERR_NETWORK_CHANGED") || failed.includes("net::ERR_NETWORK_CHANGED");
      if (i < tries && changed) continue;
      if (failed && !said.includes(failed) && e instanceof Error) e.message += ` (the page's request failed: ${failed})`;
      throw e;
    } finally {
      page.off?.("requestfailed", watch);
    }
  }
}
