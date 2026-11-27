/** Small process helpers shared by the repo tooling scripts. */
import { dirname, join } from "node:path";

export function repoRoot(): string {
  return dirname(import.meta.dir === "" ? "." : import.meta.dir);
}

export interface RunOptions {
  cwd?: string;
  env?: Record<string, string>;
  /** Inherit stdio instead of capturing (long builds). */
  inherit?: boolean;
  /** Do not throw on a non-zero exit. */
  allowFail?: boolean;
}

export function run(cmd: string[], o: RunOptions = {}): string {
  const r = Bun.spawnSync(cmd, {
    cwd: o.cwd,
    env: { ...process.env, ...o.env },
    stdout: o.inherit ? "inherit" : "pipe",
    stderr: o.inherit ? "inherit" : "pipe",
  });
  if (r.exitCode !== 0 && !o.allowFail) {
    throw new Error(
      `${cmd.join(" ")} (exit ${r.exitCode}): ${r.stderr?.toString() ?? ""}`,
    );
  }
  return r.stdout?.toString() ?? "";
}

export const git = (args: string[], o: RunOptions = {}) =>
  run(["git", ...args], o);

export function arg(argv: string[], name: string): string | undefined {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : undefined;
}

export const has = (argv: string[], name: string) => argv.includes(name);

export const inRepo = (...p: string[]) => join(repoRoot(), ...p);
