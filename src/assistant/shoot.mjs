// THROWAWAY screenshot harness for the collapsed assistant pill.
// Delete after design iteration.
import { chromium } from "playwright-core";

const BASE = "http://localhost:1420/src/assistant/preview.html";

const browser = await chromium.launch();
const page = await browser.newPage({ deviceScaleFactor: 2 });

for (const theme of ["dark", "light"]) {
  await page.goto(`${BASE}?theme=${theme}`, { waitUntil: "networkidle" });
  // Let the waveform animations settle into a representative frame.
  await page.waitForTimeout(1200);
  await page.screenshot({ path: `src/assistant/_pill_${theme}.png`, fullPage: true });
  console.log(`shot ${theme}`);
}

await browser.close();
console.log("done");
