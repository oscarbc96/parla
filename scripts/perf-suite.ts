/**
 * Repeats the perf scenarios on one release build and compares the medians
 * with the limits in `crates/parla/tests/perf_budget.toml`
 * (`[limits]`, the same source `perf:check` uses).
 *
 *   mise run perf:suite -- [--runs 3] [--scenarios idle,scroll,scroll-input]
 *                          [--seconds n] [--wait-quiet <min>]
 *
 * Defaults: idle 60 s, scroll and scroll-input 10 s, 3 runs each. Before each
 * run it checks the machine (1-minute load vs CPU count, other cargo/rustc or
 * perf processes) and waits up to `--wait-quiet` minutes (default 5) for it
 * to be quiet; after that it runs anyway and marks the run noisy. Medians
 * over quiet runs are checked against the limits; a scenario with only noisy
 * runs is reported but never fails. Writes target/perf/suite.md (the PR
 * Performance table) and suite.json. Exit 1 if a quiet median is over a limit.
 */
import { cpus, loadavg } from "node:os";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

export interface Args {
  runs: number;
  scenarios: string[];
  seconds?: number;
  waitQuietMin: number;
}

export const DEFAULT_SECONDS: Record<string, number> = {
  idle: 60,
  scroll: 10,
  "scroll-input": 10,
};

export function parseArgs(argv: string[]): Args {
  const a: Args = {
    runs: 3,
    scenarios: ["idle", "scroll", "scroll-input"],
    waitQuietMin: 5,
  };
  for (let i = 0; i < argv.length; i++) {
    const f = argv[i];
    const v = argv[++i];
    if (f === "--runs") a.runs = Number(v);
    else if (f === "--scenarios") a.scenarios = v.split(",").filter(Boolean);
    else if (f === "--seconds") a.seconds = Number(v);
    else if (f === "--wait-quiet") a.waitQuietMin = Number(v);
    else throw new Error(`unknown argument ${f}`);
  }
  return a;
}

export function median(v: number[]): number {
  const s = [...v].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
}

/** Reasons the machine is not quiet; empty when it is. */
export function isBusy(s: { load1: number; cpus: number; procs: string[] }): string[] {
  const why: string[] = [];
  if (s.load1 > s.cpus * 0.5) {
    why.push(`load ${s.load1.toFixed(1)} on ${s.cpus} cpus`);
  }
  const other = s.procs.filter((p) =>
    /(^|[\/\s])(cargo|rustc)(\s|$)|--perf-scenario|target\/release\/parla/.test(p),
  );
  if (other.length) why.push(`${other.length} other cargo/rustc/perf process(es)`);
  return why;
}

async function snapshot() {
  const ps = Bun.spawnSync(["ps", "-axo", "pid=,args="]);
  const me = String(process.pid);
  const procs = ps.stdout
    .toString()
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith(me + " "))
    .filter((l) => !/perf-suite|gates\.ts|\bps -axo/.test(l))
    .map((l) => l.replace(/^\d+\s+/, ""));
  return { load1: loadavg()[0], cpus: cpus().length, procs };
}

export interface Run {
  scenario: string;
  noisy: boolean;
  metrics: Record<string, number>;
}

export interface Stat {
  n: number;
  median: number;
  min: number;
  max: number;
}
const stat = (v: number[]): Stat | null =>
  v.length
    ? { n: v.length, median: median(v), min: Math.min(...v), max: Math.max(...v) }
    : null;

export interface ScenarioSummary {
  runs: number;
  noisyRuns: number;
  metrics: Record<string, { quiet: Stat | null; all: Stat }>;
}

export function summarize(runs: Run[]): Record<string, ScenarioSummary> {
  const out: Record<string, ScenarioSummary> = {};
  for (const sc of new Set(runs.map((r) => r.scenario))) {
    const rs = runs.filter((r) => r.scenario === sc);
    const names = new Set(rs.flatMap((r) => Object.keys(r.metrics)));
    const metrics: ScenarioSummary["metrics"] = {};
    for (const m of [...names].sort()) {
      const get = (list: Run[]) =>
        list.map((r) => r.metrics[m]).filter((x) => x !== undefined);
      metrics[m] = {
        quiet: stat(get(rs.filter((r) => !r.noisy))),
        all: stat(get(rs))!,
      };
    }
    out[sc] = { runs: rs.length, noisyRuns: rs.filter((r) => r.noisy).length, metrics };
  }
  return out;
}

export interface Finding {
  key: string;
  value: number;
  limit: number;
  ok: boolean;
  noisyOnly: boolean;
}

