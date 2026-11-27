import { describe, expect, test } from "bun:test";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  createSnapshot,
  docsFromRef,
  insertVersion,
  main,
  readVersionList,
} from "../scripts/docs-version";

function site(files: Record<string, string>) {
  const root = mkdtempSync(join(tmpdir(), "parla-docs-version-"));
  mkdirSync(join(root, "content/docs"), { recursive: true });
  for (const [name, text] of Object.entries(files)) {
    writeFileSync(join(root, "content/docs", name), text);
  }
  writeFileSync(join(root, "content/versions.json"), '{ "versions": [] }\n');
  return root;
}

describe("insertVersion", () => {
  test("keeps newest first, numerically", () => {
    expect(insertVersion([], "0.1")).toEqual(["0.1"]);
    expect(insertVersion(["0.2", "0.1"], "0.10")).toEqual([
      "0.10",
      "0.2",
      "0.1",
    ]);
    expect(insertVersion(["0.10", "0.1"], "0.2")).toEqual([
      "0.10",
      "0.2",
      "0.1",
    ]);
    expect(insertVersion(["0.2"], "0.1")).toEqual(["0.2", "0.1"]);
    expect(insertVersion(["1.0", "0.9"], "0.9")).toEqual(["1.0", "0.9"]);
  });

  test("rejects malformed ids", () => {
    expect(() => insertVersion([], "0.1.2")).toThrow(/invalid docs version/);
  });
});

describe("createSnapshot", () => {
  test("copies docs files and records the line", () => {
    const root = site({
      "index.mdx": "a",
      "index.es.mdx": "b",
      "meta.json": "{}",
      "meta.es.json": "{}",
      "notes.txt": "skip",
    });
    expect(createSnapshot(root, "0.1")).toEqual(["0.1"]);
    expect(readdirSync(join(root, "content/versions/0.1")).sort()).toEqual([
      "index.es.mdx",
      "index.mdx",
      "meta.es.json",
      "meta.json",
    ]);
    expect(readVersionList(root)).toEqual(["0.1"]);
  });

  test("a second run for the same line replaces the snapshot", () => {
    const root = site({ "index.mdx": "old", "gone.mdx": "x" });
    createSnapshot(root, "0.1");
    writeFileSync(join(root, "content/docs/index.mdx"), "new");
    rmSync(join(root, "content/docs/gone.mdx"));
    expect(createSnapshot(root, "0.1")).toEqual(["0.1"]);
    const dir = join(root, "content/versions/0.1");
    expect(readFileSync(join(dir, "index.mdx"), "utf8")).toBe("new");
    expect(existsSync(join(dir, "gone.mdx"))).toBe(false);
  });

  test("a new line goes first", () => {
    const root = site({ "index.mdx": "a" });
    createSnapshot(root, "0.1");
    expect(createSnapshot(root, "0.2")).toEqual(["0.2", "0.1"]);
  });

  test("main reports usage errors", () => {
    const root = site({ "index.mdx": "a" });
    const quiet = console.error;
    console.error = () => {};
    try {
      expect(main([], root)).toBe(2);
      expect(main(["x"], root)).toBe(2);
      expect(main(["0.1", "--ref"], root)).toBe(2);
      expect(main(["0.1"], root)).toBe(0);
    } finally {
      console.error = quiet;
    }
  });
});

describe("docsFromRef", () => {
  test("takes the docs of a git ref, not the working tree", () => {
    const repo = mkdtempSync(join(tmpdir(), "parla-docs-ref-repo-"));
    const run = (...cmd: string[]) => {
      const proc = Bun.spawnSync(cmd, {
        cwd: repo,
        stdout: "pipe",
        stderr: "pipe",
      });
      expect(proc.exitCode).toBe(0);
    };
    const root = join(repo, "site");
    mkdirSync(join(root, "content/docs"), { recursive: true });
    writeFileSync(join(root, "content/docs/index.mdx"), "released");
    run("git", "init", "-q");
    run("git", "add", ".");
    run(
      "git",
      "-c",
      "user.name=t",
      "-c",
      "user.email=t@example.com",
      "-c",
      "commit.gpgsign=false",
      "commit",
      "-q",
      "-m",
      "docs",
    );
    run("git", "tag", "v0.1.0");
    writeFileSync(join(root, "content/docs/index.mdx"), "unreleased");

    const dir = docsFromRef(root, "v0.1.0");
    expect(readFileSync(join(dir, "index.mdx"), "utf8")).toBe("released");
    writeFileSync(join(root, "content/versions.json"), '{ "versions": [] }\n');
    createSnapshot(root, "0.1", dir);
    expect(
      readFileSync(join(root, "content/versions/0.1/index.mdx"), "utf8"),
    ).toBe("released");
  });

  test("an unknown ref is a clear error", () => {
    const repo = mkdtempSync(join(tmpdir(), "parla-docs-ref-bad-"));
    Bun.spawnSync(["git", "init", "-q"], { cwd: repo });
    expect(() => docsFromRef(repo, "nope")).toThrow();
  });
});
