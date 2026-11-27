import {
  copyFile,
  cp,
  glob,
  mkdir,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "astro/config";
import react from "@astrojs/react";
import fumadocs from "@fumadocs/astro";
import tailwindcss from "@tailwindcss/vite";
import i18nConfig from "./src/i18n/config.json" with { type: "json" };
import { loadVersions } from "./src/lib/docs-versions.ts";

const root = dirname(fileURLToPath(import.meta.url));
/** One Fumadocs collection per docs version (working docs plus frozen snapshots). */
const docsCollections = Object.fromEntries(
  loadVersions(root).map((version) => [
    version.collection,
    { dir: version.dir },
  ]),
);
const schemaSource = resolve(
  root,
  "../crates/settings/tests/snapshots/settings.schema.json",
);
const schemaTarget = resolve(root, "public/settings.schema.json");
const actionsSource = resolve(
  root,
  "../crates/parla/tests/snapshots/actions.json",
);
const actionsTarget = resolve(root, "public/actions.json");

const placeholderSchema = {
  $schema: "https://json-schema.org/draft/2020-12/schema",
  title: "Parla settings",
  type: "object",
};

async function copySettingsSchema() {
  await mkdir(dirname(schemaTarget), { recursive: true });
  if (existsSync(schemaSource)) {
    await copyFile(schemaSource, schemaTarget);
    return;
  }
  await writeFile(
    schemaTarget,
    `${JSON.stringify(placeholderSchema, null, 2)}\n`,
    "utf8",
  );
}

async function copyActionsSnapshot() {
  await mkdir(dirname(actionsTarget), { recursive: true });
  if (existsSync(actionsSource)) {
    await copyFile(actionsSource, actionsTarget);
    return;
  }
  await writeFile(actionsTarget, "[]\n", "utf8");
}

/** Gives the link checker a view where the /parla base path maps to the build output. */
async function prepareLinkinator({ dir: built }) {
  // Test builds write elsewhere (for example dist-tests/); only the real one is link-checked.
  if (fileURLToPath(built).replace(/\/$/, "") !== resolve(root, "dist")) return;
  const dir = resolve(root, ".linkinator");
  const output = resolve(root, "dist");
  await rm(dir, { recursive: true, force: true });
  await mkdir(dir, { recursive: true });
  await cp(output, resolve(dir, "dist"), { recursive: true });
  await cp(output, resolve(dir, "parla"), { recursive: true });
}

/**
 * The Fumadocs integration injects a view-transition script on every page, even
 * those with no island and no client router. Drop it from pages that hydrate
 * nothing so they ship zero JavaScript files.
 */
function stripIdlePageScript() {
  return {
    name: "parla-strip-idle-page-script",
    hooks: {
      "astro:build:done": async ({ dir }) => {
        const root = fileURLToPath(dir);
        const files = await glob("**/*.html", { cwd: root });
        for await (const rel of files) {
          const file = resolve(root, rel);
          const html = await readFile(file, "utf8");
          if (html.includes("<astro-island")) continue;
          const next = html.replace(
            /<script type="module" src="[^"]*\/_astro\/page\.[^"]+\.js"><\/script>/g,
            "",
          );
          if (next !== html) await writeFile(file, next, "utf8");
        }
      },
    },
  };
}

/** Publishes the settings schema and actions snapshot, with placeholders when absent. */
function settingsSchema() {
  return {
    name: "parla-settings-schema",
    hooks: {
      "astro:config:setup": async () => {
        await copySettingsSchema();
        await copyActionsSnapshot();
      },
      "astro:build:start": async () => {
        await copySettingsSchema();
        await copyActionsSnapshot();
      },
      "astro:build:done": prepareLinkinator,
    },
  };
}

export default defineConfig({
  output: "static",
  site: "https://oscarbc96.github.io",
  base: "/parla",
  trailingSlash: "always",
  build: { inlineStylesheets: "auto" },
  i18n: {
    defaultLocale: i18nConfig.defaultLocale,
    locales: i18nConfig.locales.map((locale) => locale.code),
    routing: { prefixDefaultLocale: false },
  },
  integrations: [
    react(),
    fumadocs({
      collections: docsCollections,
      components: "./src/components/mdx.tsx",
      includeProcessedMarkdown: true,
    }),
    settingsSchema(),
    stripIdlePageScript(),
  ],
  vite: {
    plugins: [tailwindcss()],
  },
});
