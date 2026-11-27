#!/usr/bin/env bun
/**
 * A/B perf run: builds and measures the merge base and HEAD, same arguments,
 * and writes target/perf/ab.md with before, after and delta per metric.
 *
 *   mise run perf:ab -- [--runs n] [--scenarios a,b] [--seconds n] [--wait-quiet <min>] [--base <ref>]
 *
 * The merge base (versus --base, default origin/main) is checked out as a git
 * worktree in target/perf-ab/base, measured with its own perf-suite, and the
 * worktree is removed afterwards. Each side runs scripts/perf-suite.ts, so
 * every argument but --base passes through. A metric is flagged REGRESSION when
 * it is more than 10% worse than before, OVER BUDGET when it exceeds the
 * crates/parla/tests/perf_budget.toml limit. Lower is better for every metric.
 * Pure parts are tested by scripts/perf-ab.test.ts; the run is not.
 */
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { run } from "./lib";

export const REGRESSION_PCT = 10;

export type MetricMap = Record<string, Record<string, number>>;

export interface Row {
  key: string;
  scenario: string;
  metric: string;
  before: number;
  after: number;
  delta: number;
  pct: number;
  limit?: number;
  regression: boolean;
  overBudget: boolean;
}

export function flagRegression(
  before: number,
  after: number,
  limit: number | undefined,
): { regression: boolean; overBudget: boolean } {
  const regression = before === 0 ? after > 0 : ((after - before) / Math.abs(before)) * 100 > REGRESSION_PCT;
  return { regression, overBudget: limit !== undefined && after > limit };
}

export function diffMetrics(before: MetricMap, after: MetricMap, limits: Record<string, number>): Row[] {
  const rows: Row[] = [];
  for (const [scenario, ms] of Object.entries(after)) {
    for (const [metric, a] of Object.entries(ms)) {
      const b = before[scenario]?.[metric];
      if (b === undefined) continue;
      const key = `${scenario}.${metric}`;
      const limit = limits[key];
      rows.push({
        key,
        scenario,
        metric,
        before: b,
        after: a,
        delta: a - b,
        pct: b === 0 ? (a === 0 ? 0 : Infinity) : ((a - b) / Math.abs(b)) * 100,
        limit,
        ...flagRegression(b, a, limit),
      });
    }
  }
  return rows;
}

const f = (n: number) => (Math.abs(n) >= 100 ? n.toFixed(0) : n.toFixed(2));
const signed = (n: number) => (n >= 0 ? "+" : "-") + f(Math.abs(n));

export function abMarkdown(rows: Row[], baseSha: string, headSha: string): string {
  const lines = rows.map((r) => {
    const flags = [r.regression && "REGRESSION", r.overBudget && "OVER BUDGET"].filter(Boolean).join(", ");
    const pct = Number.isFinite(r.pct) ? `${r.pct >= 0 ? "+" : "-"}${Math.abs(r.pct).toFixed(1)}%` : "new";
    return `| ${r.scenario} | ${r.metric} | ${f(r.before)} | ${f(r.after)} | ${signed(r.delta)} (${pct}) | ${r.limit === undefined ? "-" : `<= ${f(r.limit)}`} | ${flags || "-"} |`;
  });
  const bad = rows.filter((r) => r.regression || r.overBudget).length;
  return [
    `Before: merge base \`${baseSha}\`. After: \`${headSha}\`. Medians from perf:suite; lower is better.`,
    "",
    "| Scenario | Metric | Before | After | Delta | Budget | Flag |",
    "| --- | --- | --- | --- | --- | --- | --- |",
    ...lines,
    "",
    bad
      ? `${bad} metric(s) flagged: REGRESSION is more than ${REGRESSION_PCT}% worse than before, OVER BUDGET exceeds the budget.`
      : `No regression beyond ${REGRESSION_PCT}% and nothing over budget.`,
    "",
  ].join("\n");
}

/** Per scenario and metric, the quiet median when there is one, else the overall median. */
function medians(suiteJson: string): MetricMap {
  const s = JSON.parse(readFileSync(suiteJson, "utf8")).summary as Record<
    string,
    { metrics: Record<string, { quiet: { median: number } | null; all: { median: number } }> }
  >;
  const out: MetricMap = {};
  for (const [sc, sum] of Object.entries(s)) {
    out[sc] = {};
    for (const [m, st] of Object.entries(sum.metrics)) out[sc][m] = (st.quiet ?? st.all).median;
  }
  return out;
}

const sh = (cmd: string[]) => run(cmd).trim();

function main(): number {
  const raw = process.argv.slice(2);
  const bi = raw.indexOf("--base");
  const baseRef = bi >= 0 ? raw[bi + 1] : "origin/main";
  const pass = bi >= 0 ? [...raw.slice(0, bi), ...raw.slice(bi + 2)] : raw;
  const root = resolve(import.meta.dir, "..");
  process.chdir(root);
  const baseSha = sh(["git", "merge-base", baseRef, "HEAD"]);
  const headSha = sh(["git", "rev-parse", "HEAD"]);
  const wt = join(root, "target/perf-ab/base");
  mkdirSync(join(root, "target/perf-ab"), { recursive: true });
  if (existsSync(wt)) {
    Bun.spawnSync(["git", "worktree", "remove", "--force", wt]);
    rmSync(wt, { recursive: true, force: true });
  }
  const suite = (cwd: string) =>
    Bun.spawnSync(["bun", "scripts/perf-suite.ts", ...pass], { cwd, stdout: "inherit", stderr: "inherit" });
  try {
    sh(["git", "worktree", "add", "--detach", wt, baseSha]);
    console.log(`perf:ab: measuring base ${baseSha.slice(0, 12)}`);
    suite(wt);
    const baseJson = join(wt, "target/perf/suite.json");
    if (!existsSync(baseJson)) throw new Error("base run produced no suite.json");
    const before = medians(baseJson);
    console.log(`perf:ab: measuring HEAD ${headSha.slice(0, 12)}`);
    suite(root);
    const after = medians(join(root, "target/perf/suite.json"));
    const limits = (Bun.TOML.parse(readFileSync(join(root, "crates/parla/tests/perf_budget.toml"), "utf8")) as {
      limits: Record<string, number>;
    }).limits;
    const md = abMarkdown(diffMetrics(before, after, limits), baseSha.slice(0, 12), headSha.slice(0, 12));
    writeFileSync(join(root, "target/perf/ab.md"), md);
    console.log("\n" + md);
    return 0;
  } catch (e) {
    console.error((e as Error).message);
    return 1;
  } finally {
    Bun.spawnSync(["git", "worktree", "remove", "--force", wt]);
    rmSync(wt, { recursive: true, force: true });
  }
}

if (import.meta.main) process.exit(main());
