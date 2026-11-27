#!/usr/bin/env bun
/**
 * Generates the action reference table at the end of the keymap docs page.
 *
 *   mise run docs:keymap          rewrite the table in en, es and ca
 *   mise run docs:keymap --check  fail when a page is out of date
 *
 * The source is `crates/parla/tests/snapshots/actions.json`, the same snapshot
 * `crates/parla/tests/print_actions.rs` reads, so every action the binary
 * registers is documented with its default keys. The curated part of each page
 * stays hand-written; only the text between the two markers is generated.
 * Tested by scripts/keymap-reference.test.ts.
 */
import { readFileSync, writeFileSync } from "node:fs";
import { has, inRepo } from "./lib";

export interface Binding {
  context: string | null;
  keys: string;
}

export interface ActionEntry {
  action: string;
  defaults: { linux: Binding[]; macos: Binding[]; windows: Binding[] };
}

export type Locale = "en" | "es" | "ca";

export const SNAPSHOT = "crates/parla/tests/snapshots/actions.json";
export const START = "{/* keymap-actions:start */}";
export const END = "{/* keymap-actions:end */}";

export const PAGES: Record<Locale, string> = {
  en: "site/content/docs/keymap.mdx",
  es: "site/content/docs/keymap.es.mdx",
  ca: "site/content/docs/keymap.ca.mdx",
};

interface Copy {
  heading: string;
  intro: string;
  columns: [string, string, string, string];
}

const COPY: Record<Locale, Copy> = {
  en: {
    heading: "## Action reference",
    intro:
      "Every action has a stable name, and the command palette and `keymap.json`\n" +
      "use it. This table lists every action with its default keys, generated from\n" +
      "the app's action list. The context in parentheses is the keymap context the\n" +
      "binding applies in, and a dash means the action has no default key.",
    columns: ["Action", "macOS", "Linux", "Windows"],
  },
  es: {
    heading: "## Referencia de acciones",
    intro:
      "Cada acción tiene un nombre estable, y la paleta de comandos y `keymap.json`\n" +
      "lo usan. Esta tabla enumera cada acción con sus teclas predeterminadas,\n" +
      "generada a partir de la lista de acciones de la app. El contexto entre\n" +
      "paréntesis es el contexto del mapa de teclas en el que se aplica el enlace, y\n" +
      "un guion significa que la acción no tiene tecla predeterminada.",
    columns: ["Acción", "macOS", "Linux", "Windows"],
  },
  ca: {
    heading: "## Referència d'accions",
    intro:
      "Cada acció té un nom estable, i la paleta d'ordres i `keymap.json` el fan\n" +
      "servir. Aquesta taula les enumera totes amb les tecles predeterminades,\n" +
      "generada a partir de la llista d'accions de l'app. El context entre\n" +
      "parèntesis és el context del mapa de tecles on s'aplica l'enllaç, i un guió\n" +
      "vol dir que l'acció no té tecla predeterminada.",
    columns: ["Acció", "macOS", "Linux", "Windows"],
  },
};

export function loadEntries(text = readFileSync(inRepo(SNAPSHOT), "utf8")): ActionEntry[] {
  return JSON.parse(text) as ActionEntry[];
}

function code(s: string): string {
  return `\`${s}\``;
}

/** One table cell: the bindings joined by `<br />`, each with its context. */
export function cell(bindings: Binding[]): string {
  if (bindings.length === 0) return "-";
  return bindings
    .map((b) => (b.context ? `${code(b.keys)} (${code(b.context)})` : code(b.keys)))
    .join("<br />");
}

function pad(s: string, width: number): string {
  return s + " ".repeat(width - s.length);
}

/** A markdown table aligned the way `bun fmt` writes it. */
export function table(rows: string[][]): string {
  const widths = rows[0].map((_, i) => Math.max(...rows.map((r) => r[i].length)));
  const line = (r: string[]) => `| ${r.map((c, i) => pad(c, widths[i])).join(" | ")} |`;
  return [
    line(rows[0]),
    `| ${widths.map((w) => "-".repeat(w)).join(" | ")} |`,
    ...rows.slice(1).map(line),
  ].join("\n");
}

export function renderSection(entries: ActionEntry[], locale: Locale): string {
  const copy = COPY[locale];
  const rows: string[][] = [copy.columns];
  for (const e of entries) {
    rows.push([
      code(e.action),
      cell(e.defaults.macos),
      cell(e.defaults.linux),
      cell(e.defaults.windows),
    ]);
  }
  return [START, "", copy.heading, "", copy.intro, "", table(rows), "", END].join("\n");
}

/** Replace the generated block, or throw when the page lost its markers. */
export function applySection(page: string, section: string): string {
  const start = page.indexOf(START);
  const end = page.indexOf(END);
  if (start < 0 || end < start) {
    throw new Error(`missing ${START} / ${END} markers`);
  }
  return page.slice(0, start) + section + page.slice(end + END.length);
}

export function renderPage(page: string, entries: ActionEntry[], locale: Locale): string {
  return applySection(page, renderSection(entries, locale));
}

/** Pages whose generated block does not match the snapshot. */
export function stalePages(
  entries: ActionEntry[],
  read: (file: string) => string = (f) => readFileSync(inRepo(f), "utf8"),
): string[] {
  const stale: string[] = [];
  for (const locale of Object.keys(PAGES) as Locale[]) {
    const file = PAGES[locale];
    const page = read(file);
    try {
      if (renderPage(page, entries, locale) !== page) stale.push(file);
    } catch {
      stale.push(file);
    }
  }
  return stale;
}

export function update(
  entries: ActionEntry[],
  read: (file: string) => string,
  write: (file: string, text: string) => void,
): string[] {
  const changed: string[] = [];
  for (const locale of Object.keys(PAGES) as Locale[]) {
    const file = PAGES[locale];
    const page = read(file);
    const next = renderPage(page, entries, locale);
    if (next !== page) {
      write(file, next);
      changed.push(file);
    }
  }
  return changed;
}

function main(): number {
  const entries = loadEntries();
  const check = has(process.argv.slice(2), "--check");
  if (check) {
    const stale = stalePages(entries);
    if (stale.length > 0) {
      console.error(`stale keymap reference: ${stale.join(", ")}`);
      console.error("run `mise run docs:keymap` and commit the result");
      return 1;
    }
    console.log("keymap reference is up to date");
    return 0;
  }
  const changed = update(
    entries,
    (f) => readFileSync(inRepo(f), "utf8"),
    (f, text) => writeFileSync(inRepo(f), text),
  );
  console.log(changed.length > 0 ? `updated ${changed.join(", ")}` : "keymap reference already up to date");
  return 0;
}

if (import.meta.main) process.exit(main());
