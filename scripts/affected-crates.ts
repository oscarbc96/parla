#!/usr/bin/env bun
/**
 * Runs the tests of only the crates a change can affect.
 *
 *   mise run test:affected -- [--base origin/main] [--list]
 *
 * Changed files are `git diff --name-only <base>...HEAD` plus uncommitted and
 * untracked files. Each file maps to the workspace crate whose directory is its
 * longest prefix; reverse dependents are added transitively. Workspace-wide
 * files (Cargo.toml, Cargo.lock, nextest config, toolchain, mise.toml) and Rust
 * inputs no crate claims mean every crate. Changes with no Rust input run
 * nothing. Classification reuses scripts/changes.ts. Tested by
 * scripts/affected-crates.test.ts.
 */
import { classify } from "./changes";
import { arg, has, git, repoRoot, run } from "./lib";

export interface Crate {
  name: string;
  dir: string;
  deps: string[];
}

export type Affected =
  | { kind: "none" }
  | { kind: "all" }
  | { kind: "crates"; names: string[] };

const WORKSPACE_WIDE =
  /^(Cargo\.toml|Cargo\.lock|rust-toolchain(\.toml)?|mise\.toml|\.config\/nextest\.toml)$/;

export function parseMetadata(meta: {
  workspace_root: string;
  packages: { name: string; manifest_path: string; dependencies: { name: string }[] }[];
}): Crate[] {
  const root = meta.workspace_root.replace(/\/$/, "") + "/";
  const names = new Set(meta.packages.map((p) => p.name));
  return meta.packages.map((p) => ({
    name: p.name,
    dir: p.manifest_path.replace(root, "").replace(/\/?Cargo\.toml$/, ""),
    deps: p.dependencies.map((d) => d.name).filter((n) => names.has(n)),
  }));
}

export function affectedCrates(crates: Crate[], files: string[]): Affected {
  const direct = new Set<string>();
  for (const f of files) {
    if (WORKSPACE_WIDE.test(f)) return { kind: "all" };
    if (!classify([f]).rust) continue;
    const owner = crates
      .filter((c) => f.startsWith(c.dir + "/"))
      .sort((a, b) => b.dir.length - a.dir.length)[0];
    if (!owner) return { kind: "all" };
    direct.add(owner.name);
  }
  if (direct.size === 0) return { kind: "none" };
  const out = new Set(direct);
  for (let grew = true; grew; ) {
    grew = false;
    for (const c of crates) {
      if (!out.has(c.name) && c.deps.some((d) => out.has(d))) {
        out.add(c.name);
        grew = true;
      }
    }
  }
  return { kind: "crates", names: [...out].sort() };
}

export function nextestArgs(a: Affected): string[] | null {
  if (a.kind === "none") return null;
  const base = ["nextest", "run", "--no-tests=pass"];
  if (a.kind === "all") return [...base, "--workspace"];
  return [...base, ...a.names.flatMap((n) => ["-p", n])];
}

function changedFiles(base: string): string[] {
  const lines = (s: string) => s.split("\n").filter(Boolean);
  const committed =
    git(["diff", "--name-only", `${base}...HEAD`], { allowFail: true }) ||
    git(["diff", "--name-only", base, "HEAD"], { allowFail: true });
  const dirty = git(["diff", "--name-only", "HEAD"], { allowFail: true });
  const untracked = git(["ls-files", "--others", "--exclude-standard"], { allowFail: true });
  return [...new Set([...lines(committed), ...lines(dirty), ...lines(untracked)])];
}

function main(): number {
  const argv = process.argv.slice(2);
  const base = arg(argv, "--base") ?? "origin/main";
  process.chdir(repoRoot());
  const files = changedFiles(base);
  const meta = JSON.parse(run(["cargo", "metadata", "--no-deps", "--format-version", "1"]));
  const affected = affectedCrates(parseMetadata(meta), files);
  const args = nextestArgs(affected);
  if (!args) {
    console.log(`test:affected: no Rust changes versus ${base}; nothing to run`);
    return 0;
  }
  const what = affected.kind === "all" ? "all crates" : affected.kind === "crates" ? affected.names.join(", ") : "";
  console.log(`test:affected: ${files.length} changed file(s) versus ${base}; running ${what}`);
  if (has(argv, "--list")) return 0;
  const r = Bun.spawnSync(["cargo", ...args], { stdout: "inherit", stderr: "inherit" });
  return r.exitCode ?? 1;
}

if (import.meta.main) process.exit(main());
