import { describe, expect, test } from "bun:test";
import { checkManifest, outputsOf, type ShotConfig } from "./site-shots";

const config: ShotConfig = {
  platform: "macos",
  shots: [
    {
      output: "thread",
      scenario: "thread_open",
      appearances: ["light", "dark"],
      crop: null,
    },
  ],
};
const manifest = {
  shots: {
    "thread-light.webp": { scenario: "thread_open", appearance: "light", source: "s-light", hash: "h1" },
    "thread-dark.webp": { scenario: "thread_open", appearance: "dark", source: "s-dark", hash: "h2" },
  },
};
const hashes = new Map([
  ["s-light", "h1"],
  ["s-dark", "h2"],
]);
const hashOf = (p: string) => hashes.get(p) ?? null;
const sourceOf = (s: string, a: string) => `s-${a}`;

describe("outputsOf", () => {
  test("one file per appearance", () => {
    expect(outputsOf(config).map((o) => o.file)).toEqual([
      "thread-light.webp",
      "thread-dark.webp",
    ]);
  });
});

describe("checkManifest", () => {
  test("clean when every hash matches", () => {
    expect(checkManifest(config, manifest, hashOf, sourceOf)).toEqual([]);
  });
  test("a changed baseline fails and names the fix", () => {
    hashes.set("s-dark", "zzz");
    const e = checkManifest(config, manifest, hashOf, sourceOf);
    hashes.set("s-dark", "h2");
    expect(e.join("\n")).toMatch(/thread-dark\.webp/);
    expect(e.join("\n")).toMatch(/mise run site:shots/);
  });
  test("a missing baseline fails", () => {
    const e = checkManifest(config, manifest, () => null, sourceOf);
    expect(e.length).toBe(2);
  });
  test("a shot missing from the manifest fails", () => {
    const e = checkManifest(config, { shots: {} }, hashOf, sourceOf);
    expect(e.join("\n")).toMatch(/not in the manifest/);
  });
  test("a manifest entry no longer in the config fails", () => {
    const m = { shots: { ...manifest.shots, "old-light.webp": { scenario: "x", appearance: "light", source: "s", hash: "h" } } };
    expect(checkManifest(config, m, hashOf, sourceOf).join("\n")).toMatch(/old-light/);
  });
});
