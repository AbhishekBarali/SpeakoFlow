#!/usr/bin/env node
/**
 * Record the README's animated demo (src/preview/readme.html) frame by frame
 * and encode it as an animated GIF or WebP.
 *
 * Dev-only. A screen recording of the preview would drop frames whenever the
 * capture fell behind, so this drives time itself: Playwright's fake clock
 * owns setTimeout, setInterval, requestAnimationFrame and performance.now,
 * and every CSS animation and transition is paused and stepped through the
 * Web Animations API. Each frame is then an exact moment, however long its
 * screenshot took, and a re-run produces the same file. Frames keep their
 * alpha, so the demo's rounded corners stay transparent on either GitHub
 * theme.
 *
 * Needs a Vite dev server for this repo (start it detached, see ui-shot.mjs),
 * `playwright-core` driving the installed Chrome, Python with Pillow for WebP
 * (scripts/readme-webp.py), and ffmpeg 5 or newer for GIF.
 *
 * The README's two clips:
 *   node scripts/readme-media.mjs --dpr 2 --out assets/readme/demo.webp
 *   node scripts/readme-media.mjs --dpr 2 --query "scenes=ask&asks=3" --out assets/readme/ask.webp
 *
 * Other uses:
 *   node scripts/readme-media.mjs --still 4200 --out frame.png
 *   node scripts/readme-media.mjs --frames tmp/f --dpr 2        (record, keep PNGs)
 *   node scripts/readme-media.mjs --from-frames tmp/f --scale 1280 --out a.webp
 *
 * Flags:
 *   --base <url>         dev server origin (default http://127.0.0.1:1420)
 *   --query "a=b"        readme.html knobs (scenes, asks, chrome, theme)
 *   --width 960 --height 600   viewport in CSS px
 *   --dpr 1              device scale factor of the capture
 *   --fps 25             frames per second
 *   --duration <ms>      length to record (default: the demo's full loop)
 *   --still <ms>         write one PNG at that time instead of an animation
 *   --frames <dir>       keep the recorded PNGs here (default: a temp dir)
 *   --from-frames <dir>  skip recording and encode PNGs recorded earlier
 *   --scale <px>         output width (keeps the aspect ratio)
 *   --colors 256         GIF palette size
 *   --quality 90         WebP effort for lossless frames (higher is smaller)
 *   --ffmpeg <path>      ffmpeg binary (or env FFMPEG; default "ffmpeg")
 *   --python <path>      Python binary (or env PYTHON; default "python")
 *   --out <path>[,<path>]  .gif / .webp outputs, or a .png with --still
 */
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const args = {};
const argv = process.argv.slice(2);
for (let i = 0; i < argv.length; i += 1) {
  const key = argv[i];
  if (!key.startsWith("--")) continue;
  const next = argv[i + 1];
  const value = next === undefined || next.startsWith("--") ? "true" : next;
  if (value !== "true") i += 1;
  args[key.slice(2)] = value;
}

const base = args.base ?? process.env.UI_SHOT_BASE ?? "http://127.0.0.1:1420";
const width = Number(args.width ?? 960);
const height = Number(args.height ?? 600);
const dpr = Number(args.dpr ?? 1);
const fps = Number(args.fps ?? 25);
const step = 1000 / fps;
const outs = (args.out ?? "assets/readme/demo.gif")
  .split(",")
  .map((path) => resolve(path.trim()));
const ffmpeg = args.ffmpeg ?? process.env.FFMPEG ?? "ffmpeg";
const python = args.python ?? process.env.PYTHON ?? "python";
const still = args.still === undefined ? null : Number(args.still);

// A run that hangs must end rather than stall whatever launched it. Encoding
// a long lossless WebP is the slow part, several minutes at 2x.
const watchdog = setTimeout(() => {
  console.log("WATCHDOG\tgave up after 40 minutes");
  process.exit(3);
}, 40 * 60_000);
watchdog.unref();

/** Run a tool to completion, failing loudly. */
const run = (command, commandArgs) => {
  const result = spawnSync(command, commandArgs, { stdio: "inherit" });
  if (result.status !== 0) {
    throw new Error(`${command} exited with ${result.status ?? result.error}`);
  }
};

/** Encode the numbered PNGs in `dir` into `target` (.gif or .webp). WebP goes
 *  through scripts/readme-webp.py, which encodes every frame lossless: any
 *  lossy frame (ffmpeg's encoder, or libwebp's mixed mode) left trails of
 *  one scene over the next. */
