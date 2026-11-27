import { describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { join } from "node:path";
import {
  buildCommand,
  lineBuffer,
  logSince,
  ensureWindowIdTool,
  outputs,
  parseArgs,
  parseWindowId,
  scrub,
  shouldBuild,
  windowId,
} from "./app-shot";

describe("parseArgs", () => {
  test("defaults", () => {
    const o = parseArgs([]);
    expect(o.out).toBeNull();
    expect(o.attach).toBe(false);
    expect(o.timeout).toBe(60000);
  });
  test("all options", () => {
    const o = parseArgs([
      "--out", "a.png", "--wait-log", "started", "--delay", "500",
      "--keep-running", "--binary", "/x/parla", "--env", "A=1", "--env", "B=x=y",
      "--appearance", "both",
    ]);
    expect(o).toMatchObject({
      out: "a.png", waitLog: "started", delay: 500, keepRunning: true,
      binary: "/x/parla", appearance: "both",
    });
    expect(o.env).toEqual({ A: "1", B: "x=y" });
  });
  test("--fixture sets PARLA_FIXTURE", () => {
    expect(parseArgs(["--fixture"]).env).toEqual({ PARLA_FIXTURE: "perf" });
  });
  test("rejects bad input", () => {
    expect(() => parseArgs(["--nope"])).toThrow("unknown option");
    expect(() => parseArgs(["--env", "novalue"])).toThrow("KEY=VAL");
    expect(() => parseArgs(["--delay", "x"])).toThrow("milliseconds");
    expect(() => parseArgs(["--appearance", "blue"])).toThrow("light, dark");
    expect(() => parseArgs(["--out"])).toThrow("needs a value");
    expect(() => parseArgs(["--attach", "--appearance", "dark"])).toThrow("--attach");
  });
});

describe("wait-log", () => {
  test("ignores log text written before the launch", () => {
    const old = "bootstrap ok\n";
    expect(logSince(old, Buffer.byteLength(old)).includes("bootstrap ok")).toBe(false);
    expect(logSince(old + "bootstrap ok\n", Buffer.byteLength(old)).includes("bootstrap ok")).toBe(true);
  });
});

describe("build before launch", () => {
  test("builds by default, --no-build skips it", () => {
    expect(parseArgs([]).build).toBe(true);
    expect(parseArgs(["--no-build"]).build).toBe(false);
    expect(shouldBuild(parseArgs([]))).toBe(true);
    expect(shouldBuild(parseArgs(["--fixture"]))).toBe(true);
    expect(shouldBuild(parseArgs(["--no-build"]))).toBe(false);
    expect(shouldBuild(parseArgs(["--attach"]))).toBe(false);
    expect(shouldBuild(parseArgs(["--binary", "/x/parla"]))).toBe(false);
  });
  test("fixture builds use the fixture feature", () => {
    expect(buildCommand(true)).toContain("fixture");
    expect(buildCommand(false)).toEqual(["run", "build"]);
  });
});

describe("helpers", () => {
  test("parseWindowId", () => {
    expect(parseWindowId("1234\n")).toBe(1234);
    expect(parseWindowId("")).toBeNull();
    expect(parseWindowId("oops")).toBeNull();
  });
  test("outputs splits both appearances", () => {
    expect(outputs("t/a.png", "both")).toEqual([
      ["light", "t/a-light.png"],
      ["dark", "t/a-dark.png"],
    ]);
    expect(outputs("t/a.png", "dark")).toEqual([["dark", "t/a.png"]]);
    expect(outputs("t/a.png", null)).toEqual([[null, "t/a.png"]]);
  });
  test("lineBuffer joins a line split across chunks and flushes the tail", () => {
    const got: string[] = [];
    const b = lineBuffer((l) => got.push(l));
    b.push("ready fo");
    expect(got).toEqual([]);
    b.push("r use\nsecond\nthi");
    expect(got).toEqual(["ready for use", "second"]);
    b.push("rd");
    b.flush();
    expect(got).toEqual(["ready for use", "second", "third"]);
  });
  test("scrub masks tokens", () => {
    expect(scrub("auth token=abc123 ok")).toBe("auth token=[scrubbed] ok");
    expect(scrub("x xoxc-1234-abc y")).toBe("x [token] y");
  });
});

describe.skipIf(process.platform !== "darwin")("window-id tool", () => {
  test("compiles and reports no window for an unused pid", () => {
    const tool = ensureWindowIdTool(join(import.meta.dir, ".."));
    expect(existsSync(tool)).toBe(true);
    expect(windowId(tool, 2147483646)).toBeNull();
  }, 180_000); // swiftc is slow under load
});
