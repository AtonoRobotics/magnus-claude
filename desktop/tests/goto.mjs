// Opening a page, again when the machine's network changed under the load.
// Chromium aborts a navigation in flight with net::ERR_NETWORK_CHANGED when
// the machine's addresses change (anything on a shared build box bringing an
// interface up or down is enough); the page was never answered, and Chromium
// leaves it to the caller to try again. Release run 42032928589 (#171) failed
// two console tests at their first page this way. Any other failure is the
// test's to report at once, and a network that never settles fails after
// `tries` loads.
export async function goto(page, url, tries = 3) {
  for (let i = 1; ; i++) {
    try {
      return await page.goto(url);
    } catch (e) {
      if (i >= tries || !String(e?.message ?? e).includes("net::ERR_NETWORK_CHANGED")) throw e;
    }
  }
}