const encode = (dir, target) => {
  mkdirSync(dirname(target), { recursive: true });
  if (extname(target).toLowerCase() === ".webp") {
    run(python, [
      join(dirname(fileURLToPath(import.meta.url)), "readme-webp.py"),
      dir,
      target,
      "--fps",
      String(fps),
      "--quality",
      String(args.quality ?? 90),
      ...(args.scale ? ["--width", String(args.scale)] : []),
    ]);
    return;
  }
  const scale = args.scale ? `scale=${args.scale}:-1:flags=lanczos,` : "";
  run(ffmpeg, [
    "-hide_banner",
    "-loglevel",
    "error",
    "-y",
    "-framerate",
    String(fps),
    "-i",
    join(dir, "%05d.png"),
    "-vf",
    // One palette for the whole loop (the scenes share their colours), and
    // only the changed rectangle of each frame is stored.
    `${scale}split[a][b];[a]palettegen=max_colors=${args.colors ?? 256}:stats_mode=full[p];` +
      "[b][p]paletteuse=dither=sierra2_4a:diff_mode=rectangle",
    "-loop",
    "0",
    target,
  ]);
  console.log(`OK\t${target}`);
};

/** Pause every CSS animation and transition, then move each to its own time:
 *  ones already seen advance by `dt`, ones that appeared since start at 0. */
const stepAnimations = (page, dt) =>
  page.evaluate((delta) => {
    const times = (window.__readmeTimes ??= new WeakMap());
    for (const animation of document.getAnimations()) {
      const time = times.has(animation) ? times.get(animation) + delta : 0;
      times.set(animation, time);
      animation.pause();
      animation.currentTime = time;
    }
  }, dt);

/** Let React commit what the timers just scheduled. Its scheduler posts
 *  through MessageChannel, which the fake clock does not own, so this is a
 *  short real wait rather than a clock tick. */
const settle = (page) => page.waitForTimeout(30);

/** Advance the page by one frame: timers first, then CSS. */
const advance = async (page, ms) => {
  await page.clock.runFor(ms);
  await settle(page);
  await stepAnimations(page, ms);
};

/** Record the demo into `dir` (or one still into `stillOut`). */
const record = async (dir, stillOut) => {
  const { chromium } = await import("playwright-core").catch(() => {
    console.error(
      "playwright-core is not installed (bun add -d playwright-core)",
    );
    process.exit(2);
  });
  const browser = await chromium.launch({
    channel: args.browser ?? "chrome",
    headless: true,
  });
  try {
    const context = await browser.newContext({
      viewport: { width, height },
      deviceScaleFactor: dpr,
      colorScheme: "dark",
      reducedMotion: "no-preference",
    });
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));

    await page.clock.install({ time: new Date("2026-10-01T09:00:00") });
    await page.goto(`${base}/src/preview/readme.html?${args.query ?? ""}`, {
      waitUntil: "networkidle",
      timeout: 60_000,
    });
    await page.evaluate(() => document.fonts.ready.then(() => undefined));
    await page.waitForFunction(() => Boolean(window.__readmeDemo), null, {
      timeout: 30_000,
    });

    // Stop the clock, then start the demo over from its first frame.
    await page.clock.pauseAt(new Date("2026-10-01T09:01:00"));
    await page.evaluate(() => window.__readmeDemo.restart());
    await settle(page);
    await page.clock.runFor(50);
    await settle(page);
    await stepAnimations(page, 0);

    const total = await page.evaluate(() => window.__readmeDemo.total);
    if (stillOut) {
      // Walk to the requested moment in frame-sized steps, so the timers
      // fire in the same order they do during a recording.
      for (let t = 0; t < still; t += step) await advance(page, step);
      mkdirSync(dirname(stillOut), { recursive: true });
      writeFileSync(
        stillOut,
        await page.screenshot({ type: "png", omitBackground: true }),
      );
      console.log(`OK\t${stillOut}`);
    } else {
      const duration = Number(args.duration ?? total);
      const count = Math.round(duration / step);
      console.log(`RECORD\t${count} frames, ${duration} ms at ${fps} fps`);
      mkdirSync(dir, { recursive: true });
      for (let index = 0; index < count; index += 1) {
        if (index > 0) await advance(page, step);
        const file = join(dir, `${String(index).padStart(5, "0")}.png`);
        writeFileSync(
          file,
          await page.screenshot({ type: "png", omitBackground: true }),
        );
        if (index % 100 === 0) console.log(`FRAME\t${index}/${count}`);
      }
    }
    if (errors.length > 0) {
      console.log(`PAGE_ERRORS\t${errors.length}`);
      for (const error of errors.slice(0, 5)) console.log(`  ${error}`);
    }
  } finally {
    await browser.close();
  }
};

if (still !== null) {
  await record(null, outs[0]);
} else if (args["from-frames"]) {
  const dir = resolve(args["from-frames"]);
  if (!existsSync(join(dir, "00000.png"))) {
    console.error(`no recorded frames in ${dir}`);
    process.exit(2);
  }
  for (const target of outs) encode(dir, target);
} else {
  const keep = Boolean(args.frames);
  const dir = keep
    ? resolve(args.frames)
    : mkdtempSync(join(tmpdir(), "readme-media-"));
  try {
    await record(dir, null);
    if (args.out) for (const target of outs) encode(dir, target);
  } finally {
    if (!keep) rmSync(dir, { recursive: true, force: true });
  }
}
