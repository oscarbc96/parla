import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const guide = readFileSync("../CONTRIBUTING.md", "utf8");

describe("CONTRIBUTING.md translation guide", () => {
  const section = guide
    .split(/^## /m)
    .find((s) => s.startsWith("Translations"));
  test("has a Translations section", () => {
    expect(section).toBeDefined();
  });
  // Every file kind the checks require when a locale is added.
  for (const kind of [
    "i18n/locales.toml",
    "site/src/i18n/config.json",
    "crates/**/i18n/<code>.ftl",
    "crates/parla/macos/<code>.lproj/InfoPlist.strings",
    "site/src/i18n/<code>.ts",
    "site/content/docs/*.<code>.mdx",
    "meta.<code>.json",
    "i18n-draft",
    "mise run i18n:check",
  ]) {
    test(`mentions ${kind}`, () => {
      expect(section ?? "").toContain(kind);
    });
  }
});
