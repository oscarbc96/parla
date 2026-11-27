/**
 * Keeps every locale in step with English.
 *
 * 1. Parity (full tree): every English docs page has a file per locale and no
 *    locale has an orphan; meta files list the same pages; UI string tables
 *    have identical key sets.
 * 2. Change sync (diff based): if an English docs page, meta file or UI table
 *    changed, the same entry for every locale must be in the change set too.
 *    A `Translations-Unchanged: <reason>` line in a commit message or PR body
 *    skips this rule (typo or formatting-only English edits).
 *
 * 3. Locale sets: `src/i18n/config.json` lists the same locales as the app's
 *    `i18n/locales.toml`. A changed app catalog `<dir>/i18n/en.ftl` needs
 *    every other locale's `.ftl`; a changed `locales.toml` needs the site
 *    config (same `Translations-Unchanged` escape).
 * 4. Drafts: `i18n-draft` markers (machine translations awaiting review) are
 *    counted per locale; any marker fails the check.
 *
 * Locales come from `src/i18n/config.json`. Run from `site/`:
 *   bun scripts/check-translations.ts                 parity only
 *   bun scripts/check-translations.ts --staged        parity + staged files
 *   bun scripts/check-translations.ts --base <sha>    parity + <sha>...HEAD
 */
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { git } from "../../scripts/lib";

export interface Locales {
  defaultLocale: string;
  /** Non-default locale codes. */
  others: string[];
}

const DOCS = "content/docs";
const I18N = "src/i18n";
const ESCAPE = /^[ \t]*Translations-Unchanged:[ \t]*(\S.*)$/m;

export function loadLocales(root: string): Locales {
  const cfg = JSON.parse(readFileSync(join(root, I18N, "config.json"), "utf8"));
  const codes: string[] = cfg.locales.map((l: { code: string }) => l.code);
  return {
    defaultLocale: cfg.defaultLocale,
    others: codes.filter((c) => c !== cfg.defaultLocale),
  };
}

/** Docs page names present for `locale` (`undefined` = English). */
function pages(root: string, loc: Locales, locale?: string): string[] {
  const suffix = locale ? `.${locale}.mdx` : ".mdx";
  const out: string[] = [];
  for (const f of readdirSync(join(root, DOCS))) {
    if (!f.endsWith(suffix)) continue;
    const name = f.slice(0, -suffix.length);
    // English files must not look like a translation (`page.es.mdx`).
    if (!locale && loc.others.some((o) => name.endsWith(`.${o}`))) continue;
    out.push(name);
  }
  return out.sort();
}

function metaPages(root: string, locale?: string): string[] {
  const file = join(root, DOCS, locale ? `meta.${locale}.json` : "meta.json");
  if (!existsSync(file)) return [];
  return [...JSON.parse(readFileSync(file, "utf8")).pages].sort();
}

/** Dotted key paths of a nested value (arrays use their index). */
export function flattenKeys(value: unknown, prefix = ""): string[] {
  if (value === null || typeof value !== "object") return [prefix];
  return Object.entries(value).flatMap(([k, v]) =>
    flattenKeys(v, prefix ? `${prefix}.${k}` : k),
  );
}

async function uiKeys(root: string, locale: string): Promise<string[] | null> {
  const file = resolve(root, I18N, `${locale}.ts`);
  if (!existsSync(file)) return null;
  const table = (await import(file))[locale];
  if (table === undefined) return null;
  // `translations` maps Fumadocs' English text to a translation; English
  // relies on the built-in defaults, so it is a free-form record, not a schema.
  const { translations: _free, ...schema } = table;
  return flattenKeys(schema).sort();
}

function diff(a: string[], b: string[]) {
  const sb = new Set(b);
  const sa = new Set(a);
  return {
    missing: a.filter((x) => !sb.has(x)),
    extra: b.filter((x) => !sa.has(x)),
  };
}

/** Rule 1. Returns one message per problem. */
export async function checkParity(root: string): Promise<string[]> {
  const loc = loadLocales(root);
  const errors: string[] = [];
  const en = pages(root, loc);
  const enMeta = metaPages(root);
  const enUi = await uiKeys(root, loc.defaultLocale);
  if (!enUi)
    errors.push(`${I18N}/${loc.defaultLocale}.ts: missing or has no export`);

  for (const l of loc.others) {
    const { missing, extra } = diff(en, pages(root, loc, l));
    for (const p of missing)
      errors.push(`${DOCS}/${p}.${l}.mdx: missing translation`);
    for (const p of extra)
      errors.push(`${DOCS}/${p}.${l}.mdx: orphan, no English ${p}.mdx`);

    if (!existsSync(join(root, DOCS, `meta.${l}.json`))) {
      errors.push(`${DOCS}/meta.${l}.json: missing`);
    } else {
      const m = diff(enMeta, metaPages(root, l));
      for (const p of m.missing)
        errors.push(`${DOCS}/meta.${l}.json: missing page "${p}"`);
      for (const p of m.extra)
        errors.push(`${DOCS}/meta.${l}.json: unknown page "${p}"`);
    }

    const keys = await uiKeys(root, l);
    if (!keys) errors.push(`${I18N}/${l}.ts: missing or has no export`);
    else if (enUi) {
      const k = diff(enUi, keys);
      for (const x of k.missing)
        errors.push(`${I18N}/${l}.ts: missing key "${x}"`);
      for (const x of k.extra)
        errors.push(`${I18N}/${l}.ts: unknown key "${x}"`);
    }
  }
  return errors;
}

