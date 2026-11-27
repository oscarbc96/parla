import { describe, expect, test } from "bun:test";
import { abMarkdown, diffMetrics, flagRegression } from "./perf-ab";

describe("flagRegression", () => {
  test("more than 10 percent worse is a regression", () => {
    expect(flagRegression(100, 111, undefined).regression).toBe(true);
    expect(flagRegression(100, 110, undefined).regression).toBe(false);
    expect(flagRegression(100, 90, undefined).regression).toBe(false);
  });
  test("over budget is flagged whatever the delta", () => {
    expect(flagRegression(100, 100, 90).overBudget).toBe(true);
    expect(flagRegression(100, 80, 90).overBudget).toBe(false);
    expect(flagRegression(100, 80, undefined).overBudget).toBe(false);
  });
  test("a zero baseline only regresses when the value grows", () => {
    expect(flagRegression(0, 0, undefined).regression).toBe(false);
    expect(flagRegression(0, 5, undefined).regression).toBe(true);
  });
});

describe("diffMetrics", () => {
  const base = { idle: { cpu: 1, rss: 100 }, scroll: { fps: 60 } };
  const head = { idle: { cpu: 1.5, rss: 95 }, scroll: { fps: 60 } };
  test("one row per metric with delta and percent", () => {
    const rows = diffMetrics(base, head, { "idle.cpu": 2, "idle.rss": 90 });
    const cpu = rows.find((r) => r.key === "idle.cpu")!;
    expect(cpu.delta).toBeCloseTo(0.5);
    expect(cpu.pct).toBeCloseTo(50);
    expect(cpu.regression).toBe(true);
    expect(cpu.overBudget).toBe(false);
    expect(rows.find((r) => r.key === "idle.rss")!.overBudget).toBe(true);
    expect(rows.find((r) => r.key === "scroll.fps")!.delta).toBe(0);
  });
  test("a metric missing on one side is skipped", () => {
    expect(diffMetrics({ a: { m: 1 } }, { b: { m: 1 } }, {})).toEqual([]);
  });
});

describe("abMarkdown", () => {
  test("renders the table and flags", () => {
    const rows = diffMetrics({ idle: { cpu: 1 } }, { idle: { cpu: 2 } }, { "idle.cpu": 1.5 });
    const md = abMarkdown(rows, "abc", "def");
    expect(md).toContain("| Scenario | Metric | Before | After | Delta | Budget | Flag |");
    expect(md).toContain("| idle | cpu | 1.00 | 2.00 | +1.00 (+100.0%) | <= 1.50 | REGRESSION, OVER BUDGET |");
    expect(md).toContain("abc");
  });
  test("says so when nothing is flagged", () => {
    const rows = diffMetrics({ idle: { cpu: 1 } }, { idle: { cpu: 1 } }, {});
    expect(abMarkdown(rows, "a", "b")).toContain("No regression");
  });
});
