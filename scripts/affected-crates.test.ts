import { describe, expect, test } from "bun:test";
import {
  affectedCrates,
  nextestArgs,
  parseMetadata,
  type Crate,
} from "./affected-crates";

const crates: Crate[] = [
  { name: "domain", dir: "crates/domain", deps: [] },
  { name: "store", dir: "crates/store", deps: ["domain"] },
  { name: "store_sqlite", dir: "crates/store_sqlite", deps: ["store", "domain"] },
  { name: "slack_api", dir: "crates/providers/slack/api", deps: ["domain"] },
  { name: "parla", dir: "crates/parla", deps: ["store_sqlite", "slack_api"] },
  { name: "tools", dir: "crates/tools", deps: [] },
];

describe("affectedCrates", () => {
  test("a leaf change selects the crate and its reverse dependents", () => {
    const r = affectedCrates(crates, ["crates/store/src/lib.rs"]);
    expect(r).toEqual({ kind: "crates", names: ["parla", "store", "store_sqlite"] });
  });
  test("a core change reaches everything that depends on it transitively", () => {
    const r = affectedCrates(crates, ["crates/domain/src/a.rs"]);
    expect(r).toEqual({
      kind: "crates",
      names: ["domain", "parla", "slack_api", "store", "store_sqlite"],
    });
  });
  test("nested crate directories use the longest prefix", () => {
    const r = affectedCrates(crates, ["crates/providers/slack/api/src/x.rs"]);
    expect(r).toEqual({ kind: "crates", names: ["parla", "slack_api"] });
  });
  test("workspace-wide files mean all crates", () => {
    for (const f of ["Cargo.toml", "Cargo.lock", ".config/nextest.toml", "rust-toolchain.toml", "mise.toml"]) {
      expect(affectedCrates(crates, [f, "crates/tools/a.rs"])).toEqual({ kind: "all" });
    }
  });
  test("non-Rust changes run nothing", () => {
    const r = affectedCrates(crates, ["site/src/a.astro", "docs/x.md", "scripts/a.ts", "crates/store/README.md"]);
    expect(r).toEqual({ kind: "none" });
  });
  test("an unclaimed Rust input fails safe to all", () => {
    expect(affectedCrates(crates, ["assets/font.ttf"])).toEqual({ kind: "all" });
  });
  test("no files means nothing changed", () => {
    expect(affectedCrates(crates, [])).toEqual({ kind: "none" });
  });
});

describe("nextestArgs", () => {
  test("one -p per package", () => {
    expect(nextestArgs({ kind: "crates", names: ["a", "b"] })).toEqual([
      "nextest", "run", "--no-tests=pass", "-p", "a", "-p", "b",
    ]);
  });
  test("all means the workspace", () => {
    expect(nextestArgs({ kind: "all" })).toEqual(["nextest", "run", "--no-tests=pass", "--workspace"]);
  });
  test("none runs nothing", () => {
    expect(nextestArgs({ kind: "none" })).toBeNull();
  });
});

describe("parseMetadata", () => {
  test("keeps workspace crates and relative dirs, ignores external deps", () => {
    const meta = {
      workspace_root: "/r",
      packages: [
        { name: "a", manifest_path: "/r/crates/a/Cargo.toml", dependencies: [{ name: "serde" }] },
        { name: "b", manifest_path: "/r/crates/b/Cargo.toml", dependencies: [{ name: "a" }] },
      ],
    };
    expect(parseMetadata(meta)).toEqual([
      { name: "a", dir: "crates/a", deps: [] },
      { name: "b", dir: "crates/b", deps: ["a"] },
    ]);
  });
});
