import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  FIXTURE_ENV,
  loadVersions,
  metaCollectionName,
  resolveVersions,
} from "../src/lib/docs-versions";

describe("resolveVersions", () => {
  test("no versions: working docs are served at /docs", () => {
    expect(resolveVersions([])).toEqual([
      {
        id: "next",
        segment: null,
        collection: "docs",
        dir: "content/docs",
        latest: true,
        unreleased: false,
      },
    ]);
  });

  test("with versions: next, latest at /docs, older under /v<X.Y>", () => {
    const versions = resolveVersions(["0.10", "0.2", "0.1"]);
    expect(versions.map((v) => [v.id, v.segment])).toEqual([
      ["next", "next"],
      ["0.10", null],
      ["0.2", "v0.2"],
      ["0.1", "v0.1"],
    ]);
    expect(versions[0]).toMatchObject({
      collection: "docs",
      dir: "content/docs",
      latest: false,
      unreleased: true,
    });
    expect(versions[1]).toMatchObject({
      collection: "v0_10",
      dir: "content/versions/0.10",
      latest: true,
      unreleased: false,
    });
    expect(versions[2]!.latest).toBe(false);
  });

  test("a fixture dir replaces the snapshot root", () => {
    const versions = resolveVersions(["0.2"], "tests/fixtures/versions/");
    expect(versions[1]!.dir).toBe("tests/fixtures/versions/0.2");
    expect(versions[0]!.dir).toBe("content/docs");
  });

  test("meta collection names", () => {
    const [next, latest] = resolveVersions(["0.2"]);
    expect(metaCollectionName(next!)).toBe("meta");
    expect(metaCollectionName(latest!)).toBe("v0_2_meta");
  });

  test("rejects malformed ids", () => {
    for (const bad of ["1", "1.2.3", "v1.2", "next", "a.b", ""]) {
      expect(() => resolveVersions([bad])).toThrow(/invalid docs version/);
    }
  });

  test("rejects duplicates and wrong order", () => {
    expect(() => resolveVersions(["0.2", "0.2"])).toThrow(/duplicate/);
    expect(() => resolveVersions(["0.1", "0.2"])).toThrow(/newest first/);
    expect(() => resolveVersions(["0.2", "0.10"])).toThrow(/newest first/);
  });
});

describe("loadVersions", () => {
  const root = mkdtempSync(join(tmpdir(), "parla-versions-"));
  mkdirSync(join(root, "content"), { recursive: true });

  test("missing versions.json means no versions", () => {
    expect(loadVersions(root, {}).map((v) => v.id)).toEqual(["next"]);
  });

  test("reads content/versions.json", () => {
    writeFileSync(
      join(root, "content/versions.json"),
      JSON.stringify({ versions: ["0.2", "0.1"] }),
    );
    expect(loadVersions(root, {}).map((v) => v.id)).toEqual([
      "next",
      "0.2",
      "0.1",
    ]);
  });

  test("honours the fixture env", () => {
    mkdirSync(join(root, "fx"), { recursive: true });
    writeFileSync(join(root, "fx/versions.json"), JSON.stringify(["0.5"]));
    const versions = loadVersions(root, { [FIXTURE_ENV]: "fx" });
    expect(versions.map((v) => v.id)).toEqual(["next", "0.5"]);
    expect(versions[1]!.dir).toBe("fx/0.5");
  });

  test("invalid json throws a clear error", () => {
    writeFileSync(join(root, "content/versions.json"), "{ nope");
    expect(() => loadVersions(root, {})).toThrow(/versions\.json/);
  });
});
