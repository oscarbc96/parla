import { describe, expect, test } from "bun:test";
import {
  compare,
  isBusy,
  median,
  parseArgs,
  suiteMarkdown,
  summarize,
  type Run,
} from "./perf-suite";

describe("stats", () => {
  test("median odd and even", () => {
    expect(median([3, 1, 2])).toBe(2);
    expect(median([4, 1, 2, 3])).toBe(2.5);
  });
});

describe("args", () => {
  test("defaults", () => {
    const a = parseArgs([]);
    expect(a.runs).toBe(3);
    expect(a.scenarios).toEqual(["idle", "scroll", "scroll-input"]);
    expect(a.seconds).toBeUndefined();
    expect(a.waitQuietMin).toBe(5);
  });
  test("flags", () => {
    const a = parseArgs([
      "--runs",
      "2",
      "--scenarios",
      "scroll,idle",
      "--seconds",
      "5",
      "--wait-quiet",
      "0",
    ]);
    expect(a).toMatchObject({
      runs: 2,
      scenarios: ["scroll", "idle"],
      seconds: 5,
      waitQuietMin: 0,
    });
  });
});

describe("busy check", () => {
  const quiet = { load1: 0.5, cpus: 10, procs: [] as string[] };
  test("quiet", () => expect(isBusy(quiet)).toEqual([]));
  test("high load", () =>
    expect(isBusy({ ...quiet, load1: 8 }).join()).toMatch(/load/));
  test("other build or perf process", () => {
    expect(isBusy({ ...quiet, procs: ["rustc --crate-name x"] }).length).toBe(1);
    expect(
      isBusy({ ...quiet, procs: ["target/release/parla --perf-scenario idle"] })
        .length,
    ).toBe(1);
  });
});

const run = (scenario: string, m: Record<string, number>, noisy = false): Run => ({
  scenario,
  noisy,
  metrics: m,
});

describe("summarize and compare", () => {
  const runs = [
    run("scroll", { cpu_pct: 10, mem_mb: 70 }),
    run("scroll", { cpu_pct: 20, mem_mb: 80 }),
    run("scroll", { cpu_pct: 90, mem_mb: 100 }, true),
  ];
  test("median and min/max over quiet runs, noisy kept apart", () => {
    const s = summarize(runs);
    const cpu = s.scroll.metrics.cpu_pct;
    expect(cpu.quiet).toEqual({ n: 2, median: 15, min: 10, max: 20 });
    expect(cpu.all).toEqual({ n: 3, median: 20, min: 10, max: 90 });
    expect(s.scroll.noisyRuns).toBe(1);
  });
  test("compare against limits", () => {
    const s = summarize(runs);
    const f = compare(s, { "scroll.cpu_pct": 30, "scroll.mem_mb": 60 });
    expect(f.find((x) => x.key === "scroll.cpu_pct")?.ok).toBe(true);
    expect(f.find((x) => x.key === "scroll.mem_mb")?.ok).toBe(false);
  });
  test("all-noisy scenario never fails", () => {
    const s = summarize([run("scroll", { cpu_pct: 99 }, true)]);
    const f = compare(s, { "scroll.cpu_pct": 30 });
    expect(f[0].ok).toBe(true);
    expect(f[0].noisyOnly).toBe(true);
  });
  test("markdown has a row per metric with the limit", () => {
    const s = summarize(runs);
    const md = suiteMarkdown(s, compare(s, { "scroll.cpu_pct": 30 }));
    expect(md).toContain("| scroll | cpu_pct |");
    expect(md).toContain("30");
    expect(md).toContain("noisy");
  });
});
