import { describe, expect, test } from "bun:test";
import {
  checkChangeSync,
  checkDrafts,
  checkLocaleSets,
  checkParity,
  checkRepoChangeSync,
  countDrafts,
  escapeReason,
  flattenKeys,
} from "../scripts/check-translations";

const fx = (name: string) => `tests/fixtures/translations/${name}`;
const pass = fx("pass");

describe("parity", () => {
  test("passes on a consistent tree", async () => {
    expect(await checkParity(pass)).toEqual([]);
  });
  test("passes on the real site", async () => {
    expect(await checkParity(".")).toEqual([]);
  });
  test("missing locale file", async () => {
    const e = await checkParity(fx("missing-locale-file"));
    expect(e).toEqual([
      expect.stringContaining("intro.es.mdx: missing translation"),
    ]);
  });
  test("orphan translation", async () => {
    const e = await checkParity(fx("orphan"));
    expect(e).toEqual([expect.stringContaining("extra.es.mdx: orphan")]);
  });
  test("meta mismatch", async () => {
    const e = await checkParity(fx("meta-mismatch"));
    expect(e).toEqual([
      expect.stringContaining('meta.es.json: missing page "intro"'),
    ]);
  });
  test("i18n key missing", async () => {
    const e = await checkParity(fx("i18n-key-missing"));
    expect(e).toEqual([
      expect.stringContaining('es.ts: missing key "nav.docs"'),
    ]);
  });
});

describe("change sync", () => {
  const en = "content/docs/intro.mdx";
  const es = "content/docs/intro.es.mdx";
  test("English without es fails", () => {
    expect(checkChangeSync(pass, [en])).toEqual([
      `${en} changed but ${es} did not`,
    ]);
  });
  test("English with es passes", () => {
    expect(checkChangeSync(pass, [en, es])).toEqual([]);
  });
  test("translation only passes", () => {
    expect(checkChangeSync(pass, [es])).toEqual([]);
  });
  test("frozen snapshots and the version list are ignored", () => {
    expect(
      checkChangeSync(pass, [
        "content/versions/0.1/intro.mdx",
        "content/versions/0.1/meta.json",
        "content/versions.json",
      ]),
    ).toEqual([]);
  });
  test("meta and i18n tables need every locale", () => {
    expect(checkChangeSync(pass, ["content/docs/meta.json"])).toHaveLength(1);
    expect(checkChangeSync(pass, ["src/i18n/en.ts"])).toHaveLength(1);
    expect(checkChangeSync(pass, ["src/i18n/en.ts", "src/i18n/es.ts"])).toEqual(
      [],
    );
  });
  test("escape hatch with a reason skips the rule", () => {
    const msg = "fix typo\n\nTranslations-Unchanged: typo fix only";
    expect(checkChangeSync(pass, [en], msg)).toEqual([]);
  });
  test("escape hatch without a reason does not", () => {
    for (const msg of [
      "Translations-Unchanged:",
      "Translations-Unchanged:   \n",
    ]) {
      expect(checkChangeSync(pass, [en], msg)).toHaveLength(1);
    }
  });
});

test("escapeReason", () => {
  expect(escapeReason("a\nTranslations-Unchanged: why\nb")).toBe("why");
  expect(escapeReason("nothing")).toBeNull();
});

test("flattenKeys walks objects and arrays", () => {
  expect(flattenKeys({ a: { b: "x" }, c: [{ d: "y" }] }).sort()).toEqual([
    "a.b",
    "c.0.d",
  ]);
});

describe("locale sets", () => {
  test("passes_on_the_real_repo", () => {
    expect(checkLocaleSets("..")).toEqual([]);
  });
  test("locales_must_match_app", () => {
    expect(checkLocaleSets(fx("locales-mismatch"))).toEqual([
      "config.json: locales differ from i18n/locales.toml (missing ca)",
    ]);
  });
  test("locale_names_must_match_app", () => {
    expect(checkLocaleSets(fx("locales-name-mismatch"))).toEqual([
      expect.stringContaining('name of es is "Spanish"'),
    ]);
  });
});

describe("app catalog sync", () => {
  const en = "crates/core/i18n/en.ftl";
  const es = "crates/core/i18n/es.ftl";
  const root = fx("ftl-sync");
  test("ftl_change_needs_every_locale", () => {
    expect(checkRepoChangeSync(root, [en])).toEqual([
      `${en} changed but ${es} did not`,
    ]);
    expect(checkRepoChangeSync(root, [en, es])).toEqual([]);
  });
  test("ftl_change_escaped", () => {
    const msg = "Translations-Unchanged: comment only";
    expect(checkRepoChangeSync(root, [en], msg)).toEqual([]);
  });
  test("locales_toml_change_needs_site_config", () => {
    expect(checkRepoChangeSync(root, ["i18n/locales.toml"])).toEqual([
      "i18n/locales.toml changed but site/src/i18n/config.json did not",
    ]);
    expect(
      checkRepoChangeSync(root, [
        "i18n/locales.toml",
        "site/src/i18n/config.json",
      ]),
    ).toEqual([]);
  });
});

describe("drafts", () => {
  test("drafts_counted", () => {
    expect(countDrafts(fx("drafts"))).toEqual({ es: 3 });
    expect(countDrafts(fx("ftl-sync"))).toEqual({});
  });
  test("drafts_fail_release", () => {
    const errors = checkDrafts(fx("drafts"));
    expect(errors).toEqual([expect.stringContaining("es: 3 i18n-draft")]);
    expect(checkDrafts(fx("ftl-sync"))).toEqual([]);
  });
});
