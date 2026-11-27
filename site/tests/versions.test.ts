import { beforeAll, describe, expect, test } from "bun:test";
import { buildSite } from "./build";

const out = "dist-tests/versions";

beforeAll(async () => {
  await buildSite(out, {
    PARLA_CHANGELOG_FIXTURE: "tests/fixtures/no-releases.json",
    PARLA_STARS_FIXTURE: "none",
    PARLA_DOCS_VERSIONS_FIXTURE: "tests/fixtures/versions",
  });
}, 240_000);

const read = (file: string) => Bun.file(`${out}/${file}`).text();
const exists = (file: string) => Bun.file(`${out}/${file}`).exists();

describe("versioned docs from a fixture", () => {
  test("/docs/ is the newest line", async () => {
    const html = await read("docs/index.html");
    expect(html).toContain("version zero point two landing page");
    expect(html).toContain(
      '<link rel="canonical" href="https://oscarbc96.github.io/parla/docs/"',
    );
  });

  test("/docs/next/ is the working docs", async () => {
    const html = await read("docs/next/install/index.html");
    expect(html).toContain("<title>Install | Parla</title>");
    expect(html).not.toContain("version zero point two");
    expect(await exists("docs/next/index.html")).toBe(true);
  });

  test("old lines live under /docs/v<X.Y>/", async () => {
    expect(await read("docs/v0.1/index.html")).toContain(
      "version zero point one landing page",
    );
    expect(await read("docs/v0.1/old-only/index.html")).toContain(
      "only in version zero point one",
    );
    expect(await exists("docs/v0.1/install/index.html")).toBe(false);
  });

  test("a translated old line renders in that language", async () => {
    const html = await read("es/docs/v0.1/index.html");
    expect(html).toContain('<html lang="es"');
    expect(html).toContain("cero punto uno");
  });

  test("a locale missing from a snapshot falls back to English", async () => {
    const html = await read("ca/docs/v0.1/index.html");
    expect(html).toContain('<html lang="ca"');
    expect(html).toContain("version zero point one landing page");
    expect(await exists("ca/docs/v0.1/old-only/index.html")).toBe(true);
  });

  test("next and old lines are noindex without a canonical", async () => {
    for (const file of [
      "docs/next/index.html",
      "docs/v0.1/index.html",
      "es/docs/v0.1/index.html",
      "ca/docs/next/install/index.html",
    ]) {
      const html = await read(file);
      expect(html).toContain('<meta name="robots" content="noindex"');
      expect(html).not.toContain('rel="canonical"');
    }
    expect(await read("docs/index.html")).not.toContain("noindex");
  });

  test("the sitemap lists only the default version", async () => {
    const xml = await read("sitemap.xml");
    expect(xml).toContain("/parla/docs/");
    expect(xml).not.toContain("/docs/next/");
    expect(xml).not.toContain("/docs/v0.1/");
  });

  test("the switcher links to the same page, or the version index", async () => {
    const html = await read("docs/install/index.html");
    expect(html).toContain("parla-version-switcher");
    expect(html).toContain('href="/parla/docs/next/install/"');
    expect(html).toContain('href="/parla/docs/v0.1/"');
    expect(html).toContain('href="/parla/docs/install/" aria-current="page"');
    expect(html).toContain("Unreleased");
    expect(html).toContain("latest");
  });

  test("Markdown routes exist per version", async () => {
    expect(await read("docs/install.md")).toContain(
      "version zero point two install",
    );
    expect(await exists("docs/next/install.md")).toBe(true);
    expect(await exists("docs/next/index.md")).toBe(true);
    expect(await read("docs/v0.1/old-only.md")).toContain(
      "only in version zero point one",
    );
    expect(await exists("es/docs/v0.1/index.md")).toBe(true);
  });

  test("banners show on next and old lines only", async () => {
    expect(await read("docs/next/index.html")).toContain(
      "These docs describe unreleased changes.",
    );
    const old = await read("docs/v0.1/index.html");
    expect(old).toContain("These docs are for Parla 0.1.");
    expect(old).toContain('href="/parla/docs/"');
    expect(await read("docs/index.html")).not.toContain("These docs are for");
    expect(await read("docs/index.html")).not.toContain("unreleased changes");
    expect(await read("es/docs/v0.1/index.html")).toContain(
      "Esta documentación es de Parla 0.1.",
    );
  });

  test("llms.txt covers only the default version", async () => {
    const llms = await read("llms.txt");
    expect(llms).not.toContain("/docs/next/");
    expect(llms).not.toContain("/docs/v0.1/");
    expect(llms).toContain("Install 0.2");
  });

  test("search has one index per version, with that version's pages", async () => {
    const def = await read("api/search.json");
    const next = await read("api/search/next.json");
    const old = await read("api/search/v0.1.json");
    expect(def).toContain("/parla/docs/install");
    expect(def).not.toContain("old-only");
    expect(def).not.toContain("/docs/next/");
    expect(next).toContain("/parla/docs/next/install");
    expect(next).not.toContain("old-only");
    expect(old).toContain("old-only");
    expect(old).toContain("/parla/docs/v0.1/old-only");
    expect(old).not.toContain("/parla/docs/install");
    expect(old).not.toContain("/docs/next/");
    expect(await exists("api/search/v0.2.json")).toBe(false);
  });

  test("the search dialog tag labels come with the docs pages", async () => {
    const html = await read("docs/v0.1/index.html");
    expect(html).toContain("Searching {version}");
    expect(html).toContain("Unreleased");
    const es = await read("es/docs/v0.1/index.html");
    expect(es).toContain("Buscando en {version}");
    expect(await read("ca/docs/next/install/index.html")).toContain(
      "Cercant a {version}",
    );
  });
});

describe("plain build without versions", () => {
  test("has no switcher, banner or /docs/next/", async () => {
    const html = await Bun.file("dist/docs/install/index.html").text();
    expect(html).not.toContain("parla-version-switcher");
    expect(html).not.toContain("Searching {version}");
    expect(html).not.toContain("These docs describe unreleased changes");
    expect(html).not.toContain('content="noindex"');
    expect(await Bun.file("dist/docs/next/index.html").exists()).toBe(false);
  });
});
