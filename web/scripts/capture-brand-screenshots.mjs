import { mkdir } from 'node:fs/promises';
import { chromium } from 'playwright';

const output = new URL('../../docs/screenshots/', import.meta.url);
await mkdir(output, { recursive: true });

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 960 }, deviceScaleFactor: 1 });

try {
  await page.goto('http://127.0.0.1:4173/', { waitUntil: 'networkidle' });
  await page.screenshot({ path: new URL('web/01-initial-empty-state.png', output), fullPage: true });

  await page.goto('http://127.0.0.1:4174/', { waitUntil: 'networkidle' });
  await page.screenshot({ path: new URL('site/home.png', output), fullPage: true });
} finally {
  await browser.close();
}
