const assert = require('node:assert/strict');
const fs = require('node:fs');
const https = require('node:https');
const path = require('node:path');

(async () => {
  const [modulePath, origin, caPath] = process.argv.slice(2);
  const { chromium } = require(modulePath);
  const ca = fs.readFileSync(caPath);
  const browser = await chromium.launch({ headless: true, executablePath: process.env.PARCELA_CHROMIUM_EXECUTABLE || undefined });
  try {
    const context = await browser.newContext({ viewport: { width: 1360, height: 900 } });
    // Verify every real Caddy response with the disposable CA, without modifying system trust.
    let failStatus = false;
    await context.route('https://*.localhost:*/**', async route => {
      const request = route.request();
      if (failStatus && new URL(request.url()).pathname === '/api/status') {
        await route.fulfill({ status: 503, body: 'Unavailable' });
        return;
      }
      const response = await new Promise((resolve, reject) => {
        const proxy = https.request(request.url(), {
          method: request.method(), ca,
          lookup: (_hostname, options, callback) => options.all ? callback(null, [{ address: '127.0.0.1', family: 4 }]) : callback(null, '127.0.0.1', 4),
          headers: request.headers(),
        }, response => {
          const chunks = [];
          response.on('data', chunk => chunks.push(chunk));
          response.on('end', () => resolve({ status: response.statusCode, headers: response.headers, body: Buffer.concat(chunks) }));
        });
        proxy.on('error', reject);
        proxy.end(request.postDataBuffer());
      });
      delete response.headers['transfer-encoding'];
      delete response.headers.connection;
      await route.fulfill(response);
    });
    const page = await context.newPage();
    const errors = [];
    const artifacts = process.env.PARCELA_DASHBOARD_ARTIFACTS;
    if (artifacts) fs.mkdirSync(artifacts, { recursive: true });
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(origin);
    await page.getByText('2 ready', { exact: false }).waitFor();
    const rows = page.locator('[data-slot=list-row]');
    assert.equal(await rows.count(), 2);
    assert.equal(await page.locator('html').getAttribute('data-theme'), 'dark');
    assert.equal(await page.locator('iframe').count(), 0);
    await page.getByRole('searchbox').fill('review');
    assert.equal(await rows.count(), 1);
    await page.getByRole('searchbox').fill('no match');
    await page.getByText('No environments match.', { exact: true }).waitFor();
    await page.getByRole('searchbox').fill('');
    const review = rows.filter({ hasText: 'review' });
    assert.ok((await review.innerText()).includes('2 processes'));
    for (const name of ['Logs', 'Restart', 'Stop']) await review.getByRole(name === 'Logs' ? 'link' : 'button', { name, exact: true }).waitFor();
    const popupEvent = page.waitForEvent('popup');
    await review.getByRole('link', { name: 'Open web', exact: true }).click();
    const application = await popupEvent;
    await application.getByRole('heading', { name: 'Local application' }).waitFor();
    await application.locator('#parcela-switcher button.badge').click();
    await application.locator('#parcela-panel').waitFor();
    await application.close();
    if (artifacts) await page.screenshot({ path: path.join(artifacts, 'dashboard.png'), fullPage: true });
    await page.setViewportSize({ width: 390, height: 844 });
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    if (artifacts) await page.screenshot({ path: path.join(artifacts, 'dashboard-mobile.png'), fullPage: true });
    failStatus = true;
    await page.getByText('Could not reach Parcela. Retrying.', { exact: true }).waitFor();
    failStatus = false;
    await page.locator('#health').waitFor({ state: 'hidden' });
    const data = await page.evaluate(() => fetch('/api/status').then(response => response.json()));
    const webUrl = data.environments[0].services.find(service => service.name === 'web').url;
    await page.goto(`${webUrl}/strict`);
    assert.equal(await page.locator('#parcela-switcher').count(), 0);
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ browser: 'chromium', checks: ['filter', 'controls', 'direct_open', 'injected_badge', 'no_frames', 'phone_width', 'lost_connection', 'strict_csp'] }));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
