import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { inRepo } from "./lib";
import {
  END,
  PAGES,
  START,
  cell,
  loadEntries,
  renderSection,
  stalePages,
} from "./keymap-reference";

const entries = loadEntries();

describe("keymap reference", () => {
  test("every keymap page is generated from the action snapshot", () => {
    expect(stalePages(entries)).toEqual([]);
  });

  test("the generated block documents every action and its default keys", () => {
    for (const locale of Object.keys(PAGES) as (keyof typeof PAGES)[]) {
      const page = readFileSync(inRepo(PAGES[locale]), "utf8");
      const start = page.indexOf(START);
      const end = page.indexOf(END);
      expect(start).toBeGreaterThanOrEqual(0);
      expect(end).toBeGreaterThan(start);
      const block = page.slice(start, end + END.length);
      for (const entry of entries) {
        expect(block).toContain(entry.action);
        for (const keys of ["macos", "linux", "windows"] as const) {
          const bindings = entry.defaults[keys];
          if (bindings.length > 0) expect(block).toContain(cell(bindings));
        }
      }
    }
  });

  test("a page with no bindings shows a dash", () => {
    expect(cell([])).toBe("-");
  });

  test("renderSection is stable for the same snapshot", () => {
    for (const locale of Object.keys(PAGES) as (keyof typeof PAGES)[]) {
      expect(renderSection(entries, locale)).toBe(renderSection(entries, locale));
    }
  });
});
