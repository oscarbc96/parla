import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "denylist-check.sh");
const TERM = "qzxv-marker"; // a made-up term used only by this test
let repo = "";
let listDir = "";
// Hooks export GIT_DIR and friends; the temporary repo must not inherit them.
const clean = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("GIT_")));
const env = (list?: string) => ({
  ...clean,
  HOME: repo,
  GIT_AUTHOR_NAME: "t",
  GIT_AUTHOR_EMAIL: "t@example.invalid",
  GIT_COMMITTER_NAME: "t",
  GIT_COMMITTER_EMAIL: "t@example.invalid",
  ...(list ? { PARLA_DENYLIST: list } : { PARLA_DENYLIST: join(repo, "missing.txt") }),
});
const git = (...a: string[]) => {
  const r = Bun.spawnSync(["git", "-c", "commit.gpgsign=false", ...a], { cwd: repo, env: env(), stdout: "pipe" });
  if (r.exitCode !== 0) throw new Error(`git ${a.join(" ")} failed`);
  return r.stdout.toString().trim();
};
const run = (list?: string) => {
  const r = Bun.spawnSync(["sh", script, "main", "HEAD"], { cwd: repo, env: env(list), stdout: "pipe", stderr: "pipe" });
  return { code: r.exitCode, out: r.stdout.toString() + r.stderr.toString() };
};

beforeAll(() => {
  repo = mkdtempSync(join(tmpdir(), "denylist-"));
  listDir = mkdtempSync(join(tmpdir(), "denylist-list-"));
  git("init", "-q", "-b", "main");
  writeFileSync(join(repo, "a.txt"), "hello\n");
  git("add", ".");
  git("commit", "-q", "-m", "chore: base");
  git("checkout", "-q", "-b", "feature");
});
afterAll(() => {
  rmSync(repo, { recursive: true, force: true });
  rmSync(listDir, { recursive: true, force: true });
});

describe("denylist-check", () => {
  test("a missing list skips with a notice", () => {
    const r = run();
    expect(r.code).toBe(0);
    expect(r.out).toContain("skipped");
  });
  test("a clean branch passes", () => {
    const list = join(listDir, "list.txt");
    writeFileSync(list, `# comment\n\n${TERM}\n`);
    writeFileSync(join(repo, "b.txt"), "fine\n");
    git("add", ".");
    git("commit", "-q", "-m", "feat: fine");
    const r = run(list);
    expect(r.code).toBe(0);
    expect(r.out).toContain("clean");
  });
  test("a match in the diff or a message fails with counts and never prints the term", () => {
    const list = join(listDir, "list.txt");
    writeFileSync(join(repo, "c.txt"), `contains ${TERM.toUpperCase()}\n`);
    git("add", ".");
    git("commit", "-q", "-m", `feat: mentions ${TERM}`);
    const r = run(list);
    expect(r.code).toBe(1);
    expect(r.out).toContain("1 commit message line(s)");
    expect(r.out).toContain("1 added line(s)");
    expect(r.out.toLowerCase()).not.toContain(TERM);
  });
});