export function compare(
  s: Record<string, ScenarioSummary>,
  limits: Record<string, number>,
): Finding[] {
  const out: Finding[] = [];
  for (const [sc, sum] of Object.entries(s)) {
    for (const [m, st] of Object.entries(sum.metrics)) {
      const limit = limits[`${sc}.${m}`];
      if (limit === undefined) continue;
      const noisyOnly = st.quiet === null;
      const value = (st.quiet ?? st.all).median;
      out.push({ key: `${sc}.${m}`, value, limit, ok: noisyOnly || value <= limit, noisyOnly });
    }
  }
  return out;
}

const f = (n: number) => (Math.abs(n) >= 100 ? n.toFixed(0) : n.toFixed(2));

export function suiteMarkdown(
  s: Record<string, ScenarioSummary>,
  findings: Finding[],
): string {
  const rows: string[] = [];
  for (const [sc, sum] of Object.entries(s)) {
    for (const [m, st] of Object.entries(sum.metrics)) {
      const basis = st.quiet ?? st.all;
      const fi = findings.find((x) => x.key === `${sc}.${m}`);
      const limit = fi ? `<= ${f(fi.limit)}` : "-";
      const verdict = !fi ? "-" : fi.noisyOnly ? "noisy, not judged" : fi.ok ? "ok" : "OVER";
      const quiet = st.quiet?.n ?? 0;
      rows.push(
        `| ${sc} | ${m} | ${f(basis.median)}${st.quiet ? "" : "*"} | ${f(basis.min)} to ${f(basis.max)} | ${quiet}/${sum.runs} | ${limit} | ${verdict} |`,
      );
    }
  }
  return [
    "| Scenario | Metric | Median | Min to max | Quiet runs | Budget | Result |",
    "| --- | --- | --- | --- | --- | --- | --- |",
    ...rows,
    "",
    "Medians use quiet runs only (`*` = all runs were noisy, so the value is not judged); a run is noisy when the machine stayed busy after the wait (load or another cargo or perf process).",
    "",
  ].join("\n");
}

function loadLimits(root: string): Record<string, number> {
  const t = Bun.TOML.parse(
    readFileSync(join(root, "crates/parla/tests/perf_budget.toml"), "utf8"),
  ) as { limits: Record<string, number> };
  return t.limits;
}

async function waitQuiet(minutes: number): Promise<string[]> {
  const deadline = Date.now() + minutes * 60_000;
  for (;;) {
    const why = isBusy(await snapshot());
    if (!why.length || Date.now() >= deadline) return why;
    console.log(`busy (${why.join("; ")}); waiting`);
    await Bun.sleep(15_000);
  }
}

export async function main(argv: string[]): Promise<number> {
  let args: Args;
  try {
    args = parseArgs(argv);
  } catch (e) {
    console.error((e as Error).message);
    return 2;
  }
  const root = resolve(import.meta.dir, "..");
  process.chdir(root);
  const limits = loadLimits(root);
  console.log("building release once");
  const b = Bun.spawnSync(
    ["cargo", "build", "--release", "-p", "parla", "--features", "fixture"],
    { stdout: "inherit", stderr: "inherit" },
  );
  if (b.exitCode !== 0) return 1;
  const runs: Run[] = [];
  for (const sc of args.scenarios) {
    const secs = args.seconds ?? DEFAULT_SECONDS[sc] ?? 10;
    for (let i = 1; i <= args.runs; i++) {
      const why = await waitQuiet(args.waitQuietMin);
      const noisy = why.length > 0;
      console.log(`${sc} run ${i}/${args.runs} (${secs} s)${noisy ? ` NOISY: ${why.join("; ")}` : ""}`);
      const p = Bun.spawnSync(["scripts/perf-measure.sh", sc, "--seconds", String(secs)], {
        stdout: "inherit",
        stderr: "inherit",
      });
      if (p.exitCode !== 0) {
        console.error(`${sc} run failed`);
        return 1;
      }
      const rep = JSON.parse(readFileSync(join(root, "target/perf", `${sc}.json`), "utf8"));
      runs.push({ scenario: sc, noisy, metrics: rep.metrics });
    }
  }
  const summary = summarize(runs);
  const findings = compare(summary, limits);
  const md = suiteMarkdown(summary, findings);
  mkdirSync(join(root, "target/perf"), { recursive: true });
  writeFileSync(join(root, "target/perf/suite.md"), md);
  writeFileSync(
    join(root, "target/perf/suite.json"),
    JSON.stringify({ args, runs, summary, findings }, null, 2) + "\n",
  );
  console.log("\n" + md);
  const bad = findings.filter((x) => !x.ok);
  for (const x of bad) console.error(`over budget: ${x.key} median ${f(x.value)} > ${f(x.limit)}`);
  return bad.length ? 1 : 0;
}

if (import.meta.main) process.exit(await main(process.argv.slice(2)));
