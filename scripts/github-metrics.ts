/**
 * GitHub traffic + release snapshot.
 *
 * GitHub's traffic API only exposes a rolling 14-day window. Views, clones,
 * referrers and popular paths older than that are deleted and cannot be
 * recovered, so the only way to measure whether a README or metadata change
 * actually worked is to snapshot the numbers before changing anything and
 * again afterwards.
 *
 * Usage:
 *   bun scripts/github-metrics.ts            # take a snapshot, print summary + deltas
 *   bun scripts/github-metrics.ts --dry-run  # print only, write nothing
 *
 * Requires the GitHub CLI (`gh`) authenticated as a user with push access to
 * the repo; the traffic endpoints are owner-only.
 */

import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";

const REPO = process.env.SPEAKOFLOW_REPO ?? "AbhishekBarali/SpeakoFlow";
const OUT_DIR = ".metrics";
const DRY_RUN = process.argv.includes("--dry-run");

type Timeseries = { count: number; uniques: number };
type Referrer = { referrer: string; count: number; uniques: number };
type PathEntry = {
  path: string;
  title: string;
  count: number;
  uniques: number;
};

interface Snapshot {
  takenAt: string;
  repo: string;
  /** Rolling 14-day window the traffic figures cover. */
  window: { from: string | null; to: string | null };
  repository: {
    description: string | null;
    topics: string[];
    stars: number;
    forks: number;
    watchers: number;
    openIssues: number;
  };
  traffic: {
    views: Timeseries;
    clones: Timeseries;
    referrers: Referrer[];
    paths: PathEntry[];
  };
  /** Cumulative since each release was published, not per-window. */
  downloads: {
    byOs: Record<string, number>;
    byRelease: Record<string, number>;
    total: number;
  };
  derived: {
    starsPer100UniqueVisitors: number | null;
    releaseVisitorsPer100RepoVisitors: number | null;
    newStarsInWindow: number | null;
  };
}

function gh<T>(endpoint: string, paginate = false): T {
  const args = ["api", endpoint, "-H", "Accept: application/vnd.github+json"];
  if (paginate) args.push("--paginate", "--slurp");
  try {
    const raw = execFileSync("gh", args, {
      encoding: "utf8",
      maxBuffer: 32 * 1024 * 1024,
    });
    return JSON.parse(raw) as T;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`gh api ${endpoint} failed.\n${message}`);
  }
}

/** Windows/macOS/Linux buckets. Signature and checksum files are excluded. */
function classifyAsset(name: string): "windows" | "macos" | "linux" | "other" {
  const n = name.toLowerCase();
  if (/\.(sig|sha256|txt|json)$/.test(n)) return "other";
  if (/\.(exe|msi)$/.test(n)) return "windows";
  if (/\.(dmg|pkg)$|app\.tar\.gz$/.test(n)) return "macos";
  if (/\.appimage$|\.deb$|\.rpm$|\.tar\.gz$/.test(n)) return "linux";
  return "other";
}

function countStarsSince(since: string): number | null {
  // Stargazer timestamps need a preview Accept header, and the API only walks
  // oldest-first, so this is a full paginated read. Cheap at a few hundred stars.
  try {
    const raw = execFileSync(
      "gh",
      [
        "api",
        `repos/${REPO}/stargazers?per_page=100`,
        "-H",
        "Accept: application/vnd.github.star+json",
        "--paginate",
        "--slurp",
      ],
      { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 },
    );
    const pages = JSON.parse(raw) as Array<Array<{ starred_at: string }>>;
    return pages
      .flat()
      .filter((s) => typeof s.starred_at === "string" && s.starred_at >= since)
      .length;
  } catch {
    return null;
  }
}

function take(): Snapshot {
  const repo = gh<{
    description: string | null;
    topics: string[];
    stargazers_count: number;
    forks_count: number;
    subscribers_count: number;
    open_issues_count: number;
  }>(`repos/${REPO}`);

  const views = gh<Timeseries & { views: Array<{ timestamp: string }> }>(
    `repos/${REPO}/traffic/views`,
  );
  const clones = gh<Timeseries>(`repos/${REPO}/traffic/clones`);
  const referrers = gh<Referrer[]>(`repos/${REPO}/traffic/popular/referrers`);
  const paths = gh<PathEntry[]>(`repos/${REPO}/traffic/popular/paths`);

  const days = views.views ?? [];
  const from = days.length ? days[0]!.timestamp.slice(0, 10) : null;
  const to = days.length ? days[days.length - 1]!.timestamp.slice(0, 10) : null;

  const releasePages = gh<
    Array<
      Array<{
        tag_name: string;
        assets: Array<{ name: string; download_count: number }>;
      }>
    >
  >(`repos/${REPO}/releases?per_page=100`, true);
  const releases = releasePages.flat();

  const byOs: Record<string, number> = {
    windows: 0,
    macos: 0,
    linux: 0,
    other: 0,
  };
  const byRelease: Record<string, number> = {};
  for (const release of releases) {
    let releaseTotal = 0;
    for (const asset of release.assets ?? []) {
      const bucket = classifyAsset(asset.name);
      byOs[bucket] = (byOs[bucket] ?? 0) + asset.download_count;
      if (bucket !== "other") releaseTotal += asset.download_count;
    }
    byRelease[release.tag_name] = releaseTotal;
  }
  const total = byOs.windows! + byOs.macos! + byOs.linux!;

  const repoPath = `/${REPO}`;
  const repoHome = paths.find((p) => p.path === repoPath);
  const releasesPage = paths.find((p) => p.path === `${repoPath}/releases`);
  const newStars = from ? countStarsSince(from) : null;

  const round1 = (n: number) => Math.round(n * 10) / 10;

  return {
    takenAt: new Date().toISOString(),
    repo: REPO,
    window: { from, to },
    repository: {
      description: repo.description,
      topics: repo.topics ?? [],
      stars: repo.stargazers_count,
      forks: repo.forks_count,
      watchers: repo.subscribers_count,
      openIssues: repo.open_issues_count,
    },
    traffic: {
      views: { count: views.count, uniques: views.uniques },
      clones: { count: clones.count, uniques: clones.uniques },
      referrers,
      paths,
    },
    downloads: { byOs, byRelease, total },
    derived: {
      starsPer100UniqueVisitors:
        newStars !== null && views.uniques > 0
          ? round1((newStars / views.uniques) * 100)
          : null,
      releaseVisitorsPer100RepoVisitors:
        releasesPage && repoHome && repoHome.uniques > 0
          ? round1((releasesPage.uniques / repoHome.uniques) * 100)
          : null,
      newStarsInWindow: newStars,
    },
  };
}

