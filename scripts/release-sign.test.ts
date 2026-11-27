// Covers the release signing tasks: signing-mode, keychain, sign, dmg, verify,
// sums, update-sign and cleanup. The tasks run for real; the tools they call
// (codesign, security, hdiutil, ...) are shims first on PATH that log their
// argv and print the minimal output each task parses.
import { describe, expect, setDefaultTimeout, test } from "bun:test";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// These spawn many shell processes; a busy machine can exceed the 5 s default.
setDefaultTimeout(60_000);

const root = join(import.meta.dir, "..");
const entitlements = join(root, "crates/parla/macos/Parla.entitlements");

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

function makeTemp(prefix: string) {
  return mkdtempSync(join(tmpdir(), prefix));
}

function writeExec(path: string, content: string) {
  writeFileSync(path, content);
  chmodSync(path, 0o755);
}

/** Quote a value for embedding in a generated /bin/sh shim. */
function sh(value: string) {
  return `'${value.replace(/'/g, "'\\''")}'`;
}

interface Shims {
  env: { PATH: string };
  log: string;
  /** Logged calls, one line per tool invocation, oldest first. */
  calls: () => string[];
}

/** Write executable shims for `defs`; each logs "<tool> <argv...>" then runs body. */
function fakeTools(dir: string, defs: Record<string, string>): Shims {
  const bin = join(dir, "bin");
  const log = join(dir, "tools.log");
  mkdirSync(bin, { recursive: true });
  for (const [name, body] of Object.entries(defs)) {
    writeExec(
      join(bin, name),
      `#!/bin/sh
LOG=${sh(log)}
printf '%s' ${sh(name)} >> "$LOG"
for a in "$@"; do printf ' %s' "$a" >> "$LOG"; done
printf '\\n' >> "$LOG"
${body}
`,
    );
  }
  return {
    env: { PATH: `${bin}:${process.env.PATH ?? ""}` },
    log,
    calls: () => (existsSync(log) ? readFileSync(log, "utf8").split("\n").filter(Boolean) : []),
  };
}

/** A tiny app bundle; the `file` shim decides which entries are Mach-O. */
function makeApp(dir: string, withHelper = false) {
  const app = join(dir, "Parla.app");
  const macos = join(app, "Contents", "MacOS");
  mkdirSync(macos, { recursive: true });
  writeExec(join(macos, "Parla"), "fake mach-o\n");
  if (withHelper) {
    const fw = join(app, "Contents", "Frameworks", "Helper.app", "Contents", "MacOS");
    mkdirSync(fw, { recursive: true });
    writeExec(join(fw, "Helper"), "fake mach-o\n");
  }
  // A non-Mach-O resource that must not be signed as a binary.
  mkdirSync(join(app, "Contents", "Resources"), { recursive: true });
  writeFileSync(join(app, "Contents", "Resources", "Info.txt"), "hi");
  return app;
}

const fileShim = `if [ "$1" = "-b" ]; then
  case "$2" in
    */Contents/MacOS/*) printf 'Mach-O 64-bit executable\\n' ;;
    *) printf 'ASCII text\\n' ;;
  esac
fi
exit 0
`;

function codesignShim(opts: {
  entitlements: string;
  requirement?: string;
  flags?: string;
}): string {
  return `if [ "$1" = "-d" ] && [ "$2" = "--entitlements" ]; then
  if [ "$3" = ":-" ]; then cat ${sh(opts.entitlements)}; else cat ${sh(opts.entitlements)} > "$3"; fi
  exit 0
fi
if [ "$1" = "-dv" ]; then
  printf '%s\\n' ${sh(
    opts.flags ??
      "CodeDirectory v=20400 size=1 flags=0x10000(runtime) hashes=1+0 location=embedded",
  )}
  exit 0
fi
if [ "$1" = "-d" ] && [ "$2" = "-r-" ]; then
  printf '%s\\n' ${sh(opts.requirement ?? 'designated => identifier "com.justparla.parla"')}
  exit 0
fi
exit 0
`;
}

function securityShim(opts: { keychains?: string[] } = {}): string {
  const keychains = opts.keychains ?? ["/Library/Keychains/System.keychain"];
  return `if [ "$1" = "list-keychains" ] && [ "$4" != "-s" ]; then
${keychains.map((k) => `  printf '    "%s"\\n' ${sh(k)}`).join("\n")}
  exit 0
fi
if [ "$1" = "import" ]; then
  if [ -f "$2" ]; then printf 'import-p12: present\\n'; else printf 'import-p12: missing\\n'; fi >> "$LOG"
fi
if [ "$1" = "set-key-partition-list" ]; then
  if [ -f "$RUNNER_TEMP/cert.p12" ]; then printf 'partition-list-p12: present\\n'; else printf 'partition-list-p12: missing\\n'; fi >> "$LOG"
fi
exit 0
`;
}

/** Records the staging tree the dmg task hands to hdiutil, then writes the image. */
const hdiutilShim = `if [ "$1" = "create" ]; then
  stage=
  out=
  prev=
  for a in "$@"; do
    [ "$prev" = "-srcfolder" ] && stage=$a
    prev=$a
    out=$a
  done
  if [ -d "$stage" ]; then
    printf 'stage:' >> "$LOG"
    for entry in "$stage"/* "$stage"/.[!.]*; do
      [ -e "$entry" ] || continue
      printf ' %s' "\${entry##*/}" >> "$LOG"
    done
    printf '\\n' >> "$LOG"
    printf 'applications-link: %s\\n' "$(readlink "$stage/Applications" 2>/dev/null || true)" >> "$LOG"
  fi
  : > "$out"
fi
exit 0
`;

function otoolShim(deps: string[]): string {
  return `if [ "$1" = "-L" ]; then
  printf '%s:\\n' "$2"
${deps.map((d) => `  printf '  %s (compatibility version 1.0.0, current version 1.0.0)\\n' ${sh(d)}`).join("\n")}
fi
exit 0
`;
}