/** Extracts the `Translations-Unchanged` reason, or `null` if absent or empty. */
export function escapeReason(text: string): string | null {
  const m = ESCAPE.exec(text);
  return m ? m[1].trim() : null;
}

/** Rule 2. `changed` holds paths relative to the site root. */
export function checkChangeSync(
  root: string,
  changed: string[],
  message = "",
): string[] {
  if (escapeReason(message)) return [];
  const loc = loadLocales(root);
  const set = new Set(changed);
  const errors: string[] = [];
  const need = (trigger: string, siblings: string[]) => {
    for (const s of siblings) {
      if (!set.has(s)) errors.push(`${trigger} changed but ${s} did not`);
    }
  };

  for (const f of changed) {
    if (f === `${DOCS}/meta.json`) {
      need(
        f,
        loc.others.map((l) => `${DOCS}/meta.${l}.json`),
      );
    } else if (f === `${I18N}/${loc.defaultLocale}.ts`) {
      need(
        f,
        loc.others.map((l) => `${I18N}/${l}.ts`),
      );
    } else if (f.startsWith(`${DOCS}/`) && f.endsWith(".mdx")) {
      const name = f.slice(DOCS.length + 1, -".mdx".length);
      if (loc.others.some((o) => name.endsWith(`.${o}`))) continue;
      need(
        f,
        loc.others.map((l) => `${DOCS}/${name}.${l}.mdx`),
      );
    }
  }
  return errors;
}

interface AppLocale {
  code: string;
  name: string;
}

function readAppLocales(repoRoot: string): {
  source: string;
  locales: AppLocale[];
} {
  const toml = Bun.TOML.parse(
    readFileSync(join(repoRoot, "i18n/locales.toml"), "utf8"),
  ) as { source: string; locales: AppLocale[] };
  return { source: toml.source, locales: toml.locales };
}

const SITE_CONFIG = "site/src/i18n/config.json";

/** Rule 3a. Site and app list the same locales, names and order. */
export function checkLocaleSets(repoRoot: string): string[] {
  const app = readAppLocales(repoRoot);
  const site = JSON.parse(
    readFileSync(join(repoRoot, SITE_CONFIG), "utf8"),
  ) as {
    defaultLocale: string;
    locales: AppLocale[];
  };
  const errors: string[] = [];
  const appCodes = app.locales.map((l) => l.code);
  const siteCodes = site.locales.map((l) => l.code);
  if (site.defaultLocale !== app.source)
    errors.push(
      `config.json: defaultLocale "${site.defaultLocale}" differs from i18n/locales.toml source "${app.source}"`,
    );
  const d = diff(appCodes, siteCodes);
  if (d.missing.length || d.extra.length) {
    const parts = [
      ...(d.missing.length ? [`missing ${d.missing.join(", ")}`] : []),
      ...(d.extra.length ? [`unknown ${d.extra.join(", ")}`] : []),
    ];
    errors.push(
      `config.json: locales differ from i18n/locales.toml (${parts.join("; ")})`,
    );
    return errors;
  }
  for (const a of app.locales) {
    const s = site.locales.find((l) => l.code === a.code);
    if (s && s.name !== a.name)
      errors.push(
        `config.json: name of ${a.code} is "${s.name}", i18n/locales.toml says "${a.name}"`,
      );
  }
  if (appCodes.join() !== siteCodes.join())
    errors.push("config.json: locale order differs from i18n/locales.toml");
  return errors;
}

/** Rule 3b. `changed` holds repo-relative paths. */
export function checkRepoChangeSync(
  repoRoot: string,
  changed: string[],
  message = "",
): string[] {
  if (escapeReason(message)) return [];
  const app = readAppLocales(repoRoot);
  const others = app.locales.map((l) => l.code).filter((c) => c !== app.source);
  const set = new Set(changed);
  const errors: string[] = [];
  const need = (trigger: string, sibling: string) => {
    if (!set.has(sibling))
      errors.push(`${trigger} changed but ${sibling} did not`);
  };
  const enFtl = `i18n/${app.source}.ftl`;
  for (const f of changed) {
    if (f === "i18n/locales.toml") need(f, SITE_CONFIG);
    else if (f === enFtl || f.endsWith(`/${enFtl}`)) {
      for (const l of others)
        need(f, f.slice(0, -enFtl.length) + `i18n/${l}.ftl`);
    }
  }
  return errors;
}

