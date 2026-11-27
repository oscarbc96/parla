import { describe, expect, test } from "bun:test";
import {
  STEPS,
  parseArgs,
  selectSteps,
  summaryMarkdown,
  type StepResult,
} from "./gates";

const names = (a: { name: string }[]) => a.map((s) => s.name);

describe("gates steps", () => {
  test("default order", () => {
    expect(names(selectSteps(parseArgs([])))).toEqual([
      "build",
      "lint",
      "clippy",
      "test",
      "test:visual",
      "test:perf",
      "i18n:check",
      "site:check-translations",
      "test:repo",
    ]);
  });
  test("clippy runs the mise clippy task (host and guest workspaces)", () => {
    const c = STEPS.find((s) => s.name === "clippy")!;
    expect(c.cmd).toEqual(["mise", "run", "clippy"]);
  });
  test("--quick", () => {
    expect(names(selectSteps(parseArgs(["--quick"])))).toEqual([
      "build",
      "lint",
      "clippy",
      "test",
    ]);
  });
  test("--only keeps the canonical order", () => {
    expect(
      names(selectSteps(parseArgs(["--only", "test,build"]))),
    ).toEqual(["build", "test"]);
  });
  test("--skip", () => {
    const n = names(selectSteps(parseArgs(["--skip", "test:perf,test:visual"])));
    expect(n).not.toContain("test:perf");
    expect(n).not.toContain("test:visual");
    expect(n).toContain("test");
  });
  test("--quick with --skip", () => {
    expect(
      names(selectSteps(parseArgs(["--quick", "--skip", "lint"]))),
    ).toEqual(["build", "clippy", "test"]);
  });
  test("unknown step is an error", () => {
    expect(() => selectSteps(parseArgs(["--only", "nope"]))).toThrow(/nope/);
  });
  test("--continue", () => {
    expect(parseArgs([]).keepGoing).toBe(false);
    expect(parseArgs(["--continue"]).keepGoing).toBe(true);
  });
});

describe("summary", () => {
  const results: StepResult[] = [
    { name: "build", status: "pass", seconds: 12.4 },
    { name: "lint", status: "fail", seconds: 3, log: "target/gates/lint.log" },
    { name: "test", status: "skipped", seconds: 0 },
  ];
  test("markdown table", () => {
    const md = summaryMarkdown(results);
    expect(md).toContain("| Step | Result | Time |");
    expect(md).toContain("| `build` | pass | 12.4 s |");
    expect(md).toContain("| `lint` | FAIL | 3.0 s |");
    expect(md).toContain("| `test` | skipped | - |");
  });
});