const noopShim = "exit 0\n";

const selfSignedSecrets = {
  MACOS_SELF_SIGNED_P12: "ZmFrZS1zZWxmLXNpZ25lZC1wMTI=",
  MACOS_SELF_SIGNED_PASSWORD: "self-sekret-pass-456",
};

const noSigningSecrets: Record<string, string | undefined> = {
  MACOS_SELF_SIGNED_P12: undefined,
  MACOS_SELF_SIGNED_PASSWORD: undefined,
};

// A real self-signed certificate for the tests that exercise the keychain's
// import path; generated once per run through the owner's own task.
let realCert: { p12: string; password: string } | undefined;

function selfSignedCert() {
  if (!realCert) {
    const dir = makeTemp("parla-cert-");
    const out = run(["mise-tasks/release/self-signed-cert", "--", dir]);
    expect(out.code, out.stderr).toBe(0);
    realCert = {
      p12: readFileSync(join(dir, "parla-release.p12.b64"), "utf8"),
      password: readFileSync(join(dir, "password.txt"), "utf8"),
    };
  }
  return realCert;
}

describe("release signing mode", () => {
  test("selects self-signed and writes the job summary", () => {
    const dir = makeTemp("parla-signing-mode-");
    const summary = join(dir, "summary.md");
    writeFileSync(summary, "");
    const out = run(["mise-tasks/release/signing-mode"], {
      env: { ...noSigningSecrets, ...selfSignedSecrets, GITHUB_STEP_SUMMARY: summary },
    });
    expect(out.code).toBe(0);
    expect(out.stdout).toBe("self-signed\n");
    expect(readFileSync(summary, "utf8")).toBe("Signing mode: self-signed\n");
  });

  test("selects adhoc and warns about repeated keychain prompts", () => {
    const out = run(["mise-tasks/release/signing-mode"], {
      env: { ...noSigningSecrets, GITHUB_STEP_SUMMARY: undefined },
    });
    expect(out.code).toBe(0);
    expect(out.stdout).toBe("adhoc\n");
    expect(out.stderr).toMatch(/keychain/i);
    expect(out.stderr).toMatch(/every update/i);
  });

  test("fails on an incomplete self-signed configuration naming the missing variable", () => {
    const p12Only = run(["mise-tasks/release/signing-mode"], {
      env: {
        ...noSigningSecrets,
        MACOS_SELF_SIGNED_P12: selfSignedSecrets.MACOS_SELF_SIGNED_P12,
      },
    });
    expect(p12Only.code).not.toBe(0);
    expect(p12Only.stderr).toContain("MACOS_SELF_SIGNED_PASSWORD");
    expect(p12Only.stdout).not.toContain("self-signed");
    expect(p12Only.stdout + p12Only.stderr).not.toContain(
      selfSignedSecrets.MACOS_SELF_SIGNED_P12,
    );

    const passwordOnly = run(["mise-tasks/release/signing-mode"], {
      env: {
        ...noSigningSecrets,
        MACOS_SELF_SIGNED_PASSWORD: selfSignedSecrets.MACOS_SELF_SIGNED_PASSWORD,
      },
    });
    expect(passwordOnly.code).not.toBe(0);
    expect(passwordOnly.stderr).toContain("MACOS_SELF_SIGNED_P12");
    expect(passwordOnly.stdout + passwordOnly.stderr).not.toContain(
      selfSignedSecrets.MACOS_SELF_SIGNED_PASSWORD,
    );
  });

  test("never prints secret values", () => {
    const out = run(["mise-tasks/release/signing-mode"], {
      env: { ...noSigningSecrets, ...selfSignedSecrets },
    });
    expect(out.code).toBe(0);
    const combined = out.stdout + out.stderr;
    for (const value of Object.values(selfSignedSecrets)) {
      expect(combined).not.toContain(value);
    }
  });
});

