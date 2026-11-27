import { describe, expect, test } from "bun:test";
import { ALL, classify, prePushTasks } from "./changes";

const tasks = (files: string[]) => prePushTasks(classify(files), files);

describe("classify", () => {
  test("a site-only change runs no Rust work", () => {
    const c = classify(["site/src/components/Home.astro", "site/content/docs/index.mdx"]);
    expect(c).toEqual({ rust: false, site: true, deps: false, package: false });
  });
  test("a Rust-only change runs no site work", () => {
    const c = classify(["crates/store/src/lib.rs", "crates/parla/src/main.rs"]);
    expect(c.rust).toBe(true);
    expect(c.site).toBe(false);
  });
  test("a docs-only change runs neither", () => {
    const c = classify(["docs/licensing.md", "README.md"]);
    expect(c).toEqual({ rust: false, site: false, deps: false, package: false });
  });
  test("catalogs and the lockfile reach both sides", () => {
    expect(classify(["crates/components/i18n/en.ftl"])).toMatchObject({ rust: true, site: true });
    expect(classify(["Cargo.lock"])).toMatchObject({ rust: true, deps: true });
    expect(classify(["site/bun.lock"])).toMatchObject({ rust: false, deps: true });
  });
  test("package inputs gate the CI packaging job", () => {
    expect(classify(["crates/parla/src/main.rs"])).toMatchObject({ rust: true, package: true });
    expect(classify(["mise-tasks/release/app"])).toMatchObject({ rust: true, package: true });
    expect(classify(["Cargo.toml"])).toMatchObject({ rust: true, deps: true, package: true });
    expect(classify(["mise.toml"])).toMatchObject({ rust: true, site: true, package: true });
  });
  test("renovate.json runs the repo checks, not the Rust jobs", () => {
    expect(classify(["renovate.json"])).toMatchObject({ rust: false, site: true });
  });
  test("the shared setup action reruns every job that uses it", () => {
    expect(classify([".github/actions/setup/action.yml"])).toMatchObject({ rust: true, site: true, package: true });
  });
  test("only ci.yml is a Rust input among the workflows", () => {
    expect(classify([".github/workflows/pages.yml"])).toMatchObject({ rust: false });
    expect(classify([".github/workflows/ci.yml"])).toMatchObject({ rust: true, site: true, package: true });
    expect(classify([".github/workflows/release.yml"])).toMatchObject({ rust: false, package: true });
  });
  test("repo tooling tests run with the site job, not the Rust jobs", () => {
    expect(classify(["scripts/changes.test.ts", "scripts/changes.ts"])).toMatchObject({ rust: false, site: true });
    expect(classify(["scripts/changes.test.ts"])).toMatchObject({ rust: false, site: true });
    expect(classify(["scripts/perf-measure.sh"])).toMatchObject({ rust: true });
  });
  test("workflow, release config and mise task changes run the repo guard tests", () => {
    for (const f of [
      ".github/workflows/release.yml",
      ".github/workflows/pages.yml",
      ".github/workflows/pr-template.yml",
      ".github/pull_request_template.md",
      "renovate.json",
      "release-please-config.json",
      ".release-please-manifest.json",
      "mise-tasks/release/sums",
    ]) expect(classify([f]).site).toBe(true);
  });
  test("an unknown path or an empty diff fails safe to the Rust checks", () => {
    expect(classify(["somewhere/new.bin"]).rust).toBe(true);
    expect(classify([])).toEqual(ALL);
  });
});

describe("pre-push tasks", () => {
  test("docs-only pushes run nothing", () => {
    expect(tasks(["docs/licensing.md", "CONTRIBUTING.md"])).toEqual([]);
  });
  test("site-only pushes run the site check only", () => {
    expect(tasks(["site/src/i18n/en.ts"])).toEqual(["site:verify"]);
  });
  test("Rust-only pushes run the tests only", () => {
    expect(tasks(["crates/store/src/lib.rs"])).toEqual(["test"]);
  });
  test("tooling pushes run the repo tests; workflow pushes rely on lint (actionlint, zizmor)", () => {
    expect(tasks(["scripts/pr-body.ts"])).toEqual(["test:repo"]);
    expect(tasks([".github/workflows/pages.yml"])).toEqual([]);
    // Release task scripts are Rust inputs too, so they run the tests as well.
    expect(tasks(["mise-tasks/release/app"])).toEqual(["test", "test:repo"]);
  });
  test("mixed pushes run each affected task once, in order", () => {
    expect(tasks(["crates/parla/src/a.rs", "site/src/a.ts", "scripts/changes.test.ts"])).toEqual([
      "test",
      "site:verify",
      "test:repo",
    ]);
  });
});
