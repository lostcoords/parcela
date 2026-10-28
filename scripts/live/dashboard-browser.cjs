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
    assert.equal(await page.locator('.environment').count(), 2);
    await page.getByRole('searchbox').fill('review');
    assert.equal(await page.locator('.environment').count(), 1);
    await page.getByRole('searchbox').fill('no match');
    await page.getByText('No environments match your search.').waitFor();
    await page.getByRole('searchbox').fill('');
    const review = page.locator('.environment').filter({ has: page.locator('.name', { hasText: 'review' }) });
    await review.getByText('2 processes').click();
    assert.equal(await review.locator('td', { hasText: 'running' }).count(), 2);
    const web = review.locator('.service').filter({ has: page.locator('.service-name', { hasText: /^web$/ }) });
    await web.getByRole('link', { name: 'In workspace', exact: true }).click();
    const application = page.frameLocator('#application');
    await application.getByRole('heading', { name: 'Local application' }).waitFor();
    await application.locator('body[data-app-ready="yes"]').waitFor();
    await application.getByRole('link', { name: 'Relative navigation' }).click();
    await application.getByRole('heading', { name: 'Local application' }).waitFor();
    await application.getByRole('button', { name: 'Open Parcela environment switcher' }).click();
    const switcher = application.frameLocator('#parcela-panel');
    await switcher.getByText('You are here', { exact: true }).waitFor();
    if (artifacts) await page.screenshot({ path: path.join(artifacts, 'workspace-switcher.png'), fullPage: true });
    await switcher.getByRole('button', { name: 'Close Parcela' }).click();
    const tab = application.getByRole('button', { name: 'Open Parcela environment switcher' });
    assert.equal(await tab.getAttribute('aria-expanded'), 'false');
    assert.equal(await tab.evaluate(element => element.getRootNode().activeElement === element), true);
    await tab.click();
    await switcher.getByRole('searchbox').press('Escape');
    assert.equal(await tab.getAttribute('aria-expanded'), 'false');
    assert.equal(await tab.evaluate(element => element.getRootNode().activeElement === element), true);
    await application.getByRole('button', { name: 'Open Parcela environment switcher' }).waitFor();
    // The frame-blocked service still has a direct navigation escape hatch.
    await page.goto(origin);
    await page.getByText('2 ready', { exact: false }).waitFor();
    const blocked = page.locator('.environment').first().locator('.service').filter({ has: page.locator('.service-name', { hasText: /^blocked$/ }) });
    await blocked.getByRole('link', { name: 'In workspace', exact: true }).click();
    const popupEvent = page.waitForEvent('popup');
    await page.getByRole('link', { name: 'Open directly ↗', exact: true }).last().click();
    const directPage = await popupEvent;
    await directPage.getByRole('heading', { name: 'Local application' }).waitFor();
    await directPage.close();
    await page.goto(origin);
    await page.getByText('2 ready', { exact: false }).waitFor();
    if (artifacts) { await page.screenshot({ path: path.join(artifacts, 'dashboard.png'), fullPage: true }); }
    await page.setViewportSize({ width: 390, height: 844 });
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    if (artifacts) await page.screenshot({ path: path.join(artifacts, 'dashboard-mobile.png'), fullPage: true });
    failStatus = true;
    await page.getByText('Connection lost. Displayed status may be stale. Retrying…', { exact: true }).waitFor();
    assert.equal(await page.locator('#environments a[href]').count(), 0);
    failStatus = false;
    await page.getByText('2 ready', { exact: false }).waitFor();
    const data = await page.evaluate(() => fetch('/api/status').then(response => response.json()));
    const webUrl = data.environments[0].services.find(service => service.name === 'web').url;
    await page.goto(`${webUrl}/strict`);
    assert.equal(await page.locator('#parcela-switcher').count(), 0);
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ browser: 'chromium', checks: ['search', 'process_details', 'workspace_iframe', 'relative_navigation', 'optional_widget', 'current_environment', 'close_and_focus', 'keyboard_escape', 'strict_csp_preserved', 'stale_status_and_reconnect', 'blocked_frame_direct_fallback', 'mobile_layout'], artifacts: artifacts || null }));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