describe("release sign task", () => {
  test("sign_self_signed_uses_the_hash_a_timestamp_and_entitlements", () => {
    const dir = makeTemp("parla-sign-");
    const app = makeApp(dir);
    const identity = join(dir, "identity");
    const sha1 = "0123456789ABCDEF0123456789ABCDEF01234567";
    writeFileSync(identity, `${sha1}\n`);
    const tools = fakeTools(dir, { codesign: codesignShim({ entitlements }), file: fileShim });
    const out = run(["mise-tasks/release/sign", "--", app, "--identity-file", identity], {
      env: { ...noSigningSecrets, ...tools.env },
    });
    expect(out.code, out.stderr).toBe(0);
    const calls = tools.calls().filter((l) => l.startsWith("codesign "));
    // The main executable and the bundle, not the resource file.
    expect(calls).toHaveLength(2);
    for (const call of calls) {
      expect(call).toContain("--options runtime");
      expect(call).toContain(`--entitlements ${entitlements}`);
      expect(call).toContain(`--sign ${sha1}`);
      expect(call).toContain("--timestamp");
    }
    expect(calls.at(-1)).toMatch(new RegExp(` ${app.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`));
    // The task itself never prints the identity hash.
    expect(out.stdout + out.stderr).not.toContain(sha1);
  });

  test("sign_adhoc_does_not_timestamp", () => {
    const dir = makeTemp("parla-sign-");
    const app = makeApp(dir);
    const tools = fakeTools(dir, { codesign: codesignShim({ entitlements }), file: fileShim });
    const out = run(["mise-tasks/release/sign", "--", app, "--adhoc"], {
      env: { ...noSigningSecrets, ...tools.env },
    });
    expect(out.code, out.stderr).toBe(0);
    const calls = tools.calls().filter((l) => l.startsWith("codesign "));
    expect(calls).toHaveLength(2);
    for (const call of calls) {
      expect(call).toContain("--sign - ");
      expect(call).toContain(`--entitlements ${entitlements}`);
      expect(call).not.toContain("timestamp");
    }
  });

  test("sign_never_uses_deep", () => {
    const dir = makeTemp("parla-sign-");
    const app = makeApp(dir, true);
    const identity = join(dir, "identity");
    writeFileSync(identity, "0123456789ABCDEF0123456789ABCDEF01234567\n");
    const tools = fakeTools(dir, { codesign: codesignShim({ entitlements }), file: fileShim });
    const out = run(["mise-tasks/release/sign", "--", app, "--identity-file", identity], {
      env: { ...noSigningSecrets, ...tools.env },
    });
    expect(out.code, out.stderr).toBe(0);
    expect(tools.calls().join("\n")).not.toContain("--deep");
    expect(out.stdout + out.stderr).not.toContain("--deep");
  });

  test("sign signs nested Mach-O before the app", () => {
    const dir = makeTemp("parla-sign-");
    const app = makeApp(dir, true);
    const identity = join(dir, "identity");
    writeFileSync(identity, "0123456789ABCDEF0123456789ABCDEF01234567\n");
    const tools = fakeTools(dir, { codesign: codesignShim({ entitlements }), file: fileShim });
    const out = run(["mise-tasks/release/sign", "--", app, "--identity-file", identity], {
      env: { ...noSigningSecrets, ...tools.env },
    });
    expect(out.code, out.stderr).toBe(0);
    const calls = tools.calls().filter((l) => l.startsWith("codesign "));
    expect(calls).toHaveLength(3);
    const helperIdx = calls.findIndex((l) => l.includes("Helper.app/Contents/MacOS/Helper"));
    const mainIdx = calls.findIndex((l) => l.includes("/Contents/MacOS/Parla"));
    const appIdx = calls.findIndex((l) => l.endsWith(` ${app}`));
    expect(helperIdx).toBeGreaterThanOrEqual(0);
    expect(mainIdx).toBeGreaterThanOrEqual(0);
    expect(appIdx).toBeGreaterThanOrEqual(0);
    expect(helperIdx).toBeLessThan(mainIdx);
    expect(mainIdx).toBeLessThan(appIdx);
  });

  test("sign rejects an empty identity file", () => {
    const dir = makeTemp("parla-sign-");
    const app = makeApp(dir);
    const identity = join(dir, "identity");
    writeFileSync(identity, "\n");
    const out = run(["mise-tasks/release/sign", "--", app, "--identity-file", identity], {
      env: noSigningSecrets,
    });
    expect(out.code).toBe(1);
    expect(out.stderr).toContain("signing identity file is empty");
  });
});

describe("release dmg task", () => {
  function dmgEnv(dir: string, tools: Shims) {
    return { ...noSigningSecrets, ...tools.env, PARLA_RELEASE_ASSETS: join(dir, "assets") };
  }

  test("dmg_self_signed_uses_a_timestamp_and_masks_the_hash", () => {
    const dir = makeTemp("parla-sign-");
    const app = makeApp(dir);
    const identity = join(dir, "identity");
    const sha1 = "0123456789ABCDEF0123456789ABCDEF01234567";
    writeFileSync(identity, `${sha1}\n`);
    const tools = fakeTools(dir, {
      codesign: codesignShim({ entitlements }),
      hdiutil: hdiutilShim,
    });
    const out = run(["mise-tasks/release/dmg", "--", app, "0.0.0", "--identity-file", identity], {
      env: dmgEnv(dir, tools),
    });
    expect(out.code, out.stderr).toBe(0);
    const dmg = join(dir, "assets", "Parla-0.0.0-macos-universal.dmg");
    const calls = tools.calls();
    const hdiutil = calls.find((l) => l.startsWith("hdiutil create "));
    expect(hdiutil).toContain("-volname Parla");
    expect(hdiutil).toContain(`-srcfolder `);
    expect(hdiutil).toContain("-fs HFS+");
    expect(hdiutil).toContain("-format UDZO");
    expect(hdiutil).toEndWith(` ${dmg}`);
    expect(calls).toContain("stage: Applications Parla.app");
    expect(calls).toContain("applications-link: /Applications");
    expect(existsSync(dmg)).toBe(true);
    const codesign = calls.filter((l) => l.startsWith("codesign "));
    expect(codesign).toHaveLength(1);
    expect(codesign[0]).toContain("--timestamp");
    expect(codesign[0]).toContain(`--sign ${sha1}`);
    expect(codesign[0]).toEndWith(` ${dmg}`);
    expect(out.stdout).toContain(`wrote ${dmg}`);
    expect(out.stdout + out.stderr).not.toContain(sha1);
  });

  test("dmg_adhoc_does_not_timestamp", () => {
    const dir = makeTemp("parla-sign-");
    const app = makeApp(dir);
    const tools = fakeTools(dir, {
      codesign: codesignShim({ entitlements }),
      hdiutil: hdiutilShim,
    });
    const out = run(["mise-tasks/release/dmg", "--", app, "0.0.0", "--adhoc"], {
      env: dmgEnv(dir, tools),
    });
    expect(out.code, out.stderr).toBe(0);
    const calls = tools.calls();
    const codesign = calls.filter((l) => l.startsWith("codesign "));
    expect(codesign).toHaveLength(1);
    expect(codesign[0]).toContain("--force --sign - ");
    expect(codesign[0]).not.toContain("timestamp");
    expect(calls).toContain("stage: Applications Parla.app");
    expect(calls).toContain("applications-link: /Applications");
  });

  test("dmg rejects a missing app bundle", () => {
    const dir = makeTemp("parla-sign-");
    const out = run(
      ["mise-tasks/release/dmg", "--", join(dir, "Missing.app"), "0.0.0", "--adhoc"],
      { env: { ...noSigningSecrets, PARLA_RELEASE_ASSETS: join(dir, "assets") } },
    );
    expect(out.code).toBe(1);
    expect(out.stderr).toContain("app bundle not found");
  });
});

