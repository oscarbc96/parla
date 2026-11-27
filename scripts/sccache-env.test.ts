import { describe, expect, test } from "bun:test";
import { chmodSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "sccache-env.sh");

function wrapper(env: Record<string, string>, withSccache: boolean): string {
  const bin = mkdtempSync(join(tmpdir(), "sccache-"));
  if (withSccache) {
    writeFileSync(join(bin, "sccache"), "#!/bin/sh\n");
    chmodSync(join(bin, "sccache"), 0o755);
  }
  const r = Bun.spawnSync(["/bin/sh", "-c", `. "${script}"; printf %s "\${RUSTC_WRAPPER:-}"`], {
    env: { PATH: `${bin}:/usr/bin:/bin`, ...env },
  });
  return r.stdout.toString();
}

describe("sccache opt-in", () => {
  test("off by default", () => {
    expect(wrapper({}, true)).toBe("");
  });
  test("PARLA_SCCACHE=1 with sccache installed sets the wrapper", () => {
    expect(wrapper({ PARLA_SCCACHE: "1" }, true)).toBe("sccache");
  });
  test("PARLA_SCCACHE=1 without sccache does nothing", () => {
    expect(wrapper({ PARLA_SCCACHE: "1" }, false)).toBe("");
  });
  test("mise.toml sources the script and does not pin it on", () => {
    const t = Bun.file(join(import.meta.dir, "..", "mise.toml"));
    return t.text().then((s) => {
      expect(s).toContain("scripts/sccache-env.sh");
      expect(s).not.toMatch(/RUSTC_WRAPPER\s*=/);
    });
  });
});
