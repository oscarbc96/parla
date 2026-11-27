// Covers mise-tasks/release/self-signed-cert.
import { afterEach, beforeEach, describe, expect, setDefaultTimeout, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// Certificate generation can exceed the 5 s default on a busy machine.
setDefaultTimeout(60_000);

const root = join(import.meta.dir, "..");
const script = join(root, "mise-tasks/release/self-signed-cert");
const cleanup: string[] = [];
let dir: string;
let home: string;
let outDir: string;

function run(
  args: string[],
  opts: { cwd?: string; env?: Record<string, string | undefined> } = {},
) {
  const p = Bun.spawnSync(args, {
    cwd: opts.cwd ?? root,
    stdout: "pipe",
    stderr: "pipe",
    env: { ...process.env, ...opts.env },
  });
  return {
    code: p.exitCode,
    stdout: new TextDecoder().decode(p.stdout),
    stderr: new TextDecoder().decode(p.stderr),
  };
}

function runScript(args: string[]) {
  return run([script, ...args], { env: { HOME: home } });
}

function openssl(args: string[]) {
  return run(["openssl", ...args], { env: { HOME: home } });
}

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "parla-release-self-signed-"));
  cleanup.push(dir);
  home = join(dir, "home");
  outDir = join(dir, "output");
  mkdirSync(home);
});

afterEach(() => {
  while (cleanup.length > 0) {
    rmSync(cleanup.pop()!, { recursive: true, force: true });
  }
});

describe("release self-signed certificate task", () => {
  test("requires exactly one output directory", () => {
    const out = runScript([]);
    expect(out.code).toBe(2);
    expect(out.stdout + out.stderr).toContain(
      "usage: mise run release:self-signed-cert -- <out-dir>",
    );
  });

  test("refuses an output directory inside the repository", () => {
    const inside = join(root, "target", `self-signed-cert-inside-${process.pid}-${Date.now()}`);
    cleanup.push(inside);
    const out = runScript(["--", inside]);
    expect(out.code).toBe(1);
    expect(out.stdout + out.stderr).toContain("inside the git repository");
    expect(existsSync(inside)).toBe(false);
  });

  test("creates a protected code-signing certificate and safe instructions", () => {
    const out = runScript(["--", outDir]);
    expect(out.code, out.stderr).toBe(0);
    expect(statSync(outDir).mode & 0o777).toBe(0o700);

    const files = [
      "parla-release.p12",
      "parla-release.p12.b64",
      "password.txt",
      "parla-release.cer.pem",
    ];
    for (const file of files) {
      const path = join(outDir, file);
      expect(existsSync(path), file).toBe(true);
      expect(statSync(path).mode & 0o777, file).toBe(0o600);
    }

    const cert = join(outDir, "parla-release.cer.pem");
    const subject = openssl(["x509", "-in", cert, "-noout", "-subject"]);
    expect(subject.code, subject.stderr).toBe(0);
    expect(subject.stdout).toContain("Parla Release (self-signed)");

    const extensions = openssl(["x509", "-in", cert, "-noout", "-text"]);
    expect(extensions.code, extensions.stderr).toBe(0);
    expect(extensions.stdout).toContain("Code Signing");

    const p12 = join(outDir, "parla-release.p12");
    const passwordPath = join(outDir, "password.txt");
    const password = readFileSync(passwordPath, "utf8");
    expect(password).toMatch(/^[0-9a-f]{48}$/);
    let p12Check = openssl([
      "pkcs12",
      "-in",
      p12,
      "-passin",
      `file:${passwordPath}`,
      "-nokeys",
    ]);
    if (p12Check.code !== 0) {
      p12Check = openssl([
        "pkcs12",
        "-legacy",
        "-in",
        p12,
        "-passin",
        `file:${passwordPath}`,
        "-nokeys",
      ]);
    }
    expect(p12Check.code, p12Check.stderr).toBe(0);

    const encoded = readFileSync(join(outDir, "parla-release.p12.b64"), "utf8");
    expect(encoded).not.toContain("\n");
    expect(encoded).not.toContain("\r");
    expect(Buffer.from(encoded, "base64").equals(readFileSync(p12))).toBe(true);

    const realOutDir = realpathSync(outDir);
    expect(out.stdout).toMatch(/[0-9A-F]{40}/);
    expect(out.stdout).toContain(
      `gh secret set MACOS_SELF_SIGNED_P12 --env release < ${realOutDir}/parla-release.p12.b64`,
    );
    expect(out.stdout).toContain(
      `gh secret set MACOS_SELF_SIGNED_PASSWORD --env release < ${realOutDir}/password.txt`,
    );
    expect(out.stdout).toContain("password manager");
    expect(out.stdout).toContain("delete it from disk");
    expect(out.stdout).not.toContain(password);
  });

  test("refuses to overwrite an existing release certificate", () => {
    const first = runScript(["--", outDir]);
    expect(first.code, first.stderr).toBe(0);
    const second = runScript(["--", outDir]);
    expect(second.code).toBe(1);
    expect(second.stdout + second.stderr).toContain("parla-release.p12");
  });

  test("never calls the macOS keychain tool", () => {
    expect(readFileSync(script, "utf8")).not.toContain("security ");
  });
});
