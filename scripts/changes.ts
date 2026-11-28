#!/usr/bin/env bun
/**
 * Decides what a change touches, so CI jobs and the pre-push hook run only
 * the checks that can be affected. One mapping serves both; it is tested by
 * scripts/changes.test.ts.
 *
 *   bun scripts/changes.ts --base <ref> [--head HEAD] [--all] --github-output
 *   bun scripts/changes.ts --base origin/main --run     (pre-push)
 *
 * `--all` marks every part changed (CI uses it for pushes to main: a pending
 * main run can be replaced by a newer one, so each main run checks everything).
 *
 * Outputs: rust, site, deps, package ("true" or "false"). `package` marks the
 * inputs of packaging (the app bundle, its icons, the release tasks and the
 * release workflows), not app source. An empty or unreadable diff, or a path
 * nobody claims, runs the Rust checks (fail safe).
 */
import { appendFileSync } from "node:fs";
import { git as libGit } from "./lib";

export type Changes = {
  rust: boolean;
  site: boolean;
  deps: boolean;
  package: boolean;
};

const NONE: Changes = { rust: false, site: false, deps: false, package: false };
export const ALL: Changes = { rust: true, site: true, deps: true, package: true };

const starts = (f: string, ...p: string[]) => p.some((x) => f.startsWith(x));
const isMarkdown = (f: string) => f.endsWith(".md");

/** Files whose content cannot change the Rust build or its tests. */
const NOT_RUST_EXACT = new Set([
  "hk.pkl",
  "release-please-config.json",
  ".release-please-manifest.json",
  "LICENSE",
  ".gitignore",
  ".gitattributes",
  "CODEOWNERS",
  "renovate.json",
]);
const isRust = (f: string) => {
  if (isMarkdown(f) || NOT_RUST_EXACT.has(f)) return false;
  if (starts(f, "site/", "docs/", ".agents/", ".claude/")) return false;
  if (starts(f, "scripts/")) return f.endsWith(".sh") || f.endsWith(".swift");
  if (starts(f, ".github/actions/")) return true;
  if (starts(f, ".github/")) return f === ".github/workflows/ci.yml";
  return true;
};

const isSite = (f: string) =>
  starts(f, "site/", "scripts/", ".github/", "mise-tasks/", "release-please") ||
  f === ".release-please-manifest.json" ||
  f === "mise.toml" ||
  f === "hk.pkl" ||
  f === "renovate.json" ||
  /^crates\/.*\/i18n\//.test(f) ||
  starts(f, "i18n/");

/** Inputs of packaging; source changes are covered by the Rust jobs, and every push to main still runs the package job (`--all`). */
const isPackage = (f: string) =>
  starts(
    f,
    "crates/parla/macos/",
    "assets/icons/app/",
    "mise-tasks/release/",
    ".github/actions/",
  ) ||
  f === "crates/parla/Cargo.toml" ||
  f === "crates/parla/build.rs" ||
  f === "Cargo.toml" ||
  f === "Cargo.lock" ||
  f === "mise.toml" ||
  f === ".github/workflows/ci.yml" ||
  f === ".github/workflows/release.yml" ||
  f === ".github/workflows/release-build.yml";

export function classify(files: string[]): Changes {
  if (files.length === 0) return { ...ALL };
  const out = { ...NONE };
  for (const f of files) {
    if (isRust(f)) out.rust = true;
    if (isSite(f)) out.site = true;
    if (/(^|\/)(Cargo\.toml|Cargo\.lock|deny\.toml|bun\.lock|package\.json)$/.test(f)) out.deps = true;
    if (isPackage(f)) out.package = true;
  }
  return out;
}

/** The mise tasks a push must run for these changes, in order. */
export function prePushTasks(c: Changes, files: string[]): string[] {
  const tasks: string[] = [];
  if (c.rust) tasks.push("test");
  if (c.site && files.some((f) => starts(f, "site/"))) tasks.push("site:verify");
  if (
    files.some(
      (f) =>
        starts(f, "scripts/", "mise-tasks/"),
    )
  )
    tasks.push("test:repo");
  return tasks;
}

function git(args: string[]): string | null {
  try {
    return libGit(args);
  } catch {
    return null;
  }
}

/** What CI runs: `--all` or an unreadable diff runs everything; otherwise the diff decides. */
export function decide(files: string[] | null, all: boolean): Changes {
  return all || !files ? { ...ALL } : classify(files);
}

export function changedFiles(base: string, head: string): string[] | null {
  const out = git(["diff", "--name-only", `${base}...${head}`]) ?? git(["diff", "--name-only", base, head]);
  return out === null ? null : out.split("\n").filter(Boolean);
}

function main() {
  const a = process.argv.slice(2);
  const opt = (n: string) => (a.includes(n) ? a[a.indexOf(n) + 1] : undefined);
  const base = opt("--base");
  if (!base) {
    console.error("usage: changes.ts --base <ref> [--head <ref>] [--github-output | --run]");
    process.exit(2);
  }
  const files = changedFiles(base, opt("--head") ?? "HEAD");
  const c = decide(files, a.includes("--all"));
  if (a.includes("--github-output")) {
    const lines = Object.entries(c).map(([k, v]) => `${k}=${v}`);
    console.log(lines.join("\n"));
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, lines.join("\n") + "\n");
    return;
  }
  if (a.includes("--run")) {
    const tasks = files ? prePushTasks(c, files) : ["test", "site:verify", "test:repo"];
    if (tasks.length === 0) {
      console.log("pre-push: no code, site or tooling changes versus " + base + "; nothing to run");
      return;
    }
    for (const t of tasks) {
      console.log(`pre-push: mise run ${t}`);
      const r = Bun.spawnSync(["mise", "run", t], { stdout: "inherit", stderr: "inherit" });
      if (r.exitCode !== 0) process.exit(r.exitCode ?? 1);
    }
    return;
  }
  console.log(JSON.stringify(c));
}

if (import.meta.main) main();
