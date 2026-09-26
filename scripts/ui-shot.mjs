#!/usr/bin/env node
/**
 * Screenshot the browser preview of the main window (src/preview), clamped so
 * no edge of the saved image ever exceeds --max (default 1400px).
 *
 * Dev-only. Exists because screenshots are reviewed by an AI model whose API
 * rejects any image with an edge over 2000px once a request carries many
 * images — and a rejected image poisons the rest of the session. So the clamp
 * is not optional and happens before the file is written: the capture is
 * downscaled in-browser (canvas, high-quality smoothing) and only the clamped
 * PNG ever touches disk. Tall pages are captured as a sequence of viewport
 * tiles instead of one full-page image.
 *
 * Needs a Vite dev server serving this repo and `playwright-core` driving the
 * installed Chrome (or Edge with --browser msedge). Start the server detached
 * (never in the foreground, and never with Start-Process -Redirect*, which
 * hangs the calling shell), e.g. with ~/.kiro/scripts/start-detached.ps1 on
 * port 1430, then pass --base http://127.0.0.1:1430. The default base is the
 * port `tauri dev` uses, 1420.
 *
 *   node scripts/ui-shot.mjs --page home --out shots/home.png
 *   node scripts/ui-shot.mjs --page models --tab cleanup --tiles 3 --out shots/cleanup.png
 *   node scripts/ui-shot.mjs --page assistant --click "text=Memory" --out shots/memory.png
 *   node scripts/ui-shot.mjs --url "http://localhost:1420/src/preview/main.html?page=home&theme=dark" --out shots/dark.png
 *
 * Flags:
 *   --page <id> --tab <slot> --settings <tab> --theme light|dark --query "a=b&c=d"
 *   --url <full url>            overrides the page/tab/theme knobs
 *   --width 1280 --height 860   viewport in CSS px (device scale factor is 1)
 *   --click <selector>          repeatable; clicked in order before the capture
 *   --hover <selector>          hovered after the clicks
 *   --scroll <px>               scroll the page layer before the capture
 *   --tiles <n>                 capture n viewport-high tiles down the page
 *   --wait <ms>                 extra settle time after load/clicks (default 450)
 *   --max <px>                  long-edge clamp (default 1400, hard cap 1900)
 *   --timeout <s>               give up (exit 3) after this long (default 90)
 *   --base <url>                dev server origin (or env UI_SHOT_BASE)
 *   --out <path>                output file (tiles get -1, -2 … suffixes)
 */
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, extname, resolve } from "node:path";

const HARD_CAP = 1900;

const parseArgs = (argv) => {
  const args = { click: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const key = argv[i];
    if (!key.startsWith("--")) continue;
    const name = key.slice(2);
    const next = argv[i + 1];
    const value = next === undefined || next.startsWith("--") ? "true" : next;
    if (value !== "true") i += 1;
    if (name === "click") args.click.push(value);
    else args[name] = value;
  }
  return args;
};

const args = parseArgs(process.argv.slice(2));

// Hard ceiling on the whole run. A hung browser or a page that never settles
// must end this process rather than stall whatever launched it. Unref'd, so it
// never keeps a finished run alive, but it still fires if the run is stuck.
const watchdog = setTimeout(
  () => {
    console.log(`WATCHDOG\tgave up after ${args.timeout ?? 90}s`);
    process.exit(3);
  },
  Number(args.timeout ?? 90) * 1000,
);
watchdog.unref();

const maxEdge = Math.min(Number(args.max ?? 1400), HARD_CAP);
const width = Number(args.width ?? 1280);
const height = Number(args.height ?? 860);
const wait = Number(args.wait ?? 450);
const tiles = Math.max(1, Number(args.tiles ?? 1));
const out = resolve(args.out ?? "shots/shot.png");

const buildUrl = () => {
  if (args.url) return args.url;
  const params = new URLSearchParams();
  for (const knob of [
    "page",
    "tab",
    "settings",
    "theme",
    "stt",
    "fresh",
    "from",
  ]) {
    if (args[knob]) params.set(knob, args[knob]);
  }
  if (args.query) {
    for (const [k, v] of new URLSearchParams(args.query)) params.set(k, v);
  }
  const base = args.base ?? process.env.UI_SHOT_BASE ?? "http://localhost:1420";
  return `${base}/src/preview/main.html?${params.toString()}`;
};

