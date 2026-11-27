/**
 * Screenshot the running Parla window (macOS) so an agent can look at it.
 *
 *   mise run app:shot -- --env PARLA_FIXTURE=perf --appearance both
 *
 * Launches Parla (or `--attach`es to one), waits for its window and an
 * optional log line, captures the window with `screencapture`, prints the PNG
 * path and the last 30 scrubbed log lines, then quits Parla. Screenshots stay
 * under `target/` and are never committed. A real-account shot can show
 * private data: only FakeProvider fixture shots go into a PR.
 */
import { spawn, spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";

export type Appearance = "light" | "dark" | "both";

export interface Options {
  out: string | null;
  waitLog: string | null;
  delay: number;
  keepRunning: boolean;
  binary: string | null;
  env: Record<string, string>;
  attach: boolean;
  appearance: Appearance | null;
  timeout: number;
  build: boolean;
  help: boolean;
}

export const USAGE = `usage: mise run app:shot -- [options]
  --out <png>              output file (default target/shots/<timestamp>.png)
  --wait-log "<text>"      wait until the text appears in stderr or parla.log
  --delay <ms>             extra wait before capturing
  --keep-running           leave Parla running afterwards
  --binary <path>          binary to launch (default target/debug/parla)
  --no-build               skip the incremental build of the default binary
  --env KEY=VAL            environment for Parla, repeatable
  --fixture                shorthand for --env PARLA_FIXTURE=perf
  --attach                 capture an already running parla instead of launching
  --appearance light|dark|both   theme mode (launched runs only)
  --timeout <ms>           give up waiting after this long (default 60000)`;

/** Parses argv (without the interpreter and script). Throws on bad input. */
export function parseArgs(argv: string[]): Options {
  const o: Options = {
    out: null,
    waitLog: null,
    delay: 0,
    keepRunning: false,
    binary: null,
    env: {},
    attach: false,
    appearance: null,
    timeout: 60000,
    build: true,
    help: false,
  };
  const num = (flag: string, v: string): number => {
    const n = Number(v);
    if (!Number.isInteger(n) || n < 0) throw new Error(`${flag} needs a number of milliseconds`);
    return n;
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = (): string => {
      const v = argv[++i];
      if (v === undefined) throw new Error(`${a} needs a value`);
      return v;
    };
    switch (a) {
      case "--out": o.out = val(); break;
      case "--wait-log": o.waitLog = val(); break;
      case "--delay": o.delay = num(a, val()); break;
      case "--timeout": o.timeout = num(a, val()); break;
      case "--keep-running": o.keepRunning = true; break;
      case "--binary": o.binary = val(); break;
      case "--attach": o.attach = true; break;
      case "--no-build": o.build = false; break;
      case "--fixture": o.env.PARLA_FIXTURE = "perf"; break;
      case "--help": case "-h": o.help = true; break;
      case "--env": {
        const kv = val();
        const eq = kv.indexOf("=");
        if (eq < 1) throw new Error(`--env needs KEY=VAL, got "${kv}"`);
        o.env[kv.slice(0, eq)] = kv.slice(eq + 1);
        break;
      }
      case "--appearance": {
        const v = val();
        if (v !== "light" && v !== "dark" && v !== "both")
          throw new Error("--appearance must be light, dark or both");
        o.appearance = v;
        break;
      }
      default: throw new Error(`unknown option ${a}`);
    }
  }
  if (o.attach && o.appearance) throw new Error("--appearance cannot be used with --attach");
  if (o.attach && (o.binary || Object.keys(o.env).length))
    throw new Error("--binary and --env cannot be used with --attach");
  return o;
}

/** The `mise` arguments that build the default binary (fixture builds add the feature). */
export function buildCommand(fixture: boolean): string[] {
  return fixture
    ? ["exec", "--", "cargo", "build", "-p", "parla", "--features", "fixture"]
    : ["run", "build"];
}

/** Whether to run the incremental build before launching: always, unless opted out. */
export function shouldBuild(o: Options): boolean {
  return o.build && !o.attach && !o.binary;
}

/** The part of the log written after `offset` bytes (this run, not earlier ones). */
export function logSince(text: string, offset: number): string {
  return Buffer.from(text).subarray(offset).toString("utf8");
}

/** Parses the `window-id` tool output: a decimal id, else null. */
export function parseWindowId(stdout: string): number | null {
  const t = stdout.trim();
  return /^\d+$/.test(t) ? Number(t) : null;
}

/** Output file names for the requested appearances. */
export function outputs(out: string, appearance: Appearance | null): [string | null, string][] {
  if (appearance !== "both") return [[appearance, out]];
  const dot = out.lastIndexOf(".");
  const [stem, ext] = dot > 0 ? [out.slice(0, dot), out.slice(dot)] : [out, ".png"];
  return [["light", `${stem}-light${ext}`], ["dark", `${stem}-dark${ext}`]];
}

/** Masks tokens and cookies in a log line. */
export function scrub(line: string): string {
  return line
    .replace(/\bxox[a-z]-[A-Za-z0-9-]+/g, "[token]")
    .replace(/\b(token|cookie|authorization|bearer|secret|password)(["']?\s*[:=]\s*)\S+/gi, "$1$2[scrubbed]");
}

/** Splits chunks into complete lines, carrying a partial last line to the next chunk. */
export function lineBuffer(onLine: (line: string) => void) {
  let rest = "";
  return {
    push(chunk: string) {
      const parts = (rest + chunk).split("\n");
      rest = parts.pop() ?? "";
      for (const l of parts) if (l) onLine(l);
    },
    flush() {
      if (rest) onLine(rest);
      rest = "";
    },
  };
}

const root = resolve(import.meta.dir, "..");
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Compiles scripts/window-id.swift into target/tools/window-id when stale. */
export function ensureWindowIdTool(repo = root): string {
  const src = join(repo, "scripts/window-id.swift");
  const bin = join(repo, "target/tools/window-id");
  if (!existsSync(bin) || statSync(bin).mtimeMs < statSync(src).mtimeMs) {
    mkdirSync(join(repo, "target/tools"), { recursive: true });
    const r = spawnSync("swiftc", ["-O", src, "-o", bin], { encoding: "utf8" });
    if (r.status !== 0) throw new Error(`swiftc failed: ${r.stderr}`);
  }
  return bin;
}

export function windowId(tool: string, pid: number): number | null {
  const r = spawnSync(tool, [String(pid)], { encoding: "utf8" });
  return r.status === 0 ? parseWindowId(r.stdout) : null;
}

function findAttachPid(): number {
  const r = spawnSync("pgrep", ["-x", "parla"], { encoding: "utf8" });
  const pids = r.stdout.split("\n").filter(Boolean).map(Number);
  if (pids.length === 0) throw new Error("no running parla process to attach to");
  return pids[pids.length - 1];
}

function timestamp(): string {
  return new Date().toISOString().replace(/[:.]/g, "-");
}

async function main(argv: string[]): Promise<number> {
  const o = parseArgs(argv);
  if (o.help) {
    console.log(USAGE);
    return 0;
  }
  if (process.platform !== "darwin") throw new Error("app:shot supports macOS only");
  const tool = ensureWindowIdTool();
  const out = resolve(o.out ?? join(root, "target/shots", `${timestamp()}.png`));
  mkdirSync(resolve(out, ".."), { recursive: true });

  const fixture = o.env.PARLA_FIXTURE !== undefined;
  const defaultBin = join(root, "target/debug/parla");
  const bin = resolve(o.binary ?? defaultBin);
  if (o.binary && !o.attach && !existsSync(bin)) throw new Error(`binary not found: ${bin}`);
  if (shouldBuild(o)) {
    // Always build first: a stale target/debug/parla hides the latest code.
    console.error("building the debug binary (incremental)");
    if (spawnSync("mise", buildCommand(fixture), { cwd: root, stdio: "inherit" }).status !== 0)
      throw new Error("build failed");
  }

  const modes = outputs(out, o.appearance);
  const shots: string[] = [];
  const logLines: string[] = [];
  let note = "";
  for (const [mode, file] of modes) {
    let child: ReturnType<typeof spawn> | null = null;
    let stderrLines: ReturnType<typeof lineBuffer> | null = null;
    let pid: number;
    let logFile: string | null = null;
    let logStart = 0;
    if (o.attach) {
      pid = findAttachPid();
      const data = process.env.PARLA_DATA_DIR ?? join(homedir(), "Library/Application Support/parla");
      logFile = join(data, "parla.log");
      logStart = existsSync(logFile) ? statSync(logFile).size : 0;
    } else {
      const scratch = mkdtempSync(join(tmpdir(), "parla-shot-"));
      const env: Record<string, string> = { ...(process.env as Record<string, string>), ...o.env };
      // A fixture run never touches real data; a real-account run uses it.
      if (fixture || mode) {
        env.PARLA_DATA_DIR ??= join(scratch, "data");
        env.PARLA_CACHE_DIR ??= join(scratch, "cache");
      }
      if (mode) {
        const cfg = env.PARLA_CONFIG_DIR ?? join(scratch, "config");
        env.PARLA_CONFIG_DIR = cfg;
        mkdirSync(cfg, { recursive: true });
        const settings = join(cfg, "settings.json");
        if (!existsSync(settings))
          writeFileSync(settings, JSON.stringify({ theme: { mode } }));
        else note += `kept existing ${settings}; --appearance not applied\n`;
      }
      logFile = join(env.PARLA_DATA_DIR ?? join(homedir(), "Library/Application Support/parla"), "parla.log");
      const c = spawn(bin, [], { env, stdio: ["ignore", "ignore", "pipe"] });
      child = c;
      pid = c.pid!;
      logStart = logFile && existsSync(logFile) ? statSync(logFile).size : 0;
      c.stderr!.setEncoding("utf8");
      const lines = lineBuffer((l) => logLines.push(scrub(l)));
      stderrLines = lines;
      c.stderr!.on("data", (d: string) => lines.push(d));
    }

    try {
      const deadline = Date.now() + o.timeout;
      let id: number | null = null;
      while (Date.now() < deadline && !(child && child.exitCode !== null)) {
        id = windowId(tool, pid);
        if (id !== null) break;
        await sleep(200);
      }
      if (id === null) throw new Error("no parla window appeared (timeout or the app exited)");
      if (o.waitLog) {
        const seen = () =>
          logLines.some((l) => l.includes(o.waitLog!)) ||
          (logFile !== null &&
            existsSync(logFile) &&
            logSince(readFileSync(logFile, "utf8"), logStart).includes(o.waitLog!));
        while (!seen()) {
          if (Date.now() > deadline) throw new Error(`log line not seen: ${o.waitLog}`);
          await sleep(200);
        }
      }
      // A covered window stops drawing (the display link parks while
      // occluded), so the capture would show stale content. Bring it front.
      spawnSync("osascript", [
        "-e",
        `tell application "System Events" to set frontmost of (first process whose unix id = ${pid}) to true`,
      ]);
      await sleep(500);
      if (o.delay) await sleep(o.delay);
      id = windowId(tool, pid) ?? id;
      const r = spawnSync("screencapture", ["-x", "-o", "-l", String(id), file], { encoding: "utf8" });
      if (r.status !== 0 || !existsSync(file))
        throw new Error(`screencapture failed (Screen Recording permission?): ${r.stderr}`);
      shots.push(file);
    } finally {
      if (child && !o.keepRunning) {
        child.kill("SIGTERM");
        await sleep(300);
      }
      stderrLines?.flush();
    }
  }
  if (note) process.stderr.write(note);
  for (const s of shots) console.log(s);
  console.log("--- last 30 log lines ---");
  for (const l of logLines.slice(-30)) console.log(l);
  console.error(
    "warning: a real-account screenshot can contain private data. Keep it under target/ and never commit or upload it; PR screenshots come from FakeProvider fixtures (--fixture).",
  );
  return 0;
}

if (import.meta.main) {
  main(process.argv.slice(2)).then(
    (c) => process.exit(c),
    (e: Error) => {
      console.error(`app:shot: ${e.message}`);
      process.exit(1);
    },
  );
}
