// Covers release-please-config.json, the release manifest and the workspace versions.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const root = join(import.meta.dir, "..");
const read = (p: string) => readFileSync(join(root, p), "utf8");
const config = JSON.parse(read("release-please-config.json"));
const manifest = JSON.parse(read(".release-please-manifest.json"));
const cargo = Bun.TOML.parse(read("Cargo.toml")) as any;
const lock = Bun.TOML.parse(read("Cargo.lock")) as any;

describe("release-please config", () => {
  test("changelog sections show feat, fix, perf and docs and hide the rest", () => {
    const visible = config["changelog-sections"]
      .filter((s: any) => !s.hidden)
      .map((s: any) => s.type)
      .sort();
    expect(visible).toEqual(["docs", "feat", "fix", "perf", "revert"]);
    const hidden = config["changelog-sections"]
      .filter((s: any) => s.hidden)
      .map((s: any) => s.type)
      .sort();
    expect(hidden).toEqual(["build", "chore", "ci", "refactor", "test"]);
  });
  test("tags are vX.Y.Z with no component; draft and force-tag-creation are on", () => {
    expect(config["release-type"]).toBe("simple");
    expect(config["include-v-in-tag"]).toBe(true);
    expect(config["include-component-in-tag"]).toBe(false);
    expect(config.draft).toBe(true);
    expect(config["force-tag-creation"]).toBe(true);
  });
  test("stable phase: no prerelease keys", () => {
    for (const k of ["prerelease", "versioning", "prerelease-type", "release-as"]) {
      expect(config[k]).toBeUndefined();
    }
  });
  test("v0.0.1 shipped, so no release-as pin keeps proposing it again", () => {
    expect(config.packages["."]["initial-version"]).toBe("0.0.1");
    expect(config.packages["."]["release-as"]).toBeUndefined();
    expect((JSON.parse(readFileSync(join(root, ".release-please-manifest.json"), "utf8")) as any)["."]).toBe("0.0.1");
  });
  test("no bootstrap-sha: the repo has no earlier release history to skip", () => {
    expect(config["bootstrap-sha"]).toBeUndefined();
  });
});

describe("versions", () => {
  test("manifest version equals the workspace version and every Cargo.lock workspace member", () => {
    const v = cargo.workspace.package.version;
    expect(manifest["."]).toBe(v);
    const members = lock.package.filter((p: any) => !p.source);
    expect(members.length).toBeGreaterThan(0);
    for (const p of members) expect(p.version).toBe(v);
  });
  test("extra-files target the workspace version and sourceless lock packages", () => {
    const files = config.packages["."]["extra-files"];
    expect(files).toContainEqual({
      type: "toml",
      path: "Cargo.toml",
      jsonpath: "$.workspace.package.version",
    });
    expect(files).toContainEqual({
      type: "toml",
      path: "Cargo.lock",
      jsonpath: "$.package[?(!@.source)].version",
    });
  });
});
