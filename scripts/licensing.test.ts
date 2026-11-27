import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const root = join(import.meta.dir, "..");
const read = (path: string) => readFileSync(join(root, path), "utf8");

function filesUnder(dir: string): string[] {
  const out: string[] = [];
  const stack = [join(root, dir)];
  while (stack.length) {
    const current = stack.pop()!;
    for (const entry of readdirSync(current)) {
      const path = join(current, entry);
      if (statSync(path).isDirectory()) stack.push(path);
      else out.push(relative(root, path));
    }
  }
  return out;
}

// Every workspace manifest: root workspace members plus the guest plugin
// workspace, which has its own Cargo.toml.
function manifests(): { path: string; license: string }[] {
  const paths: string[] = ["Cargo.toml", "plugins/Cargo.toml"];
  for (const top of ["crates", "plugins/sdk-rust", "plugins/examples"]) {
    for (const file of filesUnder(top)) {
      if (file.endsWith("Cargo.toml")) paths.push(file);
    }
  }
  return [...new Set(paths)].map((path) => {
    const text = read(path);
    const direct = text.match(/^license\s*=\s*"([^"]+)"/m);
    const inherited = /^license\.workspace\s*=\s*true/m.test(text);
    return { path, license: direct?.[1] ?? (inherited ? "workspace" : "") };
  });
}

describe("licensing", () => {
  test("the repository licence texts are present and consistent", () => {
    const license = read("LICENSE");
    expect(license).toContain("Mozilla Public License Version 2.0");
    expect(read("LICENSE-APACHE")).toContain("Apache License");
    expect(read("LICENSE-MIT")).toContain("MIT License");
    expect(existsSync(join(root, "LICENSE-DOCS"))).toBe(true);
    expect(read("LICENSE-DOCS")).toContain("Creative Commons Attribution 4.0 International Public License");

    const notice = read("NOTICE");
    expect(notice).toContain("CC-BY-4.0");
    expect(notice).toContain("LICENSE-DOCS");
    expect(notice).toContain("Apache-2.0 OR MIT");

    const readme = read("README.md");
    expect(readme).toContain("LICENSE-DOCS");
    expect(readme).toContain("CC-BY-4.0");
    expect(readme).toContain("NOTICE");
  });

  test("NOTICE names every bundled asset group and the third-party code shipped in the app", () => {
    const notice = read("NOTICE").replace(/\s+/g, " ");
    for (const path of [
      "assets/brand",
      "assets/icons",
      "assets/themes",
      "assets/keymaps",
      "assets/settings",
      "assets/sounds",
      "site/public/favicon.svg",
      "site/public/shots",
      "crates/ui_testing/fonts",
    ]) {
      expect(notice).toContain(path);
    }
    for (const term of [
      "CC0-1.0",
      "SIL Open Font License",
      "OpenSSL",
      "AWS-LC",
      "SQLCipher",
      "libopus",
      "Unicode",
      "CDLA-Permissive-2.0",
    ]) {
      expect(notice).toContain(term);
    }
  });

  test("crate manifests agree with the licence map", () => {
    const rootLicense = read("Cargo.toml").match(/^license\s*=\s*"([^"]+)"/m)?.[1];
    const pluginLicense = read("plugins/Cargo.toml").match(/^license\s*=\s*"([^"]+)"/m)?.[1];
    expect(rootLicense).toBe("MPL-2.0");
    expect(pluginLicense).toBe("MPL-2.0");

    const permissive = new Set([
      "crates/parla_plugin_api/Cargo.toml",
      "plugins/sdk-rust/Cargo.toml",
      "plugins/examples/hello-command/Cargo.toml",
    ]);
    for (const { path, license } of manifests()) {
      if (path === "Cargo.toml") continue;
      const resolved = license === "workspace"
        ? path.startsWith("plugins/") && !path.startsWith("plugins/examples/hello-command")
          ? pluginLicense
          : rootLicense
        : license;
      expect(resolved, path).toBe(permissive.has(path) ? "Apache-2.0 OR MIT" : "MPL-2.0");
    }
  });

  test("bundled assets are original or have a licence next to them, and none is a Slack asset", () => {
    for (const file of filesUnder("assets").concat(filesUnder("site/public"))) {
      expect(file.toLowerCase()).not.toContain("slack");
      if (!/\.(svg|json|jsonc|md|txt)$/.test(file)) continue;
      expect(read(file).toLowerCase()).not.toContain("slack");
    }
    // The chime says how it is made and that it is CC0, next to the file.
    expect(read("assets/sounds/README.md")).toContain("CC0 1.0");
    expect(existsSync(join(root, "scripts/gen_chime.ts"))).toBe(true);
    // Every test font has its OFL text next to it.
    for (const font of readdirSync(join(root, "crates/ui_testing/fonts")).filter((f) => f.endsWith(".ttf"))) {
      const family = font.split("-")[0];
      const licenses = readdirSync(join(root, "crates/ui_testing/fonts")).filter((f) => /LICENSE|OFL/.test(f));
      expect(licenses.some((f) => f.toLowerCase().includes(family.toLowerCase())), font).toBe(true);
    }
    // Fixture images are generated, not third-party artwork.
    expect(read("crates/domain/src/test_support/fixture.rs")).toContain(
      "Everything is generated; nothing comes from a real workspace.",
    );
  });

  test("the bundled NOTICE starts with the project notice, then the crate licences", () => {
    const task = read("mise-tasks/release/notice");
    expect(task).toContain("cat NOTICE > \"$out\"");
    expect(task).toContain("cargo about generate about.hbs >> \"$out\"");
  });
});
