import { describe, expect, test } from "bun:test";

const docs = [
  "",
  "install/",
  "updates/",
  "uninstall/",
  "sign-in/",
  "settings/",
  "keymap/",
  "troubleshooting/",
  "privacy/",
  "terms/",
];

describe("built site", () => {
  test("home has no Slack assets", async () => {
    const html = await Bun.file("dist/index.html").text();
    expect(html).toContain('<html lang="en"');
    expect(html).not.toMatch(/slack[-_]?(logo|mark)/i);
  });

  test("docs pages built", async () => {
    for (const page of [
      "docs/index.html",
      "docs/install/index.html",
      "docs/updates/index.html",
      "docs/uninstall/index.html",
      "docs/settings/index.html",
      "docs/keymap/index.html",
      "docs/troubleshooting/index.html",
      "docs/sign-in/index.html",
    ]) {
      expect(await Bun.file(`dist/${page}`).exists()).toBe(true);
    }
  });

  test("english docs keep lang en", async () => {
    for (const page of docs) {
      const html = await Bun.file(`dist/docs/${page}index.html`).text();
      expect(html).toContain('<html lang="en"');
    }
  });

  test("spanish home has lang es", async () => {
    const html = await Bun.file("dist/es/index.html").text();
    expect(html).toContain('<html lang="es"');
    expect(html).not.toMatch(/slack[-_]?(logo|mark)/i);
  });

  test("spanish docs pages built with lang es", async () => {
    for (const page of docs) {
      const html = await Bun.file(`dist/es/docs/${page}index.html`).text();
      expect(html).toContain('<html lang="es"');
    }
  });

  test("catalan home has lang ca", async () => {
    const html = await Bun.file("dist/ca/index.html").text();
    expect(html).toContain('<html lang="ca"');
    expect(html).not.toMatch(/slack[-_]?(logo|mark)/i);
  });

  test("catalan docs pages built with lang ca", async () => {
    for (const page of docs) {
      const html = await Bun.file(`dist/ca/docs/${page}index.html`).text();
      expect(html).toContain('<html lang="ca"');
    }
  });

  test("catalan changelog page builds with lang ca", async () => {
    const html = await Bun.file("dist/ca/changelog/index.html").text();
    expect(html).toContain('<html lang="ca"');
  });

  test("language switcher lists every locale", async () => {
    const html = await Bun.file("dist/index.html").text();
    for (const [code, name] of [
      ["en", "English"],
      ["es", "Español"],
      ["ca", "Català"],
    ]) {
      expect(html).toContain(name);
      expect(html).toContain(
        code === "en" ? 'hreflang="en"' : `/parla/${code}/`,
      );
    }
  });

  test("no unofficial notice anywhere on the site", async () => {
    for (const file of [
      "dist/index.html",
      "dist/es/index.html",
      "dist/ca/index.html",
      "dist/docs/index.html",
      "dist/es/docs/index.html",
      "dist/ca/docs/index.html",
      "dist/changelog/index.html",
      "dist/downloads/index.html",
    ]) {
      const html = await Bun.file(file).text();
      expect(html).not.toContain("Slack Technologies, LLC");
      expect(html).not.toMatch(/unofficial-notice/);
    }
  });

  test("terms pages build with the notice in every locale", async () => {
    for (const [file, lang] of [
      ["dist/docs/terms/index.html", "en"],
      ["dist/es/docs/terms/index.html", "es"],
      ["dist/ca/docs/terms/index.html", "ca"],
    ]) {
      const html = await Bun.file(file).text();
      expect(html).toContain(`<html lang="${lang}"`);
      expect(html).toContain("Slack Technologies, LLC");
    }
  });

  test("settings schema published", async () => {
    expect(await Bun.file("dist/settings.schema.json").exists()).toBe(true);
  });

  test("actions snapshot published", async () => {
    const text = await Bun.file("dist/actions.json").text();
    const actions = JSON.parse(text);
    expect(Array.isArray(actions)).toBe(true);
    expect(actions.length).toBeGreaterThan(0);
  });

  test("llms.txt and llms-full.txt list the docs pages in every locale", async () => {
    for (const [prefix, base, title] of [
      ["", "/parla/docs", "[Install]"],
      ["es/", "/parla/es/docs", "[Instalación]"],
      ["ca/", "/parla/ca/docs", "[Instal·lació]"],
    ]) {
      const index = await Bun.file(`dist/${prefix}llms.txt`).text();
      const full = await Bun.file(`dist/${prefix}llms-full.txt`).text();
      for (const page of ["install", "sign-in", "settings", "privacy"]) {
        expect(index).toContain(`](${base}/${page}.md)`);
      }
      expect(index).toContain(title);
      expect(full).toContain(`(${base}/install)`);
      expect(full).toContain(`(${base}/settings)`);
    }
  });

  test("markdown pages are served per locale without JSX", async () => {
    for (const file of [
      "docs/index.md",
      "docs/install.md",
      "docs/settings.md",
      "es/docs/index.md",
      "es/docs/settings.md",
      "ca/docs/index.md",
      "ca/docs/settings.md",
    ]) {
      const md = await Bun.file(`dist/${file}`).text();
      expect(md.startsWith("# ")).toBe(true);
      // no React/MDX component tags (lowercase tags inside code spans are fine)
      const outsideCode = md
        .replace(/```[\s\S]*?```/g, "")
        .replace(/`[^`]*`/g, "");
      expect(outsideCode).not.toMatch(/<[A-Z][A-Za-z]*[\s/>]/);
      expect(outsideCode).not.toMatch(/^\s*import .* from /m);
    }
  });

  test("docs pages offer the markdown page actions", async () => {
    const html = await Bun.file("dist/docs/install/index.html").text();
    expect(html).toContain("/parla/docs/install.md");
    expect(html).toContain(
      "github.com/oscarbc96/parla/blob/main/site/content/docs/install",
    );
  });

  test("footer links llms.txt", async () => {
    expect(await Bun.file("dist/index.html").text()).toContain(
      'href="/parla/llms.txt"',
    );
  });

  test("footer credits the site's own stack", async () => {
    const html = await Bun.file("dist/index.html").text();
    expect(html).toContain("This site is built with Astro and Fumadocs.");
  });

  test("docs pages have no site footer", async () => {
    for (const file of [
      "docs/index.html",
      "docs/install/index.html",
      "es/docs/index.html",
      "ca/docs/index.html",
    ]) {
      const html = await Bun.file(`dist/${file}`).text();
      expect(html).not.toContain("<footer");
    }
  });

  test("home feature cards match the UI strings", async () => {
    const { en } = await import("../src/i18n/en");
    const html = await Bun.file("dist/index.html").text();
    const cards = html.match(/<li class="flex flex-col rounded/g)?.length;
    expect(cards).toBe(en.home.features.length);
    expect(cards).toBe(9);
    for (const feature of en.home.features) {
      expect(html).toContain(feature.title);
      expect(html).toContain(feature.text);
      if (feature.linkText) expect(html).toContain(feature.linkText);
    }
  });

  test("home links each feature card to its docs page", async () => {
    const html = await Bun.file("dist/index.html").text();
    for (const href of [
      "/parla/docs/install/",
      "/parla/docs/offline/",
      "/parla/docs/messaging/",
      "/parla/docs/settings/",
      "/parla/docs/files/",
      "/parla/docs/notifications/",
      "/parla/docs/sign-in/",
      "/parla/docs/privacy/",
      "/parla/downloads/",
    ]) {
      expect(html).toContain(`href="${href}"`);
    }
  });

  test("home shows the hero shot and the gallery in every locale", async () => {
    const { en } = await import("../src/i18n/en");
    for (const prefix of ["", "es/", "ca/"]) {
      const html = await Bun.file(`dist/${prefix}index.html`).text();
      for (const shot of ["thread", ...en.home.gallery.map((g) => g.shot)]) {
        expect(html).toContain(`src="/parla/shots/${shot}-light.webp"`);
        expect(html).toContain(`src="/parla/shots/${shot}-dark.webp"`);
      }
    }
    for (const shot of ["thread", "search", "switcher", "huddle"]) {
      expect(await Bun.file(`dist/shots/${shot}-light.webp`).exists()).toBe(
        true,
      );
      expect(await Bun.file(`dist/shots/${shot}-dark.webp`).exists()).toBe(
        true,
      );
    }
  });
});
