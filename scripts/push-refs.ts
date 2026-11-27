#!/usr/bin/env bun
/**
 * pre-push helper. Git feeds a pre-push hook one line per ref:
 * `<local ref> <local sha> <remote ref> <remote sha>`. Checks and tests are
 * for branches that carry main's history. Tags, deletions and unrelated
 * branches (no common history with main) are skipped, so a tooling push
 * never needs the hooks disabled.
 *
 *   bun scripts/push-refs.ts '<hook stdin>' [--base origin/main] -- <cmd> [args]
 *
 * Runs <cmd> once per pushed branch (a `{sha}` argument becomes the pushed
 * commit), or prints a notice and exits 0 when no ref qualifies.
 */
const ZERO = /^0+$/;

export type PushRef = { localSha: string; remoteRef: string };

export function parseRefs(stdin: string): PushRef[] {
  return stdin
    .split("\n")
    .map((l) => l.trim().split(/\s+/))
    .filter((p) => p.length >= 4)
    .map((p) => ({ localSha: p[1], remoteRef: p[2] }));
}

/** Branch pushes, not deletions, whose commit shares history with `base`. */
export function qualifying(refs: PushRef[], sharesHistory: (sha: string) => boolean): string[] {
  return refs
    .filter((r) => r.remoteRef.startsWith("refs/heads/") && !ZERO.test(r.localSha) && sharesHistory(r.localSha))
    .map((r) => r.localSha);
}

function main(argv: string[]): number {
  const dash = argv.indexOf("--");
  if (dash < 1 || dash === argv.length - 1) {
    console.error("usage: push-refs.ts '<hook stdin>' [--base <ref>] -- <cmd> [args]");
    return 2;
  }
  const stdin = argv[0];
  const b = argv.indexOf("--base");
  const base = b > 0 && b < dash ? argv[b + 1] : "origin/main";
  const cmd = argv.slice(dash + 1);
  const shares = (sha: string) =>
    Bun.spawnSync(["git", "merge-base", base, sha], { stdout: "ignore", stderr: "ignore" }).exitCode === 0;
  const shas = qualifying(parseRefs(stdin), shares);
  if (shas.length === 0) {
    console.log(`pre-push: no pushed branch is based on ${base}; skipped`);
    return 0;
  }
  for (const sha of shas) {
    const r = Bun.spawnSync(
      cmd.map((a) => (a === "{sha}" ? sha : a)),
      { stdout: "inherit", stderr: "inherit" },
    );
    if (r.exitCode !== 0) return r.exitCode ?? 1;
  }
  return 0;
}

if (import.meta.main) process.exit(main(process.argv.slice(2)));