describe("release task argument validation", () => {
  test("sign, dmg and verify require exactly one mode flag", () => {
    const dir = makeTemp("parla-sign-");
    const app = makeApp(dir);
    const identity = join(dir, "identity");
    writeFileSync(identity, "0123456789ABCDEF0123456789ABCDEF01234567\n");
    const dmg = join(dir, "Parla.dmg");
    writeFileSync(dmg, "dmg");
    const env = { ...noSigningSecrets, PARLA_RELEASE_ASSETS: join(dir, "assets") };

    for (const args of [
      ["mise-tasks/release/sign", "--", app],
      ["mise-tasks/release/dmg", "--", app, "0.0.0"],
      ["mise-tasks/release/verify", "--", app, dmg],
      ["mise-tasks/release/sign", "--", app, "--adhoc", "--identity-file", identity],
      ["mise-tasks/release/dmg", "--", app, "0.0.0", "--adhoc", "--identity-file", identity],
      ["mise-tasks/release/verify", "--", app, dmg, "--adhoc", "--self-signed", "abc"],
    ]) {
      const out = run(args, { env });
      expect(out.code, args.join(" ")).toBe(2);
      expect(out.stdout + out.stderr).toContain("usage:");
    }
  });
});

describe("release tasks never print signing secrets", () => {
  test("keychain, sign, dmg, verify, sums and cleanup keep secrets off stdout and stderr", () => {
    const cert = selfSignedCert();
    const dir = makeTemp("parla-sign-");
    const app = makeApp(dir);
    const rt = join(dir, "rt");
    mkdirSync(rt, { recursive: true });
    const sha1 = "0123456789ABCDEF0123456789ABCDEF01234567";
    const tools = fakeTools(dir, {
      codesign: codesignShim({
        entitlements,
        requirement: `designated => certificate leaf = H"${sha1}"`,
      }),
      security: securityShim(),
      hdiutil: hdiutilShim,
      otool: otoolShim(["/usr/lib/libSystem.B.dylib"]),
      xcrun: noopShim,
      spctl: noopShim,
    });
    const identity = join(rt, "parla-signing-identity");
    writeFileSync(identity, `${sha1}\n`);
    const env = {
      ...tools.env,
      PARLA_SIGNING_MODE: "self-signed",
      RUNNER_TEMP: rt,
      PARLA_RELEASE_ASSETS: join(dir, "assets"),
      MACOS_SELF_SIGNED_P12: cert.p12,
      MACOS_SELF_SIGNED_PASSWORD: cert.password,
    };
    const dmg = join(dir, "fake.dmg");
    const sumsFile = join(dir, "f.txt");
    writeFileSync(dmg, "dmg");
    writeFileSync(sumsFile, "x");
    const tasks: string[][] = [
      ["mise-tasks/release/keychain"],
      ["mise-tasks/release/sign", "--", app, "--identity-file", identity],
      ["mise-tasks/release/dmg", "--", app, "0.0.0", "--identity-file", identity],
      ["mise-tasks/release/verify", "--", app, dmg, "--self-signed", sha1],
      ["mise-tasks/release/sums", "--", sumsFile],
      ["mise-tasks/release/cleanup"],
    ];
    for (const args of tasks) {
      const out = run(args, { env });
      expect(out.code, `${args.join(" ")}: ${out.stderr}`).toBe(0);
      const combined = out.stdout + out.stderr;
      expect(combined, `${args.join(" ")} leaked the p12`).not.toContain(cert.p12);
      expect(combined, `${args.join(" ")} leaked the password`).not.toContain(cert.password);
    }
  });
});