const DRAFT: Record<string, RegExp> = {
  ftl: /^\s*#\s*i18n-draft\b/gm,
  ts: /\/\/\s*i18n-draft\b/g,
  mdx: /\{\/\*\s*i18n-draft\s*\*\/\}/g,
};

function walk(dir: string, out: string[] = []): string[] {
  if (!existsSync(dir)) return out;
  for (const name of readdirSync(dir)) {
    if (name === "target" || name === "node_modules" || name.startsWith("."))
      continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

function countIn(file: string, kind: string): number {
  return readFileSync(file, "utf8").match(DRAFT[kind])?.length ?? 0;
}

/** Rule 4. `i18n-draft` markers per locale (locales with none are absent). */
export function countDrafts(repoRoot: string): Record<string, number> {
  const app = readAppLocales(repoRoot);
  const counts: Record<string, number> = {};
  const add = (locale: string, n: number) => {
    if (n > 0) counts[locale] = (counts[locale] ?? 0) + n;
  };
  const codes = app.locales.map((l) => l.code);
  for (const f of [
    ...walk(join(repoRoot, "crates")),
    ...walk(join(repoRoot, "i18n")),
  ]) {
    const m = /(?:^|\/)i18n\/([^/]+)\.ftl$/.exec(f);
    if (m && codes.includes(m[1])) add(m[1], countIn(f, "ftl"));
  }
  for (const c of codes) {
    const ts = join(repoRoot, "site/src/i18n", `${c}.ts`);
    if (existsSync(ts)) add(c, countIn(ts, "ts"));
  }
  const docs = join(repoRoot, "site", DOCS);
  if (existsSync(docs)) {
    for (const f of readdirSync(docs)) {
      const m = /\.([^.]+)\.mdx$/.exec(f);
      if (m && codes.includes(m[1])) add(m[1], countIn(join(docs, f), "mdx"));
    }
  }
  return counts;
}

/** Rule 4. Any draft marker fails (pre-commit, PR CI and release alike). */
export function checkDrafts(repoRoot: string): string[] {
  return Object.entries(countDrafts(repoRoot)).map(
    ([l, n]) =>
      `${l}: ${n} i18n-draft marker(s); review the translation and remove them`,
  );
}

function lines(out: string): string[] {
  return out.split("\n").filter(Boolean);
}

/** Repo-relative `site/...` paths from git, as site-relative paths. */
function siteFiles(files: string[]): string[] {
  return files
    .filter((f) => f.startsWith("site/"))
    .map((f) => f.slice("site/".length));
}

async function main(argv: string[]): Promise<number> {
  const root = process.cwd();
  const repoRoot = git(["rev-parse", "--show-toplevel"]).trim();
  const errors = [
    ...(await checkParity(root)),
    ...checkLocaleSets(repoRoot),
    ...checkDrafts(repoRoot),
  ];
  for (const [l, n] of Object.entries(countDrafts(repoRoot)))
    console.log(`translations: ${l} has ${n} i18n-draft marker(s)`);

  const staged = argv.includes("--staged");
  const bi = argv.indexOf("--base");
  let changed: string[] | null = null; // repo-relative
  let message = process.env.TRANSLATIONS_UNCHANGED
    ? `Translations-Unchanged: ${process.env.TRANSLATIONS_UNCHANGED}`
    : "";
  if (staged) {
    changed = lines(git(["diff", "--cached", "--name-only"]));
  } else if (bi >= 0) {
    const base = argv[bi + 1];
    if (base && !/^0+$/.test(base)) {
      changed = lines(git(["diff", "--name-only", `${base}...HEAD`]));
      message += `\n${process.env.PR_BODY ?? ""}\n${git(["log", `${base}..HEAD`, "--format=%B"])}`;
    }
  }
  if (changed) {
    errors.push(...checkChangeSync(root, siteFiles(changed), message));
    errors.push(...checkRepoChangeSync(repoRoot, changed, message));
  }

  for (const e of errors) console.error(`translations: ${e}`);
  if (errors.length) {
    console.error(
      "Update every locale with its English change, or add a " +
        "`Translations-Unchanged: <reason>` line to the commit message or PR body.",
    );
    return 1;
  }
  console.log("translations: ok");
  return 0;
}

if (import.meta.main) process.exit(await main(process.argv.slice(2)));