let chromium;
try {
  ({ chromium } = await import("playwright-core"));
} catch {
  console.error(
    "playwright-core is not installed. Run `bun add -d playwright-core@1.58.0` (dev-only) and retry.",
  );
  process.exit(2);
}

/** Downscale a PNG (base64) in the page so its long edge is <= maxEdge. */
const clampInBrowser = async (page, base64) =>
  page.evaluate(
    async ({ data, limit }) => {
      const img = new Image();
      img.src = `data:image/png;base64,${data}`;
      await img.decode();
      const long = Math.max(img.naturalWidth, img.naturalHeight);
      if (long <= limit) {
        return {
          data,
          w: img.naturalWidth,
          h: img.naturalHeight,
          scaled: false,
        };
      }
      const scale = limit / long;
      const w = Math.max(1, Math.round(img.naturalWidth * scale));
      const h = Math.max(1, Math.round(img.naturalHeight * scale));
      const canvas = document.createElement("canvas");
      canvas.width = w;
      canvas.height = h;
      const ctx = canvas.getContext("2d");
      ctx.imageSmoothingEnabled = true;
      ctx.imageSmoothingQuality = "high";
      ctx.drawImage(img, 0, 0, w, h);
      const url = canvas.toDataURL("image/png");
      return { data: url.slice(url.indexOf(",") + 1), w, h, scaled: true };
    },
    { data: base64, limit: maxEdge },
  );

const tilePath = (index) => {
  if (tiles === 1) return out;
  const ext = extname(out) || ".png";
  return `${out.slice(0, out.length - ext.length)}-${index + 1}${ext}`;
};

const browser = await chromium.launch({
  channel: args.browser ?? "chrome",
  headless: true,
});
try {
  const context = await browser.newContext({
    viewport: { width, height },
    deviceScaleFactor: 1,
    reducedMotion: args.motion === "true" ? "no-preference" : "reduce",
  });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });

  await page.goto(buildUrl(), { waitUntil: "networkidle", timeout: 30000 });
  await page.waitForTimeout(wait);

  for (const selector of args.click) {
    await page.locator(selector).first().click({ timeout: 5000 });
    await page.waitForTimeout(wait);
  }
  if (args.hover) {
    await page.locator(args.hover).first().hover({ timeout: 5000 });
    await page.waitForTimeout(wait);
  }

  // Pages scroll inside their own layer, not the document.
  const scrollLayer = async (top) =>
    page.evaluate((y) => {
      const layer =
        document.querySelector(".page-layer-active") ??
        document.querySelector("[data-page]:not([hidden])");
      if (layer) layer.scrollTo({ top: y, behavior: "instant" });
      return layer ? layer.scrollHeight - layer.clientHeight : 0;
    }, top);

  if (args.scroll) {
    await scrollLayer(Number(args.scroll));
    await page.waitForTimeout(200);
  }

  mkdirSync(dirname(out), { recursive: true });
  const start = Number(args.scroll ?? 0);
  for (let index = 0; index < tiles; index += 1) {
    if (tiles > 1) {
      const maxScroll = await scrollLayer(start + index * (height - 120));
      if (
        index > 0 &&
        start + index * (height - 120) > maxScroll + height - 120
      ) {
        break;
      }
      await page.waitForTimeout(220);
    }
    const raw = (await page.screenshot({ type: "png" })).toString("base64");
    const clamped = await clampInBrowser(page, raw);
    if (Math.max(clamped.w, clamped.h) > maxEdge) {
      throw new Error(`clamp failed: ${clamped.w}x${clamped.h}`);
    }
    const file = tilePath(index);
    writeFileSync(file, Buffer.from(clamped.data, "base64"));
    console.log(
      `${clamped.scaled ? "SHRUNK" : "OK"}\t${file}\t${clamped.w}x${clamped.h}`,
    );
  }

  if (errors.length > 0) {
    console.log(`PAGE_ERRORS\t${errors.length}`);
    for (const error of errors.slice(0, 8))
      console.log(`  ${error.slice(0, 300)}`);
  }
} finally {
  await browser.close();
}
