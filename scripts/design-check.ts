#!/usr/bin/env bun
/**
 * Design guard: inspects ADDED lines of UI Rust code and fails on colour
 * literals and `px(N)` values outside the token scale in
 * docs/design/parla.tokens.json.
 *
 *   bun scripts/design-check.ts [--base <ref>]
 *
 * Without --base it reads the staged diff (`git diff -U0 --cached`), or, when
 * nothing is staged (CI, `hk check --all`), the branch diff `origin/main...HEAD`;
 * with --base `<ref>`, the diff `<ref>...HEAD`. Escape hatch: `design: allow <reason>` (a reason is
 * required) on the same line or the line above.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

const ROOT = join(import.meta.dir, "..");
const TOKENS = join(ROOT, "docs", "design", "parla.tokens.json");

export interface Violation {
  file: string;
  line: number;
  rule: "colour" | "px";
  message: string;
}

/** Files the guard applies to (never tests/). */
export function inScope(file: string): boolean {
  return /^crates\/(components|parla)\/src\/.*\.rs$/.test(file);
}

const px = (v: unknown) => Number.parseFloat(String(v).replace("px", ""));

/** value -> const names, from the token file. */
export function loadScale(tokensPath = TOKENS): Map<number, string[]> {
  const t = JSON.parse(readFileSync(tokensPath, "utf8"));
  const scale = new Map<number, string[]>();
  const add = (n: number, name: string) => {
    if (!scale.has(n)) scale.set(n, []);
    scale.get(n)!.push(name);
  };
  const groups: [unknown, string][] = [
    [t.space, "SPACE_"],
    [t.radius, "RADIUS_"],
    [t.font.size, "FONT_SIZE_"],
    [t["icon-size"], "ICON_SIZE_"],
    [t.layout, "LAYOUT_"],
  ];
  for (const [g, prefix] of groups) {
    for (const [k, v] of Object.entries(
      g as Record<string, { $value?: string }>,
    )) {
      if (k.startsWith("$")) continue;
      add(px(v.$value), prefix + k.toUpperCase().replaceAll("-", "_"));
    }
  }
  return scale;
}

const ALLOW = /design:\s*allow\s+\S/;

const COLOUR_RULES: { re: RegExp; what: string }[] = [
  { re: /\brgba?\(\s*0x/, what: "rgb(0x..)/rgba(0x..)" },
  { re: /\bhsla\(\s*[-+]?[\d.]/, what: "hsla(<number>, ..)" },
  { re: /\brgb_to_hsla\(\s*rgb\(/, what: "rgb_to_hsla(rgb(..))" },
  { re: /\bgpui::(?:rgba?|hsla)\(/, what: "gpui::rgb/rgba/hsla(..)" },
  { re: /\b(?:white|black)\(\)/, what: "white()/black()" },
  { re: /\bHsla\s*\{/, what: "Hsla { .. } literal" },
  { re: /"#[0-9a-fA-F]{6}"/, what: '"#RRGGBB" string' },
];

/** Check one added line; `prev` is the line above (for the escape hatch). */
export function checkLine(
  line: string,
  prev: string,
  scale: Map<number, string[]>,
): { rule: "colour" | "px"; message: string }[] {
  if (ALLOW.test(line) || ALLOW.test(prev)) return [];
  const out: { rule: "colour" | "px"; message: string }[] = [];
  for (const { re, what } of COLOUR_RULES) {
    if (re.test(line)) {
      out.push({
        rule: "colour",
        message: `colour literal ${what}; take colours from the theme (theme.colors(), status(), Parla tokens)`,
      });
    }
  }
  for (const m of line.matchAll(/\bpx\(\s*(-?[\d.]+)(?:_?f32)?\s*\)/g)) {
    const n = Number.parseFloat(m[1]);
    const abs = Math.abs(n);
    if (abs === 0 || abs === 1) continue;
    const names = scale.get(abs);
    // Layout (fixed chrome) values must be used by name, never as a bare literal.
    if (names?.some((x) => !x.startsWith("LAYOUT_"))) continue;
    if (names) {
      out.push({
        rule: "px",
        message: `px(${m[1]}) is fixed chrome; use themes::metrics::${names[0]}`,
      });
      continue;
    }
    const near = [...scale.keys()].sort(
      (a, b) => Math.abs(a - n) - Math.abs(b - n),
    )[0];
    out.push({
      rule: "px",
      message: `px(${m[1]}) is not in the token scale; nearest is ${near}px, use themes::metrics::${scale.get(near)![0]}`,
    });
  }
  return out;
}

/** Added lines per file from a `git diff -U0` text: [file, lineNo, text, previous added/context text]. */
export function addedLines(
  diff: string,
): { file: string; line: number; text: string; prev: string }[] {
  const out: { file: string; line: number; text: string; prev: string }[] = [];
  let file = "";
  let n = 0;
  let prev = "";
  for (const raw of diff.split("\n")) {
    if (raw.startsWith("+++ ")) {
      file = raw.startsWith("+++ b/") ? raw.slice(6) : "";
      prev = "";
    } else if (raw.startsWith("@@")) {
      const m = raw.match(/\+(\d+)/);
      n = m ? Number.parseInt(m[1], 10) : 0;
      prev = "";
    } else if (raw.startsWith("+") && !raw.startsWith("+++")) {
      out.push({ file, line: n, text: raw.slice(1), prev });
      prev = raw.slice(1);
      n++;
    }
  }
  return out;
}

export function checkDiff(
  diff: string,
  scale: Map<number, string[]>,
): Violation[] {
  const v: Violation[] = [];
  for (const a of addedLines(diff)) {
    if (!inScope(a.file)) continue;
    for (const r of checkLine(a.text, a.prev, scale))
      v.push({ file: a.file, line: a.line, ...r });
  }
  return v;
}

/** The line above `line` (1-based) in the working-tree file, for the escape hatch. */
function lineAbove(file: string, line: number): string {
  try {
    return readFileSync(join(ROOT, file), "utf8").split("\n")[line - 2] ?? "";
  } catch {
    return "";
  }
}

function gitDiff(range: string[]): string {
  const r = Bun.spawnSync(
    [
      "git",
      "diff",
      "-U0",
      ...range,
      "--",
      "crates/components/src",
      "crates/parla/src",
    ],
    { cwd: ROOT, stdout: "pipe", stderr: "pipe" },
  );
  if (r.exitCode !== 0) {
    console.error(r.stderr.toString());
    process.exit(2);
  }
  return r.stdout.toString();
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const bi = args.indexOf("--base");
  const base = bi >= 0 ? args[bi + 1] : undefined;
  let diff = base ? gitDiff([`${base}...HEAD`]) : gitDiff(["--cached"]);
  if (!base && diff.trim() === "") diff = gitDiff(["origin/main...HEAD"]);
  const scale = loadScale();
  const violations: Violation[] = [];
  for (const a of addedLines(diff)) {
    if (!inScope(a.file)) continue;
    const prev = a.prev || lineAbove(a.file, a.line);
    for (const r of checkLine(a.text, prev, scale))
      violations.push({ file: a.file, line: a.line, ...r });
  }
  for (const x of violations)
    console.error(
      `${x.file}:${x.line}: [${x.rule}] ${x.message} (or add \`// design: allow <reason>\`)`,
    );
  if (violations.length) {
    console.error(`design-check: ${violations.length} violation(s)`);
    process.exit(1);
  }
  console.log("design-check: clean");
}
