import { mkdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const output = new URL('../../docs/screenshots/', import.meta.url);
const webOutput = new URL('web/', output);
const siteOutput = new URL('site/', output);
await Promise.all([
  mkdir(webOutput, { recursive: true }),
  mkdir(siteOutput, { recursive: true }),
]);

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 960 }, deviceScaleFactor: 1 });

try {
  await page.goto('http://127.0.0.1:4173/', { waitUntil: 'networkidle' });
  await page.screenshot({
    path: fileURLToPath(new URL('01-initial-empty-state.png', webOutput)),
    fullPage: true,
  });

  await page.goto('http://127.0.0.1:4174/', { waitUntil: 'networkidle' });
  await page.screenshot({
    path: fileURLToPath(new URL('home.png', siteOutput)),
    fullPage: true,
  });
} finally {
  await browser.close();
}