describe("release keychain task", () => {
  test("keychain imports the p12, writes the SHA-1 and prepends the search list", () => {
    const cert = selfSignedCert();
    const dir = makeTemp("parla-sign-");
    const rt = join(dir, "rt");
    mkdirSync(rt, { recursive: true });
    const tools = fakeTools(dir, {
      security: securityShim({
        keychains: ["/Users/test/Library/Keychains/login.keychain-db"],
      }),
    });
    const out = run(["mise-tasks/release/keychain"], {
      env: {
        ...noSigningSecrets,
        ...tools.env,
        PARLA_SIGNING_MODE: "self-signed",
        RUNNER_TEMP: rt,
        MACOS_SELF_SIGNED_P12: cert.p12,
        MACOS_SELF_SIGNED_PASSWORD: cert.password,
      },
    });
    expect(out.code, out.stderr).toBe(0);
    const calls = tools.calls();
    const keychain = `${rt}/parla-signing.keychain-db`;
    const p12 = `${rt}/cert.p12`;

    const create = calls.find((l) => l.startsWith("security create-keychain "));
    const unlock = calls.find((l) => l.startsWith("security unlock-keychain "));
    const partition = calls.find((l) => l.startsWith("security set-key-partition-list "));
    expect(create).toBeDefined();
    expect(create).toContain(keychain);
    expect(unlock).toBeDefined();
    // create and unlock share the generated keychain password.
    expect(create!.split(" ")[3]).toBe(unlock!.split(" ")[3]);
    expect(partition).toContain("-S apple-tool:,apple: -s");
    expect(partition!.split(" ")[6]).toBe(create!.split(" ")[3]);
    expect(calls).toContain(
      `security set-keychain-settings -lut 21600 ${keychain}`,
    );

    const imported = calls.find((l) => l.startsWith("security import "));
    expect(imported).toContain(`${p12} -k ${keychain}`);
    expect(imported).toContain(`-P ${cert.password}`);
    expect(imported).toContain("-T /usr/bin/codesign");
    expect(imported).toContain("-T /usr/bin/security");
    // The decoded p12 existed at import time and was gone before the partition list.
    expect(calls).toContain("import-p12: present");
    expect(calls).toContain("partition-list-p12: missing");

    expect(calls).toContain(
      `security list-keychains -d user -s ${keychain} /Users/test/Library/Keychains/login.keychain-db`,
    );
    expect(calls.some((l) => l.startsWith("default-keychain"))).toBe(false);

    expect(readFileSync(`${rt}/parla-signing-identity`, "utf8")).toMatch(/^[0-9A-F]{40}\n$/);
    expect(existsSync(p12)).toBe(false);
    expect(out.stdout + out.stderr).not.toContain(cert.p12);
    expect(out.stdout + out.stderr).not.toContain(cert.password);
  });

  test("keychain_adhoc_does_nothing", () => {
    const out = run(["mise-tasks/release/keychain"], {
      env: { ...noSigningSecrets, PARLA_SIGNING_MODE: "adhoc", RUNNER_TEMP: undefined },
    });
    expect(out.code).toBe(0);
    expect(out.stdout + out.stderr).toBe("");
  });

  test("keychain rejects an unknown signing mode", () => {
    const out = run(["mise-tasks/release/keychain"], {
      env: { ...noSigningSecrets, PARLA_SIGNING_MODE: "developer", RUNNER_TEMP: undefined },
    });
    expect(out.code).toBe(2);
    expect(out.stdout + out.stderr).toContain("invalid PARLA_SIGNING_MODE");
  });

  test("keychain restores the search list preserving paths with spaces", () => {
    const dir = makeTemp("parla-sign-");
    const rt = join(dir, "rt");
    mkdirSync(rt, { recursive: true });
    const tools = fakeTools(dir, {
      security: securityShim({
        keychains: [
          "/Users/test/Library/Keychains/login.keychain-db",
          "/Library/Keychains/With Space/System.keychain",
        ],
      }),
    });
    const cert = selfSignedCert();
    const out = run(["mise-tasks/release/keychain"], {
      env: {
        ...tools.env,
        PARLA_SIGNING_MODE: "self-signed",
        RUNNER_TEMP: rt,
        MACOS_SELF_SIGNED_P12: cert.p12,
        MACOS_SELF_SIGNED_PASSWORD: cert.password,
      },
    });
    expect(out.code, out.stderr).toBe(0);
    const calls = tools.calls();
    expect(
      calls.find((l) => l.startsWith("security list-keychains -d user -s ")),
    ).toBe(
      `security list-keychains -d user -s ${rt}/parla-signing.keychain-db` +
        " /Users/test/Library/Keychains/login.keychain-db" +
        " /Library/Keychains/With Space/System.keychain",
    );
    expect(calls.some((l) => l.startsWith("default-keychain"))).toBe(false);
    expect(readFileSync(join(rt, "parla-signing-identity"), "utf8")).toMatch(/^[0-9A-F]{40}\n$/);
    expect(existsSync(join(rt, "cert.p12"))).toBe(false);
  });
});

describe("release cleanup task", () => {
  test("deletes the temporary keychain, identity and p12", () => {
    const dir = makeTemp("parla-cleanup-");
    const rt = join(dir, "rt");
    mkdirSync(rt, { recursive: true });
    const keychain = join(rt, "parla-signing.keychain-db");
    const identity = join(rt, "parla-signing-identity");
    const p12 = join(rt, "cert.p12");
    writeFileSync(keychain, "keychain");
    writeFileSync(identity, "sha1");
    writeFileSync(p12, "p12");
    const tools = fakeTools(dir, {
      security: `if [ "$1" = "delete-keychain" ]; then rm -f "$2"; fi
exit 0
`,
    });
    const out = run(["mise-tasks/release/cleanup"], {
      env: { ...tools.env, RUNNER_TEMP: rt },
    });
    expect(out.code, out.stderr).toBe(0);
    expect(tools.calls()).toContain(`security delete-keychain ${keychain}`);
    expect(existsSync(keychain)).toBe(false);
    expect(existsSync(identity)).toBe(false);
    expect(existsSync(p12)).toBe(false);
  });

  test("cleanup rejects an unknown argument", () => {
    const out = run(["mise-tasks/release/cleanup", "--nope"], {
      env: { RUNNER_TEMP: "/tmp" },
    });
    expect(out.code).toBe(2);
    expect(out.stderr).toContain("unknown argument");
  });
});

