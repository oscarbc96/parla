/**
 * Runs the local gates in order and writes a summary for the PR body.
 *
 *   mise run gates -- [--quick] [--only a,b] [--skip a,b] [--continue]
 *
 * Steps: build, lint, clippy, test, test:visual, test:perf, i18n:check,
 * site:check-translations, test:repo. `--quick` is build, lint, clippy, test.
 * Output streams to target/gates/<step>.log; target/gates/summary.md is a
 * table to paste into the PR's testing section, summary.json the same data.
 * Stops at the first failure unless `--continue`. Exit 1 on any failure.
 */
import { mkdirSync, readFileSync, writeFileSync, createWriteStream } from "node:fs";
import { join, resolve } from "node:path";

export interface Step {
  name: string;
  cmd: string[];
  quick: boolean;
}

const mise = (task: string): string[] => ["mise", "run", task];

export const STEPS: Step[] = [
  { name: "build", cmd: mise("build"), quick: true },
  { name: "lint", cmd: mise("lint"), quick: true },
  // Host and guest (plugins/) workspaces, warnings denied (mise.toml).
  { name: "clippy", cmd: mise("clippy"), quick: true },
  { name: "test", cmd: mise("test"), quick: true },
  { name: "test:visual", cmd: mise("test:visual"), quick: false },
  { name: "test:perf", cmd: mise("test:perf"), quick: false },
  { name: "i18n:check", cmd: mise("i18n:check"), quick: false },
  {
    name: "site:check-translations",
    cmd: mise("site:check-translations"),
    quick: false,
  },
  { name: "test:repo", cmd: mise("test:repo"), quick: false },
];

export interface Args {
  quick: boolean;
  only: string[] | null;
  skip: string[];
  keepGoing: boolean;
}

const list = (v: string | undefined) =>
  (v ?? "").split(",").map((s) => s.trim()).filter(Boolean);

export function parseArgs(argv: string[]): Args {
  const a: Args = { quick: false, only: null, skip: [], keepGoing: false };
  for (let i = 0; i < argv.length; i++) {
    const f = argv[i];
    if (f === "--quick") a.quick = true;
    else if (f === "--continue") a.keepGoing = true;
    else if (f === "--only") a.only = list(argv[++i]);
    else if (f === "--skip") a.skip = list(argv[++i]);
    else throw new Error(`unknown argument ${f}`);
  }
  return a;
}

export function selectSteps(a: Args): Step[] {
  const known = new Set(STEPS.map((s) => s.name));
  for (const n of [...(a.only ?? []), ...a.skip]) {
    if (!known.has(n)) {
      throw new Error(`unknown step ${n} (known: ${[...known].join(", ")})`);
    }
  }
  return STEPS.filter(
    (s) =>
      (!a.quick || s.quick) &&
      (a.only === null || a.only.includes(s.name)) &&
      !a.skip.includes(s.name),
  );
}

export interface StepResult {
  name: string;
  status: "pass" | "fail" | "skipped";
  seconds: number;
  log?: string;
}

export function summaryMarkdown(results: StepResult[]): string {
  const rows = results.map((r) => {
    const res = r.status === "fail" ? "FAIL" : r.status;
    const time = r.status === "skipped" ? "-" : `${r.seconds.toFixed(1)} s`;
    return `| \`${r.name}\` | ${res} | ${time} |`;
  });
  return ["| Step | Result | Time |", "| --- | --- | --- |", ...rows, ""].join("\n");
}

function tail(path: string, n = 40): string {
  try {
    return readFileSync(path, "utf8").split("\n").slice(-n - 1).join("\n");
  } catch {
    return "";
  }
}

async function runStep(step: Step, dir: string): Promise<StepResult> {
  const log = join(dir, `${step.name.replace(/[^\w.-]/g, "_")}.log`);
  const out = createWriteStream(log);
  const t0 = performance.now();
  const p = Bun.spawn(step.cmd, {
    stdout: "pipe",
    stderr: "pipe",
    env: { ...process.env, CARGO_TERM_COLOR: "never", NO_COLOR: "1" },
  });
  const pump = async (s: ReadableStream<Uint8Array>) => {
    for await (const chunk of s) out.write(chunk);
  };
  await Promise.all([pump(p.stdout), pump(p.stderr)]);
  const code = await p.exited;
  await new Promise((r) => out.end(r));
  return {
    name: step.name,
    status: code === 0 ? "pass" : "fail",
    seconds: (performance.now() - t0) / 1000,
    log,
  };
}

export async function main(argv: string[]): Promise<number> {
  let steps: Step[];
  let args: Args;
  try {
    args = parseArgs(argv);
    steps = selectSteps(args);
  } catch (e) {
    console.error(String((e as Error).message));
    return 2;
  }
  const root = resolve(import.meta.dir, "..");
  process.chdir(root);
  const dir = join(root, "target", "gates");
  mkdirSync(dir, { recursive: true });
  const results: StepResult[] = [];
  let failed = false;
  for (const step of steps) {
    if (failed && !args.keepGoing) {
      results.push({ name: step.name, status: "skipped", seconds: 0 });
      continue;
    }
    const r = await runStep(step, dir);
    results.push(r);
    console.log(`${r.status === "pass" ? "pass" : "FAIL"}  ${step.name}  ${r.seconds.toFixed(1)} s`);
    if (r.status === "fail") {
      failed = true;
      console.log(`--- tail of ${r.log} ---\n${tail(r.log!)}\n---`);
    }
  }
  writeFileSync(join(dir, "summary.md"), summaryMarkdown(results));
  writeFileSync(join(dir, "summary.json"), JSON.stringify({ ok: !failed, steps: results }, null, 2) + "\n");
  console.log(`\nsummary: ${join(dir, "summary.md")}`);
  return failed ? 1 : 0;
}

if (import.meta.main) process.exit(await main(process.argv.slice(2)));
