// Covers mise-tasks/release/version-check.
import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";

const root = join(import.meta.dir, "..");

function run(args: string[], cwd = root) {
  const p = Bun.spawnSync(args, { cwd, stdout: "pipe", stderr: "pipe" });
  return {
    code: p.exitCode,
    stdout: new TextDecoder().decode(p.stdout),
    stderr: new TextDecoder().decode(p.stderr),
  };
}

describe("release:version-check", () => {
  test("version-check accepts v<version> only: rejects a different version, a missing v and an rc suffix the crate lacks", () => {
    const meta = run(["cargo", "metadata", "--no-deps", "--format-version", "1"]);
    expect(meta.code).toBe(0);
    const version = (JSON.parse(meta.stdout).packages as { name: string; version: string }[]).find(
      (p) => p.name === "parla",
    )!.version;

    // One cargo run for the whole test; the task reads the metadata saved
    // here, so the cases do not depend on how fast cargo is.
    const saved = join(mkdtempSync(join(tmpdir(), "version-check-")), "metadata.json");
    writeFileSync(saved, meta.stdout);
    const check = (...args: string[]) =>
      run(["env", `PARLA_CARGO_METADATA_FILE=${saved}`, "mise-tasks/release/version-check", ...args]);

    const ok = check(`v${version}`);
    expect(ok.code).toBe(0);

    const badVersion = check("v9.9.9");
    expect(badVersion.code).not.toBe(0);

    // mise forwards a literal "--" before the tag.
    const forwarded = check("--", `v${version}`);
    expect(forwarded.code).toBe(0);

    const missingV = check(version);
    expect(missingV.code).not.toBe(0);

    const rc = check(`v${version}-rc.1`);
    expect(rc.code).not.toBe(0);
  }, 30_000);
});