describe("release verify task", () => {
  function verifyTools(
    dir: string,
    opts: { requirement?: string; flags?: string; deps?: string[]; entitlements?: string } = {},
  ) {
    return fakeTools(dir, {
      codesign: codesignShim({
        entitlements: opts.entitlements ?? entitlements,
        requirement: opts.requirement,
        flags: opts.flags,
      }),
      otool: otoolShim(opts.deps ?? ["/usr/lib/libSystem.B.dylib"]),
      xcrun: noopShim,
      spctl: noopShim,
    });
  }

  test("verify_self_signed_checks_the_requirement_and_library_dependencies", () => {
    const dir = makeTemp("parla-verify-");
    const app = makeApp(dir);
    const dmg = join(dir, "Parla.dmg");
    writeFileSync(dmg, "dmg");
    const sha1 = "0123456789ABCDEF0123456789ABCDEF01234567";
    const tools = verifyTools(dir, {
      requirement: `designated => certificate leaf = H"${sha1}"`,
    });
    const out = run(
      ["mise-tasks/release/verify", "--", app, dmg, "--self-signed", sha1.toLowerCase()],
      { env: tools.env },
    );
    expect(out.code, out.stderr).toBe(0);
    expect(out.stdout).toContain("verify: ok");
    const calls = tools.calls();
    expect(calls).toContain(`codesign --verify --strict --deep ${app}`);
    expect(calls).toContain(`codesign --verify --strict --deep ${dmg}`);
    expect(calls).toContain(`codesign -d --entitlements :- ${app}`);
    expect(calls).toContain(`codesign -dv ${app}`);
    expect(calls).toContain(`codesign -d -r- ${app}`);
    expect(calls).toContain(`otool -L ${app}/Contents/MacOS/Parla`);
    expect(calls.some((l) => l.startsWith("spctl "))).toBe(false);
    expect(calls.some((l) => l.startsWith("xcrun "))).toBe(false);
  });

  test("verify_adhoc_skips_the_requirement_and_dependency_checks", () => {
    const dir = makeTemp("parla-verify-");
    const app = makeApp(dir);
    const dmg = join(dir, "Parla.dmg");
    writeFileSync(dmg, "dmg");
    const tools = verifyTools(dir);
    const out = run(["mise-tasks/release/verify", "--", app, dmg, "--adhoc"], {
      env: tools.env,
    });
    expect(out.code, out.stderr).toBe(0);
    const calls = tools.calls();
    expect(calls).toContain(`codesign --verify --strict --deep ${app}`);
    expect(calls).toContain(`codesign --verify --strict --deep ${dmg}`);
    expect(calls.some((l) => l.includes("-d -r-"))).toBe(false);
    expect(calls.some((l) => l.startsWith("otool "))).toBe(false);
    expect(calls.some((l) => l.startsWith("spctl "))).toBe(false);
    expect(calls.some((l) => l.startsWith("xcrun "))).toBe(false);
  });

  test("verify_self_signed_rejects_a_requirement_without_the_leaf_hash", () => {
    const dir = makeTemp("parla-verify-");
    const app = makeApp(dir);
    const dmg = join(dir, "Parla.dmg");
    writeFileSync(dmg, "dmg");
    const tools = verifyTools(dir, { requirement: 'designated => identifier "com.justparla.parla"' });
    const out = run(
      ["mise-tasks/release/verify", "--", app, dmg, "--self-signed", "0123456789abcdef0123456789abcdef01234567"],
      { env: tools.env },
    );
    expect(out.code).not.toBe(0);
    expect(out.stdout + out.stderr).toMatch(/certificate leaf/i);
  });

  test("verify_self_signed_rejects_external_dependencies", () => {
    const dir = makeTemp("parla-verify-");
    const app = makeApp(dir);
    const dmg = join(dir, "Parla.dmg");
    writeFileSync(dmg, "dmg");
    const sha1 = "0123456789ABCDEF0123456789ABCDEF01234567";
    const tools = verifyTools(dir, {
      requirement: `designated => certificate leaf = H"${sha1}"`,
      deps: ["/opt/homebrew/lib/libparla_bad.dylib"],
    });
    const out = run(
      ["mise-tasks/release/verify", "--", app, dmg, "--self-signed", sha1],
      { env: tools.env },
    );
    expect(out.code).not.toBe(0);
    expect(out.stdout + out.stderr).toContain("/opt/homebrew/lib/libparla_bad.dylib");
  });

  test("verify_rejects_runtime_only_in_path", () => {
    const dir = makeTemp("parla-verify-");
    const app = join(dir, "Parla.app");
    const dmg = join(dir, "Parla.dmg");
    mkdirSync(app, { recursive: true });
    writeFileSync(dmg, "dmg");
    const tools = verifyTools(dir, {
      flags: [
        "Executable=/tmp/runtime/Parla.app/Contents/MacOS/Parla",
        "Identifier=com.justparla.parla",
        "Format=app bundle with Mach-O thin (arm64)",
        "CodeDirectory v=20400 size=1 flags=0x0(none) hashes=1+0 location=embedded",
      ].join("\n"),
    });
    const out = run(["mise-tasks/release/verify", "--", app, dmg, "--adhoc"], {
      env: tools.env,
    });
    expect(out.code).not.toBe(0);
    expect(out.stdout + out.stderr).toMatch(/hardened runtime/);
  });

  test("verify_rejects_get_task_allow", () => {
    const dir = makeTemp("parla-verify-");
    const app = makeApp(dir);
    const dmg = join(dir, "Parla.dmg");
    writeFileSync(dmg, "dmg");
    const ent = join(dir, "bad.entitlements");
    writeFileSync(
      ent,
      `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>com.apple.security.get-task-allow</key><true/></dict></plist>`,
    );
    const tools = verifyTools(dir, { entitlements: ent });
    const out = run(["mise-tasks/release/verify", "--", app, dmg, "--adhoc"], {
      env: tools.env,
    });
    expect(out.code).not.toBe(0);
    expect(out.stdout + out.stderr).toMatch(/get-task-allow/);
  });
});

describe("release sums task", () => {
  test("sums format matches shasum -c", () => {
    const dir = makeTemp("parla-sums-");
    const a = join(dir, "a.txt");
    const b = join(dir, "b.bin");
    writeFileSync(a, "hello");
    writeFileSync(b, Buffer.from([0, 1, 2, 3]));
    const out = run([join(root, "mise-tasks/release/sums"), "--", a, b], { cwd: dir });
    expect(out.code, out.stderr).toBe(0);
    const sumsPath = join(dir, "SHA256SUMS");
    const sums = readFileSync(sumsPath, "utf8");
    // shasum -a 256 format: "<hash>  <filename>" (two spaces)
    for (const line of sums.trim().split("\n")) {
      expect(line).toMatch(/^[0-9a-f]{64}  .+/);
    }
    const check = Bun.spawnSync(["shasum", "-a", "256", "-c", "SHA256SUMS"], {
      cwd: dir,
      stdout: "pipe",
      stderr: "pipe",
    });
    expect(check.exitCode).toBe(0);
  });

  test("sums hashes basenames so verification works in asset dir", () => {
    const dir = makeTemp("parla-sums-");
    const assetDir = join(dir, "assets");
    const other = join(dir, "other");
    mkdirSync(assetDir, { recursive: true });
    mkdirSync(other, { recursive: true });
    const a = join(assetDir, "a.txt");
    const b = join(assetDir, "b.bin");
    writeFileSync(a, "hello");
    writeFileSync(b, Buffer.from([0, 1, 2, 3]));
    const out = run([join(root, "mise-tasks/release/sums"), "--", a, b], { cwd: other });
    expect(out.code, out.stderr).toBe(0);
    expect(out.stdout).toContain(`wrote ${assetDir}/SHA256SUMS`);
    const sums = readFileSync(join(assetDir, "SHA256SUMS"), "utf8");
    expect(sums).toContain("  a.txt\n");
    expect(sums).toContain("  b.bin\n");
    expect(sums).not.toContain(assetDir);
    const check = Bun.spawnSync(["shasum", "-a", "256", "-c", "--ignore-missing", "SHA256SUMS"], {
      cwd: assetDir,
      stdout: "pipe",
      stderr: "pipe",
    });
    expect(check.exitCode).toBe(0);
  });

  test("sums requires at least one file", () => {
    const dir = makeTemp("parla-sums-");
    const out = run([join(root, "mise-tasks/release/sums"), "--"], { cwd: dir });
    expect(out.code).toBe(2);
    expect(out.stderr).toContain("usage:");
  });

  test("sums rejects files from different directories", () => {
    const dir = makeTemp("parla-sums-");
    const one = join(dir, "one");
    const two = join(dir, "two");
    mkdirSync(one, { recursive: true });
    mkdirSync(two, { recursive: true });
    const a = join(one, "a.txt");
    const b = join(two, "b.txt");
    writeFileSync(a, "a");
    writeFileSync(b, "b");
    const out = run([join(root, "mise-tasks/release/sums"), "--", a, b], { cwd: dir });
    expect(out.code).toBe(2);
    expect(out.stderr).toContain("share one directory");
    expect(existsSync(join(one, "SHA256SUMS"))).toBe(false);
  });
});

