import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// Repo configuration and tooling: the mise pins, the nextest profiles and the
// hk hooks. Read from the source, as hk does not expose its resolved config
// offline.
const root = join(import.meta.dir, "..");
const read = (f: string) => readFileSync(join(root, f), "utf8");
const hk = read("hk.pkl");

describe("tooling pins", () => {
  const tools = (Bun.TOML.parse(read("mise.toml")) as any).tools as Record<string, string | { version: string }>;
  test("every mise tool has an exact version", () => {
    for (const [name, v] of Object.entries(tools)) {
      const version = typeof v === "string" ? v : v.version;
      expect(version, name).toMatch(/^\d+(\.\d+){1,3}$/);
    }
  });
  test("nextest, cargo-deny and cargo-about come from release binaries, not cargo builds", () => {
    for (const n of ["nextest", "cargo-deny", "cargo-about"]) {
      const key = Object.keys(tools).find((k) => k.includes(n));
      expect(key, n).toBeDefined();
      expect(key!.startsWith("cargo:"), n).toBe(false);
    }
  });
  test("every tool installs a release binary, never a source build", () => {
    for (const k of Object.keys(tools)) {
      if (["rust", "bun", "cargo-binstall"].includes(k)) continue;
      // npm: only for JavaScript tools with no binary release (installed with Bun).
      expect(k, k).toMatch(/^(aqua|github|cargo|npm):/);
    }
    // cargo: tools resolve through cargo-binstall's prebuilt binaries.
    if (Object.keys(tools).some((k) => k.startsWith("cargo:"))) {
      expect(Object.keys(tools)).toContain("cargo-binstall");
      const settings = (Bun.TOML.parse(read("mise.toml")) as any).settings.cargo;
      expect(settings).toMatchObject({ binstall_quickinstall: true, binstall_only: true });
    }
    if (Object.keys(tools).some((k) => k.startsWith("npm:"))) {
      expect((Bun.TOML.parse(read("mise.toml")) as any).settings.npm.package_manager).toBe("bun");
    }
  });
});

describe("nextest profiles", () => {
  const cfg = Bun.TOML.parse(read(".config/nextest.toml")) as any;
  test("a hung test is killed in every profile", () => {
    for (const p of ["default", "ci", "perf"]) {
      expect(cfg.profile[p]["slow-timeout"]["terminate-after"], p).toBeGreaterThan(0);
    }
  });
  test("ci reports every failure and perf runs one test at a time", () => {
    expect(cfg.profile.ci["fail-fast"]).toBe(false);
    expect(cfg.profile.perf["test-threads"]).toBe(1);
    expect(read("mise.toml")).toContain("--profile perf --run-ignored ignored-only");
    expect(read(".github/workflows/ci.yml")).toContain("mise run test -- --profile ci --locked");
  });
});

describe("hooks", () => {
  test("lint leaves clippy to its own gate; the pre-commit clippy stays", () => {
    expect(read("mise.toml")).toContain('"hk check --all --skip-step cargo_clippy"');
    expect(read("mise.toml")).toContain('"gitleaks git --redact --no-banner"');
    expect(hk).toContain('["cargo_clippy"] = (Builtins.cargo_clippy) { glob = List("**/*.rs"');
    expect(hk).toContain('["gitleaks"] = (Builtins.gitleaks) { scan = "staged" }');
  });
  test("shellcheck, jq, commit-msg and the denylist are wired", () => {
    for (const s of ["Builtins.shellcheck", "jq empty {{files}}", "SHELLCHECK_OPTS", '["commit-msg"]', "scripts/commit-msg.ts", "scripts/denylist-check.sh"]) {
      expect(hk, s).toContain(s);
    }
  });
  test("pre-push steps go through the ref filter, and no script disables hooks", () => {
    const pre = hk.slice(hk.indexOf('["pre-push"]'), hk.indexOf('["check"]'));
    expect(pre.match(/scripts\/push-refs\.ts/g)?.length).toBe(2);
    for (const f of ["scripts/push-refs.ts", "scripts/changes.ts"]) {
      expect(read(f), f).not.toMatch(/HK\s*:/);
    }
  });
});

const stepGlob = (name: string) => {
  const i = hk.indexOf(`["${name}"]`);
  expect(i, `${name} step exists`).toBeGreaterThan(-1);
  const line = hk.slice(i, hk.indexOf("\n", i + 1) + 400);
  const m = line.match(/glob\s*=\s*(List\([^)]*\)|"[^"]*")/);
  return m ? m[1] : "";
};

describe("hk globs", () => {
  test("Rust steps only see Rust inputs", () => {
    for (const s of ["cargo_fmt", "cargo_clippy"]) {
      const g = stepGlob(s);
      expect(g, s).toContain("**/*.rs");
      expect(g, s).toContain("Cargo.toml");
      expect(g, s).toContain("Cargo.lock");
      expect(g, s).not.toContain("site");
    }
  });
  test("taplo and actionlint have precise globs", () => {
    expect(stepGlob("taplo")).toContain("**/*.toml");
    expect(stepGlob("actionlint")).toContain(".github/workflows/");
  });
  test("site and translation steps only see their paths", () => {
    expect(stepGlob("site")).toContain("site/**");
    expect(stepGlob("site_translations")).toContain("i18n/**");
    expect(stepGlob("recon_check")).toContain("docs/providers/");
    expect(stepGlob("visual_baselines")).toContain("crates/ui_testing/baselines/**");
  });
  test("pre-push goes through the change mapping, not a blanket test run", () => {
    const pre = hk.slice(hk.indexOf('["pre-push"]'), hk.indexOf('["check"]'));
    expect(pre).toContain("bun scripts/changes.ts --base origin/main --head {sha} --run");
    expect(pre).not.toContain('check = "mise run test"');
  });
});