function previousSnapshot(): Snapshot | null {
  if (!existsSync(OUT_DIR)) return null;
  const files = readdirSync(OUT_DIR)
    .filter((f) => f.startsWith("github-") && f.endsWith(".json"))
    .sort();
  const last = files.pop();
  if (!last) return null;
  try {
    return JSON.parse(readFileSync(join(OUT_DIR, last), "utf8")) as Snapshot;
  } catch {
    return null;
  }
}

function delta(now: number, before: number | undefined): string {
  if (before === undefined) return "";
  const d = now - before;
  if (d === 0) return "  (no change)";
  return `  (${d > 0 ? "+" : ""}${d})`;
}

function report(snap: Snapshot, prev: Snapshot | null): void {
  const p = prev?.traffic;
  console.log(`\nSpeakoFlow GitHub snapshot — ${snap.takenAt.slice(0, 10)}`);
  console.log(
    `Traffic window: ${snap.window.from} to ${snap.window.to} (rolling 14 days)`,
  );
  if (prev)
    console.log(`Comparing against snapshot from ${prev.takenAt.slice(0, 10)}`);

  console.log("\nAudience");
  console.log(
    `  unique visitors        ${snap.traffic.views.uniques}${delta(snap.traffic.views.uniques, p?.views.uniques)}`,
  );
  console.log(
    `  views                  ${snap.traffic.views.count}${delta(snap.traffic.views.count, p?.views.count)}`,
  );
  console.log(
    `  unique cloners         ${snap.traffic.clones.uniques}${delta(snap.traffic.clones.uniques, p?.clones.uniques)}`,
  );
  console.log(
    `  stars                  ${snap.repository.stars}${delta(snap.repository.stars, prev?.repository.stars)}`,
  );
  console.log(
    `  watchers               ${snap.repository.watchers}${delta(snap.repository.watchers, prev?.repository.watchers)}`,
  );

  console.log("\nConversion");
  console.log(
    `  new stars in window    ${snap.derived.newStarsInWindow ?? "n/a"}`,
  );
  console.log(
    `  stars / 100 uniques    ${snap.derived.starsPer100UniqueVisitors ?? "n/a"}`,
  );
  console.log(
    `  /releases per 100      ${snap.derived.releaseVisitorsPer100RepoVisitors ?? "n/a"}  (repo-home visitors who reached the releases page)`,
  );

  console.log("\nDownloads (cumulative, all releases)");
  for (const os of ["windows", "macos", "linux"] as const) {
    console.log(
      `  ${os.padEnd(22)} ${snap.downloads.byOs[os]}${delta(snap.downloads.byOs[os]!, prev?.downloads.byOs[os])}`,
    );
  }
  console.log(
    `  ${"total".padEnd(22)} ${snap.downloads.total}${delta(snap.downloads.total, prev?.downloads.total)}`,
  );

  console.log("\nTop referrers (unique visitors)");
  for (const r of snap.traffic.referrers.slice(0, 8)) {
    const before = p?.referrers.find((x) => x.referrer === r.referrer)?.uniques;
    console.log(
      `  ${r.referrer.padEnd(26)} ${r.uniques}${delta(r.uniques, before)}`,
    );
  }
  console.log("");
}

const snap = take();
const prev = previousSnapshot();
report(snap, prev);

if (DRY_RUN) {
  console.log("--dry-run: nothing written.\n");
} else {
  mkdirSync(OUT_DIR, { recursive: true });
  const file = join(OUT_DIR, `github-${snap.takenAt.slice(0, 10)}.json`);
  writeFileSync(file, `${JSON.stringify(snap, null, 2)}\n`, "utf8");
  console.log(`Saved ${file}\n`);
}