describe("release update-sign", () => {
  /** A minimal Parla.app whose binary optionally embeds the update public key. */
  function makeUpdateApp(dir: string, key = "no update key here\n") {
    const macos = join(dir, "Parla.app", "Contents", "MacOS");
    mkdirSync(macos, { recursive: true });
    writeFileSync(join(macos, "parla"), key);
    return join(dir, "Parla.app");
  }

  function minisignPath() {
    const p = Bun.which("minisign");
    if (!p) {
      throw new Error("minisign is not installed; run `mise install` first");
    }
    return p;
  }

  function makeKeyPair(dir: string) {
    const pub = join(dir, "key.pub");
    const key = join(dir, "key.key");
    const gen = Bun.spawnSync([minisignPath(), "-G", "-W", "-p", pub, "-s", key, "-f"], {
      cwd: dir,
      stdout: "pipe",
      stderr: "pipe",
    });
    expect(gen.exitCode).toBe(0);
    return {
      secret: readFileSync(key, "utf8"),
      publicKey: readFileSync(pub, "utf8").trim().split("\n")[1],
    };
  }

  test("no secret exits 0, warns, and writes no .minisig", () => {
    const dir = makeTemp("parla-update-sign-");
    const dmg = join(dir, "Parla-1.2.3-macos-universal.dmg");
    writeFileSync(dmg, "dmg");
    const app = makeUpdateApp(dir);
    const summary = join(dir, "summary.md");
    writeFileSync(summary, "");
    const out = run(["mise-tasks/release/update-sign", "--", dmg, "1.2.3", app], {
      env: {
        PARLA_UPDATE_SECRET_KEY: "",
        PARLA_UPDATE_PUBLIC_KEY: "",
        GITHUB_STEP_SUMMARY: summary,
      },
    });
    expect(out.code).toBe(0);
    expect(out.stderr).toContain("download link only");
    expect(existsSync(join(dir, "Parla-1.2.3-macos-universal.dmg.minisig"))).toBe(false);
    expect(readFileSync(summary, "utf8")).toContain("download link only");
  });

  test("secret without public key exits 1", () => {
    const dir = makeTemp("parla-update-sign-");
    const dmg = join(dir, "Parla-1.2.3-macos-universal.dmg");
    writeFileSync(dmg, "dmg");
    const app = makeUpdateApp(dir);
    const out = run(["mise-tasks/release/update-sign", "--", dmg, "1.2.3", app], {
      env: { PARLA_UPDATE_SECRET_KEY: "some-key", PARLA_UPDATE_PUBLIC_KEY: "" },
    });
    expect(out.code).toBe(1);
    expect(existsSync(join(dir, "Parla-1.2.3-macos-universal.dmg.minisig"))).toBe(false);
  });

  test("signs when the app embeds the public key", () => {
    const dir = makeTemp("parla-update-sign-");
    const dmg = join(dir, "Parla-1.2.3-macos-universal.dmg");
    writeFileSync(dmg, "dmg payload");
    const tools = minisignShim(dir, "parla 1.2.3 Parla-1.2.3-macos-universal.dmg");
    const app = makeUpdateApp(dir, "compiled in: public-key\n");
    const out = run(["mise-tasks/release/update-sign", "--", dmg, "1.2.3", app], {
      env: {
        ...tools.env,
        PARLA_UPDATE_SECRET_KEY: "secret-key",
        PARLA_UPDATE_PUBLIC_KEY: "public-key",
      },
    });
    expect(out.code, out.stderr).toBe(0);
    expect(existsSync(`${dmg}.minisig`)).toBe(true);
  });

  test("fails when the app does not embed the public key", () => {
    const dir = makeTemp("parla-update-sign-");
    const dmg = join(dir, "Parla-1.2.3-macos-universal.dmg");
    writeFileSync(dmg, "dmg payload");
    const tools = minisignShim(dir, "parla 1.2.3 Parla-1.2.3-macos-universal.dmg");
    const app = makeUpdateApp(dir, "no key here\n");
    const out = run(["mise-tasks/release/update-sign", "--", dmg, "1.2.3", app], {
      env: {
        ...tools.env,
        PARLA_UPDATE_SECRET_KEY: "secret-key",
        PARLA_UPDATE_PUBLIC_KEY: "public-key",
      },
    });
    expect(out.code).toBe(1);
    expect(out.stderr).toContain("does not embed PARLA_UPDATE_PUBLIC_KEY");
    expect(existsSync(`${dmg}.minisig`)).toBe(false);
    expect(tools.calls().some((l) => l.startsWith("minisign -S "))).toBe(false);
  });

  test("real run signs, verifies, and never prints the secret key", () => {
    const dir = makeTemp("parla-update-sign-");
    const { secret, publicKey } = makeKeyPair(dir);
    const dmg = join(dir, "Parla-1.2.3-macos-universal.dmg");
    writeFileSync(dmg, "dmg payload");
    const app = makeUpdateApp(dir, `compiled in: ${publicKey}\n`);
    const out = run(["mise-tasks/release/update-sign", "--", dmg, "1.2.3", app], {
      env: { PARLA_UPDATE_SECRET_KEY: secret, PARLA_UPDATE_PUBLIC_KEY: publicKey },
    });
    expect(out.code, out.stderr).toBe(0);
    const sig = join(dir, "Parla-1.2.3-macos-universal.dmg.minisig");
    expect(existsSync(sig)).toBe(true);
    const verify = Bun.spawnSync([minisignPath(), "-V", "-P", publicKey, "-m", dmg, "-x", sig], {
      cwd: dir,
      stdout: "pipe",
      stderr: "pipe",
    });
    expect(verify.exitCode).toBe(0);
    const sigText = readFileSync(sig, "utf8");
    expect(sigText).toContain("trusted comment: parla 1.2.3 Parla-1.2.3-macos-universal.dmg\n");
    const combined = out.stdout + out.stderr;
    expect(combined).not.toContain(secret.trim().split("\n")[1]);
  });

  test("a wrong public key fails and leaves no .minisig", () => {
    const dir = makeTemp("parla-update-sign-");
    const { secret } = makeKeyPair(dir);
    const wrongPub = join(dir, "wrong.pub");
    const wrongKey = join(dir, "wrong.key");
    const gen = Bun.spawnSync([minisignPath(), "-G", "-W", "-p", wrongPub, "-s", wrongKey, "-f"], {
      cwd: dir,
      stdout: "pipe",
      stderr: "pipe",
    });
    expect(gen.exitCode).toBe(0);
    const wrongPublicKey = readFileSync(wrongPub, "utf8").trim().split("\n")[1];
    const dmg = join(dir, "Parla-1.2.3-macos-universal.dmg");
    writeFileSync(dmg, "dmg payload");
    const app = makeUpdateApp(dir, `compiled in: ${wrongPublicKey}\n`);
    const out = run(["mise-tasks/release/update-sign", "--", dmg, "1.2.3", app], {
      env: { PARLA_UPDATE_SECRET_KEY: secret, PARLA_UPDATE_PUBLIC_KEY: wrongPublicKey },
    });
    expect(out.code).toBe(1);
    expect(existsSync(join(dir, "Parla-1.2.3-macos-universal.dmg.minisig"))).toBe(false);
  });

  function minisignShim(dir: string, trustedComment: string) {
    const observedKey = join(dir, "observed-key");
    return fakeTools(dir, {
      minisign: `if [ "$1" = "-S" ]; then
  key=
  out=
  prev=
  for a in "$@"; do
    [ "$prev" = "-s" ] && key=$a
    [ "$prev" = "-x" ] && out=$a
    prev=$a
  done
  cat "$key" > ${sh(observedKey)}
  printf 'untrusted comment: signature\\n' > "$out"
  exit 0
fi
if [ "$1" = "-V" ]; then
  printf '%s\\n' ${sh(`Trusted comment: ${trustedComment}`)}
  exit 0
fi
exit 0
`,
    });
  }

  test("logs the sign and verify commands with the expected trusted comment", () => {
    const dir = makeTemp("parla-update-sign-");
    const dmg = join(dir, "Parla-1.2.3-macos-universal.dmg");
    writeFileSync(dmg, "dmg payload");
    const tools = minisignShim(dir, "parla 1.2.3 Parla-1.2.3-macos-universal.dmg");
    const app = makeUpdateApp(dir, "compiled in: public-key\n");
    const out = run(["mise-tasks/release/update-sign", "--", dmg, "1.2.3", app], {
      env: {
        ...tools.env,
        PARLA_UPDATE_SECRET_KEY: "secret-key",
        PARLA_UPDATE_PUBLIC_KEY: "public-key",
      },
    });
    expect(out.code, out.stderr).toBe(0);
    const calls = tools.calls();
    const sign = calls.find((l) => l.startsWith("minisign -S "));
    expect(sign).toMatch(
      /^minisign -S -s \S+ -m .*Parla-1\.2\.3-macos-universal\.dmg -x .*\.minisig -t parla 1\.2\.3 Parla-1\.2\.3-macos-universal\.dmg -c Parla 1\.2\.3$/,
    );
    expect(calls).toContain(
      `minisign -V -P public-key -m ${dmg} -x ${dmg}.minisig`,
    );
    expect(readFileSync(join(dir, "observed-key"), "utf8")).toBe("secret-key");
    expect(existsSync(`${dmg}.minisig`)).toBe(true);
    expect(out.stdout + out.stderr).not.toContain("secret-key");
  });

  test("a trusted comment mismatch fails and removes the signature", () => {
    const dir = makeTemp("parla-update-sign-");
    const dmg = join(dir, "Parla-1.2.3-macos-universal.dmg");
    writeFileSync(dmg, "dmg payload");
    const tools = minisignShim(dir, "parla 9.9.9 other.dmg");
    const app = makeUpdateApp(dir, "compiled in: public-key\n");
    const out = run(["mise-tasks/release/update-sign", "--", dmg, "1.2.3", app], {
      env: {
        ...tools.env,
        PARLA_UPDATE_SECRET_KEY: "secret-key",
        PARLA_UPDATE_PUBLIC_KEY: "public-key",
      },
    });
    expect(out.code).toBe(1);
    expect(out.stderr).toContain("trusted comment mismatch");
    expect(existsSync(`${dmg}.minisig`)).toBe(false);
  });
});
